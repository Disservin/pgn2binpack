use std::io::{Read, Seek, Write};

use anyhow::Result;
use sfbinpack::CompressedTrainingDataEntryReader;

pub fn view_entries<T: Write + Read + Seek>(file: T, limit: Option<usize>) -> Result<()> {
    let mut reader = CompressedTrainingDataEntryReader::new(file)?;
    let mut i: usize = 0;

    while reader.has_next() {
        let entry = reader.next();

        println!("fen {}", entry.pos.fen().unwrap());
        println!("uci move {:?}", entry.mv.as_uci());
        println!("score {}", entry.score);
        println!("ply {}", entry.ply);
        println!("result {}", entry.result);
        println!("\n");

        i = i + 1;

        if limit.is_some() && i >= limit.unwrap() {
            break;
        }
    }

    Ok(())
}
