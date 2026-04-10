use crate::{Color};
use crate::board::Board;
use crate::position::Position;
use alloc::vec::Vec;


#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PieceType {
    
    General,
    Advisor,
    Elephant,
    Horse,
    Rook,
    Cannon,
    Pawn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Piece {
    pub piece_type: PieceType,
    pub pos: Position,
    pub color: Color,
}

impl Piece {
    pub const fn new(color: Color, piece_type: PieceType, pos: Position) -> Self {
        Self {piece_type, color, pos}
    }

    pub fn is_legal_move(&self, new_pos: Position, board: &Board) -> bool {
        // rule these out first as postion helpers don't check for legality
        if board.has_ally_piece(new_pos, self.color) || new_pos.is_off_board(){
            return false
        }
        match self.piece_type {
             PieceType::General => {
                let up = self.pos.pawn_up(self.color);
                let down = self.pos.pawn_down(self.color);
                let right = self.pos.next_right();
                let left = self.pos.next_left();

                if board.postions_match(new_pos, up) 
                || board.postions_match(new_pos, down) 
                || board.postions_match(new_pos, right) 
                || board.postions_match(new_pos, left) {
                    return true
                }
                else {false}
            }
             PieceType::Pawn => {
                let up = self.pos.pawn_up(self.color);
                let right = self.pos.next_right();
                let left = self.pos.next_left();

                if self.pos.is_past_river(self.color)  {
                    if board.postions_match(new_pos, up) 
                    || board.postions_match(new_pos, left) 
                    || board.postions_match(new_pos, right) {
                        return true
                    } else {false}
                } else {
                    if board.postions_match(new_pos, up) {
                        return true
                    } else {false}
                }   
            }
             PieceType::Cannon => {
                
            }
             PieceType::Rook => {
                
            }
             PieceType::Horse => {
                
            }
             PieceType::Advisor => {
                
            }
             PieceType::Elephant => {
                
            }
        }

    }
}