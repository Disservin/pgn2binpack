use std::collections::VecDeque;
use std::io::{self, BufReader, ErrorKind, IsTerminal, Read, Seek, Write};

use anyhow::{anyhow, Context, Result};
use crossterm::cursor::MoveTo;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use sfbinpack::CompressedTrainingDataEntryReader;
use viriformat::dataformat::Game as ViriGame;

use crate::cli::Backend;

#[derive(Clone, Debug)]
struct ViewFrame {
    game_index: usize,
    position_in_game: usize,
    fen: String,
    uci_move: String,
    score: String,
    ply: u32,
    result: String,
}

pub fn view_entries<T: Read + Seek>(file: T, limit: Option<usize>, backend: Backend) -> Result<()> {
    let mut session = ViewSession::new(file, limit, backend)?;

    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        browse_frames(&mut session)
    } else {
        dump_frames(&mut session)
    }
}

struct ViewSession<T: Read + Seek> {
    source: ViewSource<T>,
    frames: Vec<ViewFrame>,
    eof: bool,
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

    fn ensure_loaded(&mut self, index: usize) -> Result<bool> {
        while self.frames.len() <= index && !self.eof {
            match self.source.next_frame()? {
                Some(frame) => self.frames.push(frame),
                None => self.eof = true,
            }
        }

        Ok(self.frames.len() > index)
    }

