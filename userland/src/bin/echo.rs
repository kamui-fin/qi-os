#![no_std]
#![no_main]

use core::{
    arch::global_asm,
    ffi::{c_char, CStr},
    panic::PanicInfo,
    ptr::null,
};

use userland::{get_pid, println};

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

#[no_mangle]
pub extern "C" fn main(argc: usize, argv: *const *const c_char) -> u8 {
    println("I'm inside the echo process!");

    let pid = get_pid();
    let mut buf = [0u8; 64];
    let s = format_no_std::show(&mut buf, format_args!("My PID: {}", pid)).unwrap();

    let mut buf = [0u8; 64];
    let s = format_no_std::show(&mut buf, format_args!("I have this many args: {}", argc)).unwrap();

    for i in 0..argc {
        unsafe {
            let ptr = *argv.add(i);
            let c_string = CStr::from_ptr(ptr);
            let string = c_string.to_str().unwrap();
            println(string);
        }
    }

    println(s);

    return 0;
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
