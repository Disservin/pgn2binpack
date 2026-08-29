use std::io::{Read, Seek};

use anyhow::Result;

use super::{render_board, ViewSession};

pub(super) fn dump_frames<T: Read + Seek>(session: &mut ViewSession<T>) -> Result<()> {
    let mut index = 0usize;

    while session.ensure_loaded(index)? {
        let frame = &session.frames[index];
        println!("position {}", index + 1);
        println!("game {} move {}", frame.game_index, frame.position_in_game);
        println!("fen {}", frame.fen);
        println!("uci move {}", frame.uci_move);
        println!("score {}", frame.score);
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