    fn next_game_index(&mut self, index: usize) -> Result<Option<usize>> {
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

    fn previous_game_index(&self, index: usize) -> Option<usize> {
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

    fn total_display(&self) -> String {
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
            score: entry.score.to_string(),
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
            score: eval.get().to_string(),
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

fn browse_frames<T: Read + Seek>(session: &mut ViewSession<T>) -> Result<()> {
    if !session.ensure_loaded(0)? {
        println!("No positions found.");
        return Ok(());
    }

    let _raw_mode = RawModeGuard::new()?;
    let mut index = 0usize;
    let mut message = String::new();
    let mut jump_buffer = String::new();
    let mut stdout = io::stdout();

    loop {
        execute!(stdout, MoveTo(0, 0))?;
        write!(stdout, "\x1b[2J")?;
        render_frame(
            &session.frames[index],
            index,
            &session.total_display(),
            &message,
            &jump_buffer,
        )?;
        stdout.flush()?;
        message.clear();

        let Event::Key(key) = event::read()? else {
            continue;
        };

        if key.kind != KeyEventKind::Press {
            continue;
        }

        match key.code {
            KeyCode::Right | KeyCode::Char('n') | KeyCode::Char(' ') => {
                if session.ensure_loaded(index + 1)? {
                    index += 1;
                } else {
                    message = "Already at the last position.".to_string();
                }
                jump_buffer.clear();
            }
            KeyCode::Left | KeyCode::Char('p') => {
                if jump_buffer.is_empty() {
                    if index > 0 {
                        index -= 1;
                    } else {
                        message = "Already at the first position.".to_string();
                    }
                } else {
                    jump_buffer.pop();
                }
            }
            KeyCode::Char(']') => {
                if let Some(next) = session.next_game_index(index)? {
                    index = next;
                } else {
                    message = "Already at the last game.".to_string();
                }
                jump_buffer.clear();
            }
            KeyCode::Char('[') => {
                if let Some(prev) = session.previous_game_index(index) {
                    index = prev;
                } else {
                    message = "Already at the first game.".to_string();
                }
                jump_buffer.clear();
            }
            KeyCode::Backspace => {
                if jump_buffer.is_empty() {
                    if index > 0 {
                        index -= 1;
                    } else {
                        message = "Already at the first position.".to_string();
                    }
                } else {
                    jump_buffer.pop();
                }
            }
            KeyCode::Enter => {
                if jump_buffer.is_empty() {
                    if session.ensure_loaded(index + 1)? {
                        index += 1;
                    } else {
                        message = "Already at the last position.".to_string();
                    }
                } else {
                    match jump_buffer.parse::<usize>() {
                        Ok(position) if position >= 1 => {
                            let target = position - 1;
                            if session.ensure_loaded(target)? {
                                index = target;
                            } else {
                                message = format!(
                                    "Only {} positions are available.",
                                    session.frames.len()
                                );
                            }
                        }
                        Ok(_) => {
                            message = "Position must be 1 or greater.".to_string();
                        }
                        Err(_) => {
                            message = "Jump target must be a positive number.".to_string();
                        }
                    }
                    jump_buffer.clear();
                }
            }
            KeyCode::Char('q') | KeyCode::Esc => break,
            KeyCode::Char(ch)
                if ch.is_ascii_digit()
                    && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT) =>
            {
                jump_buffer.push(ch);
            }
            _ => {
                message = "Keys: Right/Space next, Left prev, [ prev game, ] next game, digits+Enter jump, q quit.".to_string();
            }
        }
    }

    Ok(())
}

struct RawModeGuard;

impl RawModeGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

fn render_frame(
    frame: &ViewFrame,
    index: usize,
    total_display: &str,
    message: &str,
    jump_buffer: &str,
) -> Result<()> {
    let board = render_board(&frame.fen, true)?;

    let mut screen = String::new();
    push_line(
        &mut screen,
        &format!(
            "Position {}/{} | Game {} | Move {}",
            index + 1,
            total_display,
            frame.game_index,
            frame.position_in_game
        ),
    );
    push_line(&mut screen, "");
    for line in board.lines() {
        push_line(&mut screen, line);
    }
    push_line(&mut screen, &format!("Move:   {}", frame.uci_move));
    push_line(&mut screen, &format!("Score:  {}", frame.score));
    push_line(&mut screen, &format!("Ply:    {}", frame.ply));
    push_line(&mut screen, &format!("Result: {}", frame.result));
    push_line(&mut screen, &format!("FEN:    {}", frame.fen));
    push_line(&mut screen, "");
    push_line(
        &mut screen,
        "Keys: Right/Space next, Left prev, [ prev game, ] next game, digits+Enter jump, q quit",
    );

    if !jump_buffer.is_empty() {
        push_line(&mut screen, &format!("Jump:   {}", jump_buffer));
    }

    if !message.is_empty() {
        push_line(&mut screen, message);
    }

    print!("{}", screen);
    Ok(())
}

fn push_line(buf: &mut String, line: &str) {
    buf.push_str(line);
    buf.push_str("\r\n");
}

fn dump_frames<T: Read + Seek>(session: &mut ViewSession<T>) -> Result<()> {
    let mut index = 0usize;

    while session.ensure_loaded(index)? {
        let frame = &session.frames[index];
        println!("position {}", index + 1);
        println!("game {} move {}", frame.game_index, frame.position_in_game);
        if let Ok(board) = render_board(&frame.fen, false) {
            println!("{}", board);
        }
        println!("fen {}", frame.fen);
        println!("uci move {}", frame.uci_move);
        println!("score {}", frame.score);
        println!("ply {}", frame.ply);
        println!("result {}", frame.result);
        println!();

        index += 1;
    }

    if index == 0 {
        println!("No positions found.");
    }

    Ok(())
}

fn render_board(fen: &str, use_color: bool) -> Result<String> {
    let board = fen
        .split_whitespace()
        .next()
        .context("FEN is missing board layout")?;
    let ranks: Vec<&str> = board.split('/').collect();

    if ranks.len() != 8 {
        anyhow::bail!("FEN board layout must have 8 ranks: {}", fen);
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

fn render_square(piece: Option<char>, rank_idx: usize, file_idx: usize, use_color: bool) -> String {
    let dark_square = (rank_idx + file_idx) % 2 == 1;
    let symbol = piece
        .map(unicode_piece)
        .unwrap_or(if use_color { ' ' } else { '.' });

    if !use_color {
        return format!(" {} ", symbol);
    }

    let bg = if dark_square {
        "\x1b[48;5;101m"
    } else {
        "\x1b[48;5;223m"
    };
    let fg = match piece {
        Some(ch) if ch.is_uppercase() => "\x1b[38;5;255m",
        Some(_) => "\x1b[38;5;16m",
        None => "",
    };

    format!("{}{fg} {} \x1b[0m", bg, symbol)
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
