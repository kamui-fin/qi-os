mod transparent;
use core::str::FromStr;
use transparent::TransparentDrawTarget;
use embedded_graphics::{
    image::Image, mono_font::{MonoTextStyle,ascii::FONT_9X18_BOLD}, pixelcolor::Rgb888, 
    prelude::*, primitives::{Rectangle, Circle, PrimitiveStyle},
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
    Color, GameResult, Move, board::{self, Board}, piece::{Piece, PieceType}, position::Position, fen
};
use tinytga::Tga;

pub fn get_img(piece: Piece, piece_images:PieceImages) -> Tga<Rgb888>{
        match piece.piece_type {
            PieceType::Advisor => {
               match piece.color {
                Color::Red   => {piece_images.red_advisor}
                Color::Black => {piece_images.black_advisor}
               }
            }
            PieceType::Cannon => {
                match piece.color {
                Color::Red   => {piece_images.red_cannon}
                Color::Black => {piece_images.black_cannon}
               }
            }
            PieceType::Elephant => {
                match piece.color {
                Color::Red   => {piece_images.red_elephant}
                Color::Black => {piece_images.black_elephant}
               }
            }
            PieceType::General => {
                match piece.color {
                Color::Red   => {piece_images.red_general}
                Color::Black => {piece_images.black_general}
               }
            }
            PieceType::Horse => {
                match piece.color {
                Color::Red   => {piece_images.red_horse}
                Color::Black => {piece_images.black_horse}
               }
            }
            PieceType::Pawn => {
                match piece.color {
                Color::Red   => {piece_images.red_pawn}
                Color::Black => {piece_images.black_pawn}
               }
            }
            PieceType::Rook => {
                match piece.color {
                Color::Red   => {piece_images.red_rook}
                Color::Black => {piece_images.black_rook}
               }
            }
        }
    }
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PieceImages<'s> {
    board: Tga<'s,Rgb888>,
    loading_screen: Tga<'s,Rgb888>,

    red_victory: Tga<'s,Rgb888>,
    black_victory: Tga<'s,Rgb888>,

    red_pawn: Tga<'s,Rgb888>,
    red_rook: Tga<'s,Rgb888>,
    red_cannon: Tga<'s,Rgb888>,
    red_general: Tga<'s,Rgb888>,
    red_elephant: Tga<'s,Rgb888>,
    red_horse: Tga<'s,Rgb888>,
    red_advisor: Tga<'s,Rgb888>,

    black_pawn: Tga<'s,Rgb888>,
    black_rook: Tga<'s,Rgb888>,
    black_cannon: Tga<'s,Rgb888>, 
    black_general: Tga<'s,Rgb888>,
    black_elephant: Tga<'s,Rgb888>,
    black_horse: Tga<'s,Rgb888>,
    black_advisor: Tga<'s,Rgb888>, 
}

fn drawboard(board: Board, display: &mut SimulatorDisplay<Rgb888>, window: &mut Window, piece_images:PieceImages){
    let gameboard: Tga<Rgb888> = piece_images.board;
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
                    let img: Tga<Rgb888> = get_img(piece, piece_images);
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
    window.update(&display)
}

