mod dump;
mod tui;

use std::collections::VecDeque;
use std::io::{self, BufReader, ErrorKind, IsTerminal, Read, Seek};

use anyhow::{anyhow, Context, Result};
use sfbinpack::chess::piece::Piece;
use sfbinpack::CompressedTrainingDataEntryReader;
use viriformat::dataformat::Game as ViriGame;

use crate::cli::Backend;

pub(super) const LARGE_SQUARE_WIDTH: usize = 7;
pub(super) const LARGE_BOARD_LEFT_MARGIN: usize = 3;
pub(super) const BOARD_FILES: usize = 8;
const LIGHT_SQUARE_BG: &str = "\x1b[48;5;250m";
const DARK_SQUARE_BG: &str = "\x1b[48;5;60m";
const LIGHT_PIECE_FG: &str = "\x1b[1;38;5;255m";
const DARK_PIECE_FG: &str = "\x1b[1;38;5;16m";
const VALUE_NONE_SCORE: i32 = 32002;

#[derive(Clone, Debug)]
pub(super) struct ViewFrame {
    pub(super) game_index: usize,
    pub(super) position_in_game: usize,
    pub(super) fen: String,
    pub(super) uci_move: String,
    pub(super) score: String,
    pub(super) score_detail: Option<String>,
    pub(super) ply: u32,
    pub(super) result: String,
}

pub fn view_entries<T: Read + Seek>(file: T, limit: Option<usize>, backend: Backend) -> Result<()> {
    let mut session = ViewSession::new(file, limit, backend)?;

    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        tui::browse_frames(&mut session)
    } else {
        dump::dump_frames(&mut session)
    }
}

pub(super) struct ViewSession<T: Read + Seek> {
    source: ViewSource<T>,
    pub(super) frames: Vec<ViewFrame>,
    pub(super) eof: bool,
}

impl<T: Read + Seek> ViewSession<T> {
    fn new(file: T, limit: Option<usize>, backend: Backend) -> Result<Self> {
        let source = match backend {
            Backend::Sfbinpack => ViewSource::Sf(SfSource::new(file, limit)?),
            Backend::Viriformat => ViewSource::Viriformat(ViriformatSource::new(file, limit)),
        };

        Ok(Self {
            source,
            frames: Vec::new(),
            eof: false,
        })
    }

    pub(super) fn ensure_loaded(&mut self, index: usize) -> Result<bool> {
        while self.frames.len() <= index && !self.eof {
            match self.source.next_frame()? {
                Some(frame) => self.frames.push(frame),
                None => self.eof = true,
            }
        }

        Ok(self.frames.len() > index)
    }

    pub(super) fn next_game_index(&mut self, index: usize) -> Result<Option<usize>> {
        let current_game = self.frames[index].game_index;
        let mut scan = index + 1;

        loop {
            while scan < self.frames.len() {
                if self.frames[scan].game_index != current_game {
                    return Ok(Some(scan));
                }
                scan += 1;
            }

            if self.eof {
                return Ok(None);
            }

            if !self.ensure_loaded(self.frames.len())? {
                return Ok(None);
            }
        }
    }

    pub(super) fn previous_game_index(&self, index: usize) -> Option<usize> {
        let current_game = self.frames[index].game_index;
        self.frames[..index]
            .iter()
            .rposition(|frame| frame.game_index != current_game)
            .map(|i| {
                let target_game = self.frames[i].game_index;
                self.frames[..=i]
                    .iter()
                    .position(|frame| frame.game_index == target_game)
                    .expect("target game exists")
            })
    }

    pub(super) fn total_display(&self) -> String {
        if self.eof {
            self.frames.len().to_string()
        } else {
            "?".to_string()
        }
    }
}

enum ViewSource<T: Read + Seek> {
    Sf(SfSource<T>),
    Viriformat(ViriformatSource<T>),
}

