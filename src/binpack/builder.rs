use std::{
    fs::File,
    io::{BufReader, Seek, Write},
    ops::ControlFlow,
    path::PathBuf,
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

pub struct BinpackBuilder<T: Write + Seek> {
    input: PathBuf,
    output: T,
    total_pos: u64,
}

impl<T: Write + Seek> BinpackBuilder<T> {
    pub fn new<P: Into<PathBuf>>(input_pgn: P, output_file: T) -> Self {
        Self {
            input: input_pgn.into(),
            output: output_file,
            total_pos: 0,
        }
    }

    pub fn create_binpack(&mut self) -> Result<()> {
        let reader_input = self.get_reader()?;
        let buf_reader = BufReader::new(reader_input);
        let mut reader = Reader::new(buf_reader);

        let mut writer = CompressedTrainingDataEntryWriter::new(&mut self.output)
            .context("creating binpack writer")?;
        let mut visitor = TrainingVisitor::new(&mut writer);

        for res in reader.read_games(&mut visitor) {
            let game_result = res.with_context(|| format!("reading PGN game: {:?}", self.input))?;
            let moves = game_result.context("processing game moves")?;
            self.total_pos += moves as u64;
        }
        Ok(())
    }

    fn get_reader(&self) -> Result<Box<dyn std::io::Read>> {
        let reader_input: Box<dyn std::io::Read> =
            if self.input.extension().and_then(|s| s.to_str()) == Some("gz") {
                let file = File::open(&self.input)
                    .with_context(|| format!("opening gz file {:?}", self.input))?;
                Box::new(MultiGzDecoder::new(file))
            } else {
                let file = File::open(&self.input)
                    .with_context(|| format!("opening file {:?}", self.input))?;
                Box::new(file)
            };

        Ok(reader_input)
    }

    pub fn into_inner(self) -> std::io::Result<T> {
        Ok(self.output)
    }

    pub fn total_positions(&self) -> u64 {
        self.total_pos
    }
}

// ---------------- Visitor & parsing logic ----------------

struct TrainingVisitor<'a, T: Write + Seek> {
    writer: &'a mut CompressedTrainingDataEntryWriter<T>,
    // todo: could apply directly
    start_fen: Option<String>,
    // game result from the PGN tags: 1 = white win, -1 = black win, 0 = draw/unknown
    result: i16,
    // shakmaty crate representation of the board
    chess: Chess,
    // binpack crate representation of the board
    binpack_board: SfPosition,
    pending_entry: Option<TrainingDataEntry>,
    pending_score_set: bool,
    game_end_time: Option<String>,
    // number of moves processed per game
    moves: u32,
}

impl<'a, T: Write + Seek> TrainingVisitor<'a, T> {
    fn new(writer: &'a mut CompressedTrainingDataEntryWriter<T>) -> Self {
        Self {
            writer,
            start_fen: None,
            result: 0,
            chess: Chess::default(),
            binpack_board: SfPosition::default(),
            pending_entry: None,
            pending_score_set: false,
            game_end_time: None,
            moves: 0,
        }
    }

    fn reset_game(&mut self) {
        self.start_fen = None;
        self.result = 0;
        self.chess = Chess::default();
        self.binpack_board = SfPosition::default();
        self.moves = 0;
        self.pending_entry = None;
        self.pending_score_set = false;
    }

    fn apply_start_fen(&mut self) -> Result<()> {
        if let Some(fen) = &self.start_fen {
            let f = shakmaty::fen::Fen::from_ascii(fen.as_bytes())
                .with_context(|| format!("parsing FEN: {}", fen))?;

            let pos = f
                .into_position(shakmaty::CastlingMode::Standard)
                .with_context(|| format!("creating position from FEN: {}", fen))?;

            self.chess = pos;
            self.binpack_board = SfPosition::from_fen(fen).unwrap();
        } else {
            self.chess = Chess::default();
            self.binpack_board = SfPosition::default();
        }
        Ok(())
    }

    fn flush_pending(&mut self) -> Result<()> {
        if let Some(entry) = self.pending_entry.take() {
            self.writer
                .write_entry(&entry)
                .context("writing entry to binpack")?;
        } else if self.pending_score_set {
            anyhow::bail!("pending score set but no pending entry");
        } else {
            anyhow::bail!("no pending entry to flush");
        }

        self.pending_score_set = false;
        Ok(())
    }

