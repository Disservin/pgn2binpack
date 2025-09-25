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

use shakmaty::{Chess, EnPassantMode, Move, Position};

use pgn_reader::{RawComment, RawTag, Reader, SanPlus, Skip, Visitor};

use crate::util::util;
use crate::wdl::wdl;

const STARTPOS: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

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
            if let Err(e) = res.with_context(|| format!("reading game in {:?}", path)) {
                return Err(e);
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
            if let Err(e) = res.with_context(|| format!("reading game in {:?}", path)) {
                return Err(e);
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
    sf_pos: SfPosition,
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
            sf_pos: SfPosition::from_fen(STARTPOS).unwrap(),
            ply: 0,
            pending_entry: None,
            pending_score_set: false,
        }
    }

    fn reset_game(&mut self) {
        self.start_fen = None;
        self.result = 0;
        self.chess = Chess::default();
        self.sf_pos = SfPosition::from_fen(STARTPOS).unwrap();

        self.ply = 0;
        self.pending_entry = None;
        self.pending_score_set = false;
    }

    fn apply_start_fen(&mut self) -> Result<()> {
        if let Some(fen) = &self.start_fen {
            if let Ok(f) = shakmaty::fen::Fen::from_ascii(fen.as_bytes()) {
                if let Ok(pos) = f.into_position(shakmaty::CastlingMode::Standard) {
                    self.chess = pos;

                    match SfPosition::from_fen(fen) {
                        Ok(p) => self.sf_pos = p,
                        Err(e) => Err(anyhow::anyhow!("SF position from fen error: {:?}", e))?,
                    }

                    return Ok(());
                }
            }
        }
        self.chess = Chess::default();
        self.sf_pos = SfPosition::from_fen(STARTPOS).unwrap();
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

        let sf_mv = util::convert_move(&mv, self.sf_pos.side_to_move());

        // if white stm and white won, result = 1
        // if black stm and black won, result = 1
        // if draw, result = 0
        // else result = -1
        let result = if self.result == 0 {
            0
        } else {
            let stm = self.sf_pos.side_to_move();
            if (stm == SfColor::White && self.result == 1)
                || (stm == SfColor::Black && self.result == -1)
            {
                1
            } else {
                -1
            }
        };

        let entry = TrainingDataEntry {
            pos: self.sf_pos.clone(),
            mv: sf_mv,
            score: 0, // will update if a comment with eval follows
            ply: self.ply,
            result: result,
        };
        self.pending_entry = Some(entry);
        self.pending_score_set = false;

        self.chess.play_unchecked(mv);
        let fen_after =
            shakmaty::fen::Fen::from_position(&self.chess.clone(), EnPassantMode::Legal);

        match SfPosition::from_fen(&fen_after.to_string()) {
            Ok(p) => self.sf_pos = p,
            Err(e) => Err(anyhow::anyhow!("SF position from fen error: {:?}", e))?,
        }

        self.ply += 1;
        Ok(())
    }

    fn attach_comment_eval(&mut self, comment: &str) {
        if let Some(cp) = util::parse_eval_cp(comment) {
            let internal = wdl::external_cp_to_internal(cp as i32, &self.chess);
            if let Some(entry) = self.pending_entry.as_mut() {
                entry.score = internal;
                self.pending_score_set = true;
            }
        }
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
                    "parsing SAN to move failed: {:?}",
                    e
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
            self.attach_comment_eval(c);
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
