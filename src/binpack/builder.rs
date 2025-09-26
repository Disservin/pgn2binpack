use std::{
    fs::File,
    io::{BufReader, Read, Seek, Write},
    ops::ControlFlow,
    path::PathBuf,
};

use flate2::read::MultiGzDecoder;

use sfbinpack::{
    chess::{color::Color as SfColor, position::Position as SfPosition},
    CompressedTrainingDataEntryWriter, TrainingDataEntry,
};

use shakmaty::{fen::Fen, Chess, EnPassantMode, Move, Position};

use pgn_reader::{RawComment, RawTag, Reader, SanPlus, Skip, Visitor};

use crate::util::util;
use crate::wdl::wdl;

pub struct BinpackBuilder<T: Write + Read + Seek> {
    input: PathBuf,
    output: T,
}

impl<T: Write + Read + Seek> BinpackBuilder<T> {
    pub fn new<P: Into<PathBuf>>(input_pgn: P, output_file: T) -> Self {
        Self {
            input: input_pgn.into(),
            output: output_file,
        }
    }

    pub fn create_binpack(&mut self) {
        if self.input.extension().and_then(|s| s.to_str()) == Some("gz") {
            self.process_gz_pgn();
        } else {
            self.process_pgn();
        }
    }

    fn process_gz_pgn(&mut self) {
        let mut writer = CompressedTrainingDataEntryWriter::new(&mut self.output)
            .expect("creating binpack writer");

        let file = File::open(&self.input).expect(&format!("open {:?}", self.input));
        let decoder = MultiGzDecoder::new(file);
        let buf_reader = BufReader::new(decoder);
        let mut reader = Reader::new(buf_reader);
        let mut visitor = TrainingVisitor::new(&mut writer, self.input.clone());
        for res in reader.read_games(&mut visitor) {
            match res {
                Err(e) => panic!("{:?}", e),
                Ok(()) => {}
            }
        }
    }

    fn process_pgn(&mut self) {
        let mut writer = CompressedTrainingDataEntryWriter::new(&mut self.output)
            .expect("creating binpack writer");

        let file = File::open(&self.input).expect(&format!("open {:?}", self.input));
        let buf_reader = BufReader::new(file);
        let mut reader = Reader::new(buf_reader);
        let mut visitor = TrainingVisitor::new(&mut writer, self.input.clone());
        for res in reader.read_games(&mut visitor) {
            if let Err(e) = res {
                panic!("{:?}", e);
            }
        }
    }

    pub fn into_inner(self) -> std::io::Result<T> {
        Ok(self.output)
    }
}

// ---------------- Visitor & parsing logic ----------------

struct TrainingVisitor<'a, T: Write + Read + Seek> {
    writer: &'a mut CompressedTrainingDataEntryWriter<T>,
    start_fen: Option<String>,
    result: i16,
    chess: Chess,
    binpack_board: SfPosition,
    ply: u16,
    pending_entry: Option<TrainingDataEntry>,
    pending_score_set: bool,
    input: PathBuf,
    game_end_time: Option<String>,
}

impl<'a, T: Write + Read + Seek> TrainingVisitor<'a, T> {
    fn new(writer: &'a mut CompressedTrainingDataEntryWriter<T>, input: PathBuf) -> Self {
        Self {
            writer,
            start_fen: None,
            result: 0,
            chess: Chess::default(),
            binpack_board: SfPosition::default(),
            ply: 0,
            pending_entry: None,
            pending_score_set: false,
            input,
            game_end_time: None,
        }
    }

    fn reset_game(&mut self) {
        self.start_fen = None;
        self.result = 0;
        self.chess = Chess::default();
        self.binpack_board = SfPosition::default();

        self.ply = 0;
        self.pending_entry = None;
        self.pending_score_set = false;
    }

    fn apply_start_fen(&mut self) {
        if let Some(fen) = &self.start_fen {
            let f = shakmaty::fen::Fen::from_ascii(fen.as_bytes())
                .expect(&format!("Invalid FEN format: {}", fen));

            let pos = f
                .into_position(shakmaty::CastlingMode::Standard)
                .expect("Invalid chess position");

            self.chess = pos;
            self.binpack_board = SfPosition::from_fen(fen).unwrap();
        } else {
            self.chess = Chess::default();
            self.binpack_board = SfPosition::default();
        }
    }

