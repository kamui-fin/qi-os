// **TTY Driver**
// - Input handling (keyboard)
// - Output side
// - Line discipline

use crate::{
    print, serial_print,
    task::{
        keyboard::ScancodeStream,
        tty::{Color, ColorCode, ScreenChar},
    },
};
use alloc::vec::Vec;
use crossbeam_queue::ArrayQueue;
use futures_util::stream::StreamExt;
use pc_keyboard::{layouts, DecodedKey, HandleControl, KeyCode, Keyboard, ScancodeSet1};

// For now, we'll only support canonical mode
// - buffered until enter
// - line editing
// - echoing
struct TTY {
    output_char_queue: ArrayQueue<ScreenChar>,
    input_char_queue: ArrayQueue<ScreenChar>,
    line_buf: Vec<ScreenChar>,
    held_control: bool,
}

impl TTY {
    fn new() -> Self {
        Self {
            output_char_queue: ArrayQueue::new(80 * 40),
            input_char_queue: ArrayQueue::new(80 * 40),
            line_buf: Vec::with_capacity(80),
        }
    }

    fn input_byte(&mut self, byte: u8) {
        self.line_buf.push(ScreenChar {
            ascii_character: byte,
            color_code: ColorCode::new(Color::Yellow, Color::Black),
        });
    }

    fn output_byte(&self, byte: u8) {
        self.output_char_queue.force_push(ScreenChar {
            ascii_character: byte,
            color_code: ColorCode::new(Color::Yellow, Color::Black),
        });
    }

    pub fn write(&self, s: &str) {
        for byte in s.bytes() {
            match byte {
                // printable ASCII byte or newline
                0x20..=0x7e | b'\n' => self.output_byte(byte),
                // not part of printable ASCII range
                _ => self.output_byte(0xfe),
            }
        }
    }

    pub fn read(&self) {
        // drain from input buffer
        // block until \n is read
    }

    fn handle_key(&mut self, key: DecodedKey) {
        match key {
            DecodedKey::Unicode(character) => {
                self.input_byte(character as u8);
                self.output_byte(character as u8)
            }
            DecodedKey::RawKey(key) => {
                if let pc_keyboard::KeyCode::LControl | pc_keyboard::KeyCode::RControl = key {
                    self.held_control = true;
                    return;
                }
                match key {
                    pc_keyboard::KeyCode::Backspace => {
                        // delete last char from line buffer
                        // append \b \b to output (visual deletion)
                    }
                    pc_keyboard::KeyCode::Return => {
                        // \n - done with line
                    }
                    pc_keyboard::KeyCode::C => {
                        if self.held_control {
                            // ctrl + c - Send KILL signal
                        }
                    }
                    pc_keyboard::KeyCode::D => {
                        if self.held_control {
                            // ctrl + d - EOF
                        }
                    }
                    _ => {}
                }
            }
        }
        self.held_control = false;
    }
}

pub async fn handle_keyboard() {
    /*
     * We must handle:
     *   - backspace
     *   - newline
     *   - ctrl+d
     *   - ctrl+c
     * Also echo to output buffer
     */
    let mut scancodes = ScancodeStream::new();
    let mut keyboard = Keyboard::new(
        ScancodeSet1::new(),
        layouts::Us104Key,
        HandleControl::Ignore,
    );

    // assume tty refers to the current active tty
    // assume we can access this globally in order for processes to read() and write()
    let mut tty = TTY::new();

    while let Some(scancode) = scancodes.next().await {
        if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
            if let Some(key) = keyboard.process_keyevent(key_event) {
                tty.handle_key(key);
            }
        }
    }
}
