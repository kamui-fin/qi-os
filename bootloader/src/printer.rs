use core::fmt;
use core::fmt::Write;

pub struct VgaWriter {
    pub row: isize,
    pub col: isize,
    pub color: u8,
}

impl VgaWriter {
    pub fn new(row: isize, col: isize, color: u8) -> Self {
        Self { row, col, color }
    }
}

impl fmt::Write for VgaWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            // Handle Newlines
            if byte == b'\n' {
                self.row += 1;
                self.col = 0;
                continue;
            }

            // Calculate offset (row * width + col) * 2
            let offset = (self.row * 80 + self.col) * 2;

            unsafe {
                let vga_buffer = 0xb8000 as *mut u8;
                *vga_buffer.offset(offset) = byte;
                *vga_buffer.offset(offset + 1) = self.color;
            }

            self.col += 1;

            // Simple wrap-around if line is too long
            if self.col >= 80 {
                self.col = 0;
                self.row += 1;
            }
        }
        Ok(())
    }
}

pub fn print_debug<T: core::fmt::Debug>(value: &T, row: isize, col: isize, color: u8) {
    let mut writer = VgaWriter::new(row, col, color);
    // The write! macro uses the fmt::Write implementation we just wrote
    let _ = write!(writer, "{:?}", value);
}