impl<T: Read + Seek> ViewSource<T> {
    fn next_frame(&mut self) -> Result<Option<ViewFrame>> {
        match self {
            Self::Sf(source) => source.next_frame(),
            Self::Viriformat(source) => source.next_frame(),
        }
    }
}

struct SfSource<T: Read + Seek> {
    reader: CompressedTrainingDataEntryReader<T>,
    game_index: usize,
    position_in_game: usize,
    new_game: bool,
    emitted: usize,
    limit: Option<usize>,
}

impl<T: Read + Seek> SfSource<T> {
    fn new(file: T, limit: Option<usize>) -> Result<Self> {
        Ok(Self {
            reader: CompressedTrainingDataEntryReader::new(file)?,
            game_index: 0,
            position_in_game: 0,
            new_game: true,
            emitted: 0,
            limit,
        })
    }

    fn next_frame(&mut self) -> Result<Option<ViewFrame>> {
        if self.limit.is_some_and(|limit| self.emitted >= limit) || !self.reader.has_next() {
            return Ok(None);
        }

        let entry = self.reader.next();

        if self.new_game {
            self.game_index += 1;
            self.position_in_game = 1;
            self.new_game = false;
        }

        let frame = ViewFrame {
            game_index: self.game_index,
            position_in_game: self.position_in_game,
            fen: entry
                .pos
                .fen()
                .map_err(|err| anyhow!("failed to render FEN for entry: {err:?}"))?,
            uci_move: entry.mv.as_uci().to_string(),
            score: format_score(i32::from(entry.score)),
            score_detail: format_score_detail(&entry),
            ply: entry.ply.into(),
            result: format!("{:?}", entry.result),
        };

        self.emitted += 1;

        if self.reader.has_next() && self.reader.is_next_entry_continuation() {
            self.position_in_game += 1;
        } else {
            self.new_game = true;
        }

        Ok(Some(frame))
    }
}

struct ViriformatSource<T: Read + Seek> {
    reader: BufReader<T>,
    pending_frames: VecDeque<ViewFrame>,
    game_index: usize,
    emitted: usize,
    limit: Option<usize>,
}

impl<T: Read + Seek> ViriformatSource<T> {
    fn new(file: T, limit: Option<usize>) -> Self {
        Self {
            reader: BufReader::new(file),
            pending_frames: VecDeque::new(),
            game_index: 0,
            emitted: 0,
            limit,
        }
    }

    fn next_frame(&mut self) -> Result<Option<ViewFrame>> {
        if self.limit.is_some_and(|limit| self.emitted >= limit) {
            return Ok(None);
        }

        loop {
            if let Some(frame) = self.pending_frames.pop_front() {
                self.emitted += 1;
                return Ok(Some(frame));
            }

            match ViriGame::deserialise_from(&mut self.reader, Vec::new()) {
                Ok(game) => {
                    self.game_index += 1;
                    self.pending_frames = build_viriformat_frames(game, self.game_index)?;
                }
                Err(err) if err.kind() == ErrorKind::UnexpectedEof => return Ok(None),
                Err(err) => return Err(err.into()),
            }
        }
    }
}

fn build_viriformat_frames(game: ViriGame, game_index: usize) -> Result<VecDeque<ViewFrame>> {
    let (mut board, _, _, _) = game.initial_position.unpack();
    let mut frames = VecDeque::with_capacity(game.moves.len());

    for (position_in_game, (mv, eval)) in game.moves.iter().enumerate() {
        frames.push_back(ViewFrame {
            game_index,
            position_in_game: position_in_game + 1,
            fen: board.to_string(),
            uci_move: mv.display(false).to_string(),
            score: format_score(i32::from(eval.get())),
            score_detail: None,
            ply: board.ply() as u32,
            result: format!("{:?}", game.outcome()),
        });

        board
            .make_move_simple(*mv)
            .then_some(())
            .context("illegal move in viriformat stream")?;
    }

    Ok(frames)
}

