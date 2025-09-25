use std::{
    fs::File,
    io::BufReader,
    ops::ControlFlow,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use flate2::read::MultiGzDecoder;

use sfbinpack::{
    chess::{color::Color as SfColor, position::Position as SfPosition},
    CompressedTrainingDataEntryWriter, TrainingDataEntry,
};

use shakmaty::{fen::Fen, Chess, EnPassantMode, Move, Position};

use pgn_reader::{RawComment, RawTag, Reader, SanPlus, Skip, Visitor};

use crate::util::util;
use crate::wdl::wdl;

pub struct BinpackBuilder {
    input: PathBuf,
    output: PathBuf,
}

impl BinpackBuilder {
    pub fn new<P: Into<PathBuf>, Q: Into<PathBuf>>(input_pgn: P, output_binpack: Q) -> Self {
        Self {
            input: input_pgn.into(),
            output: output_binpack.into(),
        }
    }

    pub fn create_binpack(&self) -> Result<()> {
        let mut writer =
            CompressedTrainingDataEntryWriter::new(self.output.to_str().unwrap(), true)
                .context("creating binpack writer")?;

        if self.input.extension().and_then(|s| s.to_str()) == Some("gz") {
            self.process_gz_pgn(&self.input, &mut writer)?;
        } else {
            self.process_pgn(&self.input, &mut writer)?;
        }

        Ok(())
    }

    fn process_gz_pgn(
        &self,
        path: &Path,
        writer: &mut CompressedTrainingDataEntryWriter,
    ) -> Result<()> {
        let file = File::open(path).with_context(|| format!("open {:?}", path))?;
        let decoder = MultiGzDecoder::new(file);
        let buf_reader = BufReader::new(decoder);

        let mut reader = Reader::new(buf_reader);
        let mut visitor = TrainingVisitor::new(writer);

        for res in reader.read_games(&mut visitor) {
            match res {
                Ok(Ok(())) => {}                // Success
                Ok(Err(e)) => return Err(e),    // Game processing error
                Err(e) => return Err(e.into()), // Reader error
            }
        }

        Ok(())
    }

    fn process_pgn(
        &self,
        path: &Path,
        writer: &mut CompressedTrainingDataEntryWriter,
    ) -> Result<()> {
        let file = File::open(path).with_context(|| format!("open {:?}", path))?;
        let buf_reader = BufReader::new(file);

        let mut reader = Reader::new(buf_reader);
        let mut visitor = TrainingVisitor::new(writer);

        for res in reader.read_games(&mut visitor) {
            if let Err(e) = res {
                return Err(e.into());
            }
        }

        Ok(())
    }
}

// ---------------- Visitor & parsing logic ----------------

struct TrainingVisitor<'a> {
    writer: &'a mut CompressedTrainingDataEntryWriter,
    start_fen: Option<String>,
    result: i16,
    chess: Chess,
    ply: u16,
    pending_entry: Option<TrainingDataEntry>,
    pending_score_set: bool,
}

impl<'a> TrainingVisitor<'a> {
    fn new(writer: &'a mut CompressedTrainingDataEntryWriter) -> Self {
        Self {
            writer,
            start_fen: None,
            result: 0,
            chess: Chess::default(),
            ply: 0,
            pending_entry: None,
            pending_score_set: false,
        }
    }

    fn reset_game(&mut self) {
        self.start_fen = None;
        self.result = 0;
        self.chess = Chess::default();

        self.ply = 0;
        self.pending_entry = None;
        self.pending_score_set = false;
    }

    fn apply_start_fen(&mut self) -> Result<()> {
        if let Some(fen) = &self.start_fen {
            let f = shakmaty::fen::Fen::from_ascii(fen.as_bytes())
                .map_err(|e| anyhow::anyhow!("Invalid FEN format: {}", e))?;

            let pos = f
                .into_position(shakmaty::CastlingMode::Standard)
                .map_err(|e| anyhow::anyhow!("Invalid chess position: {}", e))?;

            self.chess = pos;
        } else {
            self.chess = Chess::default();
        }

        Ok(())
    }

    fn flush_pending(&mut self) -> Result<()> {
        if let Some(entry) = self.pending_entry.take() {
            self.writer
                .write_entry(&entry)
                .context("write pending entry")?;
        }
        self.pending_score_set = false;
        Ok(())
    }

