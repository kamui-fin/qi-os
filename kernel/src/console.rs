use core::fmt::Binary;

use alloc::sync::Arc;
use conquer_once::spin::OnceCell;
use embedded_graphics::{
    mono_font::{ascii::FONT_10X20, MonoTextStyleBuilder},
    pixelcolor::{BinaryColor, Rgb565},
    prelude::{DrawTarget, Point, Primitive, RgbColor, Size, WebColors},
    primitives::{PrimitiveStyleBuilder, Rectangle},
    text::{Baseline, Text, TextStyleBuilder},
    Drawable,
};
use futures_util::{FutureExt, StreamExt};
use pc_keyboard::{layouts, DecodedKey, HandleControl, KeyCode, Keyboard, ScancodeSet1};
use spin::Mutex;

use crate::{
    driver::serial,
    fs::vfs::FileOps,
    graphics::Screen,
    serial_println,
    task::{
        keyboard::ScancodeStream,
        thread::{switch_if_needed, SCHEDULER},
        tty::ConsoleStream,
    },
    tty::TTY,
    SCREEN,
};

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
    needs_full_redraw: bool,

    column_position: usize,
    row_position: usize,
}

impl VirtualTerminal {
    pub fn new() -> Self {
        Self {
            buffer: [[ScreenChar::default(); BUFFER_WIDTH]; BUFFER_HEIGHT],
            needs_full_redraw: false,
            column_position: 0,
            row_position: 0,
        }
    }
    fn new_line(&mut self) {
        if self.row_position == BUFFER_HEIGHT - 1 {
            for row in 1..BUFFER_HEIGHT {
                for col in 0..BUFFER_WIDTH {
                    let character = self.buffer[row][col];
                    self.buffer[row - 1][col] = character;
                }
            }
            self.clear_row(BUFFER_HEIGHT - 1);
            self.needs_full_redraw = true;
        } else {
            self.row_position += 1;
        }
        self.column_position = 0;
    }

    fn clear_row(&mut self, row: usize) {
        for col in 0..BUFFER_WIDTH {
            self.buffer[row][col] = ScreenChar::default();
        }
    }

    pub fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
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
            }
        }
    }

    fn get(&self, row: usize, col: usize) -> ScreenChar {
        self.buffer[row][col]
    }

    pub fn last_written_pos(&self) -> (usize, usize) {
        if self.column_position == 0 {
            if self.row_position == 0 {
                (0, 0)
            } else {
                (self.row_position - 1, BUFFER_WIDTH - 1)
            }
        } else {
            (self.row_position, self.column_position - 1)
        }
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

struct ConsoleRenderer;

impl ConsoleRenderer {
    pub fn paint_full(terminal: &VirtualTerminal) {
        let mut screen = SCREEN.get().unwrap().lock();
        screen.clear(Rgb565::BLACK);

        // Self::paint_cursor(&mut screen, terminal.row_position, terminal.column_position);

        for row in 0..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                Self::paint_char(terminal, &mut screen, row, col);
            }
        }

        screen.flush();
    }

    fn paint_char(vt: &VirtualTerminal, screen: &mut Screen, row: usize, col: usize) {
        let line_height = 3;
        let font = &FONT_10X20;

        let style = MonoTextStyleBuilder::new()
            .font(font)
            .text_color(Rgb565::CSS_FOREST_GREEN)
            .background_color(Rgb565::BLACK)
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
        let font = &FONT_10X20;

        let y = row * (font.character_size.height + line_height) as usize;
        let x = col * (font.character_size.width + font.character_spacing) as usize;

        let x = x + 20;
        let y = y + 20;

        let style = PrimitiveStyleBuilder::new()
            .fill_color(Rgb565::GREEN)
            .build();

        Rectangle::new(Point::new(x as i32, y as i32), font.character_size)
            .into_styled(style)
            .draw(screen);
    }
}

pub struct TtyDeviceHandle {
    pub tty_id: usize, // 1, 2, ..
}

impl FileOps for TtyDeviceHandle {
    fn read(&self, _: &crate::fs::vfs::File, buffer: &mut [u8]) -> usize {
        let mlt = MLT.get().unwrap().lock();
        mlt.ttys[self.tty_id - 1].read(buffer)
    }

