use core::fmt::Binary;

use alloc::sync::Arc;
use conquer_once::spin::OnceCell;
use embedded_graphics::{
    mono_font::{
        ascii::{FONT_10X20, FONT_7X13, FONT_7X14, FONT_8X13, FONT_9X15, FONT_9X18},
        MonoFont, MonoTextStyleBuilder,
    },
    pixelcolor::{BinaryColor, Rgb565},
    prelude::{DrawTarget, Point, Primitive, RgbColor, Size, WebColors},
    primitives::{PrimitiveStyleBuilder, Rectangle},
    text::{Baseline, Text, TextStyleBuilder},
    Drawable,
};
use futures_util::{FutureExt, StreamExt};
use pc_keyboard::{layouts, DecodedKey, HandleControl, KeyCode, Keyboard, ScancodeSet1};
use crate::{spinlock::Spinlock, task::scheduler::SCHEDULER};

use crate::{
    driver::{cmos::get_unix_time, serial},
    fs::vfs::FileOps,
    graphics::Screen,
    serial_println,
    task::{
        keyboard::ScancodeStream,
        tty::ConsoleStream,
    },
    tty::TTY,
    SCREEN,
};

// Zenbones terminal color scheme
/* [colors.primary]
foreground = "#bbbbbb"
background = "#191919"

[colors.cursor]
text = "#191919"
cursor = "#c9c9c9"

[colors.selection]
text = "#bbbbbb"
background = "#404040"

[colors.normal]
black = "#191919"
red = "#de6e7c"
green = "#819b69"
yellow = "#b77e64"
blue = "#6099c0"
magenta = "#b279a7"
cyan = "#66a5ad"
white = "#bbbbbb"

[colors.bright]
black = "#3d3839"
red = "#e8838f"
green = "#8bae68"
yellow = "#d68c67"
blue = "#61abda"
magenta = "#cf86c1"
cyan = "#65b8c1"
white = "#8e8e8e" */

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct ColorCode(u8);

impl ColorCode {
    pub fn new(foreground: Color, background: Color) -> ColorCode {
        ColorCode((background as u8) << 4 | (foreground as u8))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct ScreenChar {
    pub ascii_character: u8,
    pub color_code: ColorCode,
}

impl Default for ScreenChar {
    fn default() -> Self {
        Self {
            ascii_character: ' ' as u8,
            color_code: ColorCode::new(Color::Black, Color::Black),
        }
    }
}

const BUFFER_HEIGHT: usize = 40;
const BUFFER_WIDTH: usize = 150;

pub struct VirtualTerminal {
    buffer: [[ScreenChar; BUFFER_WIDTH]; BUFFER_HEIGHT],
    dirty_row: [bool; BUFFER_HEIGHT],

    column_position: usize,
    row_position: usize,
    needs_scroll: bool,
}

impl VirtualTerminal {
    pub fn new() -> Self {
        Self {
            buffer: [[ScreenChar::default(); BUFFER_WIDTH]; BUFFER_HEIGHT],
            dirty_row: [false; BUFFER_HEIGHT],
            needs_scroll: false,
            column_position: 0,
            row_position: 0,
        }
    }

    // TODO: use optimized scrolling instead
    // Figure out what needs to be dirty here truly
    fn new_line(&mut self) {
        self.mark_dirty(self.row_position);
        if self.row_position == BUFFER_HEIGHT - 1 {
            // trigger scroll
            for row in 1..BUFFER_HEIGHT {
                for col in 0..BUFFER_WIDTH {
                    let character = self.buffer[row][col];
                    self.buffer[row - 1][col] = character;
                }
            }
            self.clear_row(BUFFER_HEIGHT - 1);
            self.needs_scroll = true;
            self.mark_dirty(BUFFER_HEIGHT - 2);
        } else {
            self.row_position += 1;
        }
        self.column_position = 0;
    }

    fn mark_dirty(&mut self, row: usize) {
        self.dirty_row[row] = true;
    }

    fn clear_row(&mut self, row: usize) {
        self.mark_dirty(row);
        for col in 0..BUFFER_WIDTH {
            self.buffer[row][col] = ScreenChar::default();
        }
    }

    pub fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            b'\x08' => {
                self.mark_dirty(self.row_position);
                self.column_position = self.column_position.saturating_sub(1);
            }
            byte => {
                if self.column_position >= BUFFER_WIDTH {
                    self.new_line();
                }

                let row = self.row_position;
                let col = self.column_position;

                let default_color = ColorCode::new(Color::Green, Color::Black);
                self.buffer[row][col] = ScreenChar {
                    ascii_character: byte,
                    color_code: default_color,
                };
                self.column_position += 1;

                self.mark_dirty(row);
            }
        }
    }

    fn get(&self, row: usize, col: usize) -> ScreenChar {
        self.buffer[row][col]
    }

    fn cursor_pos(&self) -> (usize, usize) {
        (self.row_position, self.column_position)
    }
}

impl Default for VirtualTerminal {
    fn default() -> Self {
        Self::new()
    }
}

const FONT: MonoFont = FONT_7X14;

struct ConsoleRenderer;

