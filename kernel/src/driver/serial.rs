use core::fmt::{self, Write};

use spin::{Mutex, Once};
use uart_16550::SerialPort;
use x86_64::instructions::interrupts::without_interrupts;

static SERIAL_DBG: Once<Mutex<SerialPort>> = Once::new();

pub fn init() {
    SERIAL_DBG.call_once(|| {
        let mut port = unsafe { uart_16550::SerialPort::new(0x3F8) };
        port.init();
        Mutex::new(port)
    });
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => ($crate::driver::serial::_serial_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($($arg:tt)*) => ($crate::serial_print!("{}\n", format_args!($($arg)*)));
}

#[doc(hidden)]
pub fn _serial_print(args: fmt::Arguments) {
    without_interrupts(|| {

    SERIAL_DBG.wait().unwrap().lock().write_fmt(args).unwrap();
    })
}

pub fn serial_write(buffer: &[u8]) {
    let mut port = SERIAL_DBG.wait().unwrap().lock();
    for &byte in buffer {
        port.send(byte);
    }
}   
