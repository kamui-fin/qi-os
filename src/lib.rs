#![no_std]

extern crate alloc;
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

pub mod board;
pub mod piece;
pub mod position;


pub const RED: Color = Color::Red;
pub const BLACK: Color = Color::Black;



#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Color{
    Red,
    Black,
}

/// A color can be inverted using the `!` operator.
/// `!Color::White` becomes `Color::Black` and vice versa.
impl core::ops::Not for Color {
    type Output = Self;
    fn not(self) -> Self {
        match self {
            Self::Red => Self::Black,
            Self::Black => Self::Red,
        }
    }
}

