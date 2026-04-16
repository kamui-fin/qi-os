#![no_std]
#![no_main]

// Syscalls

use common::UserWindow;

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

pub fn println(string: &str) {
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