impl ConsoleRenderer {
    pub fn paint_dirty(terminal: &mut VirtualTerminal, screen: &mut Screen) {
        if terminal.needs_scroll {
            screen.scroll(FONT.character_size.height as usize + 3);
            terminal.needs_scroll = false;
        }
        for row in 0..BUFFER_HEIGHT {
            if terminal.dirty_row[row] {
                for col in 0..BUFFER_WIDTH {
                    Self::paint_char(terminal, screen, row, col);
                }
                terminal.dirty_row[row] = false;
            }
        }
    }

    fn paint_char(vt: &VirtualTerminal, screen: &mut Screen, row: usize, col: usize) {
        let line_height = 3;
        let font = &FONT;

        let style = MonoTextStyleBuilder::new()
            .font(font)
            .text_color(Rgb565::new(187, 187, 187))
            .background_color(Rgb565::new(3, 6, 3))
            .build();

        let y = row * (font.character_size.height + line_height) as usize;
        let x = col * (font.character_size.width + font.character_spacing) as usize;

        // padding
        let x = x + 20;
        let y = y + 20;

        let character = vt.get(row, col);
        let printable_char = if character.ascii_character < 32 || character.ascii_character == 127 {
            b' '
        } else {
            character.ascii_character
        };

        let mut buf = [0u8; 1];
        buf[0] = printable_char;
        let string = core::str::from_utf8(&buf).unwrap_or(" ");

        let text_style = TextStyleBuilder::new().baseline(Baseline::Top).build();

        Text::with_text_style(&string, Point::new(x as i32, y as i32), style, text_style)
            .draw(&mut *screen)
            .unwrap();
    }

    fn paint_cursor(screen: &mut Screen, row: usize, col: usize) {
        let line_height = 3;
        let font = &FONT;

        let y = row * (font.character_size.height + line_height) as usize;
        let x = col * (font.character_size.width + font.character_spacing) as usize;

        let x = x + 20;
        let y = y + 20;

        let style = PrimitiveStyleBuilder::new()
            .fill_color(Rgb565::new(22, 31, 12))
            .build();

        Rectangle::new(Point::new(x as i32, y as i32), font.character_size)
            .into_styled(style)
            .draw(screen);
    }
}

pub struct Console {
    pub tty: Arc<TTY>,
    last_cursor_pos: (usize, usize),
}

impl Console {
    pub fn new() -> Self {
        Self {
            tty: Arc::new(TTY::new(1)),
            last_cursor_pos: (0, 0),
        }
    }

    pub fn paint(&mut self) {
        let vt = &mut self.tty.terminal.lock();
        let mut screen = SCREEN.get().unwrap().lock();

        ConsoleRenderer::paint_dirty(vt, &mut screen);

        // paint new cursor
        let (r, c) = vt.cursor_pos();
        ConsoleRenderer::paint_cursor(&mut screen, r, c);

        self.last_cursor_pos = (r, c);
        screen.flush();
    }
}

// FIXME: vt + tty + cons relationship rn is circular
pub static CONS: OnceCell<Spinlock<Console>> = OnceCell::uninit();

pub fn init_ttys() {
    CONS.init_once(|| Spinlock::new(Console::new()));
}

pub async fn handle_keyboard() {
    serial_println!("Spawned keyboard task");
    let mut scancodes = ScancodeStream::new();
    let mut keyboard = Keyboard::new(
        ScancodeSet1::new(),
        layouts::Us104Key,
        HandleControl::MapLettersToUnicode,
    );
    while let Some(scancode) = scancodes.next().await {
        if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
            if let Some(key) = keyboard.process_keyevent(key_event) {
                serial_println!("Received {:#?}", key);
                let tty = {
                    let cons = CONS.get().unwrap().lock();
                    cons.tty.clone()
                };
                if tty.handle_key(key) {
                    wake_tty_renderer();
                }
            }
        }
    }
}

pub async fn listen_console_buffer() {
    let mut console_chars = ConsoleStream::new();
    while let Some(char) = console_chars.next().await {
        let tty = {
            let cons = CONS.get().unwrap().lock();
            cons.tty.clone()
        };
        tty.output_byte(char.ascii_character);
        // flush rest of queue
        while let Some(Some(char)) = console_chars.next().now_or_never() {
            tty.output_byte(char.ascii_character);
        }
        {
            wake_tty_renderer();
        }
    }
}

pub fn wake_tty_renderer() {
    let mut sched = SCHEDULER.lock();
    sched.unblock_task(5);
}

pub struct TtyDeviceHandle {
    pub tty_id: usize, // 1, 2, ..
}

impl FileOps for TtyDeviceHandle {
    fn read(&self, _: &crate::fs::vfs::File, buffer: &mut [u8]) -> usize {
        let tty = {
            let cons = CONS.get().unwrap().lock();
            cons.tty.clone()
        };
        // for now it all redirects to one terminal
        tty.read(buffer)
    }

    fn write(&self, _: &crate::fs::vfs::File, buffer: &[u8]) -> usize {
        let tty = {
            let cons = CONS.get().unwrap().lock();
            cons.tty.clone()
        };
        let string = str::from_utf8(buffer).unwrap_or_default();
        let bytes_written = tty.write(string);

        // THIS CAUSES IT!
        wake_tty_renderer();

        bytes_written
    }
}
