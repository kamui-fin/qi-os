use crate::console::{Color, ColorCode, ScreenChar};
use crate::driver::serial;
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

static WAKER: AtomicWaker = AtomicWaker::new();

static CONSOLE_CHAR_QUEUE: OnceCell<ArrayQueue<ScreenChar>> = OnceCell::uninit();

pub fn init_console_char_queue() {
    CONSOLE_CHAR_QUEUE
        .try_init_once(|| ArrayQueue::new(80 * 40))
        .expect("Console queue initialization failed or called twice");
}

fn write_byte(byte: u8) {
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
