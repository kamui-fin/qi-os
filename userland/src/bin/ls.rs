#![no_std]
#![no_main]

extern crate alloc;

use core::{arch::global_asm, ffi::c_char, panic::PanicInfo};
use userland::println;
use userland::{exit, init_heap};

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

#[no_mangle]
pub extern "C" fn main(argc: usize, argv: *const *const c_char) -> u8 {
    init_heap();

    println!("Hello World from XiangQi!");

    0
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    println!("panic: {:#?}", _info);
    exit(1);
    unreachable!();
}
