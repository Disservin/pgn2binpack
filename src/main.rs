use anyhow::Result;
use clap::Parser;

mod analytics;
mod binpack;
mod cli;
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

    if let Some(path) = cli.view {
        let file = std::fs::File::options()
            .read(true)
            .write(false)
            .create(false)
            .open(&path)?;
        let t0 = std::time::Instant::now();
        analytics::view::view_entries(file, cli.limit)?;
        println!("Completed in {:.2?}", t0.elapsed());
    }

    if let Some(rescore_input) = cli.rescore.as_ref() {
        if cli.engine.is_none() {
            anyhow::bail!("--engine is required when using --rescore");
        }
        if cli.rescore_output.is_none() {
            anyhow::bail!("--rescore-output is required when using --rescore");
        }

        if !rescore_input.exists() {
            anyhow::bail!("Rescore input file does not exist: {:?}", rescore_input);
        }

        let output_path = cli.rescore_output.as_ref().unwrap().as_path();
        if output_path.exists() {
            if !cli.force {
                anyhow::bail!(
                    "Rescore output file already exists: {:?}. Use --force to overwrite.",
                    output_path
                );
            }
            std::fs::remove_file(output_path)?;
        }

        let engine_path = cli.engine.as_ref().unwrap().as_path();

        let input_file = std::fs::File::options()
            .read(true)
            .write(false)
            .create(false)
            .open(rescore_input)?;
        let output_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(output_path)?;

        let nodes = cli.rescore_nodes.unwrap_or(5000).max(1);
        println!(
            "Rescoring {} -> {}",
            rescore_input.display(),
            output_path.display()
        );
        println!("Engine: {}", engine_path.display());
        println!("Search nodes: {}", nodes);

        let t0 = std::time::Instant::now();
        let count = analytics::rescore::rescore_binpack(
            input_file,
            output_file,
            engine_path,
            nodes,
            cli.limit,
        )?;
        println!("Rescored entries: {}", count);
        println!("Completed in {:.2?}", t0.elapsed());
    }

    Ok(())
}
