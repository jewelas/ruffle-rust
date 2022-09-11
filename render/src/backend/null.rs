use crate::backend::{RenderBackend, ShapeHandle, ViewportDimensions};
use crate::bitmap::{Bitmap, BitmapHandle, BitmapInfo, BitmapSource};
use crate::commands::CommandList;
use crate::error::Error;
use crate::shape_utils::DistilledShape;
use swf::Color;

pub struct NullBitmapSource;

impl BitmapSource for NullBitmapSource {
    fn bitmap(&self, _id: u16) -> Option<BitmapInfo> {
        None
    }
}

pub struct NullRenderer {
    dimensions: ViewportDimensions,
}

impl NullRenderer {
    pub fn new(dimensions: ViewportDimensions) -> Self {
        Self { dimensions }
    }
}

impl RenderBackend for NullRenderer {
    fn viewport_dimensions(&self) -> ViewportDimensions {
        self.dimensions
    }
    fn set_viewport_dimensions(&mut self, dimensions: ViewportDimensions) {
        self.dimensions = dimensions;
    }
    fn register_shape(
        &mut self,
        _shape: DistilledShape,
        _bitmap_source: &dyn BitmapSource,
    ) -> ShapeHandle {
        ShapeHandle(0)
    }
    fn replace_shape(
        &mut self,
        _shape: DistilledShape,
        _bitmap_source: &dyn BitmapSource,
        _handle: ShapeHandle,
    ) {
    }
    fn register_glyph_shape(&mut self, _shape: &swf::Glyph) -> ShapeHandle {
        ShapeHandle(0)
    }

    fn render_offscreen(
        &mut self,
        _handle: BitmapHandle,
        _width: u32,
        _height: u32,
        _commands: CommandList,
        _clear_color: Color,
    ) -> Result<Bitmap, Error> {
        Err(Error::Unimplemented)
    }

    fn submit_frame(&mut self, _clear: Color, _commands: CommandList) {}

    fn get_bitmap_pixels(&mut self, _bitmap: BitmapHandle) -> Option<Bitmap> {
        None
    }
    fn register_bitmap(&mut self, _bitmap: Bitmap) -> Result<BitmapHandle, Error> {
        Ok(BitmapHandle(0))
    }
    fn unregister_bitmap(&mut self, _bitmap: BitmapHandle) {}

    fn update_texture(
        &mut self,
        _bitmap: BitmapHandle,
        _width: u32,
        _height: u32,
        _rgba: Vec<u8>,
    ) -> Result<(), Error> {
        Ok(())
    }
}
