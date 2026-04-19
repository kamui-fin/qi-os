#![no_std]
#![no_main]

// Syscalls

use core::fmt::{self, Write};

use common::UserWindow;

pub fn get_unix_time() -> usize {
    let mut timestamp: usize = 0;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 0x9,
            lateout("rax") timestamp
        );
    }
    timestamp
}

pub fn get_pid() -> usize {
    let mut pid: usize = 0;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 0x6,
            lateout("rax") pid
        );
    }
    pid
}

pub fn exit(status: u8) {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 0x1,
            in("rdi") status as u64,
            options(noreturn)
        );
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    let mut writer = ConsoleWriter;
    writer.write_fmt(args).unwrap();
}

pub fn sys_print(string: &str) {
    let ptr = string.as_ptr().addr();
    let len = string.len();

    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 0x0,
            in("rdi") ptr,
            in("rsi") len,
        );
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

struct ConsoleWriter;

impl Write for ConsoleWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        crate::sys_print(s);
        Ok(())
    }
}

pub fn syscall_get_backbuffer() -> UserWindow {
    let user_window = UserWindow::default();
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 0x6,
            in("rdi") &user_window,
        );
    }
    user_window
}

pub fn syscall_notify_frame_update() {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 0x7,
        );
    }
}

pub fn syscall_sleep(millisec: usize) {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 0x8,
            in("rdi") millisec
        );
    }
}

pub fn sbrk(size: usize) -> usize {
    let mut start_addr = 0;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 0x4,
            in("rdi") size,
            lateout("rax") start_addr
        );
    }
    start_addr
}

// User heap
use linked_list_allocator::LockedHeap;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();
pub const HEAP_SIZE: usize = 1024 * 1024 * 5;

pub fn init_heap() {
    let start = sbrk(HEAP_SIZE);
    unsafe { ALLOCATOR.lock().init(start, HEAP_SIZE) };
}
