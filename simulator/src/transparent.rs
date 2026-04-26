use embedded_graphics::{
    draw_target::DrawTarget,
    geometry::Dimensions,
    primitives::Rectangle,
    Pixel,
};
pub struct TransparentDrawTarget<'a, T: DrawTarget> {
    pub target: &'a mut T,
    pub transparent_color: T::Color,
}

impl<'a, T: DrawTarget> Dimensions for TransparentDrawTarget<'a, T> {
    fn bounding_box(&self) -> Rectangle {
        self.target.bounding_box()
    }
}

impl<'a, T: DrawTarget> DrawTarget for TransparentDrawTarget<'a, T> {
    type Color = T::Color;
    type Error = T::Error;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        self.target.draw_iter(
            pixels
                .into_iter()
                .filter(|pixel| pixel.1 != self.transparent_color),
        )
    }
}