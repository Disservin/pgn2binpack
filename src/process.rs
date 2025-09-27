use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
};

use anyhow::Result;
use rayon::prelude::*;

use crate::binpack::BinpackBuilder;
use crate::io::{collect_pgn_files, create_temp_file, write_output};

pub fn process_pgn_files(pgn_root: &Path, output_file: &Path, use_memory: bool) -> Result<u64> {
    let files = collect_pgn_files(pgn_root)?;

    if files.is_empty() {
        anyhow::bail!("No PGN files found in {}", pgn_root.display());
    }

    println!("Found {} PGN files to process", files.len());
    let completed = AtomicUsize::new(0);

    if use_memory {
        process_with_memory_buffer(files, output_file, &completed)
    } else {
        process_with_temp_files(files, output_file, &completed)
    }
}

fn process_with_memory_buffer(
    files: Vec<PathBuf>,
    output_file: &Path,
    completed: &AtomicUsize,
) -> Result<u64> {
    let total = files.len();
    let (tx, rx) = mpsc::channel();

    // writer thread
    let writer = thread::spawn({
        let path = output_file.to_path_buf();
        move || write_output(&path, rx)
    });

    // produce buffers in parallel and send to writer
    let positions: Vec<u64> = files
        .par_iter()
        .map(|file| process_single_file_memory(file, &tx, completed, total))
        .collect();

    // drop the sender to close the channel
    drop(tx);
    writer.join().unwrap()?;

    Ok(positions.into_iter().sum())
}

fn process_single_file_memory(
    pgn_file: &Path,
    tx: &mpsc::Sender<Vec<u8>>,
    completed: &AtomicUsize,
    total: usize,
) -> u64 {
    let mut builder = BinpackBuilder::new(pgn_file, Cursor::new(Vec::new()));

    if let Err(e) = builder.create_binpack() {
        eprintln!("\nError processing file {}: {:?}", pgn_file.display(), e);
    }

    let positions = builder.total_positions();
    let buffer = builder.into_inner().unwrap().into_inner();
    let _ = tx.send(buffer);

    update_progress(completed, total);
    positions
}

fn process_with_temp_files(
    files: Vec<PathBuf>,
    output_file: &Path,
    completed: &AtomicUsize,
) -> Result<u64> {
    let total = files.len();

    let results: Vec<_> = files
        .par_iter()
        .map(|file| process_single_file_temp(file, completed, total))
        .collect();

    println!();

    let total_positions = results.iter().map(|(_, n)| n).sum();
    let temp_files: Vec<_> = results.into_iter().map(|(p, _)| p).collect();

    crate::io::concatenate_files(&temp_files, output_file)?;
    Ok(total_positions)
}

fn process_single_file_temp(
    pgn_file: &Path,
    completed: &AtomicUsize,
    total: usize,
) -> (PathBuf, u64) {
    let (file, path) = create_temp_file().expect("failed to create tempfile");

    let mut builder = BinpackBuilder::new(pgn_file, file);

    if let Err(e) = builder.create_binpack() {
        eprintln!("\nError processing file {}: {:?}", pgn_file.display(), e);
    }

    let positions = builder.total_positions();
    update_progress(completed, total);

    (path, positions)
}

fn update_progress(completed: &AtomicUsize, total: usize) {
    use std::io::Write;

    let done = completed.fetch_add(1, Ordering::SeqCst) + 1;
    print!("\rProcessing: {}/{}", done, total);
    let _ = std::io::stdout().flush();
}
