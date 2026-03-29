#![no_std]
#![no_main]

use core::fmt::{Debug, Write};
use core::panic::PanicInfo;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Screen {
    pub width: u32,
    pub height: u32,
    pub bpp: u32,
    pub bytes_per_pixel: u32,
    pub bytes_per_line: u32,
    pub screen_size: u32,
    pub screen_size_dqwords: u32,
    pub framebuffer: u32,
    pub x: u32,
    pub y: u32,
    pub x_max: u32,
    pub y_max: u32,
}

unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val);
}

unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!("in al, dx", out("al") val, in("dx") port);
    val
}

fn serial_init() {
    unsafe {
        let port = 0x3F8;

        outb(port + 1, 0x00); // disable interrupts
        outb(port + 3, 0x80); // enable DLAB
        outb(port + 0, 0x03); // divisor low (38400 baud)
        outb(port + 1, 0x00); // divisor high
        outb(port + 3, 0x03); // 8 bits, no parity, 1 stop
        outb(port + 2, 0xC7); // enable FIFO
        outb(port + 4, 0x0B); // IRQs enabled, RTS/DSR
    }
}
fn serial_ready() -> bool {
    unsafe { (inb(0x3F8 + 5) & 0x20) != 0 }
}

fn serial_write(byte: u8) {
    while !serial_ready() {}
    unsafe {
        outb(0x3F8, byte);
    }
}

fn print(s: &str) {
    for b in s.bytes() {
        serial_write(b);
    }
}

fn print_hex(mut x: u64) {
    let mut buf = [0u8; 16];

    for i in (0..16).rev() {
        let digit = (x & 0xF) as u8;
        buf[i] = match digit {
            0..=9 => b'0' + digit,
            _ => b'A' + (digit - 10),
        };
        x >>= 4;
    }

    print("0x");
    for b in buf {
        serial_write(b);
    }
}

#[no_mangle]
pub extern "C" fn _start(screen: *const Screen) -> ! {
    serial_init();
    print("hello from kernel\n");

    let s = unsafe { &*screen };

    print("screen ptr: ");
    print_hex(screen as u64);
    print("\n");

    print("width: ");
    print_hex(s.width as u64);
    print("\n");

    print("height: ");
    print_hex(s.height as u64);
    print("\n");

    print("bpp: ");
    print_hex(s.bpp as u64);
    print("\n");

    print("pitch: ");
    print_hex(s.bytes_per_line as u64);
    print("\n");

    print("framebuffer: ");
    print_hex(s.framebuffer as u64);
    print("\n");

    print("x_max: ");
    print_hex(s.x_max as u64);
    print("\n");

    print("y_max: ");
    print_hex(s.y_max as u64);
    print("\n");

    let fb = s.framebuffer as *mut u32; // 32-bit framebuffer
    let width = s.width as usize;
    let height = s.height as usize;
    let pitch = s.bytes_per_line as usize / 4; // pitch in pixels

    // Draw a simple gradient
    for y in 0..height {
        for x in 0..width {
            let pixel = ((x * 255 / width) as u32) << 16  // red
                      | ((y * 255 / height) as u32) << 8 // green
                      | 0x00; // blue
            unsafe {
                *fb.add(y * pitch + x) = pixel;
            }
        }
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
