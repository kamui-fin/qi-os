use conquer_once::spin::OnceCell;
use core::fmt::{self, Write};
use core::{
    pin::Pin,
    task::{Context, Poll},
};
use crossbeam_queue::ArrayQueue;
use futures_util::stream::Stream;
use futures_util::task::AtomicWaker;
use lazy_static::lazy_static;
use spin::Mutex;
use volatile::Volatile;
use x86_64::structures::paging::PhysFrame;

use crate::serial_println;

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::task::tty::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

struct ConsoleWriter;

impl Write for ConsoleWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        crate::task::tty::write_string(s);
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    use x86_64::instructions::interrupts;

    interrupts::without_interrupts(|| {
        let mut writer = ConsoleWriter;
        writer.write_fmt(args).unwrap();
    });
}

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

static WAKER: AtomicWaker = AtomicWaker::new();

static CONSOLE_CHAR_QUEUE: OnceCell<ArrayQueue<ScreenChar>> = OnceCell::uninit();

pub fn init_console_char_queue() {
    CONSOLE_CHAR_QUEUE
        .try_init_once(|| ArrayQueue::new(80 * 40))
        .expect("Console queue initialization failed or called twice");
}

pub fn write_byte(byte: u8) {
    if let Ok(queue) = CONSOLE_CHAR_QUEUE.try_get() {
        queue.force_push(ScreenChar {
            ascii_character: byte,
            color_code: ColorCode::new(Color::Yellow, Color::Black),
        });
        WAKER.wake();
    } else {
        serial_println!("WARNING: console char queue uninitialized");
    }
}

pub fn write_string(s: &str) {
    for byte in s.bytes() {
        match byte {
            // printable ASCII byte or newline
            0x20..=0x7e | b'\n' => write_byte(byte),
            // not part of printable ASCII range
            _ => write_byte(0xfe),
        }
    }
}

pub struct ConsoleStream {
    _private: (),
}

impl ConsoleStream {
    pub fn new() -> Self {
        ConsoleStream { _private: () }
    }
}

impl Stream for ConsoleStream {
    type Item = ScreenChar;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<ScreenChar>> {
        let queue = CONSOLE_CHAR_QUEUE.try_get().expect("not initialized");
        // fast path
        if let Some(char) = queue.pop() {
            return Poll::Ready(Some(char));
        }

        WAKER.register(&cx.waker());
        match queue.pop() {
            Some(code) => {
                WAKER.take();
                Poll::Ready(Some(code))
            }
            None => Poll::Pending,
        }
    }
}
