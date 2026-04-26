use crate::{piece::{Piece, PieceType}, position::Position};
extern crate alloc;
use super::*;
use core::panic;
use alloc::vec;
use alloc::vec::Vec;

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
        let mut board = Board::empty();
        
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
    full_move: u8,
}

impl Board {

    pub fn move_piece(&self, from: Position, to: Position) -> Self {
        let mut result = *self;

        if from.is_off_board() || to.is_off_board() {
            return result;
        }
        let from_point = from.to_board_index();
        let to_point = to.to_board_index();
        result.points[to_point] = Some(self.get_piece(from).expect("move_piece: from square should always contain a piece").move_to(to));    
        result.points[from_point] = None;
    
        result
    }

    #[inline]
    pub fn set_turn(&self, color:Color) -> Self{
        let mut result = *self;
        result.turn = color;
        result
    }

    #[inline] 
    pub fn set_full_move(&self, fm: u8) -> Self {
        let mut result = *self;
        result.full_move = fm;
        result
    }

    #[inline]
    pub fn get_full_move(&self) -> u8 {
        self.full_move
    }

    pub(crate) fn is_legal_move(&self, m: Move, player_color: Color) -> bool {
        match m {
            Move::Piece(from, to) => match self.get_piece(from) {
                Some(piece) => {
                    piece.is_legal_move(to, self)
                        && piece.color == player_color
                        && !self.move_piece(from, to).is_in_check(player_color)
                }
                _ => false,
            },
            Move::Resign => true,
        }
    }

    pub fn type_at_pos(&self, pos: Position) -> PieceType {
        self.points[self.pos_to_index(pos)].unwrap().piece_type

    }

    pub fn empty() -> Self {
        Self {
            points: [None; 90],
            turn: Color::Red, 
            full_move: 1,
        }
    }

    pub fn set_piece(&mut self, piece: Piece) {
        self.points[self.pos_to_index(piece.pos)] = Some(piece);
    }
    
    pub fn pos_to_index(&self, pos: Position) -> usize {
        (pos.get_row() as usize) * 9 + (pos.get_col() as usize)
    }
    
    pub fn get_piece(&self, pos: Position) -> Option<Piece> {
        self.points[self.pos_to_index(pos)]
    }

    //checks if point on board has ally piece 
    pub fn has_ally_piece(&self, pos: Position, self_color: Color) -> bool {
        match self.points[self.pos_to_index(pos)] {
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

    #[inline]
    fn get_current_player_color(&self) -> Color {
        self.turn
    }

    #[inline]
    pub fn get_legal_moves(&self) -> Vec<Move> {
        let mut result = vec![];
        let color = self.get_current_player_color();
        for p in &self.points {
            if let Some(piece) = p {
                if piece.color == color {
                    result.extend(piece.get_legal_moves(self))
                }
            }
        }

        result
    }

    #[inline]
    pub fn get_turn_color(&self) -> Color {
        self.turn
    }

    pub fn postions_match(&self, pos1: Position, pos2: Position) -> bool {
        (pos1.get_col() == pos2.get_col()) && (pos1.get_row() == pos2.get_row())
    }

    pub fn is_threatened(&self, pos: Position, ally_color: Color) -> bool {
        for point in self.points {
            if let Some(piece) = point {
                let point_pos = piece.pos;
                if !point_pos.is_orthogonal_to(pos)
                && !point_pos.is_diagonal_to(pos)
                && !point_pos.is_horse_move(pos)
                {
                    continue
                }
                if piece.color == ally_color {
                    continue
                }
                if piece.is_legal_move(pos, self) {
                    return true;
                }
            } else {continue}              
        }
        false
    }

    #[inline]
    pub fn change_turn(mut self) -> Self {
        self.turn = !self.turn;
        self
    }

    pub fn is_in_check(&self, color: Color) -> bool {
        self.is_threatened(self.get_general_pos(color), color)
    }

     pub fn is_checkmate(&self) -> bool {
        self.is_in_check(self.get_current_player_color()) && self.get_legal_moves().is_empty()
    }

    pub fn get_general_pos(&self, color: Color) -> Position {
        match color {
            Color::Red => {
                for pos in RED_PALACE {
                    if let Some(piece) = self.get_piece(pos){
                        if piece.piece_type == PieceType::General{
                            return pos
                        } else {continue} 
                    } else {continue}
                }
            }
            Color::Black => {
                for pos in BLACK_PALACE {
                    if let Some(piece) = self.get_piece(pos){
                        if piece.piece_type == PieceType::General{
                            return pos
                        } else {continue} 
                    } else {continue}
                }
            }
        }
        panic!()
    }

    pub fn is_stalemate(&self) -> bool {
         self.get_legal_moves().is_empty() && !self.is_in_check(self.get_current_player_color())
    }

    /// Play a move and confirm it is legal.
    pub fn play_move(&self, m: Move) -> GameResult {
        let current_color = self.get_turn_color();
        
        match m {
            Move::Piece(from, to) => {
                if self.is_legal_move(m, current_color) {
                    let next_turn = self.move_piece(from, to).change_turn();
                    if next_turn.is_checkmate() {
                        GameResult::Victory(current_color)
                    } else if next_turn.is_stalemate() {
                        GameResult::Victory(!current_color)
                    } else {
                        GameResult::Continuing(next_turn)
                    }
                } else {
                    GameResult::IllegalMove(m)
                }
            },
            Move::Resign => GameResult::Victory(!current_color),
        }
    }
}
