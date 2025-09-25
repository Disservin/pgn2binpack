use anyhow::Result;
use clap::Parser;
use rayon::prelude::*;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use walkdir::WalkDir;

mod binpack;
mod errors;
mod util;
mod wdl;

use crate::binpack::BinpackBuilder;

#[derive(Parser)]
#[command(name = "pgn2binpack")]
#[command(about = "Convert PGN chess files to binpack format", long_about = None)]
struct Cli {
    /// Directory to search for PGN files
    #[arg(value_name = "DIR")]
    input_dir: PathBuf,

    /// Output binpack file
    #[arg(short, long, default_value = "output.binpack")]
    output: PathBuf,

    /// Number of threads to use (default: all CPU cores)
    #[arg(short, long)]
    threads: Option<usize>,

    /// Overwrite output file if it exists
    #[arg(short = 'f', long)]
    force: bool,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(threads) = cli.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()?;
    }

    let input_dir = cli.input_dir;

    if !input_dir.exists() {
        anyhow::bail!("Input directory does not exist: {:?}", input_dir);
    }

    if cli.output.exists() {
        if !cli.force {
            anyhow::bail!(
                "Output file already exists: {:?}. Use --force to overwrite.",
                cli.output
            );
        }
        std::fs::remove_file(&cli.output)?;
    }

    println!("Searching directory: {}", input_dir.display());
    println!("Output file: {}", cli.output.display());
    println!("Using {} threads", rayon::current_num_threads());
    println!();

    process_pgn_files(input_dir, &cli.output, cli.verbose)?;

    let filesize = std::fs::metadata(&cli.output)?.len();
    println!("\n✓ Binpack created successfully");
    println!("  Output: {}", cli.output.display());
    println!("  Size: {}", human_bytes::human_bytes(filesize as f64));

    Ok(())
}

pub fn process_pgn_files(pgn_root: PathBuf, output_file: &Path, verbose: bool) -> Result<()> {
    let files = collect_pgn_files(&pgn_root)?;
    let total = files.len();

    if total == 0 {
        anyhow::bail!("No PGN files found in {}", pgn_root.display());
    }

    println!("Found {} PGN files to process", total);

    let completed = Arc::new(AtomicUsize::new(0));
    let errors = Arc::new(AtomicUsize::new(0));

    let results: Vec<_> = files
        .par_iter()
        .enumerate()
        .map(|(i, pgn_file)| {
            let thread_output = format!("{}.thread_{:04}", output_file.display(), i);

            let builder = BinpackBuilder::new(&pgn_file, &thread_output);

            let result = match builder.create_binpack() {
                Ok(_) => {
                    if verbose {
                        eprintln!("\n✓ Processed: {:?}", pgn_file.file_name().unwrap());
                    }
                    Ok(thread_output)
                }
                Err(e) => {
                    errors.fetch_add(1, Ordering::SeqCst);
                    eprintln!("\n✗ Failed: {:?} - {}", pgn_file.file_name().unwrap(), e);
                    Err(e)
                }
            };

            let done = completed.fetch_add(1, Ordering::SeqCst) + 1;
            let err_count = errors.load(Ordering::SeqCst);

            if err_count > 0 {
                print!("\rProcessing: {}/{} ({} errors)", done, total, err_count);
            } else {
                print!("\rProcessing: {}/{}", done, total);
            }
            std::io::stdout().flush().unwrap();

            result
        })
        .collect();

    println!();

    // Filter out errors but continue with successful files
    let thread_files: Vec<String> = results.into_iter().filter_map(|r| r.ok()).collect();

    if thread_files.is_empty() {
        anyhow::bail!("All files failed to process");
    }

    let error_count = errors.load(Ordering::SeqCst);
    if error_count > 0 {
        println!("⚠ Processed with {} errors", error_count);
    }

    print!("Concatenating thread files...");
    std::io::stdout().flush()?;
    concatenate_files(&thread_files, output_file)?;
    println!(" done");

    Ok(())
}

fn concatenate_files(thread_files: &[String], output_file: &Path) -> Result<()> {
    let mut output = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(output_file)?;

    for thread_file in thread_files {
        let mut input = File::open(thread_file)?;
        let mut buffer = Vec::new();
        input.read_to_end(&mut buffer)?;
        output.write_all(&buffer)?;
        // remove the thread file after concatenation frees up space
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

// use sfbinpack::CompressedTrainingDataEntryReader;

// fn main() {
//     let mut reader = CompressedTrainingDataEntryReader::new("./output.binpack").unwrap();

//     let mut i = 0;

//     while reader.has_next() {
//         let entry = reader.next();

//         println!("entry:");
//         println!("fen {}", entry.pos.fen());
//         println!("uci move {:?}", entry.mv.as_uci());
//         println!("score {}", entry.score);
//         println!("ply {}", entry.ply);
//         println!("result {}", entry.result);
//         println!("\n");

//         i = i + 1;
//         if i > 200 {
//             break;
//         }
//     }
// }
