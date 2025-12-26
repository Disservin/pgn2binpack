use std::io::{BufReader, ErrorKind, Read, Seek};

use anyhow::{Context, Result};
use sfbinpack::CompressedTrainingDataEntryReader;
use viriformat::dataformat::Game as ViriGame;

use crate::cli::Backend;

pub fn view_entries<T: Read + Seek>(file: T, limit: Option<usize>, backend: Backend) -> Result<()> {
    match backend {
        Backend::Sfbinpack => view_sf(file, limit),
        Backend::Viriformat => view_viriformat(file, limit),
    }
}

fn view_sf<T: Read + Seek>(file: T, limit: Option<usize>) -> Result<()> {
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

fn view_viriformat<T: Read + Seek>(file: T, limit: Option<usize>) -> Result<()> {
    let mut reader = BufReader::new(file);
    let mut processed = 0usize;

    loop {
        match ViriGame::deserialise_from(&mut reader, Vec::new()) {
            Ok(game) => {
                let (mut board, _, _, _) = game.initial_position.unpack();
                for (mv, eval) in &game.moves {
                    println!("fen {}", board.to_string());
                    println!("uci move {}", mv.display(false));
                    println!("score {}", eval.get());
                    println!("ply {}", board.ply());
                    println!("result {:?}", game.outcome());
                    println!("\n");

                    processed += 1;
                    if let Some(limit) = limit {
                        if processed >= limit {
                            return Ok(());
                        }
                    }

                    board
                        .make_move_simple(*mv)
                        .then_some(())
                        .context("illegal move in viriformat stream")?;
                }
            }
            Err(err) if err.kind() == ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err.into()),
        }
    }

    Ok(())
}
