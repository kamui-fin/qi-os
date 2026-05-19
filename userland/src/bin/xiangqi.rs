#![no_std]
#![no_main]

extern crate alloc;

use alloc::{format, string::String};
use core::{
    arch::global_asm,
    ffi::c_char,
    panic::PanicInfo,
    str::FromStr,
};

use common::UserWindow;
use embedded_graphics::{
    draw_target::DrawTarget,
    geometry::Dimensions,
    image::Image,
    mono_font::{ascii::FONT_9X18_BOLD, MonoTextStyle},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Circle, PrimitiveStyle, Rectangle},
    Pixel,
};

use tinytga::Tga;
use userland::{
    init_heap, open, read, syscall_get_backbuffer, syscall_notify_frame_update, syscall_sleep,
    serial_println,
};

// We import the engine. (Make sure you added it to userland/Cargo.toml using the path to userland/games/xiangqi/engine)
use engine::{
    board::Board, fen, piece::{Piece, PieceType}, position::Position, Color, GameResult, Move
};

global_asm!(
    ".section .text._start",
    ".global _start",
    "_start:      ",
    "   xor rbp, rbp ",
    "   pop rdi      ",
    "   mov rsi, rsp ",
    "   and rsp, -16 ",
    "   call main    ",
    "   mov rdi, rax ",
    "   mov rax, 20   ",
    "   int 0x80     ",
);

// --- Transparent Draw Target (Copied from your transparent.rs) ---
pub struct TransparentDrawTarget<'a, T: DrawTarget> {
    pub target: &'a mut T,
    pub transparent_color: T::Color,
}

impl<'a, T: DrawTarget> Dimensions for TransparentDrawTarget<'a, T> {
    fn bounding_box(&self) -> Rectangle {
        self.target.bounding_box()
    }
}

impl<'a, T: DrawTarget> DrawTarget for TransparentDrawTarget<'a, T> {
    type Color = T::Color;
    type Error = T::Error;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        self.target.draw_iter(
            pixels
                .into_iter()
                .filter(|pixel| pixel.1 != self.transparent_color),
        )
    }
}

// --- Image Assets & Rendering Functions ---

// Note: Switched to Rgb565 for the OS Framebuffer
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PieceImages<'s> {
    board: Tga<'s, Rgb565>,
    loading_screen: Tga<'s, Rgb565>,

    red_victory: Tga<'s, Rgb565>,
    black_victory: Tga<'s, Rgb565>,

    red_pawn: Tga<'s, Rgb565>,
    red_rook: Tga<'s, Rgb565>,
    red_cannon: Tga<'s, Rgb565>,
    red_general: Tga<'s, Rgb565>,
    red_elephant: Tga<'s, Rgb565>,
    red_horse: Tga<'s, Rgb565>,
    red_advisor: Tga<'s, Rgb565>,

    black_pawn: Tga<'s, Rgb565>,
    black_rook: Tga<'s, Rgb565>,
    black_cannon: Tga<'s, Rgb565>,
    black_general: Tga<'s, Rgb565>,
    black_elephant: Tga<'s, Rgb565>,
    black_horse: Tga<'s, Rgb565>,
    black_advisor: Tga<'s, Rgb565>,
}

pub fn get_img(piece: Piece, piece_images: PieceImages) -> Tga<Rgb565> {
    match piece.piece_type {
        PieceType::Advisor => match piece.color {
            Color::Red => piece_images.red_advisor,
            Color::Black => piece_images.black_advisor,
        },
        PieceType::Cannon => match piece.color {
            Color::Red => piece_images.red_cannon,
            Color::Black => piece_images.black_cannon,
        },
        PieceType::Elephant => match piece.color {
            Color::Red => piece_images.red_elephant,
            Color::Black => piece_images.black_elephant,
        },
        PieceType::General => match piece.color {
            Color::Red => piece_images.red_general,
            Color::Black => piece_images.black_general,
        },
        PieceType::Horse => match piece.color {
            Color::Red => piece_images.red_horse,
            Color::Black => piece_images.black_horse,
        },
        PieceType::Pawn => match piece.color {
            Color::Red => piece_images.red_pawn,
            Color::Black => piece_images.black_pawn,
        },
        PieceType::Rook => match piece.color {
            Color::Red => piece_images.red_rook,
            Color::Black => piece_images.black_rook,
        },
    }
}

