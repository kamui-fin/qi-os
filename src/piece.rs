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
                let traveling: Vec<Position>;
                match self.color {
                    Color::Red => {
                        traveling = new_pos.orthogonals_to(Position::new(9, new_pos.get_col())); 
                    }
                    Color::Black => {
                        traveling = new_pos.orthogonals_to(Position::new(0, new_pos.get_col()));
                    }
                }
                
                for point in traveling.iter() {
                    if board.has_piece(*point) {
                        if board.type_at_pos(*point) == PieceType::General {
                            return false
                        } else {break}
                    }
                }
                let up = self.pos.pawn_up(self.color);
                let down = self.pos.pawn_down(self.color);
                let right = self.pos.next_right();
                let left = self.pos.next_left();
                if board.postions_match(new_pos, up) 
                || board.postions_match(new_pos, down) 
                || board.postions_match(new_pos, right) 
                || board.postions_match(new_pos, left) {
                    return true
                } else {return false}
            }

             PieceType::Pawn => {
                let up = self.pos.pawn_up(self.color);

                if self.pos.is_past_river(self.color)  {
                    let right = self.pos.next_right();
                    let left = self.pos.next_left();

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
                if !self.pos.is_orthogonal_to(new_pos) {
                    return false;
                }
                if board.has_enemy_piece(new_pos, self.color){
                    let mut jmp_count = 0;
                    let mut traveling: Vec<Position> = self.pos.orthogonals_to(new_pos);
                    traveling.pop();

                    for point in traveling.iter() {
                        if board.has_piece(*point) {
                            jmp_count += 1;
                        } else {continue;}
                    }
                    if jmp_count > 1{
                        return false;
                    } else {return true}
                } else {
                    let mut traveling: Vec<Position> = self.pos.orthogonals_to(new_pos);
                    traveling.pop();

                    for point in traveling.iter() {
                        if board.has_piece(*point) {
                            return false;
                        } else {continue;}
                    }
                    return true;
                }
                
            }
             PieceType::Rook => {
                //check orthagonals_to position final
                //check if any of the squares inbetween final have a piece
                // if so move is not legal
                // else move is legal
                if !self.pos.is_orthogonal_to(new_pos) {
                    return false;
                }
                let mut traveling: Vec<Position> = self.pos.orthogonals_to(new_pos);
                traveling.pop();

                for point in traveling.iter() {
                    if board.has_piece(*point) {
                        return false;
                    }
                }
                return true;
            }
            // is this not the most elegant code the worlds ever seen lmao
             PieceType::Horse => {
                
                if !self.pos.is_horse_move(new_pos) {
                    return false
                }
                
                let up = self.pos.next_above();
                let down = self.pos.next_below();
                let right = self.pos.next_right();
                let left = self.pos.next_left();

                if new_pos.is_above(self.pos) {
                    if (new_pos.get_row() - self.pos.get_row()) > 1 {
                        if board.has_piece(up){
                            return false
                        } else {return true}
                    } else {
                        if new_pos.is_right_of(self.pos) {
                            if board.has_piece(right){
                                return false
                            } else {return true}
                        } else {
                            if board.has_piece(left){
                                return false
                            } else {return true}
                        }
                    }
                } else {
                    if (self.pos.get_row() - new_pos.get_row()) > 1 {
                        if board.has_piece(down){
                            return false
                        } else {return true}
                    } else {
                        if new_pos.is_right_of(self.pos) {
                            if board.has_piece(right){
                                return false
                            } else {return true}
                        } else {
                            if board.has_piece(left){
                                return false
                            } else {return true}
                        }
                    }
                }
            }
             PieceType::Advisor => {
                if !new_pos.is_diagonal_to(self.pos) {
                    return false
                }
                if !new_pos.is_in_palace(self.color) {
                    return false
                }
                let up = new_pos.pawn_up(self.color);
                let down = new_pos.pawn_down(self.color);
                let uright = up.next_right();
                let uleft = up.next_left();
                let dright = down.next_right();
                let dleft = down.next_left();

                if board.postions_match(self.pos, uright)
                || board.postions_match(self.pos, dright)
                || board.postions_match(self.pos, dleft)
                || board.postions_match(self.pos, uleft)
                {
                    return true
                } else {return false}
            }
                
             PieceType::Elephant => {
                if new_pos.is_past_river(self.color) 
                || !self.pos.is_diagonal_to(new_pos)
                || self.pos.diagonal_distance(new_pos) < 2 {
                    return false
                }
                let mut traveling: Vec<Position> = self.pos.diagonals_to(new_pos);
                traveling.pop();

                for point in traveling.iter() {
                    if board.has_piece(*point) {
                        return false;
                    } else {continue;}
                }
                return true;               
                
            }
        }

    }

    //pub fn get_legal_moves
}