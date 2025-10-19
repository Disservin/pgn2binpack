use std::{
    fs::File,
    io::{BufReader, Seek, Write},
    ops::ControlFlow,
    path::PathBuf,
};

use anyhow::{bail, Context, Result};
use flate2::read::MultiGzDecoder;

use sfbinpack::{
    chess::{color::Color as SfColor, position::Position as SfPosition},
    CompressedTrainingDataEntryWriter, TrainingDataEntry,
};

use shakmaty::{fen::Fen, Chess, EnPassantMode, Move, Position};

use pgn_reader::{RawComment, RawTag, Reader, SanPlus, Skip, Visitor};

use viriformat::{
    chess::{
        board::{Board as ViriBoard, DrawType, GameOutcome, WinType},
        chessmove::Move as ViriMove,
        CHESS960,
    },
    dataformat::Game as ViriGame,
};

use crate::cli::Backend;
use crate::util::util;
use crate::wdl::wdl;

pub struct BinpackBuilder<T: Write + Seek> {
    input: PathBuf,
    output: T,
    total_pos: u64,
    backend: Backend,
}

impl<T: Write + Seek> BinpackBuilder<T> {
    pub fn new<P: Into<PathBuf>>(input_pgn: P, output_file: T, backend: Backend) -> Self {
        Self {
            input: input_pgn.into(),
            output: output_file,
            total_pos: 0,
            backend,
        }
    }

    pub fn create_binpack(&mut self) -> Result<()> {
        let reader_input = self.get_reader()?;
        let buf_reader = BufReader::new(reader_input);
        let mut reader = Reader::new(buf_reader);

        match self.backend {
            Backend::Sfbinpack => {
                let mut writer = CompressedTrainingDataEntryWriter::new(&mut self.output)
                    .context("creating binpack writer")?;
                let mut visitor = SfVisitor::new(&mut writer);

                for res in reader.read_games(&mut visitor) {
                    let game_result =
                        res.with_context(|| format!("reading PGN game: {:?}", self.input))?;
                    let moves = game_result.context("processing game moves")?;
                    self.total_pos += moves as u64;
                }
            }
            Backend::Viriformat => {
                let mut visitor = ViriformatVisitor::new(&mut self.output);
                for res in reader.read_games(&mut visitor) {
                    let game_result =
                        res.with_context(|| format!("reading PGN game: {:?}", self.input))?;
                    let moves = game_result.context("processing game moves")?;
                    self.total_pos += moves as u64;
                }
            }
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

struct SfVisitor<'a, T: Write + Seek> {
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

impl<'a, T: Write + Seek> SfVisitor<'a, T> {
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

impl<'a, T: Write + Seek> Visitor for SfVisitor<'a, T> {
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

struct ViriformatVisitor<'a, T: Write + Seek> {
    writer: &'a mut T,
    start_fen: Option<String>,
    result: Option<GameOutcome>,
    chess: Chess,
    viri_board: ViriBoard,
    game: Option<ViriGame>,
    pending_move: Option<ViriMove>,
    pending_eval_set: bool,
    moves: u32,
}

impl<'a, T: Write + Seek> ViriformatVisitor<'a, T> {
    fn new(writer: &'a mut T) -> Self {
        Self {
            writer,
            start_fen: None,
            result: None,
            chess: Chess::default(),
            viri_board: ViriBoard::default(),
            game: None,
            pending_move: None,
            pending_eval_set: false,
            moves: 0,
        }
    }

    fn reset_game(&mut self) {
        self.start_fen = None;
        self.result = None;
        self.chess = Chess::default();
        self.viri_board = ViriBoard::default();
        self.game = None;
        self.pending_move = None;
        self.pending_eval_set = false;
        self.moves = 0;
    }

    fn apply_start_fen(&mut self) -> Result<()> {
        if let Some(fen) = &self.start_fen {
            let f = shakmaty::fen::Fen::from_ascii(fen.as_bytes())
                .with_context(|| format!("parsing FEN: {}", fen))?;
            let pos = f
                .into_position(shakmaty::CastlingMode::Standard)
                .with_context(|| format!("creating position from FEN: {}", fen))?;
            self.chess = pos;

            let mut board = ViriBoard::new();
            board
                .set_from_fen(fen)
                .with_context(|| format!("creating viriformat board from FEN: {}", fen))?;
            self.viri_board = board;
        } else {
            self.chess = Chess::default();
            self.viri_board = ViriBoard::default();
        }

        let mut game = ViriGame::new(&self.viri_board);
        if let Some(result) = self.result {
            game.set_outcome(result);
        }
        self.game = Some(game);

        Ok(())
    }

    fn flush_pending(&mut self, eval: i16) -> Result<()> {
        let mv = self
            .pending_move
            .take()
            .context("no pending move available")?;

        if !self.pending_eval_set {
            bail!("pending evaluation was not set before flush");
        }

        let game = self
            .game
            .as_mut()
            .context("game state not initialised before writing")?;
        game.add_move(mv, eval);

        self.pending_eval_set = false;
        Ok(())
    }

    fn handle_move(&mut self, mv: Move) -> Result<()> {
        self.moves += 1;
        if self.pending_move.is_some() {
            bail!("previous move is still pending evaluation");
        }

        let viri_move = util::convert_move_viriformat(&mv)?;
        self.pending_move = Some(viri_move);
        self.pending_eval_set = false;

        self.chess.play_unchecked(mv);
        if !self.viri_board.make_move_simple(viri_move) {
            bail!("failed to apply move on viriformat board");
        }

        Ok(())
    }

    fn attach_comment_eval(&mut self, comment: &str) -> Result<()> {
        let cp = match util::parse_eval_cp(comment) {
            Ok(Some(v)) => v,
            Ok(None) => return Ok(()),
            Err(_) => bail!("failed to parse evaluation from comment: {}", comment),
        };

        self.pending_eval_set = true;
        self.flush_pending(cp)
    }
}

impl<'a, T: Write + Seek> Visitor for ViriformatVisitor<'a, T> {
    type Tags = ();
    type Movetext = ();
    type Output = Result<u32>;

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
                    "1-0" => Some(GameOutcome::WhiteWin(WinType::Adjudication)),
                    "0-1" => Some(GameOutcome::BlackWin(WinType::Adjudication)),
                    "1/2-1/2" => Some(GameOutcome::Draw(DrawType::Adjudication)),
                    "*" => None,
                    _ => {
                        return ControlFlow::Break(Err(anyhow::anyhow!(
                            "invalid result format: {}",
                            v
                        )))
                    }
                };
            }
            "Variant" => {
                CHESS960.store(true, std::sync::atomic::Ordering::SeqCst);
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
        ControlFlow::Continue(Skip(true))
    }

    fn end_game(&mut self, _movetext: Self::Movetext) -> Self::Output {
        if self.pending_move.is_some() {
            return Err(anyhow::anyhow!(
                "pending move without evaluation at end of game"
            ));
        }

        let mut game = self
            .game
            .take()
            .context("missing game state when finishing viriformat output")?;
        game.serialise_into(self.writer)
            .context("writing viriformat game")?;

        Ok(self.moves)
    }
}
