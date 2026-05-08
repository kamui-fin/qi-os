#![no_std]
#![no_main]

extern crate alloc;
use alloc::string::{String, ToString};
use core::{arch::global_asm, ffi::c_char, panic::PanicInfo};
use userland::{exit, init_heap, print, println, read};

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
    "   mov rax, 1   ",
    "   int 0x80     ",
);

fn read_line() -> String {
    let mut buffer = [0u8; 100];
    read(0, &mut buffer);

    let s = str::from_utf8(&buffer).expect("Invalid UTF-8");
    s.to_string()
}

#[no_mangle]
pub extern "C" fn main(argc: usize, argv: *const *const c_char) -> u8 {
    init_heap();

    print!("[~] > ");
    loop {
        let line = read_line();
        println!("{}", line);
        print!("[~] > ");
    }

    0
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // println!("panic: {:#?}", _info);
    exit(1);
    unreachable!();
}
