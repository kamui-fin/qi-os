#![no_std]
#![no_main]

extern crate alloc;

// Syscalls

use core::{
    ffi::{c_char, c_str, CStr},
    fmt::{self, Write},
};

use alloc::vec;
use alloc::{
    ffi::CString,
    string::{String, ToString},
    vec::Vec,
};
use common::UserWindow;

pub fn waitpid(pid: u64) {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 29,
            in("rdi") pid,
        );
    };
}
pub fn fork() -> u64 {
    let child_pid: u64;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 18,
            lateout("rax") child_pid
        );
    };
    child_pid
}

pub fn execvp(program_name: &str, argv: &[String]) {
    let prog = CString::new(program_name).unwrap();
    let c_strings: Vec<CString> = argv
        .iter()
        .map(|s| CString::new(s.as_str()).unwrap())
        .collect();
    let mut ptrs: Vec<*const c_char> = c_strings.iter().map(|c| c.as_ptr()).collect();
    ptrs.push(core::ptr::null());
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 28,
            in("rdi") prog.as_ptr(),
            in("rsi") argv.len(),
            in("rdx") ptrs.as_ptr(),
        );
    };
}

// open, close, read, write
pub fn open(path: &str) -> u64 {
    let mut fd: u64 = 0;
    let path = CString::new(path).unwrap();
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 0,
            in("rdi") path.as_ptr() as u64,
            lateout("rax") fd
        );
    }
    fd
}

pub fn close(fd: u64) {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 1,
            in("rdi") fd,
        );
    };
}

pub fn read(fd: u64, buffer: &mut [u8]) -> usize {
    let mut bytes_read: usize = 0;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 2,
            in("rdi") fd,
            in("rsi") buffer.as_mut_ptr() as u64,
            in("rdx") buffer.len(),
            lateout("rax") bytes_read

        );
    }
    bytes_read
}

pub fn write(fd: u64, buffer: &[u8]) {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 3,
            in("rdi") fd,
            in("rsi") buffer.as_ptr() as u64,
            in("rdx") buffer.len(),
        );
    }
}

pub fn get_unix_time() -> usize {
    let mut timestamp: usize = 0;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 26,
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
            in("rax") 21,
            lateout("rax") pid
        );
    }
    pid
}

pub fn exit(status: u8) {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 20,
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
    write(1, string.as_bytes());
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
            in("rax") 24,
            in("rdi") &user_window,
        );
    }
    user_window
}

pub fn syscall_notify_frame_update() {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 25,
        );
    }
}

pub fn syscall_sleep(millisec: usize) {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 22,
            in("rdi") millisec
        );
    }
}

pub fn sbrk(size: usize) -> usize {
    let mut start_addr = 0;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 23,
            in("rdi") size,
            lateout("rax") start_addr
        );
    }
    start_addr
}

pub fn pwd() -> String {
    let mut path = [0u8; 200];
    let mut written = 0;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 8,
            in("rdi") path.as_mut_ptr(),
            in("rsi") path.len(),
            lateout("rax") written
        );
    };

    let s = str::from_utf8(&path[..written]).expect("Invalid UTF-8");
    s.to_string()
}

pub fn chdir(path: String) {
    let path = CString::new(path.as_str()).unwrap();
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 7,
            in("rdi") path.as_ptr(),
        );
    }
}

// TODO: move these duplicate typs to shared library
#[derive(Debug, Clone, Copy)]
pub enum NodeType {
    File,
    Directory,
    Pipe,
    CharDevice,
}

#[derive(Clone)]
pub struct DEntryMinimal {
    pub name: [u8; 64],
    pub inum: u64,
    pub filetype: NodeType,
    pub size: usize,
}

impl Default for DEntryMinimal {
    fn default() -> Self {
        DEntryMinimal {
            name: [0u8; 64],
            inum: 0,
            filetype: NodeType::File,
            size: 0,
        }
    }
}

pub fn get_dents(fd: u64) -> Vec<String> {
    let mut buffer = vec![DEntryMinimal::default(); 10];
    let dentry_mut_ptr = buffer.as_mut_ptr();
    let mut entries_written = 0;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 11,
            in("rdi") fd,
            in("rsi") dentry_mut_ptr,
            in("rdx") 10,
            lateout("rax") entries_written
        );

        if (entries_written as isize) >= 0 {
            buffer.set_len(entries_written);
        } else {
            buffer.set_len(0);
        }
    }
    buffer
        .iter()
        .map(|f| str::from_utf8(&f.name).expect("invalid utf8").to_string())
        .collect()
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
