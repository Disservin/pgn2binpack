use std::{
    fs::File,
    io::BufReader,
    ops::ControlFlow,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use flate2::read::MultiGzDecoder;
use walkdir::WalkDir;

use sfbinpack::{
    chess::{
        color::Color as SfColor,
        coords::Square as SfSquare,
        piece::Piece as SfPiece,
        piecetype::PieceType as SfPieceType,
        position::Position as SfPosition,
        r#move::{Move as SfMove, MoveType as SfMoveType},
    },
    CompressedTrainingDataEntryWriter, TrainingDataEntry,
};

use shakmaty::{Chess, EnPassantMode, Move, Position, Role, Square};

use pgn_reader::{RawComment, RawTag, Reader, SanPlus, Skip, Visitor};

use crate::wdl::wdl;

pub struct BinpackBuilder {
    root: PathBuf,
    output: PathBuf,
}

impl BinpackBuilder {
    pub fn new<P: Into<PathBuf>, Q: Into<PathBuf>>(pgn_root: P, output_binpack: Q) -> Self {
        Self {
            root: pgn_root.into(),
            output: output_binpack.into(),
        }
    }

    pub fn create_binpack(&self) -> Result<()> {
        let mut writer =
            CompressedTrainingDataEntryWriter::new(self.output.to_str().unwrap(), false)
                .context("creating binpack writer")?;

        let files = self.collect_pgn_gz_files()?;
        let total = files.len();
        for (i, gz_file) in files.iter().enumerate() {
            let filesize = std::fs::metadata(gz_file)
                .with_context(|| format!("getting metadata for {:?}", gz_file))?
                .len();
            println!(
                "Processing {:?} ({}/{}) - {}",
                gz_file,
                i + 1,
                total,
                human_bytes::human_bytes(filesize as f64)
            );
            let t0 = std::time::Instant::now();
            self.process_gz_pgn(&gz_file, &mut writer)
                .with_context(|| format!("processing file {:?}", gz_file))?;
            let elapsed = t0.elapsed();
            println!("  done in {:.2?}", elapsed);
        }
        Ok(())
    }

    fn collect_pgn_gz_files(&self) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        for entry in WalkDir::new(&self.root).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let p = entry.path();
            if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
                if ext.eq_ignore_ascii_case("gz") {
                    if let Some(stem_ext) = p.file_stem().and_then(|s| s.to_str()) {
                        if stem_ext.ends_with(".pgn")
                            || stem_ext.to_ascii_lowercase().contains("pgn")
                        {
                            out.push(p.to_path_buf());
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    fn process_gz_pgn(
        &self,
        path: &Path,
        writer: &mut CompressedTrainingDataEntryWriter,
    ) -> Result<()> {
        let file = File::open(path).with_context(|| format!("open {:?}", path))?;
        let decoder = MultiGzDecoder::new(file);
        let buf_reader = BufReader::new(decoder);

        // pgn_reader::Reader works on any BufRead
        let mut reader = Reader::new(buf_reader);

        let mut visitor = TrainingVisitor::new(writer);
        let out = reader.read_games(&mut visitor);

        // iterate oer all games
        for res in out {
            res.with_context(|| format!("reading game in {:?}", path))?;
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
            sf_pos: SfPosition::from_fen(
                "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            ),
            ply: 0,
            pending_entry: None,
            pending_score_set: false,
        }
    }

    fn reset_game(&mut self) {
        self.start_fen = None;
        self.result = 0;
        self.chess = Chess::default();
        self.sf_pos =
            SfPosition::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        self.ply = 0;
        self.pending_entry = None;
        self.pending_score_set = false;
    }

    fn apply_start_fen(&mut self) {
        if let Some(fen) = &self.start_fen {
            if let Ok(f) = shakmaty::fen::Fen::from_ascii(fen.as_bytes()) {
                if let Ok(pos) = f.into_position(shakmaty::CastlingMode::Standard) {
                    self.chess = pos;
                    self.sf_pos = SfPosition::from_fen(fen);
                    return;
                }
            }
        }
        self.chess = Chess::default();
        self.sf_pos = SfPosition::from_fen("startpos");
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

        let sf_mv = convert_move(&mv, self.sf_pos.side_to_move());

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
        self.sf_pos = SfPosition::from_fen(&fen_after.to_string());
        self.ply += 1;
        Ok(())
    }

    fn attach_comment_eval(&mut self, comment: &str) {
        if let Some(cp) = parse_eval_cp(comment) {
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
                        _ => 0,
                    }
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
                if let Err(e) = self.handle_move(mv) {
                    eprintln!("move handling error: {e}");
                }
            }
            Err(_) => { /* skip invalid */ }
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
        // Ignore side variations: only take mainline.
        ControlFlow::Continue(Skip(true))
    }

    fn end_game(&mut self, _movetext: Self::Movetext) -> Self::Output {
        if let Err(e) = self.flush_pending() {
            eprintln!("flush error: {e}");
        }
    }
}

// --------------- Parsing helpers ------------------

fn parse_eval_cp(comment: &str) -> Option<i16> {
    // Matches examples like:
    // {+1.01/26 1.2s} {-0.34/15} {+0.00} {-M21/32 0.5s} {+M21/32 0.5s}
    for part in comment.split(|c: char| c.is_whitespace() || c == '{' || c == '}') {
        if part.is_empty() {
            continue;
        }
        let p = part.trim_matches(|c| c == '{' || c == '}');

        // mate
        if p.starts_with("+M") || p.starts_with("-M") {
            let sign = if p.starts_with("+M") { 1 } else { -1 };
            if let Ok(_) = p[2..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<i32>()
            {
                return Some(32000 as i16 * sign);
            }
        } else {
            let num = p.split('/').next().unwrap_or(p);
            if let Some(first) = num.chars().next() {
                if first == '+' || first == '-' || first.is_ascii_digit() {
                    let mut cleaned = String::new();
                    for ch in num.chars() {
                        if ch.is_ascii_digit() || ch == '+' || ch == '-' || ch == '.' {
                            cleaned.push(ch);
                        } else {
                            break;
                        }
                    }
                    if cleaned == "+" || cleaned == "-" {
                        continue;
                    }
                    if let Ok(f) = cleaned.parse::<f32>() {
                        return Some((f * 100.0).round() as i16);
                    }
                }
            }
        }
    }
    None
}

// --------------- Move conversion helpers ------------------

fn convert_move(mv: &Move, color: SfColor) -> SfMove {
    let from_idx = square_index(mv.from().unwrap());
    let to_idx = square_index(mv.to());

    let mut move_type = SfMoveType::Normal;
    let mut promo_piece = SfPiece::none();

    if mv.is_en_passant() {
        move_type = SfMoveType::EnPassant;
    } else if mv.is_castle() {
        move_type = SfMoveType::Castle;
    } else if let Some(promo) = mv.promotion() {
        move_type = SfMoveType::Promotion;
        promo_piece = match promo {
            Role::Queen => SfPiece::new(SfPieceType::Queen, color),
            Role::Rook => SfPiece::new(SfPieceType::Rook, color),
            Role::Bishop => SfPiece::new(SfPieceType::Bishop, color),
            Role::Knight => SfPiece::new(SfPieceType::Knight, color),
            Role::King | Role::Pawn => SfPiece::none(),
        };
    }

    SfMove::new(
        SfSquare::new(from_idx as u32),
        SfSquare::new(to_idx as u32),
        move_type,
        promo_piece,
    )
}

// a1 = 0 indexing
fn square_index(sq: Square) -> u32 {
    let file = sq.file().char() as u32 - 'a' as u32;
    let rank = sq.rank().char() as u32 - '1' as u32;
    rank * 8 + file
}