fn draw_legal_moves(selected_piece: Piece, board: Board, display: &mut SimulatorDisplay<Rgb888>, window: &mut Window){
    for m in selected_piece.get_legal_moves(&board).iter(){
        if board.is_legal_move(*m, selected_piece.color){
            let mut x:i32 = 0;
            let mut y:i32 = 0;
            match m {
                Move::Piece(_, to) => {
                    x = 38 + (to.get_col() as i32* 90);
                    y = 845 - (to.get_row() as i32* 90);
                }
                Move::Resign => {},
            }
            Circle::new(Point::new(x.try_into().unwrap(),y.try_into().unwrap()), 14)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::GREEN))
            .draw(display);
        }
    }
    window.update(&display)
}
fn main() -> Result<(), core::convert::Infallible> {
    let piece_images: PieceImages = PieceImages
    {
        board : Tga::from_slice(include_bytes!("assets/board.tga")).unwrap(),
        loading_screen : Tga::from_slice(include_bytes!("assets/loadingscreen.tga")).unwrap(),

        red_victory : Tga::from_slice(include_bytes!("assets/TitleScreen.tga")).unwrap(),
        black_victory : Tga::from_slice(include_bytes!("assets/TitleScreen.tga")).unwrap(),

        red_pawn : Tga::from_slice(include_bytes!("assets/Chinese-Pawn-Red.tga")).unwrap(),
        red_rook : Tga::from_slice(include_bytes!("assets/Chinese-Rook-Red.tga")).unwrap(),
        red_cannon : Tga::from_slice(include_bytes!("assets/Chinese-Cannon-Red.tga")).unwrap(),
        red_general : Tga::from_slice(include_bytes!("assets/Chinese-King-Red.tga")).unwrap(),
        red_elephant : Tga::from_slice(include_bytes!("assets/Chinese-Elephant-Red.tga")).unwrap(),
        red_horse : Tga::from_slice(include_bytes!("assets/Chinese-Horse-Red.tga")).unwrap(),
        red_advisor : Tga::from_slice(include_bytes!("assets/Chinese-Advisor-Red.tga")).unwrap(),

        black_pawn : Tga::from_slice(include_bytes!("assets/Chinese-Pawn-Black.tga")).unwrap(),
        black_rook : Tga::from_slice(include_bytes!("assets/Chinese-Rook-Black.tga")).unwrap(),
        black_cannon : Tga::from_slice(include_bytes!("assets/Chinese-Cannon-Black.tga"),).unwrap(),
        black_general : Tga::from_slice(include_bytes!("assets/Chinese-King-Black.tga")).unwrap(),
        black_elephant : Tga::from_slice(include_bytes!("assets/Chinese-Elephant-Black.tga")).unwrap(),
        black_horse : Tga::from_slice(include_bytes!("assets/Chinese-Horse-Black.tga")).unwrap(),
        black_advisor : Tga::from_slice(include_bytes!("assets/Chinese-Advisor-Black.tga")).unwrap(),
    };

    let mut display: SimulatorDisplay<Rgb888> = SimulatorDisplay::new(Size::new(1280, 1024));
    let mut window = Window::new("Xiangqi", &OutputSettings::default());

    let character_style = MonoTextStyle::new(&FONT_9X18_BOLD,Rgb888::WHITE);

    let textbox_style = TextBoxStyleBuilder::new()
        .alignment(HorizontalAlignment::Left)
        .vertical_alignment(VerticalAlignment::Top)
        .build();

    //let mut board = Board::default();
    let fen = "1R7/4kc3/9/9/9/9/9/9/4A4/3KC4 w - - - 1";
    let mut board: Board = 
        Board::from_str(fen).unwrap();
        drawboard(board, &mut display,&mut window,piece_images);
    
    let mut selected: Option<Piece> = None;
    let mut redraw = false;
    let mut drawlegalmoves = false;  

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
                                GameResult::IllegalMove(m) => {
                                    match m {
                                        Move::Piece(_, to) => {
                                           if board.get_turn_color() == board.get_piece(to).unwrap().color {
                                                selected = board.get_piece(to);
                                                redraw = true;
                                                drawlegalmoves = true;
                                           } else {
                                                selected = None; 
                                                redraw = true; 
                                                continue 'Game
                                           }
                                        }
                                        _ => {continue 'Game},
                                    }
                                    
                                }
                                GameResult::Victory(_color) => {break 'Game}
                            }
                        }
                        None => {
                            if board.has_ally_piece(new_pos, board.get_turn_color()){
                                selected = Some(board.get_piece(new_pos).unwrap());
                                drawlegalmoves = true; 
                            } else {continue 'Game}
                            
                        } 
                    }
                }
                _ => {}
            }
        }
        //display.clear(Rgb888::BLACK).unwrap();
        
        if redraw {drawboard(board, &mut display, &mut window, piece_images);}
        if drawlegalmoves {draw_legal_moves(selected.unwrap(),board,&mut display, &mut window);}
        drawlegalmoves = false;
        redraw = false; 
    }
    Ok(())
}