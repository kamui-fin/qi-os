use core::{ops::RangeBounds, ptr::copy_nonoverlapping};

use alloc::vec;
use alloc::vec::Vec;
use embedded_graphics::{
    pixelcolor::{raw::ToBytes, Rgb565},
    prelude::{Dimensions, OriginDimensions, Point, PointsIter, RgbColor, Size},
    primitives::Rectangle,
    Pixel,
};

use crate::serial_println;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BootScreenInfo {
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

#[derive(Debug)]
pub struct Screen {
    pub bytes_per_line: u32,
    pub bytes_per_pixel: u32,
    pub width: u32,
    pub height: u32,
    pub vesa_lfb: u32,
    pub back_lfb: Vec<u8>,
}

impl Screen {
    pub fn new(screen_info: BootScreenInfo) -> Self {
        Self {
            vesa_lfb: screen_info.framebuffer,
            width: screen_info.width,
            height: screen_info.height,
            bytes_per_line: screen_info.bytes_per_line,
            bytes_per_pixel: screen_info.bytes_per_pixel,
            back_lfb: vec![0; (screen_info.bytes_per_line * screen_info.height) as usize],
        }
    }

    pub fn flush(&mut self) {
        unsafe {
            copy_nonoverlapping(
                self.back_lfb.as_ptr(),
                self.buffer_mut().as_mut_ptr(),
                self.back_lfb.len(),
            );
        }
    }

    pub fn buffer_mut(&mut self) -> &mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(
                (self.vesa_lfb as usize) as *mut u8,
                (self.bytes_per_line * self.height) as usize,
            )
        }
    }

    pub fn set_pixel_in(&mut self, position: Point, color: Rgb565) {
        if position.x < 0
            || position.x >= self.width as i32
            || position.y < 0
            || position.y >= self.height as i32
        {
            return;
        }

        // calculate offset to first byte of pixel
        let byte_offset = {
            // use stride (bytes_per_line) to calculate byte offset of target line
            let line_offset = position.y as u32 * self.bytes_per_line;
            // add x position in bytes to get the absolute pixel byte offset in buffer
            line_offset + (position.x as u32 * self.bytes_per_pixel)
        } as usize;

        let pixel_buffer = &mut self.back_lfb[byte_offset..];
        let bytes = color.to_le_bytes();
        pixel_buffer[0] = bytes[0];
        pixel_buffer[1] = bytes[1];
    }

    pub fn scroll(&mut self, height: usize) {
        let bf: &mut [u16] = unsafe {
            core::slice::from_raw_parts_mut(
                self.back_lfb.as_mut_ptr() as *mut u16,
                self.back_lfb.len() / 2,
            )
        };

        let width_num_pixels = (self.bytes_per_line / 2) as usize;
        bf.copy_within((width_num_pixels * height).., 0);
    }
}

impl embedded_graphics::draw_target::DrawTarget for Screen {
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

    fn fill_contiguous<I>(&mut self, area: &Rectangle, colors: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Self::Color>,
    {
        // every time char prints it calls
        // total of like 1000s of times
        // still very slow
        let drawable_area = area.intersection(&self.bounding_box());

        if drawable_area.size == Size::zero() {
            return Ok(());
        }

        // TODO: refactor (problem was that map with and without filter are two diff types)
        if drawable_area != *area {
            let mut colors_iter = area
                .points()
                .zip(colors)
                .filter(|(pos, _color)| drawable_area.contains(*pos))
                .map(|(_, color)| u16::from_le_bytes(color.to_le_bytes()));

            let bf: &mut [u16] = unsafe {
                core::slice::from_raw_parts_mut(
                    self.back_lfb.as_mut_ptr() as *mut u16,
                    self.back_lfb.len() / 2,
                )
            };
            let width = drawable_area.size.width as usize;
            for y in drawable_area.rows() {
                let x = drawable_area.columns().start;
                let byte_offset = {
                    let line_offset = y as u32 * self.bytes_per_line / 2;
                    line_offset + (x as u32 * self.bytes_per_pixel / 2)
                } as usize;

                let row_slice = &mut bf[byte_offset..(byte_offset + width)];
                for (pixel, color) in row_slice.iter_mut().zip(&mut colors_iter) {
                    *pixel = color;
                }
            }
        } else {
            let mut colors_iter = area
                .points()
                .zip(colors)
                .map(|(_, color)| u16::from_le_bytes(color.to_le_bytes()));

            let bf: &mut [u16] = unsafe {
                core::slice::from_raw_parts_mut(
                    self.back_lfb.as_mut_ptr() as *mut u16,
                    self.back_lfb.len() / 2,
                )
            };
            let width = drawable_area.size.width as usize;
            for y in drawable_area.rows() {
                let x = drawable_area.columns().start;
                let byte_offset = {
                    let line_offset = y as u32 * self.bytes_per_line / 2;
                    line_offset + (x as u32 * self.bytes_per_pixel / 2)
                } as usize;

                let row_slice = &mut bf[byte_offset..(byte_offset + width)];
                for (pixel, color) in row_slice.iter_mut().zip(&mut colors_iter) {
                    *pixel = color;
                }
            }
        }

        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        // x: top left
        // x--------------
        // |             |
        // |             |
        // |             |
        // ---------------
        //
        // only guarenteed contiguous areas are each row of rectangle
        // so we just want to memset row by row?
        let drawable_area = area.intersection(&self.bounding_box());
        if drawable_area.size == Size::zero() {
            return Ok(());
        }

        let color = u16::from_le_bytes(color.to_le_bytes());
        let bf: &mut [u16] = unsafe {
            core::slice::from_raw_parts_mut(
                self.back_lfb.as_mut_ptr() as *mut u16,
                self.back_lfb.len() / 2,
            )
        };

        let width = drawable_area.size.width as usize;
        for y in drawable_area.rows() {
            let x = drawable_area.columns().start;
            let byte_offset = {
                let line_offset = y as u32 * self.bytes_per_line / 2;
                line_offset + (x as u32 * self.bytes_per_pixel / 2)
            } as usize;
            bf[byte_offset..(byte_offset + width)].fill(color);
        }

        Ok(())
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        let color = u16::from_le_bytes(color.to_le_bytes());
        // u16 must start on even addr
        let (_, mid, _) = unsafe { self.back_lfb.align_to_mut::<u16>() };

        mid.fill(color);

        Ok(())
    }
}

impl OriginDimensions for Screen {
    fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }
}
