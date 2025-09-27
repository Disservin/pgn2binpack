use anyhow::Result;
use clap::Parser;

mod analytics;
mod binpack;
mod cli;
mod errors;
mod io;
mod process;
mod util;
mod wdl;

use crate::cli::Cli;
use crate::process::process_pgn_files;

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(threads) = cli.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()?;
    }

    if let Some(input) = cli.input_dir {
        if let Some(ref output) = cli.output {
            if output.is_dir() {
                anyhow::bail!("Output path is a directory: {:?}", output);
            }
        } else {
            anyhow::bail!("Output file must be specified with --output");
        }

        let output = cli.output.as_ref().unwrap();

        if output.exists() {
            if !cli.force {
                anyhow::bail!(
                    "Output file already exists: {:?}. Use --force to overwrite.",
                    output
                );
            }
            std::fs::remove_file(&output)?;
        }

        if !input.exists() {
            anyhow::bail!("Input directory does not exist: {:?}", input);
        }

        println!("Searching directory: {}", input.display());
        println!("Output file: {}", output.display());
        println!("Using {} threads", rayon::current_num_threads());
        println!("Using memory: {}", if cli.memory { "yes" } else { "no" });
        println!();

        let t0 = std::time::Instant::now();
        let count = process_pgn_files(&input, &output, cli.memory)?;
        println!("Time taken: {:.2?}", t0.elapsed());

        let filesize = std::fs::metadata(&output)?.len();
        println!("\n✓ Binpack created successfully");
        println!("  Output: {}", output.display());
        println!("  Size: {}", human_bytes::human_bytes(filesize as f64));
        println!("  Positions: {}", count);
    }

    if let Some(unique) = cli.unique {
        let file = std::fs::File::options()
            .read(true)
            .write(false)
            .create(false)
            .open(&unique)?;
        let t0 = std::time::Instant::now();
        let unique_count = analytics::unique::unique_positions_from_file(file, cli.limit)?;
        println!("Completed in {:.2?}", t0.elapsed());
        println!("Unique positions (Zobrist hashes): {}", unique_count);
    }

    Ok(())
}

// use std::fs::File;

// use sfbinpack::CompressedTrainingDataEntryReader;

// fn main() {
//     let file = File::options()
//         .read(true)
//         .write(false)
//         .create(false)
//         .open("./fishtest-binpack/fishpack-diss-v1.binpack")
//         .unwrap();
//     let mut reader = CompressedTrainingDataEntryReader::new(file).unwrap();

//     let mut i: u64 = 0;

//     while reader.has_next() {
//         let entry = reader.next();

//         // println!("entry:");
//         // println!("fen {}", entry.pos.fen().unwrap());
//         // println!("uci move {:?}", entry.mv.as_uci());
//         // println!("score {}", entry.score);
//         // println!("ply {}", entry.ply);
//         // println!("result {}", entry.result);
//         // println!("\n");

//         i = i + 1;
//         // if i > 200 {
//         //     break;
//         // }
//     }

//     println!("read {} entries", i);
// }
