#![no_std]

use embedded_graphics::{
    Pixel,
    pixelcolor::{Rgb565, raw::ToBytes},
    prelude::{OriginDimensions, Point, RgbColor, Size},
};

// User-land graphics
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct UserWindow {
    pub base_addr: u64,
    pub width: u32,
    pub height: u32,
    pub bytes_per_pixel: u32,
    pub bytes_per_line: u32,
}

impl UserWindow {
    pub fn buffer_mut(&mut self) -> &mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(
                self.base_addr as *mut u8,
                (self.bytes_per_line * self.height) as usize,
            )
        }
    }

    pub fn set_pixel_in(&mut self, position: Point, color: Rgb565) {
        // calculate offset to first byte of pixel
        let byte_offset = {
            // use stride (bytes_per_line) to calculate byte offset of target line
            let line_offset = position.y as u32 * self.bytes_per_line;
            // add x position in bytes to get the absolute pixel byte offset in buffer
            line_offset + (position.x as u32 * self.bytes_per_pixel)
        } as usize;

        let pixel_buffer = &mut self.buffer_mut()[byte_offset..];
        let bytes = color.to_le_bytes();
        pixel_buffer[0] = bytes[0];
        pixel_buffer[1] = bytes[1];
    }
}

impl embedded_graphics::draw_target::DrawTarget for UserWindow {
    type Color = embedded_graphics::pixelcolor::Rgb565;

    /// Drawing operations can never fail.
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coordinates, color) in pixels.into_iter() {
            self.set_pixel_in(coordinates, color);
        }
        Ok(())
    }
}

impl OriginDimensions for UserWindow {
    fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }
}
