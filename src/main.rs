use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::Result;
use walkdir::WalkDir;

use crate::binpack::BinpackBuilder;

mod binpack;
mod errors;
mod util;
mod wdl;

pub fn process_pgn_files<P: Into<PathBuf>, Q: Into<PathBuf>>(
    pgn_root: P,
    output_file: Q,
) -> Result<()> {
    let root = pgn_root.into();
    let output_file = output_file.into();

    let files = collect_pgn_files(&root)?;
    let total = files.len();

    for (i, pgn_file) in files.iter().enumerate() {
        println!("Processing file {}/{}: {:?}", i + 1, total, pgn_file);

        let builder = BinpackBuilder::new(&pgn_file, &output_file);

        if let Err(e) = builder.create_binpack() {
            println!("  Failed to create binpack: {:?}", e);
            return Err(e);
        }
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

        // Match .pgn or .pgn.gz files
        if name.ends_with(".pgn") || name.ends_with(".pgn.gz") {
            out.push(p.to_path_buf());
        }
    }

    Ok(out)
}

fn main() {
    let search_dir = env::args().nth(1).unwrap_or_else(|| {
        let current_dir = env::current_dir().expect("Failed to get current directory");
        current_dir.to_string_lossy().into_owned()
    });

    println!("Searching directory: {}", search_dir);

    // delete file if exists
    let output_path = std::path::Path::new("output.binpack");
    if output_path.exists() {
        std::fs::remove_file(output_path).expect("Failed to delete existing output.binpack");
    }

    process_pgn_files(&search_dir, "output.binpack").expect("Failed to process PGN files");

    println!("Binpack created successfully.");

    let filesize = std::fs::metadata("output.binpack")
        .expect("Failed to get metadata")
        .len();

    println!(
        "Output file size: {} ",
        human_bytes::human_bytes(filesize as f64)
    );
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
