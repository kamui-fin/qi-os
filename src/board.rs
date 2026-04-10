use crate::{piece::Piece, position::Position};
extern crate alloc;
use alloc::vec::Vec;


use super::*;
use core::{cmp::Ordering, fmt::Pointer};

pub const MAX_MOVES: u8 = 17;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Board {
    
    points: [Option<Piece>; 90],
    turn: Color,
}

impl Board{

    pub fn empty() -> Self {
        Self {
            points: [None; 90],
            turn: Color::Red,
        }
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

}   
