use std::{
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::mpsc,
};

use anyhow::Result;
use tempfile::NamedTempFile;
use walkdir::WalkDir;

pub fn collect_pgn_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        if is_pgn_file(path) {
            files.push(path.to_path_buf());
        }
    }

    files.sort();
    Ok(files)
}

fn is_pgn_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|s| {
            let lower = s.to_ascii_lowercase();
            lower == "pgn" || path.to_str().map_or(false, |p| p.ends_with(".pgn.gz"))
        })
        .unwrap_or(false)
}

pub fn create_temp_file() -> Result<(File, PathBuf)> {
    let tmp = NamedTempFile::with_prefix("pgn2binpack_")?;
    Ok(tmp.keep()?)
}

pub fn write_output(path: &Path, rx: mpsc::Receiver<Vec<u8>>) -> Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    for buffer in rx {
        writer.write_all(&buffer)?;
    }

    writer.flush()?;
    Ok(())
}

pub fn concatenate_files(parts: &[PathBuf], output: &Path) -> Result<()> {
    let file = File::create(output)?;

    let mut writer = BufWriter::new(file);

    for part in parts {
        let mut input = File::open(part)?;
        std::io::copy(&mut input, &mut writer)?;
        let _ = std::fs::remove_file(part);
    }

    writer.flush()?;
    Ok(())
}
