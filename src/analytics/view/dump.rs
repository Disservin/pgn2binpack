use std::io::{Read, Seek};

use anyhow::Result;

use super::ViewSession;

pub(super) fn dump_frames<T: Read + Seek>(session: &mut ViewSession<T>) -> Result<()> {
    let mut index = 0usize;

    while session.ensure_loaded(index)? {
        let frame = &session.frames[index];
        println!("position {}", index + 1);
        println!("game {} move {}", frame.game_index, frame.position_in_game);
        println!("fen {}", frame.fen);
        println!("uci move {}", frame.uci_move);
        println!("score {}", frame.score);
        if let Some(detail) = &frame.score_detail {
            println!("score detail {}", detail);
        }
        println!("ply {}", frame.ply);
        println!("result {}", frame.result);
        println!();

        index += 1;
    }

    if index == 0 {
        println!("No positions found.");
    }

    Ok(())
}