fn drawboard(board: Board, display: &mut UserWindow, piece_images: PieceImages) {
    let game_board = Image::new(&piece_images.board, Point::new(0, 0));
    game_board.draw(display).unwrap();
    
    let mut filter = TransparentDrawTarget {
        target: display,
        transparent_color: Rgb565::BLACK, // Note: updated to Rgb565::BLACK
    };

    let mut r: i32 = 0;
    while r < 10 {
        let mut c: i32 = 0;
        while c < 9 {
            let piece = board.get_piece(Position::new(r.try_into().unwrap(), c.try_into().unwrap()));
            if let Some(piece) = piece {
                let img: Tga<Rgb565> = get_img(piece, piece_images);
                let y: i32 = 822 - (r * 90);
                let x: i32 = 12 + (c * 90);
                let piece_img = Image::new(&img, Point::new(x, y));
                piece_img.draw(&mut filter).unwrap();
            }
            c += 1
        }
        r += 1;
    }
    
    // Update deferred to main loop for smooth cursor tracking
}

fn draw_legal_moves(selected_piece: Piece, board: Board, display: &mut UserWindow) {
    for m in selected_piece.get_legal_moves(&board).iter() {
        if board.is_legal_move(*m, selected_piece.color) {
            let mut x: i32 = 0;
            let mut y: i32 = 0;
            match m {
                Move::Piece(_, to) => {
                    x = 38 + (to.get_col() as i32 * 90);
                    y = 845 - (to.get_row() as i32 * 90);
                }
                Move::Resign => {}
            }
            Circle::new(Point::new(x.try_into().unwrap(), y.try_into().unwrap()), 14)
                .into_styled(PrimitiveStyle::with_fill(Rgb565::GREEN)) // updated to Rgb565::GREEN
                .draw(display).unwrap();
        }
    }
}


// --- Main Application Loop ---

