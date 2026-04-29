mod transparent;

use transparent::TransparentDrawTarget;
use embedded_graphics::{
    image::Image, mono_font::{MonoTextStyle,ascii::FONT_9X18_BOLD}, pixelcolor::Rgb888, 
    prelude::*, primitives::Rectangle, text::Text
};
use embedded_text::{
    alignment::{HorizontalAlignment, VerticalAlignment},
    style::TextBoxStyleBuilder,
    TextBox,
};
use embedded_graphics_simulator::{
    OutputSettings, SimulatorDisplay, SimulatorEvent, Window, 
};
use engine::{
    Color, GameResult, Move, board::Board, piece::{Piece, PieceType}, position::Position
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

fn drawboard(board: Board, display: &mut SimulatorDisplay<Rgb888>, window: &mut Window){
    let gameboard: Tga<Rgb888> = Tga::from_slice(include_bytes!("assets/board.tga")).unwrap();
    let game_board = Image::new(&gameboard, Point::new(0, 0));
        game_board.draw( display);
    let mut filter = TransparentDrawTarget {
        target: display,
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
    
    window.update(&display);
    
}
fn main() -> Result<(), core::convert::Infallible> {
    let mut display: SimulatorDisplay<Rgb888> = SimulatorDisplay::new(Size::new(1280, 1024));
    let mut window = Window::new("Xiangqi", &OutputSettings::default());

    let character_style = MonoTextStyle::new(&FONT_9X18_BOLD,Rgb888::WHITE);

    let textbox_style = TextBoxStyleBuilder::new()
        .alignment(HorizontalAlignment::Left)
        .vertical_alignment(VerticalAlignment::Top)
        .build();

    let mut board = Board::default();
    drawboard(board, &mut display,&mut window);
    
    let mut selected: Option<Piece> = None;
    let mut redraw:bool = false;

    'Game: loop {
    let turn_text = format!("Current Turn: {}", board.get_turn_color());
        TextBox::with_textbox_style(
            &turn_text,
            Rectangle::new(Point { x: 900, y: 90 }, Size { width: 270, height: 180}),
            character_style,
            textbox_style,
        ).draw(&mut display)?;
        for event in window.events() {
            match event {
                SimulatorEvent::Quit => break 'Game,
                
                SimulatorEvent::MouseButtonUp { point, .. } => {
                    if !(12 <= point.x) || !(point.x <= 821)
                    || !(12 <= point.y) || !(point.y <= 911) {
                        continue 'Game
                    }
                    let col:i8 = ((point.x - 12) / 90).try_into().unwrap();
                    let row:i8 = ((911 - point.y) / 90).try_into().unwrap();
                    let new_pos = Position::new(row, col);

                    match selected {
                        
                        Some(piece) => {
                            let m = Move::Piece(piece.pos, new_pos);
                            let result = board.play_move(m);
                            match result {
                                GameResult::Continuing(next_turn) => {
                                    board = next_turn; 
                                    selected = None;
                                    display.clear(Rgb888::BLACK).unwrap();
                                    redraw = true;
                                }
                                GameResult::IllegalMove(_m) => {selected = None; continue 'Game}
                                GameResult::Victory(_color) => {break 'Game}
                            }
                        }
                        None => {
                            if board.has_ally_piece(new_pos, board.get_turn_color()){
                                selected = Some(board.get_piece(new_pos).unwrap());
                            } else {continue 'Game}
                            
                        }
                    }
                }
                _ => {}
            }
        }
        //display.clear(Rgb888::BLACK).unwrap();
        if redraw {drawboard(board, &mut display, &mut window);}
    }
    Ok(())
}