    fn flush_pending(&mut self) {
        if let Some(entry) = self.pending_entry.take() {
            self.writer
                .write_entry(&entry)
                .expect("write pending entry");
        } else if self.pending_score_set {
            panic!("Pending score set but no pending entry");
        } else {
            panic!("No pending entry to flush");
        }

        self.pending_score_set = false;
    }

    fn handle_move(&mut self, mv: Move) {
        // Flush previous if it never received a comment with eval.
        // assert!(self.pending_entry.is_none());

        if !self.pending_entry.is_none() {
            println!(
                "Warning: pending entry without eval, input: {:?}",
                self.input.as_path()
            );
            println!("entry {:?}", self.pending_entry.as_ref().unwrap());
            println!("move {}", mv.to_uci(shakmaty::CastlingMode::Standard));
            println!("GameEndTime {:?}", self.game_end_time.as_ref().unwrap());
            panic!("Pending entry without eval");
        }

        // let fen = Fen::from_position(&self.chess.clone(), EnPassantMode::Legal);
        // keep track of position and play move instead
        // let sfpos = SfPosition::from_fen(&fen.to_string())
        //     .expect(&format!("SF position from fen error: {}", fen));

        // assert_eq!(sfpos.fen(), self.binpack_board.fen());
        // assert_eq!(sfpos, self.binpack_board);

        let sf_mv = util::convert_move(&mv, self.binpack_board.side_to_move());

        // if white stm and white won, result = 1
        // if black stm and black won, result = 1
        // if draw, result = 0
        // else result = -1

        let result = match (self.result, self.binpack_board.side_to_move()) {
            (0, _) => 0,
            (1, SfColor::White) | (-1, SfColor::Black) => 1,
            (1, SfColor::Black) | (-1, SfColor::White) => -1,
            _ => unreachable!(),
        };

        let entry = TrainingDataEntry {
            pos: self.binpack_board,
            mv: sf_mv,
            score: 0,                      // will update if a comment with eval follows
            ply: self.binpack_board.ply(), // todo: will use the ply from the fen tag if present, correct or wrong?
            result: result,
        };

        // assert_eq!(
        //     self.binpack_board.ply(),
        //     self.ply,
        //     "{:?}",
        //     self.binpack_board.fen()
        // );

        self.pending_entry = Some(entry);
        self.pending_score_set = false;

        self.chess.play_unchecked(mv);
        self.binpack_board.do_move(sf_mv);

        self.ply += 1;
    }

    fn attach_comment_eval(&mut self, comment: &str) {
        let cp = match util::parse_eval_cp(comment) {
            Ok(Some(v)) => v,
            Ok(None) => return, // known non-eval comment
            Err(_) => panic!("Failed to parse evaluation from comment: {}", comment),
        };

        let internal = wdl::external_cp_to_internal(cp as i32, &self.chess);

        let entry = self
            .pending_entry
            .as_mut()
            .expect("No pending entry available");

        entry.score = internal;
        self.pending_score_set = true;

        self.flush_pending();
    }
}

impl<'a, T: Write + Read + Seek> Visitor for TrainingVisitor<'a, T> {
    type Tags = ();
    type Movetext = ();
    type Output = ();

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
                        _ => panic!("Invalid result format: {}", v),
                    }
                }
                "Variant" => {
                    // panic for now later skip
                    panic!("Variant tag not supported");
                }
                "GameEndTime" => {
                    self.game_end_time = Some(v.to_string());
                    // ignore
                }
                _ => {}
            }
        }

        ControlFlow::Continue(())
    }

    fn begin_movetext(&mut self, _tags: Self::Tags) -> ControlFlow<Self::Output, Self::Movetext> {
        self.apply_start_fen();
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
                self.handle_move(mv);
            }
            Err(e) => {
                // Invalid move in current position: skip rest of game.
                panic!(
                    "parsing SAN to move failed: {:?}, san: {:?}, fen: {:?}",
                    e,
                    san_plus.san,
                    Fen::from_position(&self.chess, EnPassantMode::Legal).to_string()
                );
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
        } else {
            panic!("Invalid UTF-8 in comment");
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
        // self.flush_pending();
    }
}
