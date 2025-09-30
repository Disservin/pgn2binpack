use shakmaty::{Chess, Position, Role};

// Reverse of Stockfish to_cp(): internal_value = external_cp * a / 100
pub fn external_cp_to_internal(external_cp: i32, pos: &Chess) -> i16 {
    external_cp_to_internal_mat(external_cp, material_count(pos))
}

pub fn external_cp_to_internal_mat(external_cp: i32, material: i32) -> i16 {
    if external_cp.abs() >= 29000 {
        return external_cp.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    }
    let a = win_rate_a(material);
    let internal = (external_cp as f64) * a / 100.0;
    let rounded = if internal.is_sign_positive() {
        (internal + 0.5).floor()
    } else {
        (internal - 0.5).ceil()
    };
    rounded.clamp(i16::MIN as f64, i16::MAX as f64) as i16
}

// Compute material like Stockfish: sum piece values (P=1 N=3 B=3 R=5 Q=9) both sides.
fn material_count(pos: &Chess) -> i32 {
    let board = pos.board();
    fn count(board: &shakmaty::Board, role: Role) -> i32 {
        board.by_role(role).count() as i32
    }
    count(board, Role::Pawn)
        + 3 * count(board, Role::Knight)
        + 3 * count(board, Role::Bishop)
        + 5 * count(board, Role::Rook)
        + 9 * count(board, Role::Queen)
}

// Polynomial producing 'a' parameter (b unused here) as per WinRateParams.
fn win_rate_a(material: i32) -> f64 {
    let m = (material.clamp(17, 78) as f64) / 58.0;
    // Coefficients from Stockfish
    const AS: [f64; 4] = [-13.50030198, 40.92780883, -36.82753545, 386.83004070];
    (((AS[0] * m + AS[1]) * m + AS[2]) * m) + AS[3]
}
