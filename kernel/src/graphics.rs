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

impl Screen {
    pub fn buffer_mut(&mut self) -> &mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(
                self.framebuffer as *mut u8,
                (self.bytes_per_line * self.height) as usize,
            )
        }
    }

    pub fn set_pixel_in(&mut self, position: Position, color: Color) {
        // calculate offset to first byte of pixel
        let byte_offset = {
            // use stride to calculate pixel offset of target line
            let line_offset = position.y * self.width as usize;
            // add x position to get the absolute pixel offset in buffer
            let pixel_offset = line_offset + position.x;
            // convert to byte offset
            pixel_offset * self.bytes_per_pixel as usize
        };

        // set pixel based on color format
        let pixel_buffer = &mut self.buffer_mut()[byte_offset..];
        pixel_buffer[0] = color.red;
        pixel_buffer[1] = color.green;
        pixel_buffer[2] = color.blue;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub x: usize,
    pub y: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}