    fn write(&self, _: &crate::fs::vfs::File, buffer: &[u8]) -> usize {
        let mut mlt = MLT.get().unwrap().lock();
        let string = str::from_utf8(buffer).unwrap_or_default();
        let bytes_written = mlt.ttys[self.tty_id - 1].write(string);

        if bytes_written > 0 && mlt.active_buffer == self.tty_id {
            mlt.needs_repaint = true;
            wake_tty_renderer();
        }

        bytes_written
    }
}

pub struct ConsoleMultiplexer {
    pub ttys: [TTY; 2],
    pub kcons: VirtualTerminal,
    pub active_buffer: usize,

    needs_repaint: bool,
    last_cursor_pos: (usize, usize),
}

impl ConsoleMultiplexer {
    pub fn new() -> Self {
        let ttys = [TTY::new(1), TTY::new(2)];

        Self {
            ttys,
            kcons: VirtualTerminal::new(),
            active_buffer: 1,
            needs_repaint: true,
            last_cursor_pos: (0, 0),
        }
    }

    pub fn is_switch_tty(key: DecodedKey) -> Option<usize> {
        if let DecodedKey::RawKey(keycode) = key {
            match keycode {
                KeyCode::F1 => Some(0),
                KeyCode::F2 => Some(1),
                KeyCode::F3 => Some(2),
                _ => None,
            }
        } else {
            None
        }
    }

    fn switch(&mut self, index: usize) {
        self.active_buffer = index;
        self.needs_repaint = true;

        if index == 0 {
            self.kcons.needs_full_redraw = true;
        } else {
            self.ttys[index - 1].terminal.lock().needs_full_redraw = true;
        }
    }

    fn is_tty_active(&self) -> bool {
        self.active_buffer > 0
    }

    fn handle_key(&mut self, key: DecodedKey) -> bool {
        if self.ttys[self.active_buffer - 1].handle_key(key) {
            self.needs_repaint = true;
            true
        } else {
            false
        }
    }

    pub fn paint_active(&mut self) {
        if !self.needs_repaint {
            return;
        }

        let vt = if self.is_tty_active() {
            &mut self.ttys[self.active_buffer - 1].terminal.lock()
        } else {
            &mut self.kcons
        };

        if vt.needs_full_redraw {
            ConsoleRenderer::paint_full(vt);
            vt.needs_full_redraw = false;
            self.last_cursor_pos = vt.cursor_pos();
        } else {
            let mut screen = SCREEN.get().unwrap().lock();

            let (old_r, old_c) = self.last_cursor_pos;
            ConsoleRenderer::paint_char(vt, &mut screen, old_r, old_c);

            let (r, c) = vt.last_written_pos();
            ConsoleRenderer::paint_char(vt, &mut screen, r, c);

            let (r, c) = vt.cursor_pos();
            ConsoleRenderer::paint_cursor(&mut screen, r, c);

            self.last_cursor_pos = (r, c);
            screen.flush();
        }

        self.needs_repaint = false;
    }
}

pub static MLT: OnceCell<Mutex<ConsoleMultiplexer>> = OnceCell::uninit();

pub fn init_ttys() {
    MLT.init_once(|| Mutex::new(ConsoleMultiplexer::new()));
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
                let mut mlt = MLT.get().unwrap().lock();
                // first check if we want to switch terminal multiplexer
                if let Some(index) = ConsoleMultiplexer::is_switch_tty(key) {
                    mlt.switch(index);
                    drop(mlt);
                    wake_tty_renderer();
                    continue;
                }

                if mlt.is_tty_active() {
                    if mlt.handle_key(key) {
                        drop(mlt);
                        wake_tty_renderer();
                    }
                }
            }
        }
    }
}

pub async fn listen_console_buffer() {
    let mut console_chars = ConsoleStream::new();
    while let Some(char) = console_chars.next().await {
        let mut mlt = MLT.get().unwrap().lock();
        mlt.kcons.write_byte(char.ascii_character);
        let mut batch = 1;
        // flush rest of queue
        while let Some(Some(char)) = console_chars.next().now_or_never() {
            mlt.kcons.write_byte(char.ascii_character);
            batch += 1;
        }

        if batch > 1 {
            mlt.kcons.needs_full_redraw = true;
        }

        if mlt.active_buffer == 0 {
            mlt.needs_repaint = true;
            drop(mlt);
            wake_tty_renderer();
        }
    }
}

pub fn wake_tty_renderer() {
    let mut sched = SCHEDULER.lock();
    sched.unblock_task(5);
}