#[no_mangle]
pub extern "C" fn main(argc: usize, argv: *const *const c_char) -> u8 {
    init_heap();
    serial_println!("Starting Xiangqi...");

    let piece_images: PieceImages = PieceImages {
        board: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/board.tga")).unwrap(),
        loading_screen: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/loadingscreen.tga")).unwrap(),

        red_victory: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/TitleScreen.tga")).unwrap(),
        black_victory: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/TitleScreen.tga")).unwrap(),

        red_pawn: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/Chinese-Pawn-Red.tga")).unwrap(),
        red_rook: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/Chinese-Rook-Red.tga")).unwrap(),
        red_cannon: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/Chinese-Cannon-Red.tga")).unwrap(),
        red_general: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/Chinese-King-Red.tga")).unwrap(),
        red_elephant: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/Chinese-Elephant-Red.tga")).unwrap(),
        red_horse: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/Chinese-Horse-Red.tga")).unwrap(),
        red_advisor: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/Chinese-Advisor-Red.tga")).unwrap(),

        black_pawn: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/Chinese-Pawn-Black.tga")).unwrap(),
        black_rook: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/Chinese-Rook-Black.tga")).unwrap(),
        black_cannon: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/Chinese-Cannon-Black.tga")).unwrap(),
        black_general: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/Chinese-King-Black.tga")).unwrap(),
        black_elephant: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/Chinese-Elephant-Black.tga")).unwrap(),
        black_horse: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/Chinese-Horse-Black.tga")).unwrap(),
        black_advisor: Tga::from_slice(include_bytes!("../../games/xiangqi/assets/Chinese-Advisor-Black.tga")).unwrap(),
    };

    // Grab the OS display instead of SimulatorDisplay
    let mut display = syscall_get_backbuffer();

    let fen_str = "1R7/4kc3/9/9/9/9/9/9/4A4/3KC4 w - - - 1";
    let mut board = Board::from_str(fen_str).unwrap();
    
    // Draw initial board
    drawboard(board, &mut display, piece_images);

    let mut selected: Option<Piece> = None;
    let mut redraw = false;
    let mut drawlegalmoves = false;

    // We open the mouse driver
    let mouse_fd = open("/dev/mouse");
    let mut mouse_buffer = [0u8; 30]; // 10 packets
    
    // Keep track of our own mouse coordinates
    let mut mouse_x = display.bounding_box().size.width as i32 / 2;
    let mut mouse_y = display.bounding_box().size.height as i32 / 2;
    
    // Keep track of the click state to prevent spamming
    let mut was_left_clicked = false;

    'Game: loop {
        // --- SIMULATOR TEXT DRAWING ---
        // The simulator used embedded_text crate which isn't imported here yet. 
        // You'll need to either import `embedded_text` in userland/Cargo.toml 
        // or draw text manually using `embedded_graphics::text::Text`.
        
        /* 
        let turn_text = format!("Current Turn: {}", board.get_turn_color());
        TextBox::with_textbox_style(...) 
        */

        // --- MOUSE READING (REPLACES window.events()) ---
        // Instead of `for event in window.events()`, we read raw mouse packets:
        let bytes_read = read(mouse_fd, &mut mouse_buffer);
        if bytes_read > 0 {
            let old_mouse_x = mouse_x;
            let old_mouse_y = mouse_y;
            let mut i = 0;
            let mut is_left_clicked = false;
            
            while i + 2 < bytes_read {
                let status = mouse_buffer[i];
                let x_mov = mouse_buffer[i + 1];
                let y_mov = mouse_buffer[i + 2];

                // Check left click bit
                is_left_clicked = (status & 1) != 0;

                // Decode X/Y
                let mut dx = x_mov as u16;
                if (status & (1 << 4)) != 0 { dx |= 0xFF00; }
                let dx = dx as i16 as i32;

                let mut dy = y_mov as u16;
                if (status & (1 << 5)) != 0 { dy |= 0xFF00; }
                let dy = dy as i16 as i32;

                mouse_x += dx;
                mouse_y -= dy;
                
                i += 3;
            }

            // Clamp mouse bounds
            let max_x = (display.bounding_box().size.width - 1) as i32;
            let max_y = (display.bounding_box().size.height - 1) as i32;
            mouse_x = mouse_x.clamp(0, max_x);
            mouse_y = mouse_y.clamp(0, max_y);

            if mouse_x != old_mouse_x || mouse_y != old_mouse_y {
                redraw = true;
            }

            // --- CLICK HANDLING (REPLACES MouseButtonUp) ---
            // If the mouse went from NOT clicked to CLICKED:
            if is_left_clicked && !was_left_clicked {
                if !(12 <= mouse_x) || !(mouse_x <= 821) || !(12 <= mouse_y) || !(mouse_y <= 911) {
                    // Out of bounds click
                    was_left_clicked = is_left_clicked;
                    continue 'Game;
                }
                
                let col: i8 = ((mouse_x - 12) / 90).try_into().unwrap();
                let row: i8 = ((911 - mouse_y) / 90).try_into().unwrap();
                let new_pos = Position::new(row, col);

                match selected {
                    Some(piece) => {
                        let m = Move::Piece(piece.pos, new_pos);
                        let result = board.play_move(m);
                        match result {
                            GameResult::Continuing(next_turn) => {
                                board = next_turn; 
                                selected = None;
                                display.clear(Rgb565::BLACK).unwrap();
                                redraw = true;
                            }
                            GameResult::IllegalMove(m) => match m {
                                Move::Piece(_, to) => {
                                    if board.get_turn_color() == board.get_piece(to).unwrap().color {
                                        selected = board.get_piece(to);
                                        redraw = true;
                                        drawlegalmoves = true;
                                    } else {
                                        selected = None; 
                                        redraw = true; 
                                    }
                                }
                                _ => {},
                            },
                            GameResult::Victory(_color) => break 'Game,
                        }
                    }
                    None => {
                        if board.has_ally_piece(new_pos, board.get_turn_color()) {
                            selected = Some(board.get_piece(new_pos).unwrap());
                            drawlegalmoves = true; 
                        }
                    } 
                }
            }
            
            // Save the state for the next packet loop to prevent spam-clicking
            was_left_clicked = is_left_clicked;
            
            if redraw || drawlegalmoves {
                if redraw {
                    drawboard(board, &mut display, piece_images);
                }
                
                if selected.is_some() {
                    draw_legal_moves(selected.unwrap(), board, &mut display);
                }
                
                // Draw cursor on top
                Circle::new(Point::new(mouse_x - 5, mouse_y - 5), 10)
                    .into_styled(PrimitiveStyle::with_fill(Rgb565::BLUE))
                    .draw(&mut display)
                    .unwrap();

                // Flush the frame to the screen
                syscall_notify_frame_update();
            }
            
            // Reset flags
            drawlegalmoves = false;
            redraw = false;
        }
    }
    0
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    serial_println!("Panic in Xiangqi: {:#?}", _info);
    userland::exit(1);
    loop {}
}