pub(super) fn render_board(fen: &str, use_color: bool) -> Result<String> {
    let board = fen
        .split_whitespace()
        .next()
        .context("FEN is missing board layout")?;
    let ranks: Vec<&str> = board.split('/').collect();

    if ranks.len() != 8 {
        anyhow::bail!("FEN board layout must have 8 ranks: {}", fen);
    }

    if use_color {
        return render_large_board(&ranks);
    }

    let mut out = String::new();
    for (rank_idx, rank) in ranks.iter().enumerate() {
        let rank_label = 8 - rank_idx;
        out.push_str(&format!("{} ", rank_label));

        let mut file_idx = 0usize;
        for ch in rank.chars() {
            match ch {
                '1'..='8' => {
                    for _ in 0..ch.to_digit(10).expect("digit") {
                        out.push_str(&render_square(None, rank_idx, file_idx, use_color));
                        file_idx += 1;
                    }
                }
                'p' | 'r' | 'n' | 'b' | 'q' | 'k' | 'P' | 'R' | 'N' | 'B' | 'Q' | 'K' => {
                    out.push_str(&render_square(Some(ch), rank_idx, file_idx, use_color));
                    file_idx += 1;
                }
                _ => anyhow::bail!("invalid board character in FEN: {}", ch),
            }
        }

        if file_idx != 8 {
            anyhow::bail!("FEN rank does not contain 8 files: {}", rank);
        }

        out.push('\n');
    }
    out.push_str("  a  b  c  d  e  f  g  h");

    Ok(out)
}

fn render_large_board(ranks: &[&str]) -> Result<String> {
    let mut out = String::new();

    for (rank_idx, rank) in ranks.iter().enumerate() {
        let rank_label = 8 - rank_idx;
        let mut top = String::from("   ");
        let mut middle = format!("{}  ", rank_label);
        let mut bottom = String::from("   ");
        let mut file_idx = 0usize;

        for ch in rank.chars() {
            match ch {
                '1'..='8' => {
                    for _ in 0..ch.to_digit(10).expect("digit") {
                        top.push_str(&render_large_square(
                            None,
                            rank_idx,
                            file_idx,
                            SquareBand::Top,
                        ));
                        middle.push_str(&render_large_square(
                            None,
                            rank_idx,
                            file_idx,
                            SquareBand::Middle,
                        ));
                        bottom.push_str(&render_large_square(
                            None,
                            rank_idx,
                            file_idx,
                            SquareBand::Bottom,
                        ));
                        file_idx += 1;
                    }
                }
                'p' | 'r' | 'n' | 'b' | 'q' | 'k' | 'P' | 'R' | 'N' | 'B' | 'Q' | 'K' => {
                    top.push_str(&render_large_square(
                        Some(ch),
                        rank_idx,
                        file_idx,
                        SquareBand::Top,
                    ));
                    middle.push_str(&render_large_square(
                        Some(ch),
                        rank_idx,
                        file_idx,
                        SquareBand::Middle,
                    ));
                    bottom.push_str(&render_large_square(
                        Some(ch),
                        rank_idx,
                        file_idx,
                        SquareBand::Bottom,
                    ));
                    file_idx += 1;
                }
                _ => anyhow::bail!("invalid board character in FEN: {}", ch),
            }
        }

        if file_idx != 8 {
            anyhow::bail!("FEN rank does not contain 8 files: {}", rank);
        }

        out.push_str(&top);
        out.push('\n');
        out.push_str(&middle);
        out.push('\n');
        out.push_str(&bottom);
        out.push('\n');
    }

    out.push_str("   ");
    for file in ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'] {
        out.push_str(&format!("   {}   ", file));
    }
    Ok(out)
}

#[derive(Clone, Copy)]
enum SquareBand {
    Top,
    Middle,
    Bottom,
}

