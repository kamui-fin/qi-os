#![no_std]
use core::fmt;

use crate::position::Position;

extern crate alloc;

pub mod board;
use board::Board;
pub mod piece;
pub mod position;
pub mod fen;

pub const RED: Color = Color::Red;
pub const BLACK: Color = Color::Black;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum GameResult {
    /// The game is not finished, and the game is still in play.
    Continuing(Board),
    
    Victory(Color),
    
    //Draw,
    
    IllegalMove(Move),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Color{
    Red,
    Black,
}

/// A color can be inverted using the `!` operator.
impl core::ops::Not for Color {
    type Output = Self;
    fn not(self) -> Self {
        match self {
            Self::Red => Self::Black,
            Self::Black => Self::Red,
        }
    }
}
impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Color::Red => write!(f, "Red"),
            Color::Black => write!(f, "Black"),
        }
    }
}
impl Color {
    pub fn as_str(&self) -> &'static str {
        match self {
            Color::Red => "Red",
            Color::Black => "Black",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Move{

    Piece(Position, Position),

    Resign,
}

#[cfg(test)]
mod testpositions;
