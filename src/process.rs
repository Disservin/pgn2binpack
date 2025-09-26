use anyhow::Result;
use rayon::prelude::*;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use walkdir::WalkDir;

use crate::binpack::BinpackBuilder;

pub fn process_pgn_files(pgn_root: PathBuf, output_file: &Path, use_memory: bool) -> Result<()> {
    let files = collect_pgn_files(&pgn_root)?;
    let total = files.len();

    if total == 0 {
        anyhow::bail!("No PGN files found in {}", pgn_root.display());
    }

    println!("Found {} PGN files to process", total);

    let completed = Arc::new(AtomicUsize::new(0));
    let errors = Arc::new(AtomicUsize::new(0));

    if use_memory {
        process_with_memory(files, output_file, completed, errors)?;
    } else {
        let parts = process_with_files(files, output_file, completed, errors)?;
        concatenate_files(&parts, output_file)?;
    }
    Ok(())
}

fn process_with_memory(
    files: Vec<PathBuf>,
    output_file: &Path,
    completed: Arc<AtomicUsize>,
    errors: Arc<AtomicUsize>,
) -> Result<Vec<String>> {
    let total = files.len();
    let queue = Arc::new(Mutex::new(VecDeque::<Vec<u8>>::new()));
    let queue_clone = Arc::clone(&queue);
    let processing_done = Arc::new(AtomicBool::new(false));
    let processing_done_clone = Arc::clone(&processing_done);

    let output_path = output_file.to_path_buf();
    let writer_handle = thread::spawn(move || -> Result<()> {
        let mut output = File::create(output_path)?;
        while !processing_done_clone.load(Ordering::SeqCst)
            || !queue_clone.lock().unwrap().is_empty()
        {
            let data = {
                let mut q = queue_clone.lock().unwrap();
                q.pop_front()
            };

            if let Some(buffer) = data {
                output.write_all(&buffer)?;
            } else {
                thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        Ok(())
    });

    let results: Vec<_> = files
        .par_iter()
        .enumerate()
        .map(|(i, pgn_file)| {
            let memory_file = Cursor::new(Vec::new());
            let mut builder = BinpackBuilder::new(&pgn_file, memory_file);
            builder.create_binpack();

            let buffer = builder.into_inner().unwrap();
            {
                let mut q = queue.lock().unwrap();
                q.push_back(buffer.into_inner());
            }

            let done = completed.fetch_add(1, Ordering::SeqCst) + 1;
            let err_count = errors.load(Ordering::SeqCst);

            if err_count > 0 {
                print!("\rProcessing: {}/{} ({} errors)", done, total, err_count);
            } else {
                print!("\rProcessing: {}/{}", done, total);
            }
            std::io::stdout().flush().unwrap();

            format!("processed_{:04}", i)
        })
        .collect();

    processing_done.store(true, Ordering::SeqCst);
    writer_handle.join().unwrap()?;

    println!();
    let error_count = errors.load(Ordering::SeqCst);
    if error_count > 0 {
        println!("⚠ Processed with {} errors", error_count);
    }

    Ok(results)
}

fn process_with_files(
    files: Vec<PathBuf>,
    output_file: &Path,
    completed: Arc<AtomicUsize>,
    errors: Arc<AtomicUsize>,
) -> Result<Vec<String>> {
    let total = files.len();

    let results: Vec<_> = files
        .par_iter()
        .enumerate()
        .map(|(i, pgn_file)| {
            let thread_output = format!("{}.thread_{:04}", output_file.display(), i);
            let thread_file = File::create(&thread_output).unwrap();

            let mut builder = BinpackBuilder::new(&pgn_file, thread_file);
            builder.create_binpack();

            let done = completed.fetch_add(1, Ordering::SeqCst) + 1;
            let err_count = errors.load(Ordering::SeqCst);

            if err_count > 0 {
                print!("\rProcessing: {}/{} ({} errors)", done, total, err_count);
            } else {
                print!("\rProcessing: {}/{}", done, total);
            }
            std::io::stdout().flush().unwrap();

            thread_output
        })
        .collect();

    println!();
    let error_count = errors.load(Ordering::SeqCst);
    if error_count > 0 {
        println!("⚠ Processed with {} errors", error_count);
    }

    Ok(results)
}

fn concatenate_files(thread_files: &Vec<String>, output_file: &Path) -> Result<()> {
    let mut output = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(output_file)?;

    for thread_file in thread_files {
        let mut input = File::open(thread_file)?;
        std::io::copy(&mut input, &mut output)?;
        std::fs::remove_file(thread_file).ok();
    }
    Ok(())
}

fn collect_pgn_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();

    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }

        let p = entry.path();
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");

        if name.ends_with(".pgn") || name.ends_with(".pgn.gz") {
            out.push(p.to_path_buf());
        }
    }

    out.sort();
    Ok(out)
}