fn render_large_square(
    piece: Option<char>,
    rank_idx: usize,
    file_idx: usize,
    band: SquareBand,
) -> String {
    let dark_square = (rank_idx + file_idx) % 2 == 1;
    let bg = if dark_square {
        DARK_SQUARE_BG
    } else {
        LIGHT_SQUARE_BG
    };
    let fg = match piece {
        Some(ch) if ch.is_uppercase() => LIGHT_PIECE_FG,
        Some(_) => DARK_PIECE_FG,
        None => "",
    };
    let content = match band {
        SquareBand::Top | SquareBand::Bottom => "       ".to_string(),
        SquareBand::Middle => format!("   {}   ", piece.map(unicode_piece).unwrap_or(' ')),
    };

    format!("{}{fg}{}\x1b[0m", bg, content)
}

fn render_square(piece: Option<char>, rank_idx: usize, file_idx: usize, use_color: bool) -> String {
    let dark_square = (rank_idx + file_idx) % 2 == 1;
    let symbol = piece
        .map(unicode_piece)
        .unwrap_or(if use_color { ' ' } else { '.' });

    if !use_color {
        return format!(" {} ", symbol);
    }

    let bg = if dark_square {
        DARK_SQUARE_BG
    } else {
        LIGHT_SQUARE_BG
    };
    let fg = match piece {
        Some(ch) if ch.is_uppercase() => LIGHT_PIECE_FG,
        Some(_) => DARK_PIECE_FG,
        None => "",
    };

    format!("{}{fg} {} \x1b[0m", bg, symbol)
}

pub(super) fn side_to_move(fen: &str) -> &'static str {
    match fen.split_whitespace().nth(1) {
        Some("w") => "White",
        Some("b") => "Black",
        _ => "?",
    }
}

fn format_score(score: i32) -> String {
    if score == VALUE_NONE_SCORE {
        "VALUE_NONE".to_string()
    } else {
        score.to_string()
    }
}

fn format_score_detail(entry: &sfbinpack::TrainingDataEntry) -> Option<String> {
    if entry.score != 0 {
        return None;
    }

    let is_capture = is_capturing_move(entry);
    let is_in_check = in_check(entry);

    if !(is_capture || is_in_check) {
        return None;
    }

    let reason = match (is_capture, is_in_check) {
        (true, true) => "capture + in-check",
        (true, false) => "capture",
        (false, true) => "in-check",
        (false, false) => unreachable!("guarded above"),
    };

    Some(format!("skipped ({reason})"))
}

fn is_capturing_move(entry: &sfbinpack::TrainingDataEntry) -> bool {
    entry.pos.piece_at(entry.mv.to()) != Piece::none()
        && entry.pos.piece_at(entry.mv.to()).color() != entry.pos.piece_at(entry.mv.from()).color()
}

fn in_check(entry: &sfbinpack::TrainingDataEntry) -> bool {
    entry.pos.is_checked(entry.pos.side_to_move())
}

fn unicode_piece(piece: char) -> char {
    match piece {
        'K' => '♔',
        'Q' => '♕',
        'R' => '♖',
        'B' => '♗',
        'N' => '♘',
        'P' => '♙',
        'k' => '♚',
        'q' => '♛',
        'r' => '♜',
        'b' => '♝',
        'n' => '♞',
        'p' => '♟',
        _ => '?',
    }
}

#[cfg(test)]
mod tests {
    use super::render_board;

    #[test]
    fn renders_start_position_board() {
        let board = render_board(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            false,
        )
        .expect("valid board");

        let lines: Vec<&str> = board.lines().collect();

        assert_eq!(lines[0], "8  ♜  ♞  ♝  ♛  ♚  ♝  ♞  ♜ ");
        assert_eq!(lines[7], "1  ♖  ♘  ♗  ♕  ♔  ♗  ♘  ♖ ");
        assert!(board.contains("  a  b  c  d  e  f  g  h"));
    }
}
