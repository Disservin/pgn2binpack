use std::collections::HashSet;
use std::io::{BufReader, ErrorKind, Read, Seek};

use anyhow::Result;
use sfbinpack::CompressedTrainingDataEntryReader;
use shakmaty::{
    fen::Fen, uci::UciMove, zobrist::Zobrist64, zobrist::ZobristHash, CastlingMode, Chess,
    EnPassantMode, Position,
};
use viriformat::dataformat::Game as ViriGame;

use crate::cli::Backend;

pub fn unique_positions_from_file<T: Read + Seek>(
    file: T,
    limit: Option<usize>,
    backend: Backend,
) -> Result<u64> {
    match backend {
        Backend::Sfbinpack => unique_sf(file, limit),
        Backend::Viriformat => unique_viriformat(file, limit),
    }
}

fn unique_sf<T: Read + Seek>(file: T, limit: Option<usize>) -> Result<u64> {
    let mut reader = CompressedTrainingDataEntryReader::new(file)?;
    let mut position = Chess::default();
    let mut unique: HashSet<u64> = HashSet::new();
    let mut new_game = true;
    let mut count: usize = 0;

    while reader.has_next() {
        let entry = reader.next();

        if new_game {
            let fen = entry.pos.fen().expect("entry has FEN");
            let fen = Fen::from_ascii(fen.as_bytes()).expect("invalid FEN");
            position = fen
                .into_position(CastlingMode::Standard)
                .expect("invalid position");
            new_game = false;
        }

        let hash = position.zobrist_hash::<Zobrist64>(EnPassantMode::Legal);
        unique.insert(hash.0);

        if reader.has_next() && reader.is_next_entry_continuation() {
            let uci: UciMove = entry.mv.as_uci().parse().expect("invalid UCI move");
            let m = uci.to_move(&position).expect("illegal move in stream");
            position.play_unchecked(m);
        } else {
            new_game = true;
        }

        count += 1;
        if let Some(limit) = limit {
            if count >= limit {
                break;
            }
        }
    }

    Ok(unique.len() as u64)
}

fn unique_viriformat<T: Read + Seek>(file: T, limit: Option<usize>) -> Result<u64> {
    let mut reader = BufReader::new(file);
    let mut unique: HashSet<u64> = HashSet::new();
    let mut processed = 0usize;

    loop {
        match ViriGame::deserialise_from(&mut reader, Vec::new()) {
            Ok(game) => {
                let (board, _, _, _) = game.initial_position.unpack();
                let fen_str = board.to_string();
                let fen = Fen::from_ascii(fen_str.as_bytes()).map_err(|_| {
                    anyhow::anyhow!("invalid FEN in viriformat stream: {}", fen_str)
                })?;
                let mut position: Chess =
                    fen.into_position(CastlingMode::Standard).map_err(|_| {
                        anyhow::anyhow!("unable to convert FEN to position: {}", fen_str)
                    })?;

                for (mv, _) in &game.moves {
                    let hash = position.zobrist_hash::<Zobrist64>(EnPassantMode::Legal);
                    unique.insert(hash.0);

                    processed += 1;
                    if let Some(limit) = limit {
                        if processed >= limit {
                            return Ok(unique.len() as u64);
                        }
                    }

                    let uci_string = mv.display(false).to_string();
                    let uci: UciMove = uci_string.parse()?;
                    let chess_move = uci.to_move(&position)?;
                    position.play_unchecked(chess_move);
                }
            }
            Err(err) if err.kind() == ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err.into()),
        }
    }

    Ok(unique.len() as u64)
}
