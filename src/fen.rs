use crate::board::Board;
use super::*;
use crate::piece::Piece;
use crate::position::Position;


///TODO: implement halfmove clock logic for effective and progress rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardParseError {
    /// The FEN string does not contain all required fields.
    FenTooShort,

    /// The side-to-move field is missing.
    MissingSideToMove,

    /// The fullmove number field is missing.
    MissingFullmoveNumber,

    /// A rank in the board description does not contain exactly 8 squares.
    InvalidRowLength,

    /// The board layout is invalid or malformed.
    InvalidBoardLayout,

    /// An invalid piece character was found in the board layout.
    InvalidPiece(PieceParseError),

    /// The side-to-move field is invalid (expected `'w'` or `'b'`).
    InvalidSideToMove,

    /// The fullmove number is not a valid number.
    InvalidFullmoveNumber,

    /// The fullmove number is zero or negative.
    FullmoveMustBePositive,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PieceParseError {
    /// The character does not correspond to any valid piece.
    InvalidChar(char),
}

impl core::str::FromStr for Board {
    type Err = BoardParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut fen_iter: core::str::SplitWhitespace<'_> = s.split_whitespace();

        let board_str: &str = fen_iter.next().ok_or(BoardParseError::FenTooShort)?;
        let side_str: &str = fen_iter.next().ok_or(BoardParseError::MissingSideToMove)?;
        let fullmove_str: &str = fen_iter.nth(3).ok_or(BoardParseError::MissingFullmoveNumber)?;

        let mut row = 9;
        let mut col = 0;
        let mut board: Board = Self::empty();
        let mut count: i32 = 0;

        for token in board_str.chars() {
            match token {
                '/' => {
                    if count != 9 {
                        return Err(BoardParseError::InvalidRowLength);
                    };

                    row -= 1;
                    col = 0;
                    count = 0;
                }
                '1'..='9' => {
                    for _ in '1'..=token {
                        col += 1;
                        count += 1;
                    }
                }
                _ => {
                    board.set_piece(
                        Piece::try_from(token).map_err(BoardParseError::InvalidPiece)?.move_to(Position::new(row, col)),
                    ); 
                    col += 1;
                    count += 1;
                }
            }
        }

        if count != 9 {
            return Err(BoardParseError::InvalidBoardLayout);
        }
        
        let color_to_move = match side_str {
            "w" => {
                Color::Red
            }
            "b" => Color::Black,
            _ => return Err(BoardParseError::InvalidSideToMove),
        }; 
        board.set_turn(color_to_move);

        let fullmove = fullmove_str
            .parse::<u8>()
            .map_err(|_| BoardParseError::InvalidFullmoveNumber)?;
        if fullmove == 0 {
            return Err(BoardParseError::FullmoveMustBePositive);
        }

        board.set_full_move(fullmove);

        Ok(board)
    }
}