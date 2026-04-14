use std::io::{self, Read, Seek, Write};

use anyhow::Result;
use crossterm::cursor::MoveTo;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size, Clear, ClearType};

use super::{
    render_board, side_to_move, ViewSession, BOARD_FILES, LARGE_BOARD_LEFT_MARGIN,
    LARGE_SQUARE_WIDTH,
};

pub(super) fn browse_frames<T: Read + Seek>(session: &mut ViewSession<T>) -> Result<()> {
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
        let _ = execute!(io::stdout(), Clear(ClearType::All), MoveTo(0, 0));
    }
}

fn render_frame(
    frame: &super::ViewFrame,
    index: usize,
    total_display: &str,
    message: &str,
    jump_buffer: &str,
) -> Result<()> {
    let board_lines: Vec<String> = render_board(&frame.fen, true)?
        .lines()
        .map(ToOwned::to_owned)
        .collect();
    let board_height = board_lines.len();
    let board_width = interactive_board_width();
    let (term_width, term_height) = size()
        .map(|(width, height)| (usize::from(width), usize::from(height)))
        .unwrap_or((120, 40));
    let board_x = term_width.saturating_sub(board_width) / 2;
    let board_y = term_height.saturating_sub(board_height) / 2;
    let panel_gap = 2usize;
    let left_panel_width = board_x.saturating_sub(panel_gap);
    let right_panel_x = board_x + board_width + panel_gap;
    let right_panel_width = term_width.saturating_sub(right_panel_x);

    let left_panel: Vec<String> = Vec::new();
    let eval_line = match &frame.score_detail {
        Some(detail) => format!("Eval:  {} ({})", frame.score, detail),
        None => format!("Eval:  {}", frame.score),
    };

    let mut right_panel = vec![
        format!("Position {}", index + 1),
        format!("Total:    {}", total_display),
        format!("Game:     {}", frame.game_index),
        format!("Move #:    {}", frame.position_in_game),
        format!("Side:     {}", side_to_move(&frame.fen)),
        format!("Ply:      {}", frame.ply),
        format!("Result:   {}", frame.result),
        String::new(),
        format!("Move:  {}", frame.uci_move),
        eval_line,
        String::new(),
        "FEN".to_string(),
        String::new(),
        "Keys".to_string(),
        "->/Space  next".to_string(),
        "<-/p      prev".to_string(),
        "[ / ]     games".to_string(),
        "digits+Enter jump".to_string(),
        "q / Esc   quit".to_string(),
    ];

    if !jump_buffer.is_empty() {
        right_panel.push(String::new());
        right_panel.push(format!("Jump: {}", jump_buffer));
    }

    if !message.is_empty() {
        right_panel.push(String::new());
        right_panel.push(message.to_string());
    }

    if right_panel_width > 0 {
        let fen_insert_at = 11;
        let fen_lines = wrap_text(&frame.fen, right_panel_width);
        right_panel.splice(fen_insert_at..fen_insert_at, fen_lines);
    }

    for (row, line) in board_lines.iter().enumerate() {
        draw_at(board_x, board_y + row, line)?;
    }

    let left_panel_y = panel_origin(board_y, board_height, left_panel.len());
    for (row, line) in left_panel.iter().enumerate() {
        draw_at(
            0,
            left_panel_y + row,
            &fit_panel_line(line, left_panel_width),
        )?;
    }

    let right_panel_y = panel_origin(board_y, board_height, right_panel.len());
    for (row, line) in right_panel.iter().enumerate() {
        draw_at(
            right_panel_x,
            right_panel_y + row,
            &fit_panel_line(line, right_panel_width),
        )?;
    }

    Ok(())
}

fn draw_at(x: usize, y: usize, text: &str) -> Result<()> {
    execute!(io::stdout(), MoveTo(x as u16, y as u16))?;
    print!("{}", text);
    Ok(())
}

fn fit_panel_line(line: &str, width: usize) -> String {
    let line_width = line.chars().count();
    if width == 0 {
        return String::new();
    }
    if line_width >= width {
        return line
            .chars()
            .take(width.saturating_sub(1))
            .collect::<String>()
            + " ";
    }

    let mut out = String::with_capacity(width);
    out.push_str(line);
    out.push_str(&" ".repeat(width - line_width));
    out
}

fn panel_origin(board_y: usize, board_height: usize, panel_height: usize) -> usize {
    let board_mid = board_y + board_height / 2;
    board_mid.saturating_sub(panel_height / 2)
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0usize;

    while start < chars.len() {
        let end = (start + width).min(chars.len());
        lines.push(chars[start..end].iter().collect());
        start = end;
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}

fn interactive_board_width() -> usize {
    LARGE_BOARD_LEFT_MARGIN + (LARGE_SQUARE_WIDTH * BOARD_FILES)
}
