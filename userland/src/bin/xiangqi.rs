#![no_std]
#![no_main]

use core::{
    arch::global_asm,
    ffi::{c_char, CStr},
    panic::PanicInfo,
    ptr::null,
};

use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Circle, PrimitiveStyle},
    text::Text,
    Drawable,
};
use userland::{
    get_unix_time, println, syscall_get_backbuffer, syscall_notify_frame_update, syscall_sleep,
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
    "   mov rax, 1   ",
    "   int 0x80     ",
);

#[no_mangle]
pub extern "C" fn main(argc: usize, argv: *const *const c_char) -> u8 {
    println("Xiangqi INIT");

    let mut display = syscall_get_backbuffer();

    /* let style = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);
    Text::new("Hello Rust!", Point::new(20, 30), style)
        .draw(&mut display)
        .unwrap(); */

    // Circle with blue fill and no stroke with a translation applied
    /* Circle::new(Point::new(10, 20), 30)
    .translate(Point::new(20, 10))
    .into_styled(PrimitiveStyle::with_fill(Rgb565::BLUE))
    .draw(&mut display); */

    syscall_notify_frame_update();

    loop {
        get_unix_time();
        syscall_sleep(2 * 1000);
    }

    0
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
