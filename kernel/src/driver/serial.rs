use core::fmt::{self, Write};

use conquer_once::spin::OnceCell;
use uart_16550::SerialPort;
use x86_64::instructions::interrupts::without_interrupts;

use crate::spinlock::Spinlock;

static SERIAL_DBG: OnceCell<Spinlock<SerialPort>> = OnceCell::uninit();

pub fn init() {
    SERIAL_DBG.init_once(|| {
        let mut port = unsafe { uart_16550::SerialPort::new(0x3F8) };
        port.init();
        Spinlock::new(port)
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
        SERIAL_DBG.get().unwrap().lock().write_fmt(args).unwrap();
    })
}
