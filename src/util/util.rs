use sfbinpack::chess::{
    color::Color as SfColor,
    coords::Square as SfSquare,
    piece::Piece as SfPiece,
    piecetype::PieceType as SfPieceType,
    r#move::{Move as SfMove, MoveType as SfMoveType},
};

use shakmaty::{Move, Role, Square};

pub fn parse_eval_cp(comment: &str) -> Result<Option<i16>, &'static str> {
    if (comment == "book") || (comment == "Book") || (comment == "No result") {
        return Ok(None);
    }
    // Matches examples like:
    // {+1.01/26 1.2s} {-0.34/15} {+0.00} {-M21/32 0.5s} {+M21/32 0.5s}
    for part in comment.split(|c: char| c.is_whitespace() || c == '{' || c == '}') {
        if part.is_empty() {
            continue;
        }
        let p = part.trim_matches(|c| c == '{' || c == '}');

        // mate
        if p.starts_with("+M") || p.starts_with("-M") {
            let sign = if p.starts_with("+M") { 1 } else { -1 };
            if let Ok(_) = p[2..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<i32>()
            {
                return Ok(Some(32000 as i16 * sign));
            }
        } else {
            let num = p.split('/').next().unwrap_or(p);
            if let Some(first) = num.chars().next() {
                if first == '+' || first == '-' || first.is_ascii_digit() {
                    let mut cleaned = String::new();
                    for ch in num.chars() {
                        if ch.is_ascii_digit() || ch == '+' || ch == '-' || ch == '.' {
                            cleaned.push(ch);
                        } else {
                            break;
                        }
                    }
                    if cleaned == "+" || cleaned == "-" {
                        continue;
                    }
                    if let Ok(f) = cleaned.parse::<f32>() {
                        return Ok(Some((f * 100.0).round() as i16));
                    }
                }
            }
        }
    }

    Err("Unable to parse evaluation")
}

pub fn convert_move(mv: &Move, color: SfColor) -> SfMove {
    let from_idx = square_index(mv.from().unwrap());
    let to_idx = square_index(mv.to());

    let mut move_type = SfMoveType::Normal;
    let mut promo_piece = SfPiece::none();

    if mv.is_en_passant() {
        move_type = SfMoveType::EnPassant;
    } else if mv.is_castle() {
        move_type = SfMoveType::Castle;
    } else if let Some(promo) = mv.promotion() {
        move_type = SfMoveType::Promotion;
        promo_piece = match promo {
            Role::Queen => SfPiece::new(SfPieceType::Queen, color),
            Role::Rook => SfPiece::new(SfPieceType::Rook, color),
            Role::Bishop => SfPiece::new(SfPieceType::Bishop, color),
            Role::Knight => SfPiece::new(SfPieceType::Knight, color),
            Role::King | Role::Pawn => SfPiece::none(),
        };
    }

    SfMove::new(
        SfSquare::new(from_idx as u32),
        SfSquare::new(to_idx as u32),
        move_type,
        promo_piece,
    )
}

// a1 = 0 indexing
fn square_index(sq: Square) -> u32 {
    let file = sq.file().char() as u32 - 'a' as u32;
    let rank = sq.rank().char() as u32 - '1' as u32;
    rank * 8 + file
}
