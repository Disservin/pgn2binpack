use std::io::{BufRead, BufReader, BufWriter, Read, Seek, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use anyhow::{anyhow, bail, Result};
use sfbinpack::chess::piecetype::PieceType;
use sfbinpack::chess::position::Position;
use sfbinpack::{CompressedTrainingDataEntryReader, CompressedTrainingDataEntryWriter};

use crate::wdl::wdl::{external_cp_to_internal, external_cp_to_internal_mat};

pub fn rescore_binpack<R, W>(
    input: R,
    output: W,
    engine_path: &Path,
    depth: u8,
    limit: Option<usize>,
) -> Result<usize>
where
    R: Read + Seek,
    W: Read + Write + Seek,
{
    let mut reader = CompressedTrainingDataEntryReader::new(input)?;
    let mut writer = CompressedTrainingDataEntryWriter::new(output)?;
    let mut engine = UciEngine::start(engine_path)?;
    engine.new_game()?;

    let mut processed = 0usize;

    let mut fen: String = "".to_string();
    let mut moves: Vec<String> = Vec::new();

    while reader.has_next() {
        let mut entry = reader.next();
        if processed == 0 || reader.is_next_entry_continuation() {
            fen = entry.pos.fen().unwrap();
            moves.clear();
        }

        let score = engine.evaluate_moves(&fen, &moves, depth, &entry.pos)?;
        entry.score = score.into();
        writer.write_entry(&entry)?;
        processed += 1;

        if let Some(limit) = limit {
            if processed >= limit {
                break;
            }
        }

        moves.push(entry.mv.as_uci());
    }

    drop(writer);
    engine.shutdown()?;
    Ok(processed)
}

struct UciEngine {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl UciEngine {
    fn start(path: &Path) -> Result<Self> {
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to capture engine stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("failed to capture engine stdout"))?;

        let mut engine = Self {
            child,
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
        };

        engine.send_command("uci")?;
        engine.wait_for("uciok")?;
        engine.wait_ready()?;

        Ok(engine)
    }

    fn new_game(&mut self) -> Result<()> {
        self.send_command("ucinewgame")?;
        self.wait_ready()
    }

    fn evaluate(&mut self, fen: &str, depth: u8, pos: &Position) -> Result<i16> {
        let depth = depth.max(1);
        self.send_command(&format!("position fen {}", fen))?;
        self.send_command(&format!("go depth {}", depth))?;

        let mut last_score: Option<i16> = None;

        loop {
            let line = self.read_line()?;
            let trimmed = line.trim();
            if trimmed.starts_with("info") {
                if let Some(score) = parse_score(trimmed, pos) {
                    last_score = Some(score);
                }
            } else if trimmed.starts_with("bestmove") {
                let score = last_score.ok_or_else(|| anyhow!("engine returned no score"))?;
                self.wait_ready()?;
                return Ok(score);
            }
        }
    }

    fn evaluate_moves(
        &mut self,
        fen: &str,
        moves: &[String],
        depth: u8,
        pos: &Position,
    ) -> Result<i16> {
        let depth = depth.max(1);
        let moves_str = moves.join(" ");
        self.send_command(&format!("position fen {} moves {}", fen, moves_str))?;
        self.send_command(&format!("go depth {}", depth))?;

        let mut last_score: Option<i16> = None;

        loop {
            let line = self.read_line()?;
            let trimmed = line.trim();
            if trimmed.starts_with("info") {
                if let Some(score) = parse_score(trimmed, pos) {
                    last_score = Some(score);
                }
            } else if trimmed.starts_with("bestmove") {
                let score = last_score.ok_or_else(|| anyhow!("engine returned no score"))?;
                self.wait_ready()?;
                return Ok(score);
            }
        }
    }

    fn shutdown(&mut self) -> Result<()> {
        let _ = self.send_command("quit");
        let _ = self.child.wait();
        Ok(())
    }

    fn wait_ready(&mut self) -> Result<()> {
        self.send_command("isready")?;
        self.wait_for("readyok")
    }

    fn send_command(&mut self, command: &str) -> Result<()> {
        writeln!(self.stdin, "{command}")?;
        self.stdin.flush()?;
        Ok(())
    }

    fn read_line(&mut self) -> Result<String> {
        let mut buf = String::new();
        let read = self.stdout.read_line(&mut buf)?;
        if read == 0 {
            bail!("engine closed unexpectedly");
        }
        Ok(buf)
    }

    fn wait_for(&mut self, token: &str) -> Result<()> {
        loop {
            let line = self.read_line()?;
            if line.contains(token) {
                break Ok(());
            }
        }
    }
}

fn parse_score(info_line: &str, pos: &Position) -> Option<i16> {
    let mut parts = info_line.split_whitespace();
    while let Some(token) = parts.next() {
        if token != "score" {
            continue;
        }

        let kind = parts.next()?;
        let raw = parts.next()?;

        let value = raw.parse::<i32>().ok()?;
        let cp = match kind {
            "cp" => value,
            "mate" => mate_to_cp(value),
            _ => return None,
        };

        return Some(external_cp_to_internal_mat(cp, material_count(pos)));
    }

    None
}

fn mate_to_cp(mate: i32) -> i32 {
    const BASE: i32 = 32000;
    if mate > 0 {
        BASE - mate
    } else {
        -BASE + mate.abs()
    }
}

fn material_count(pos: &Position) -> i32 {
    pos.pieces_bb_type(PieceType::Pawn).count() as i32
        + 3 * pos.pieces_bb_type(PieceType::Knight).count() as i32
        + 3 * pos.pieces_bb_type(PieceType::Bishop).count() as i32
        + 5 * pos.pieces_bb_type(PieceType::Rook).count() as i32
        + 9 * pos.pieces_bb_type(PieceType::Queen).count() as i32
}
