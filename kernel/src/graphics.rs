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

pub struct FrameBuffer {

}