    fn handle_move(&mut self, mv: Move) -> Result<()> {
        self.moves += 1;

        assert!(self.pending_entry.is_none());

        let sf_mv = util::convert_move(&mv, self.binpack_board.side_to_move());

        let result = match (self.result, self.binpack_board.side_to_move()) {
            (0, _) => 0,
            (1, SfColor::White) | (-1, SfColor::Black) => 1,
            (1, SfColor::Black) | (-1, SfColor::White) => -1,
            _ => anyhow::bail!("invalid result/color combination: {}", self.result),
        };

        let entry = TrainingDataEntry {
            pos: self.binpack_board,
            mv: sf_mv,
            score: 0,                      // will update if a comment with eval follows
            ply: self.binpack_board.ply(), // will use the ply from the fen tag if present
            result,
        };

        self.pending_entry = Some(entry);
        self.pending_score_set = false;

        self.chess.play_unchecked(mv);
        self.binpack_board.do_move(sf_mv);

        Ok(())
    }

    fn attach_comment_eval(&mut self, comment: &str) -> Result<()> {
        let cp = match util::parse_eval_cp(comment) {
            Ok(Some(v)) => v,
            Ok(None) => return Ok(()), // known non-eval comment
            Err(_) => anyhow::bail!("failed to parse evaluation from comment: {}", comment),
        };

        let internal = wdl::external_cp_to_internal(cp as i32, &self.chess);

        let entry = self
            .pending_entry
            .as_mut()
            .context("no pending entry available")?;

        entry.score = internal;
        self.pending_score_set = true;

        self.flush_pending()
    }
}

impl<'a, T: Write + Seek> Visitor for TrainingVisitor<'a, T> {
    type Tags = ();
    type Movetext = ();
    type Output = Result<u32>; // number of moves processed per game

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
        let n = match std::str::from_utf8(name) {
            Ok(val) => val,
            Err(e) => {
                return ControlFlow::Break(Err(anyhow::anyhow!(
                    "invalid UTF-8 in tag name: {:?}, error: {}",
                    name,
                    e
                )))
            }
        };

        let v = match std::str::from_utf8(value.0) {
            Ok(val) => val,
            Err(e) => {
                return ControlFlow::Break(Err(anyhow::anyhow!(
                    "invalid UTF-8 in tag value: {:?}, error: {}",
                    value.0,
                    e
                )))
            }
        };

        match n {
            "FEN" => self.start_fen = Some(v.to_string()),
            "Result" => {
                self.result = match v {
                    "1-0" => 1,
                    "0-1" => -1,
                    "1/2-1/2" | "*" => 0,
                    _ => {
                        return ControlFlow::Break(Err(anyhow::anyhow!(
                            "invalid result format: {}",
                            v
                        )))
                    }
                }
            }
            "Variant" => {
                return ControlFlow::Break(Err(anyhow::anyhow!("variant tag not supported")));
            }
            "GameEndTime" => {
                self.game_end_time = Some(v.to_string());
            }
            _ => {}
        }

        ControlFlow::Continue(())
    }

    fn begin_movetext(&mut self, _tags: Self::Tags) -> ControlFlow<Self::Output, Self::Movetext> {
        if let Err(e) = self.apply_start_fen() {
            return ControlFlow::Break(Err(e));
        }
        ControlFlow::Continue(())
    }

    fn san(
        &mut self,
        _movetext: &mut Self::Movetext,
        san_plus: SanPlus,
    ) -> ControlFlow<Self::Output> {
        match san_plus.san.to_move(&self.chess) {
            Ok(mv) => {
                if let Err(e) = self.handle_move(mv) {
                    return ControlFlow::Break(Err(e));
                }
            }
            Err(e) => {
                let fen = Fen::from_position(&self.chess, EnPassantMode::Legal).to_string();
                return ControlFlow::Break(Err(anyhow::anyhow!(
                    "parsing SAN to move failed: {:?}, san: {:?}, fen: {:?}",
                    e,
                    san_plus.san,
                    fen
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
        let c = match std::str::from_utf8(comment.0) {
            Ok(val) => val,
            Err(e) => return ControlFlow::Break(Err(anyhow::anyhow!(e))),
        };

        if let Err(e) = self.attach_comment_eval(c) {
            return ControlFlow::Break(Err(e));
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
        Ok(self.moves)
    }
}
