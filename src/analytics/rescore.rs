use std::io::{BufRead, BufReader, BufWriter, Read, Seek, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{mpsc, Arc};
use std::thread;

use anyhow::{anyhow, bail, Result};
use sfbinpack::chess::piecetype::PieceType;
use sfbinpack::chess::position::Position;
use sfbinpack::{
    CompressedTrainingDataEntryReader, CompressedTrainingDataEntryWriter, TrainingDataEntry,
};

use crate::wdl::wdl::external_cp_to_internal_mat;

type WorkItem = (usize, String, Vec<String>, TrainingDataEntry, usize);

pub fn rescore_binpack<R, W>(
    input: R,
    output: W,
    engine_path: &Path,
    nodes: usize,
    limit: Option<usize>,
) -> Result<usize>
where
    R: Read + Seek + Send + 'static,
    W: Read + Write + Seek,
{
    let num_threads = thread::available_parallelism()?.get();

    let mut writer = CompressedTrainingDataEntryWriter::new(output)?;

    let (work_tx, work_rx) = mpsc::sync_channel::<WorkItem>(num_threads * 2);
    let (result_tx, result_rx) = mpsc::sync_channel(num_threads * 2);

    let work_rx = Arc::new(std::sync::Mutex::new(work_rx));

    // Spawn worker threads
    let workers: Vec<_> = (0..num_threads)
        .map(|_| {
            let work_rx = Arc::clone(&work_rx);
            let result_tx = result_tx.clone();
            let engine_path = engine_path.to_owned();

            thread::spawn(move || -> Result<()> {
                let mut engine = UciEngine::start(&engine_path)?;
                engine.new_game()?;

                loop {
                    let msg = work_rx.lock().unwrap().recv();
                    match msg {
                        Ok((idx, fen, moves, mut entry, depth)) => {
                            let original_score = entry.score;

                            // Skip entries with VALUE_NONE
                            if original_score == 32002 || original_score == -32002 {
                                result_tx.send((idx, entry)).ok();
                                continue;
                            }

                            let new_score =
                                engine.evaluate_moves(&fen, &moves, depth, &entry.pos)?;
                            entry.score = new_score.into();

                            result_tx.send((idx, entry)).ok();
                        }
                        Err(_) => break,
                    }
                }

                engine.shutdown()?;
                Ok(())
            })
        })
        .collect();

    drop(work_rx);
    drop(result_tx);

    // Reader thread
    let reader_handle = thread::spawn(move || -> Result<usize> {
        let mut reader = CompressedTrainingDataEntryReader::new(input)?;

        let mut processed = 0usize;
        let mut fen: String = "".to_string();
        let mut moves: Vec<String> = Vec::new();

        while reader.has_next() {
            let entry = reader.next();
            if processed == 0 || reader.is_next_entry_continuation() {
                fen = entry.pos.fen().unwrap();
                moves.clear();
            }

            if work_tx
                .send((processed, fen.clone(), moves.clone(), entry, nodes))
                .is_err()
            {
                break;
            }
            processed += 1;

            if let Some(limit) = limit {
                if processed >= limit {
                    break;
                }
            }

            moves.push(entry.mv.as_uci());
        }

        Ok(processed)
    });

    // Write results in order with bounded buffer
    let mut next_idx = 0;
    let mut buffer = std::collections::BTreeMap::new();
    const MAX_BUFFER_SIZE: usize = 10000;

    let t0 = std::time::Instant::now();

    for (idx, entry) in result_rx {
        buffer.insert(idx, entry);

        while let Some(entry) = buffer.remove(&next_idx) {
            writer.write_entry(&entry)?;
            next_idx += 1;
        }

        if buffer.len() > MAX_BUFFER_SIZE {
            bail!("reordering buffer exceeded maximum size - possible ordering issue");
        }

        // every 1000 show progress
        // if next_idx % 1000 == 0 {
        let elapsed = t0.elapsed().as_secs_f64();
        let rate = next_idx as f64 / elapsed;

        print!(
            "\rProcessed: {} entries, Rate: {:.2} entries/sec, Elapsed: {:.2?}",
            next_idx,
            rate,
            std::time::Duration::from_secs_f64(elapsed),
        );
    }

    let processed = reader_handle.join().unwrap()?;

    for worker in workers {
        worker.join().unwrap()?;
    }

    writer.flush();

    drop(writer);
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

    fn evaluate_moves(
        &mut self,
        fen: &str,
        moves: &[String],
        nodes: usize,
        pos: &Position,
    ) -> Result<i16> {
        let moves_str = moves.join(" ");
        self.send_command(&format!("position fen {} moves {}", fen, moves_str))?;
        self.send_command(&format!("go nodes {}", nodes))?;

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
