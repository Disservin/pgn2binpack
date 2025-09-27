use std::collections::HashSet;
use std::io::{Read, Seek};

use anyhow::Result;
use sfbinpack::CompressedTrainingDataEntryReader;
use shakmaty::{
    fen::Fen, uci::UciMove, zobrist::Zobrist64, zobrist::ZobristHash, CastlingMode, Chess,
    EnPassantMode, Position,
};

pub fn unique_positions_from_file<T: Read + Seek>(file: T, limit: Option<usize>) -> Result<u64> {
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
