#![no_std]
#![no_main]

use core::panic::PanicInfo;

// The VGA text buffer is exactly here in physical memory
const VGA_BUFFER: *mut u8 = 0xB8000 as *mut u8;
const VGA_WIDTH: isize = 80;

/// Writes a string to the screen at the specified row and column.
fn print_str(s: &str, row: isize, col: isize, color: u8) {
    let mut offset = (row * VGA_WIDTH + col) * 2;

    for byte in s.bytes() {
        // We must use 'unsafe' because Rust cannot verify what is at 0xB8000.
        // We are promising the compiler that this hardware memory exists.
        unsafe {
            *VGA_BUFFER.offset(offset) = byte; // The ASCII character
            *VGA_BUFFER.offset(offset + 1) = color; // The color attribute
        }
        offset += 2;
    }
}

/// Dumps a 64-bit integer to the screen in hexadecimal format (e.g., 0xDEADBEEF...)
fn print_hex(val: u64, row: isize, col: isize, color: u8) {
    let mut offset = (row * VGA_WIDTH + col) * 2;

    // Print the '0x' prefix
    unsafe {
        *VGA_BUFFER.offset(offset) = b'0';
        *VGA_BUFFER.offset(offset + 1) = color;
        *VGA_BUFFER.offset(offset + 2) = b'x';
        *VGA_BUFFER.offset(offset + 3) = color;
    }
    offset += 4;

    // A 64-bit integer has 16 hex characters (nibbles)
    for i in (0..16).rev() {
        let nibble = ((val >> (i * 4)) & 0xF) as u8;

        // Convert the number (0-15) to an ASCII character ('0'-'9' or 'A'-'F')
        let hex_char = match nibble {
            0..=9 => b'0' + nibble,
            _ => b'A' + (nibble - 10),
        };

        unsafe {
            *VGA_BUFFER.offset(offset) = hex_char;
            *VGA_BUFFER.offset(offset + 1) = color;
        }
        offset += 2;
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // 0x0A is Light Green text on a Black background
    print_str("RUST KERNEL ONLINE. 64-BIT MODE CONFIRMED.", 0, 0, 0x0A);

    // 0x0F is Bright White text on a Black background
    print_str("Hacking the mainframe...", 2, 0, 0x0F);

    // 0x0E is Yellow text on a Black background
    // Let's print a recognizable magic number to prove our math and compilation works
    print_str("Magic Hex Dump: ", 4, 0, 0x0F);
    print_hex(0xDEADBEEFCAFEBABE, 4, 16, 0x0E);

    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // 0x4F is White text on a Red background
    print_str("KERNEL PANIC!", 24, 0, 0x4F);
    loop {}
}
