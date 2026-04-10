use super::Color;
use alloc::vec::Vec;


#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    row: i8,
    col: i8,
} 

impl Position {
    #[inline]
    pub const fn new(row: i8, col: i8) -> Self {
        Self { row, col }
    }

    /// Is this position a valid spot on the board?
    #[inline]
    pub fn is_on_board(&self) -> bool {
        !self.is_off_board()
    }

    /// Is this position NOT a valid spot on the board?
    #[inline]
    pub fn is_off_board(&self) -> bool {
        self.row < 0 || self.row > 8 || self.col < 0 || self.col > 9
    }

    /// Get the row number of the position.
    /// This can be any of 0, 1, 2, 3, 4, 5, 6, 7, 8, 9
    #[inline]
    pub fn get_row(&self) -> i8 {
        self.row
    }

    pub fn add_row(&self, step: i8) -> Self {
        let mut result = *self;
        result.row += step;
        result
    }

    pub fn add_col(&self, step: i8) -> Self {
        let mut result = *self;
        result.col += step;
        result
    }

    ///Get the column of the position 
    /// This can be any of 0,1,2,3,4,5,6,7
    #[inline]
    pub fn get_col(&self) -> i8 {
        self.col
    }

    pub fn next_right(&self) -> Self { 
        Self::new(self.row, self.col + 1)
    }

    pub fn next_left(&self) -> Self { 
        Self::new(self.row, self.col - 1)
    }
    
    pub fn next_above(&self) -> Self { 
        Self::new(self.row + 1, self.col)
    }
    
    pub fn next_below(&self) -> Self { 
        Self::new(self.row -1, self.col)
    }
    pub fn is_in_palace(&self, self_color: Color) -> bool {
        match self_color {
            Color::Red => self.col >=3 && self.col <= 5 && self.row <= 2,
            Color::Black => self.col >=3 && self.col <= 5 && self.row >=7 && self.row <= 9,
        }
    }
    
    //above and below change based on which side is moving, grid does not change 
    pub fn pawn_up(&self, ally_color: Color) -> Self {
        match ally_color {
            Color::Red => self.next_above(),
            Color::Black => self.next_below(),
        }
    }

    pub fn pawn_down(&self, ally_color: Color) -> Self {
        match ally_color {
            Color::Red => self.next_below(),
            Color::Black => self.next_above(),
        }
    }
    pub fn is_past_river(&self, self_color: Color) -> bool {
        match self_color {
            Color::Red => self.row >= 5,
            Color::Black => self.row <= 4,
        }
    }

    pub fn is_above(&self, other: Self) -> bool {
        self.row > other.row
    }

    pub fn is_below(&self, other: Self) -> bool {
        self.row < other.row
    }
    
    pub fn is_right_of(&self, other: Self) -> bool {
        self.col > other.col
    }

    pub fn is_left_of(&self, other: Self) -> bool {
        self.col < other.col
    }

    pub fn is_orthogonal_to(&self, other: Self) -> bool {
        (self.col == other.col) || (self.row == other.row)
    }

    pub fn is_diagonal_to(&self, other: Self) -> bool {
        // | x1 - x2 | = | y1 - y2 |
        ((self.col - other.col).abs() == (self.row - other.row).abs())
    }

    pub fn orthogonal_distance(&self, new_pos: Self) -> i8 {
        if self.is_above(new_pos) || self.is_below(new_pos) {
            return (self.row - new_pos.row).abs();
        } else {
            return (self.col - new_pos.col).abs()
        }
    }

    pub fn diagonal_distance(&self, new_pos: Self) -> i8 {
        if 
    }

    // orthogonal - horizontal and vertical
    // returns Vec with positons left, right, up or down from the current position
    pub fn orthogonals_to(&self, to_pos: Self) -> Vec<Self> {
        if !self.is_orthogonal_to(to_pos) {
            return Vec::new();
        }
        let mut row_step:i8 = 0;
        let mut col_step:i8 = 0;

        if self.is_above(to_pos){
            row_step = -1;
        } else if self.is_below(to_pos) {
            row_step = 1;
        } else if self.is_right_of(to_pos) {
            col_step = -1
        } else if self.is_left_of(to_pos) {
            col_step = 1
        }
        let accumulator = *self;
        let mut result = Vec::new();
        // iterate through the otthogonal distance between points and increase or decrease col/ row then push to Vec
        //just need to interate use '_'
        for _ in 0..self.orthogonal_distance(to_pos) {
            accumulator.add_col(col_step).add_row(row_step);
            result.push(accumulator);
        }
        result

    }

    

}