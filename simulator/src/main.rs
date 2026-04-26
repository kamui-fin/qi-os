mod transparent;
use transparent::TransparentDrawTarget;
use embedded_graphics::{
    image::Image, 
    pixelcolor::Rgb888, 
    prelude::*
};
use embedded_graphics_simulator::{OutputSettingsBuilder, SimulatorDisplay, Window};
use engine::{
    Color, board::Board, piece::{Piece, PieceType}, position::Position
};
use tinytga::Tga;

pub fn get_img(piece: Piece) -> &'static [u8]{
        match piece.piece_type {
            PieceType::Advisor => {
               match piece.color {
                Color::Red   => {include_bytes!("assets/Chinese-Advisor-Red.tga")}
                Color::Black => {include_bytes!("assets/Chinese-Advisor-Black.tga")}
               }
            }
            PieceType::Cannon => {
                match piece.color {
                Color::Red   => {include_bytes!("assets/Chinese-Cannon-Red.tga")}
                Color::Black => {include_bytes!("assets/Chinese-Cannon-Black.tga")}
               }
            }
            PieceType::Elephant => {
                match piece.color {
                Color::Red   => {include_bytes!("assets/Chinese-Elephant-Red.tga")}
                Color::Black => {include_bytes!("assets/Chinese-Elephant-Black.tga")}
               }
            }
            PieceType::General => {
                match piece.color {
                Color::Red   => {include_bytes!("assets/Chinese-King-Red.tga")}
                Color::Black => {include_bytes!("assets/Chinese-King-Black.tga")}
               }
            }
            PieceType::Horse => {
                match piece.color {
                Color::Red   => {include_bytes!("assets/Chinese-Horse-Red.tga")}
                Color::Black => {include_bytes!("assets/Chinese-Horse-Black.tga")}
               }
            }
            PieceType::Pawn => {
                match piece.color {
                Color::Red   => {include_bytes!("assets/Chinese-Pawn-Red.tga")}
                Color::Black => {include_bytes!("assets/Chinese-Pawn-Black.tga")}
               }
            }
            PieceType::Rook => {
                match piece.color {
                Color::Red   => {include_bytes!("assets/Chinese-Rook-Red.tga")}
                Color::Black => {include_bytes!("assets/Chinese-Rook-Black.tga")}
               }
            }
        }
    }

fn main() -> Result<(), core::convert::Infallible> {
    let mut display: SimulatorDisplay<Rgb888> = SimulatorDisplay::new(Size::new(1280, 1024));
    let tga1: Tga<Rgb888> = Tga::from_slice(include_bytes!("assets/board.tga")).unwrap();
    let game_board = Image::new(&tga1, Point::new(0, 0));
        game_board.draw(&mut display)?;

    let board = Board::default();
    let mut filter = TransparentDrawTarget {
        target: &mut display,
        transparent_color: Rgb888::BLACK,
    };
    
    let mut r:i32 = 0;
    let mut c:i32 = 0;
    while r < 10 {
        while c < 9 {
            let piece = board.get_piece(Position::new(r.try_into().unwrap(), c.try_into().unwrap()));
            match piece {
                Some(piece) => {
                    let img: Tga<Rgb888> = Tga::from_slice(get_img(piece)).unwrap();
                    let y:i32 = 822 - (r * 90);
                    let x:i32 = 12 + (c * 90);
                    let piece = Image::new(&img, Point::new(x, y));
                    piece.draw(&mut filter).unwrap();
                }
                _ => {}
            }
            c +=1
        }
        c = 0;
        r +=1;
    }
    
    let output_settings = OutputSettingsBuilder::new().build();
    Window::new("Xiangqi", &output_settings).show_static(&display);

    Ok(())
}