use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Cursor, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
};

use anyhow::{Context, Result};
use rayon::prelude::*;
use tempfile::NamedTempFile;
use walkdir::WalkDir;

use crate::binpack::BinpackBuilder;

pub fn process_pgn_files(pgn_root: &Path, output_file: &Path, use_memory: bool) -> Result<u64> {
    let files = collect_pgn_files(pgn_root)?;
    let total = files.len();

    if total == 0 {
        anyhow::bail!("No PGN files found in {}", pgn_root.display());
    }

    println!("Found {} PGN files to process", total);

    let completed = AtomicUsize::new(0);

    if use_memory {
        let total_pos = process_with_memory(files, output_file, &completed)?;
        return Ok(total_pos);
    }

    let (parts, total_pos) = process_with_files(files, output_file, &completed)?;
    concatenate_files(&parts, output_file)?;
    Ok(total_pos)
}

fn process_with_memory(
    files: Vec<PathBuf>,
    output_file: &Path,
    completed: &AtomicUsize,
) -> Result<u64> {
    let total = files.len();

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let out_path = output_file.to_path_buf();

    // writer thread
    let writer = thread::spawn(move || -> Result<()> {
        let file = File::create(&out_path)
            .with_context(|| format!("Failed creating output file {}", out_path.display()))?;
        let mut out = BufWriter::new(file);

        for buf in rx {
            out.write_all(&buf)?;
        }
        out.flush()?;
        Ok(())
    });

    // produce buffers in parallel and send to writer
    let results: Vec<u64> = files
        .par_iter()
        .map(|pgn_file| {
            let memory_file = Cursor::new(Vec::new());
            let mut builder = BinpackBuilder::new(pgn_file, memory_file);
            builder.create_binpack();
            let positions = builder.total_positions();

            let buffer = builder.into_inner().unwrap();
            let _ = tx.send(buffer.into_inner());

            let done = completed.fetch_add(1, Ordering::SeqCst) + 1;
            print_progress(done, total);

            positions
        })
        .collect();

    let total_pos: u64 = results.iter().map(|n| *n).sum();

    // drop the sender to close the channel
    drop(tx);

    writer.join().expect("writer thread panicked")?;

    println!();
    Ok(total_pos)
}

fn process_with_files(
    files: Vec<PathBuf>,
    _output_file: &Path,
    completed: &AtomicUsize,
) -> Result<(Vec<PathBuf>, u64)> {
    let total = files.len();

    let results: Vec<(PathBuf, u64)> = files
        .par_iter()
        .map(|pgn_file| {
            let tmp = NamedTempFile::new().expect("failed to create tempfile");

            // dont delete when dropped
            let (thread_file, part_path) = tmp.keep().expect("failed to keep tempfile");

            let mut builder = BinpackBuilder::new(pgn_file, thread_file);
            builder.create_binpack();
            let positions = builder.total_positions();

            let done = completed.fetch_add(1, Ordering::SeqCst) + 1;
            print_progress(done, total);

            (part_path, positions)
        })
        .collect();

    let total_pos: u64 = results.iter().map(|(_, n)| *n).sum();
    let parts: Vec<PathBuf> = results.into_iter().map(|(p, _)| p).collect();

    println!();
    Ok((parts, total_pos))
}

fn concatenate_files(thread_files: &[PathBuf], output_file: &Path) -> Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(output_file)
        .with_context(|| format!("Failed opening {}", output_file.display()))?;
    let mut out = BufWriter::new(file);

    for part in thread_files {
        let mut input =
            File::open(part).with_context(|| format!("Failed opening part {}", part.display()))?;
        std::io::copy(&mut input, &mut out)
            .with_context(|| format!("Failed copying part {}", part.display()))?;
        let _ = std::fs::remove_file(part);
    }
    out.flush()?;
    Ok(())
}

fn collect_pgn_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();

    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let p = entry.path();
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();

        if name.ends_with(".pgn") || name.ends_with(".pgn.gz") {
            out.push(p.to_path_buf());
        }
    }

    out.sort();
    Ok(out)
}

#[inline]
fn print_progress(done: usize, total: usize) {
    use std::io::Write as _;
    if total > 0 {
        print!("\rProcessing: {}/{}", done, total);
        let _ = std::io::stdout().flush();
    }
}
