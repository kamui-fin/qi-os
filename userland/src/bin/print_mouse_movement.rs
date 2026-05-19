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
    chdir, close, execvp, exit, fork, get_dents, init_heap, open, print, println, pwd, read, serial_log, syscall_get_backbuffer, syscall_notify_frame_update, waitpid
};
use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Circle, PrimitiveStyle},
    Drawable,
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

pub fn print_mouse_movement() {
    use userland::serial_println;
    let fd = open("/dev/mouse");
    let mut buffer = [0u8; 30];

    let mut display = syscall_get_backbuffer();

    let mut x = display.bounding_box().size.width as i32 / 2;
    let mut y = display.bounding_box().size.height as i32 / 2;

    loop {
        let bytes_read = read(fd, &mut buffer);
        if bytes_read > 0 {
            let mut i = 0;
            while i + 2 < bytes_read {
                let status = buffer[i];
                let x_mov = buffer[i + 1];
                let y_mov = buffer[i + 2];

                // 9-bit relative offset requires sign extension
                let mut dx = x_mov as u16;
                if (status & (1 << 4)) != 0 {
                    dx |= 0xFF00;
                }
                let dx = dx as i16 as i32;

                let mut dy = y_mov as u16;
                if (status & (1 << 5)) != 0 {
                    dy |= 0xFF00;
                }
                let dy = dy as i16 as i32;

                // In PS/2, positive Y means moving UP, but in screen coordinates, positive Y is DOWN.
                x += dx;
                y -= dy; 
                
                i += 3;
            }

            // Clamp coordinates to screen bounds
            let max_x = (display.bounding_box().size.width - 1) as i32;
            let max_y = (display.bounding_box().size.height - 1) as i32;

            if x < 0 { x = 0; }
            if y < 0 { y = 0; }
            if x > max_x { x = max_x; }
            if y > max_y { y = max_y; }

            // Render background
            // display.clear(Rgb565::BLACK).unwrap();

            // Draw a red cursor
            // Circle::new(Point::new(x, y), 5)
            //     .into_styled(PrimitiveStyle::with_fill(Rgb565::RED))
            //     .draw(&mut display)
            //     .unwrap();

            // syscall_notify_frame_update();
            //println!("Cursor: ({}, {})", x, y);
            serial_println!("{}, {}", x, y);
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
