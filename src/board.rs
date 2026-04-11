use crate::{piece::{Piece, PieceType}, position::Position};
extern crate alloc;
use super::*;
use core::panic;

pub const MAX_MOVES: u8 = 17;

pub const RED_PALACE: [Position; 9] =
    [
    Position::new(0, 3), Position::new(1, 3), Position::new(2, 3),
    Position::new(0, 4), Position::new(1, 4), Position::new(2, 4),
    Position::new(0, 5), Position::new(1, 5), Position::new(2, 5),
    ];
pub const BLACK_PALACE: [Position; 9] = 
    [
    Position::new(7, 3), Position::new(8, 3), Position::new(9, 3),
    Position::new(7, 4), Position::new(8, 4), Position::new(9, 4),
    Position::new(7, 5), Position::new(8, 5), Position::new(9, 5),
    ];

impl Default for Board {
    fn default() -> Self {
        let board = Board::empty();
        
        // manually setting red pieces
        board.set_piece(Piece { piece_type: PieceType::Rook,     pos: Position::new(0, 0), color: RED });
        board.set_piece(Piece { piece_type: PieceType::Horse,    pos: Position::new(0, 1), color: RED });
        board.set_piece(Piece { piece_type: PieceType::Elephant, pos: Position::new(0, 2), color: RED });
        board.set_piece(Piece { piece_type: PieceType::Advisor,  pos: Position::new(0, 3), color: RED });
        board.set_piece(Piece { piece_type: PieceType::General,  pos: Position::new(0, 4), color: RED });
        board.set_piece(Piece { piece_type: PieceType::Advisor,  pos: Position::new(0, 5), color: RED });
        board.set_piece(Piece { piece_type: PieceType::Elephant, pos: Position::new(0, 6), color: RED });
        board.set_piece(Piece { piece_type: PieceType::Horse,    pos: Position::new(0, 7), color: RED });
        board.set_piece(Piece { piece_type: PieceType::Rook,     pos: Position::new(0, 8), color: RED });
        board.set_piece(Piece { piece_type: PieceType::Cannon,   pos: Position::new(2, 1), color: RED });
        board.set_piece(Piece { piece_type: PieceType::Cannon,   pos: Position::new(2, 7), color: RED });
        board.set_piece(Piece { piece_type: PieceType::Pawn,     pos: Position::new(3, 0), color: RED });
        board.set_piece(Piece { piece_type: PieceType::Pawn,     pos: Position::new(3, 2), color: RED });
        board.set_piece(Piece { piece_type: PieceType::Pawn,     pos: Position::new(3, 4), color: RED });
        board.set_piece(Piece { piece_type: PieceType::Pawn,     pos: Position::new(3, 6), color: RED });
        board.set_piece(Piece { piece_type: PieceType::Pawn,     pos: Position::new(3, 8), color: RED });

        // manually setting black pieces
        board.set_piece(Piece { piece_type: PieceType::Rook,     pos: Position::new(9, 0), color: BLACK });
        board.set_piece(Piece { piece_type: PieceType::Horse,    pos: Position::new(9, 1), color: BLACK });
        board.set_piece(Piece { piece_type: PieceType::Elephant, pos: Position::new(9, 2), color: BLACK });
        board.set_piece(Piece { piece_type: PieceType::Advisor,  pos: Position::new(9, 3), color: BLACK });
        board.set_piece(Piece { piece_type: PieceType::General,  pos: Position::new(9, 4), color: BLACK });
        board.set_piece(Piece { piece_type: PieceType::Advisor,  pos: Position::new(9, 5), color: BLACK });
        board.set_piece(Piece { piece_type: PieceType::Elephant, pos: Position::new(9, 6), color: BLACK });
        board.set_piece(Piece { piece_type: PieceType::Horse,    pos: Position::new(9, 7), color: BLACK });
        board.set_piece(Piece { piece_type: PieceType::Rook,     pos: Position::new(9, 8), color: BLACK });
        board.set_piece(Piece { piece_type: PieceType::Cannon,   pos: Position::new(7, 1), color: BLACK });
        board.set_piece(Piece { piece_type: PieceType::Cannon,   pos: Position::new(7, 7), color: BLACK });
        board.set_piece(Piece { piece_type: PieceType::Pawn,     pos: Position::new(6, 0), color: BLACK });
        board.set_piece(Piece { piece_type: PieceType::Pawn,     pos: Position::new(6, 2), color: BLACK });
        board.set_piece(Piece { piece_type: PieceType::Pawn,     pos: Position::new(6, 4), color: BLACK });
        board.set_piece(Piece { piece_type: PieceType::Pawn,     pos: Position::new(6, 6), color: BLACK });
        board.set_piece(Piece { piece_type: PieceType::Pawn,     pos: Position::new(6, 8), color: BLACK });

        board

    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Board {
    
    points: [Option<Piece>; 90],
    turn: Color,
}

impl Board {

    pub fn type_at_pos(&self, pos: Position) -> PieceType {
        self.points[self.pos_to_index(pos)].unwrap().piece_type

    }

    pub fn empty() -> Self {
        Self {
            points: [None; 90],
            turn: Color::Red, 
        }
    }

    pub fn set_piece(mut self, piece: Piece) -> Self {
        self.points[self.pos_to_index(piece.pos)] = Some(piece);

        self
    }
    
    pub fn pos_to_index(&self, pos: Position) -> usize {
        (pos.get_row() as usize) * 9 + (pos.get_col() as usize)
    }
    
    pub fn get_piece(&self, pos: Position) -> Option<Piece> {
        self.points[self.pos_to_index(pos)]
    }

    //checks if point on board has ally piece 
    pub fn has_ally_piece(&self, pos: Position, self_color: Color) -> bool {
        match  self.points[self.pos_to_index(pos)] {
            Some(piece) if piece.color == self_color => true,
            _ => false,
            
        }
    }

    pub fn has_enemy_piece(&self, pos: Position, self_color: Color) -> bool {
        match  self.points[self.pos_to_index(pos)] {
            Some(piece) if piece.color != self_color => true,
            _ => false,
            
        }
    }

    pub fn has_piece(&self, pos:Position) -> bool {
        self.get_piece(pos).is_some()
    }

    pub fn has_no_piece(&self, pos: Position) -> bool{
        self.get_piece(pos).is_none()
    }

    pub fn get_legal_moves(&self, pos: Position) -> MoveList {

    }

    pub fn get_turn_color(&self) -> Color {
        self.turn
    }

    pub fn postions_match(&self, pos1: Position, pos2: Position) -> bool {
        (pos1.get_col() == pos2.get_col()) && (pos1.get_row() == pos2.get_row())
    }

    pub fn get_general_pos(&self, color: Color) -> Position {
        match color {
            Color::Red => {
                for pos in RED_PALACE {
                    if self.type_at_pos(pos) == PieceType::General{
                        return pos
                    }
                }
            }
            Color::Black => {
                for pos in BLACK_PALACE {
                    if self.type_at_pos(pos) == PieceType::General{
                        return pos
                    }
                }
            }
        }
        panic!()
    }  
} 
