use embedded_graphics::{
    mono_font::{ascii::FONT_10X20, MonoTextStyleBuilder},
    pixelcolor::Rgb565,
    prelude::{Point, RgbColor, WebColors},
    text::Text,
    Drawable,
};
use futures_util::{FutureExt, StreamExt};

use crate::{
    task::tty::{Color, ColorCode, ConsoleStream, ScreenChar},
    SCREEN,
};

const BUFFER_HEIGHT: usize = 40;
const BUFFER_WIDTH: usize = 150;

pub struct ConsoleRenderer {
    buffer: [[ScreenChar; BUFFER_WIDTH]; BUFFER_HEIGHT],
    column_position: usize,
}

impl ConsoleRenderer {
    pub fn new() -> Self {
        Self {
            buffer: [[ScreenChar::default(); BUFFER_WIDTH]; BUFFER_HEIGHT],
            column_position: 0,
        }
    }
    fn new_line(&mut self) {
        for row in 1..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                let character = self.buffer[row][col];
                self.buffer[row - 1][col] = character;
            }
        }
        self.clear_row(BUFFER_HEIGHT - 1);
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

                let row = BUFFER_HEIGHT - 1;
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

    pub fn paint(&mut self) {
        // holding tihs lock throughout render pass might be bad... isolate out screen lock
        let mut screen = SCREEN.get().unwrap().lock();

        let line_height = 3;
        let font = &FONT_10X20;

        let style = MonoTextStyleBuilder::new()
            .font(font)
            .text_color(Rgb565::CSS_FOREST_GREEN)
            .background_color(Rgb565::BLACK)
            .build();

        for row in 0..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                let y = row * (font.character_size.height + line_height) as usize;
                let x = col * (font.character_size.width + font.character_spacing) as usize;

                let character = self.buffer[row][col];

                let mut buf = [0u8; 1];
                buf[0] = character.ascii_character;
                let string = core::str::from_utf8(&buf).unwrap_or(" ");

                Text::new(&string, Point::new(x as i32, y as i32), style)
                    .draw(&mut *screen)
                    .unwrap();
            }
        }

        screen.flush();
    }
}

impl Default for ConsoleRenderer {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn render_tty_buffer() {
    let mut renderer = ConsoleRenderer::new();
    let mut console_chars = ConsoleStream::new();
    while let Some(char) = console_chars.next().await {
        renderer.write_byte(char.ascii_character);
        // flush rest of queue
        while let Some(Some(char)) = console_chars.next().now_or_never() {
            renderer.write_byte(char.ascii_character);
        }

        renderer.paint();
    }
}
