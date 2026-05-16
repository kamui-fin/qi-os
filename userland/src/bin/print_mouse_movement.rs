#![no_std]
#![no_main]

extern crate alloc;
use alloc::{
    ffi::CString,
    string::{String, ToString},
    vec::Vec,
};
use core::{
    arch::global_asm,
    ffi::{c_char, CStr},
    panic::PanicInfo,
};
use userland::{
    chdir, close, execvp, exit, fork, get_dents, init_heap, open, print, println, pwd, read,
    waitpid,
};

global_asm!(
    ".section .text._start",
    ".global _start",
    "_start:      ",
    "   xor rbp, rbp ",
    "   pop rdi      ",
    "   mov rsi, rsp ",
    "   and rsp, -16 ",
    "   call main    ",
    "   mov rdi, rax ",
    "   mov rax, 20   ",
    "   int 0x80     ",
);


pub fn print_mouse_movement(){
    let fd = open("/dev/mouse");
    let mut buffer = [0u8;3];

    loop {
        let bytes_read = read(fd, &mut buffer);
        if bytes_read == 3 {
            let mut x = buffer[1] as i32;
            let mut y = buffer[2] as i32;
            println!("{} : {}", x, y);
        }

    }
}

#[no_mangle]
pub extern "C" fn main(argc: usize, argv: *const *const c_char) -> u8 {
    init_heap();

    print_mouse_movement();

    0
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    println!("panic: {:#?}", _info);
    exit(1);
    unreachable!();
}