    fn handle_move(&mut self, mv: Move) -> Result<()> {
        // Flush previous if it never received a comment with eval.
        self.flush_pending()?;

        let fen = Fen::from_position(&self.chess.clone(), EnPassantMode::Legal);
        // keep track of position and play move instead
        let sfpos = SfPosition::from_fen(&fen.to_string())
            .map_err(|e| anyhow::anyhow!("SF position from fen error: {:?}", e))?;

        let sf_mv = util::convert_move(&mv, sfpos.side_to_move());

        // if white stm and white won, result = 1
        // if black stm and black won, result = 1
        // if draw, result = 0
        // else result = -1

        let result = match (self.result, sfpos.side_to_move()) {
            (0, _) => 0,
            (1, SfColor::White) | (-1, SfColor::Black) => 1,
            (1, SfColor::Black) | (-1, SfColor::White) => -1,
            _ => unreachable!(),
        };

        let entry = TrainingDataEntry {
            pos: sfpos,
            mv: sf_mv,
            score: 0, // will update if a comment with eval follows
            ply: self.ply,
            result: result,
        };

        self.pending_entry = Some(entry);
        self.pending_score_set = false;

        self.chess.play_unchecked(mv);

        self.ply += 1;
        Ok(())
    }

    fn attach_comment_eval(&mut self, comment: &str) -> Result<()> {
        let cp = match util::parse_eval_cp(comment) {
            Ok(Some(v)) => v,
            Ok(None) => return Ok(()), // known non-eval comment
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "Failed to parse evaluation from comment: {}",
                    comment
                ))
            }
        };

        let internal = wdl::external_cp_to_internal(cp as i32, &self.chess);

        let entry = self
            .pending_entry
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("No pending entry available"))?;

        entry.score = internal;
        self.pending_score_set = true;

        Ok(())
    }
}

impl<'a> Visitor for TrainingVisitor<'a> {
    type Tags = ();
    type Movetext = ();
    type Output = Result<(), anyhow::Error>;

    fn begin_tags(&mut self) -> ControlFlow<Self::Output, Self::Tags> {
        self.reset_game();
        ControlFlow::Continue(())
    }

    fn tag(
        &mut self,
        _tags: &mut Self::Tags,
        name: &[u8],
        value: RawTag<'_>,
    ) -> ControlFlow<Self::Output> {
        if let (Ok(n), Ok(v)) = (std::str::from_utf8(name), std::str::from_utf8(value.0)) {
            match n {
                "FEN" => self.start_fen = Some(v.to_string()),
                "Result" => {
                    self.result = match v {
                        "1-0" => 1,
                        "0-1" => -1,
                        "1/2-1/2" | "*" => 0,
                        _ => {
                            return ControlFlow::Break(Err(anyhow::anyhow!(
                                "Invalid result format"
                            )))
                        }
                    }
                }
                _ => {}
            }
        }

        ControlFlow::Continue(())
    }

    fn begin_movetext(&mut self, _tags: Self::Tags) -> ControlFlow<Self::Output, Self::Movetext> {
        if let Err(e) = self.apply_start_fen() {
            return ControlFlow::Break(Err(anyhow::anyhow!("applying start FEN failed: {:?}", e)));
        }

        ControlFlow::Continue(())
    }

    fn san(
        &mut self,
        _movetext: &mut Self::Movetext,
        san_plus: SanPlus,
    ) -> ControlFlow<Self::Output> {
        // SanPlus has .san giving the SAN which we convert using current position.
        match san_plus.san.to_move(&self.chess) {
            Ok(mv) => {
                if let Err(e) = self.handle_move(mv) {
                    return ControlFlow::Break(Err(anyhow::anyhow!(
                        "handling move failed: {:?}",
                        e
                    )));
                }
            }
            Err(e) => {
                // Invalid move in current position: skip rest of game.
                return ControlFlow::Break(Err(anyhow::anyhow!(
                    "parsing SAN to move failed: {:?}, san: {:?}, fen: {:?}",
                    e,
                    san_plus.san,
                    Fen::from_position(&self.chess, EnPassantMode::Legal).to_string()
                )));
            }
        }
        ControlFlow::Continue(())
    }

    fn comment(
        &mut self,
        _movetext: &mut Self::Movetext,
        comment: RawComment<'_>,
    ) -> ControlFlow<Self::Output> {
        if let Ok(c) = std::str::from_utf8(comment.0) {
            if let Err(e) = self.attach_comment_eval(c) {
                return ControlFlow::Break(Err(anyhow::anyhow!(e)));
            }
        }
        ControlFlow::Continue(())
    }

    fn begin_variation(
        &mut self,
        _movetext: &mut Self::Movetext,
    ) -> ControlFlow<Self::Output, Skip> {
        ControlFlow::Continue(Skip(true)) // stay in the mainline
    }

    fn end_game(&mut self, _movetext: Self::Movetext) -> Self::Output {
        if let Err(e) = self.flush_pending() {
            return Err(e);
        }
        Ok(())
    }
}
