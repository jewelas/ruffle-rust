use ruffle_core::backend::render::{
    swf::{self, CharacterId, GradientInterpolation, GradientSpread},
    Bitmap, BitmapFormat, BitmapHandle, BitmapInfo, BitmapSource, Color, JpegTagFormat,
    NullBitmapSource, RenderBackend, ShapeHandle, Transform,
};
use ruffle_core::color_transform::ColorTransform;
use ruffle_core::matrix::Matrix;
use ruffle_core::shape_utils::{DistilledShape, DrawCommand};
use ruffle_web_common::{JsError, JsResult};
use std::cell::{Ref, RefCell};
use wasm_bindgen::{Clamped, JsCast};
use web_sys::{
    CanvasGradient, CanvasPattern, CanvasRenderingContext2d, CanvasWindingRule, DomMatrix, Element,
    HtmlCanvasElement, HtmlImageElement, ImageData, Path2d, SvgsvgElement,
};

const GRADIENT_TRANSFORM_THRESHOLD: f32 = 0.0001;

type Error = Box<dyn std::error::Error>;

pub struct WebCanvasRenderBackend {
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
    root_canvas: HtmlCanvasElement,
    render_targets: Vec<(HtmlCanvasElement, CanvasRenderingContext2d)>,
    cur_render_target: usize,
    color_matrix: Element,
    shapes: Vec<ShapeData>,
    bitmaps: Vec<BitmapData>,
    viewport_width: u32,
    viewport_height: u32,
    use_color_transform_hack: bool,
    pixelated_property_value: &'static str,
    deactivating_mask: bool,
}

/// Canvas-drawable shape data extracted from an SWF file.
struct ShapeData(Vec<CanvasDrawCommand>);

struct CanvasColor(String, u8, u8, u8, u8);

impl CanvasColor {
    /// Apply a color transformation to this color.
    fn color_transform(&self, cxform: &ColorTransform) -> Self {
        let Self(_, r, g, b, a) = self;
        let r = (*r as f32 * cxform.r_mult.to_f32() + (cxform.r_add as f32)) as u8;
        let g = (*g as f32 * cxform.g_mult.to_f32() + (cxform.g_add as f32)) as u8;
        let b = (*b as f32 * cxform.b_mult.to_f32() + (cxform.b_add as f32)) as u8;
        let a = (*a as f32 * cxform.a_mult.to_f32() + (cxform.a_add as f32)) as u8;
        let colstring = format!("rgba({},{},{},{})", r, g, b, f32::from(a) / 255.0);
        Self(colstring, r, g, b, a)
    }
}

/// An individual command to be drawn to the canvas.
enum CanvasDrawCommand {
    /// A command to draw a path stroke with a given style.
    Stroke {
        path: Path2d,
        line_width: f64,
        stroke_style: CanvasFillStyle,
        line_cap: String,
        line_join: String,
        miter_limit: f64,
    },

    /// A command to fill a path with a given style.
    Fill {
        path: Path2d,
        fill_style: CanvasFillStyle,
    },

    /// A command to draw a particular image (such as an SVG)
    DrawImage {
        image: HtmlImageElement,
        x_min: f64,
        y_min: f64,
    },
}

enum CanvasFillStyle {
    Color(CanvasColor),
    Gradient(CanvasGradient),
    TransformedGradient(TransformedGradient),
    Pattern(CanvasPattern, bool),
}

struct TransformedGradient {
    gradient: CanvasGradient,
    gradient_matrix: [f64; 6],
    inverse_gradient_matrix: DomMatrix,
}

/// Stores the actual bitmap data on the browser side in one of two ways.
/// Each better suited for different scenarios and source data formats.
/// ImageBitmap could unify these somewhat, but Safari doesn't support it.
enum BitmapDataStorage {
    /// Utilizes the JPEG decoder of the browser, and can be drawn onto a canvas directly.
    /// Needs to be drawn onto a temporary canvas to retrieve the stored pixel data.
    ImageElement(HtmlImageElement),
    /// Much easier to create from raw RGB[A] data, through a temporary ImageData.
    /// The pixel data can also be retrieved through a temporary ImageData.
    CanvasElement(HtmlCanvasElement, CanvasRenderingContext2d),
}

impl BitmapDataStorage {
    /// Puts the image data into a newly created <canvas>, and caches it.
    fn from_image_data(data: ImageData) -> Self {
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();

        let canvas: HtmlCanvasElement = document
            .create_element("canvas")
            .unwrap()
            .dyn_into()
            .unwrap();

        canvas.set_width(data.width());
        canvas.set_height(data.height());

        let context: CanvasRenderingContext2d = canvas
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();

        context.put_image_data(&data, 0.0, 0.0).unwrap();

        BitmapDataStorage::CanvasElement(canvas, context)
    }
}

#[allow(dead_code)]
struct BitmapData {
    image: BitmapDataStorage,
    width: u32,
    height: u32,
    /// Might be computed lazily if not available at creation.
    data_uri: RefCell<Option<String>>,
}

impl BitmapData {
    pub fn get_pixels(&self) -> Option<Bitmap> {
        let newcontext: CanvasRenderingContext2d; // temporarily created, only for image elements
        let context = match &self.image {
            BitmapDataStorage::ImageElement(image) => {
                let window = web_sys::window().unwrap();
                let document = window.document().unwrap();

                let canvas: HtmlCanvasElement = document
                    .create_element("canvas")
                    .unwrap()
                    .dyn_into()
                    .unwrap();

                canvas.set_width(self.width);
                canvas.set_height(self.height);

                newcontext = canvas
                    .get_context("2d")
                    .unwrap()
                    .unwrap()
                    .dyn_into()
                    .unwrap();

                newcontext.set_image_smoothing_enabled(false);
                newcontext
                    .draw_image_with_html_image_element(image, 0.0, 0.0)
                    .unwrap();

                &newcontext
            }
            BitmapDataStorage::CanvasElement(_canvas, context) => context,
        };

        if let Ok(bitmap_pixels) =
            context.get_image_data(0.0, 0.0, self.width as f64, self.height as f64)
        {
            Some(Bitmap {
                width: self.width,
                height: self.height,
                data: BitmapFormat::Rgba(bitmap_pixels.data().to_vec()),
            })
        } else {
            None
        }
    }

    /// Converts an RGBA image into a PNG encoded as a data URI referencing a Blob.
    fn bitmap_to_png_data_uri(bitmap: Bitmap) -> Result<String, Box<dyn std::error::Error>> {
        use png::Encoder;
        let mut png_data: Vec<u8> = vec![];
        {
            let mut encoder = Encoder::new(&mut png_data, bitmap.width, bitmap.height);
            encoder.set_depth(png::BitDepth::Eight);
            let data = match bitmap.data {
                BitmapFormat::Rgba(mut data) => {
                    ruffle_core::backend::render::unmultiply_alpha_rgba(&mut data[..]);
                    encoder.set_color(png::ColorType::Rgba);
                    data
                }
                BitmapFormat::Rgb(data) => {
                    encoder.set_color(png::ColorType::Rgb);
                    data
                }
            };
            let mut writer = encoder.write_header()?;
            writer.write_image_data(&data)?;
        }

        Ok(format!(
            "data:image/png;base64,{}",
            &base64::encode(&png_data[..])
        ))
    }

    pub fn get_or_compute_data_uri(&self) -> Ref<Option<String>> {
        {
            let mut uri = self.data_uri.borrow_mut();

            if uri.is_none() {
                *uri = Some(Self::bitmap_to_png_data_uri(self.get_pixels().unwrap()).unwrap());
            }
        }
        self.data_uri.borrow()
    }
}

impl WebCanvasRenderBackend {
    pub fn new(
        canvas: &HtmlCanvasElement,
        is_transparent: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Request the CanvasRenderingContext2d.
        // Disable alpha for possible speedup.
        // TODO: Allow user to enable transparent background (transparent wmode in legacy Flash).
        let context_options = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &context_options,
            &"alpha".into(),
            &if is_transparent {
                wasm_bindgen::JsValue::TRUE
            } else {
                wasm_bindgen::JsValue::FALSE
            },
        );
        let context: CanvasRenderingContext2d = canvas
            .get_context_with_context_options("2d", &context_options)
            .into_js_result()?
            .ok_or("Could not create context")?
            .dyn_into()
            .map_err(|_| "Expected CanvasRenderingContext2d")?;

        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();

        // Create a color matrix filter to handle Flash color effects.
        // We may have a previous instance if this canvas was re-used, so remove it.
        if let Ok(Some(element)) = canvas.query_selector("#_svgfilter") {
            element.remove();
        }

        let svg = document
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "svg")
            .map_err(|_| "Couldn't make SVG")?;

        svg.set_id("_svgfilter");

        svg.set_attribute("width", "0")
            .map_err(|_| "Couldn't make SVG")?;

        svg.set_attribute("height", "0")
            .map_err(|_| "Couldn't make SVG")?;

        svg.set_attribute_ns(
            Some("http://www.w3.org/2000/xmlns/"),
            "xmlns:xlink",
            "http://www.w3.org/1999/xlink",
        )
        .map_err(|_| "Couldn't make SVG")?;

        let filter = document
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "filter")
            .map_err(|_| "Couldn't make SVG filter")?;
        filter
            .set_attribute("id", "_cm")
            .map_err(|_| "Couldn't make SVG filter")?;
        filter
            .set_attribute("color-interpolation-filters", "sRGB")
            .map_err(|_| "Couldn't make SVG filter")?;

        let color_matrix = document
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "feColorMatrix")
            .map_err(|_| "Couldn't make SVG feColorMatrix element")?;
        color_matrix
            .set_attribute("type", "matrix")
            .map_err(|_| "Couldn't make SVG feColorMatrix element")?;
        color_matrix
            .set_attribute("values", "1 0 0 0 0 0 1 0 0 0 0 0 1 0 0 0 0 0 1 0")
            .map_err(|_| "Couldn't make SVG feColorMatrix element")?;

        filter
            .append_child(&color_matrix)
            .map_err(|_| "append_child failed")?;

        svg.append_child(&filter)
            .map_err(|_| "append_child failed")?;

        canvas
            .append_child(&svg)
            .map_err(|_| "append_child failed")?;

        // Check if we are on Firefox to use the color transform hack.
        // TODO: We could turn this into a general util function to detect browser
        // type, version, OS, etc.
        let is_firefox = window
            .navigator()
            .user_agent()
            .map(|s| s.contains("Firefox"))
            .unwrap_or(false);

        let render_targets = vec![(canvas.clone(), context.clone())];
        let renderer = Self {
            canvas: canvas.clone(),
            root_canvas: canvas.clone(),
            render_targets,
            cur_render_target: 0,
            color_matrix,
            context,
            shapes: vec![],
            bitmaps: vec![],
            viewport_width: 0,
            viewport_height: 0,
            use_color_transform_hack: is_firefox,
            deactivating_mask: false,

            // For rendering non-smoothed bitmaps.
            // crisp-edges works in Firefox, pixelated works in Chrome (and others)?
            pixelated_property_value: if is_firefox {
                "crisp-edges"
            } else {
                "pixelated"
            },
        };
        Ok(renderer)
    }

    // Pushes a fresh canvas onto the stack to use as a render target.
    fn push_render_target(&mut self) {
        self.cur_render_target += 1;
        if self.cur_render_target >= self.render_targets.len() {
            // Create offscreen canvas to use as the render target.
            let window = web_sys::window().unwrap();
            let document = window.document().unwrap();
            let canvas: HtmlCanvasElement = document
                .create_element("canvas")
                .unwrap()
                .dyn_into()
                .unwrap();
            let context: CanvasRenderingContext2d = canvas
                .get_context("2d")
                .unwrap()
                .unwrap()
                .dyn_into()
                .unwrap();
            canvas
                .style()
                .set_property("display", "none")
                .warn_on_error();
            self.root_canvas.append_child(&canvas).warn_on_error();
            self.render_targets.push((canvas, context));
        }

        let (canvas, context) = &self.render_targets[self.cur_render_target];
        canvas.set_width(self.viewport_width);
        canvas.set_height(self.viewport_height);
        self.canvas = canvas.clone();
        self.context = context.clone();
        let width = self.canvas.width();
        let height = self.canvas.height();
        self.context
            .clear_rect(0.0, 0.0, width.into(), height.into());
    }

    fn pop_render_target(&mut self) -> (HtmlCanvasElement, CanvasRenderingContext2d) {
        if self.cur_render_target > 0 {
            let out = (self.canvas.clone(), self.context.clone());
            self.cur_render_target -= 1;
            let (canvas, context) = &self.render_targets[self.cur_render_target];
            self.canvas = canvas.clone();
            self.context = context.clone();
            out
        } else {
            log::error!("Render target stack underflow");
            (self.canvas.clone(), self.context.clone())
        }
    }

    #[allow(clippy::float_cmp)]
    #[inline]
    fn set_transform(&mut self, matrix: &Matrix) {
        self.context
            .set_transform(
                matrix.a.into(),
                matrix.b.into(),
                matrix.c.into(),
                matrix.d.into(),
                matrix.tx.to_pixels(),
                matrix.ty.to_pixels(),
            )
            .unwrap();
    }

    #[allow(clippy::float_cmp)]
    #[inline]
    fn set_color_filter(&self, transform: &Transform) {
        let color_transform = &transform.color_transform;
        if color_transform.r_mult.is_one()
            && color_transform.g_mult.is_one()
            && color_transform.b_mult.is_one()
            && color_transform.r_add == 0
            && color_transform.g_add == 0
            && color_transform.b_add == 0
            && color_transform.a_add == 0
        {
            // Values outside the range of 0 and 1 are ignored in canvas, unlike Flash that clamps them.
            self.context
                .set_global_alpha(f64::from(color_transform.a_mult).clamp(0.0, 1.0));
        } else {
            let mult = color_transform.mult_rgba_normalized();
            let add = color_transform.add_rgba_normalized();

            // TODO HACK: Firefox is having issues with additive alpha in color transforms (see #38).
            // Hack this away and just use multiplicative (not accurate in many cases, but won't look awful).
            let (a_mult, a_add) = if self.use_color_transform_hack && color_transform.a_add != 0 {
                (mult[3] + add[3], 0.0)
            } else {
                (mult[3], add[3])
            };

            let matrix_str = format!(
                "{} 0 0 0 {} 0 {} 0 0 {} 0 0 {} 0 {} 0 0 0 {} {}",
                mult[0], add[0], mult[1], add[1], mult[2], add[2], a_mult, a_add
            );

            self.color_matrix
                .set_attribute("values", &matrix_str)
                .unwrap();

            self.context.set_filter("url('#_cm')");
        }
    }

    #[inline]
    fn clear_color_filter(&self) {
        self.context.set_filter("none");
        self.context.set_global_alpha(1.0);
    }

    fn register_bitmap_pure_jpeg(&mut self, data: &[u8]) -> Result<BitmapInfo, Error> {
        let data = ruffle_core::backend::render::remove_invalid_jpeg_data(data);
        let mut decoder = jpeg_decoder::Decoder::new(&data[..]);
        decoder.read_info()?;
        let metadata = decoder.info().ok_or("Expected JPEG metadata")?;

        let image = HtmlImageElement::new().into_js_result()?;
        let jpeg_encoded = format!("data:image/jpeg;base64,{}", &base64::encode(&data[..]));
        image.set_src(&jpeg_encoded);

        let handle = BitmapHandle(self.bitmaps.len());
        self.bitmaps.push(BitmapData {
            image: BitmapDataStorage::ImageElement(image),
            width: metadata.width.into(),
            height: metadata.height.into(),
            data_uri: RefCell::new(Some(jpeg_encoded)),
        });
        Ok(BitmapInfo {
            handle,
            width: metadata.width,
            height: metadata.height,
        })
    }

    /// Puts the contents of the given Bitmap into an ImageData on the browser side,
    /// doing the RGB to RGBA expansion if needed.
    fn swf_bitmap_to_js_imagedata(bitmap: &Bitmap) -> ImageData {
        match &bitmap.data {
            BitmapFormat::Rgb(rgb_data) => {
                let mut rgba_data = vec![0u8; (bitmap.width * bitmap.height * 4) as usize];
                for (rgba, rgb) in rgba_data.chunks_exact_mut(4).zip(rgb_data.chunks_exact(3)) {
                    rgba.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
                }
                ImageData::new_with_u8_clamped_array(Clamped(&rgba_data), bitmap.width)
            }
            BitmapFormat::Rgba(rgba_data) => {
                ImageData::new_with_u8_clamped_array(Clamped(rgba_data), bitmap.width)
            }
        }
        .unwrap()
    }

    fn register_bitmap_raw(&mut self, bitmap: Bitmap) -> Result<BitmapInfo, Error> {
        let (width, height) = (bitmap.width, bitmap.height);

        let image = Self::swf_bitmap_to_js_imagedata(&bitmap);

        let handle = BitmapHandle(self.bitmaps.len());
        self.bitmaps.push(BitmapData {
            image: BitmapDataStorage::from_image_data(image),
            width,
            height,
            data_uri: RefCell::new(None),
        });

        Ok(BitmapInfo {
            handle,
            width: width.try_into().expect("Bitmap dimensions too large"),
            height: height.try_into().expect("Bitmap dimensions too large"),
        })
    }
}

impl RenderBackend for WebCanvasRenderBackend {
    fn set_viewport_dimensions(&mut self, width: u32, height: u32) {
        self.viewport_width = width;
        self.viewport_height = height;
    }

    fn register_shape(
        &mut self,
        shape: DistilledShape,
        bitmap_source: &dyn BitmapSource,
    ) -> ShapeHandle {
        let handle = ShapeHandle(self.shapes.len());

        let data = swf_shape_to_canvas_commands(
            &shape,
            bitmap_source,
            &self.bitmaps,
            self.pixelated_property_value,
            &self.context,
        )
        .unwrap_or_else(|| {
            swf_shape_to_svg(
                shape,
                bitmap_source,
                &self.bitmaps,
                self.pixelated_property_value,
            )
        });

        self.shapes.push(data);

        handle
    }

    fn replace_shape(
        &mut self,
        shape: DistilledShape,
        bitmap_source: &dyn BitmapSource,
        handle: ShapeHandle,
    ) {
        let data = swf_shape_to_canvas_commands(
            &shape,
            bitmap_source,
            &self.bitmaps,
            self.pixelated_property_value,
            &self.context,
        )
        .unwrap_or_else(|| {
            swf_shape_to_svg(
                shape,
                bitmap_source,
                &self.bitmaps,
                self.pixelated_property_value,
            )
        });
        self.shapes[handle.0] = data;
    }

    fn register_glyph_shape(&mut self, glyph: &swf::Glyph) -> ShapeHandle {
        let shape = ruffle_core::shape_utils::swf_glyph_to_shape(glyph);
        self.register_shape((&shape).into(), &NullBitmapSource)
    }

    fn register_bitmap_jpeg(
        &mut self,
        data: &[u8],
        jpeg_tables: Option<&[u8]>,
    ) -> Result<BitmapInfo, Error> {
        let data = ruffle_core::backend::render::glue_tables_to_jpeg(data, jpeg_tables);
        self.register_bitmap_pure_jpeg(&data)
    }

    fn register_bitmap_jpeg_2(&mut self, data: &[u8]) -> Result<BitmapInfo, Error> {
        if ruffle_core::backend::render::determine_jpeg_tag_format(data) == JpegTagFormat::Jpeg {
            self.register_bitmap_pure_jpeg(data)
        } else {
            let bitmap = ruffle_core::backend::render::decode_define_bits_jpeg(data, None)?;
            self.register_bitmap_raw(bitmap)
        }
    }

    fn register_bitmap_jpeg_3_or_4(
        &mut self,
        jpeg_data: &[u8],
        alpha_data: &[u8],
    ) -> Result<BitmapInfo, Error> {
        let bitmap =
            ruffle_core::backend::render::decode_define_bits_jpeg(jpeg_data, Some(alpha_data))?;
        self.register_bitmap_raw(bitmap)
    }

    fn register_bitmap_png(
        &mut self,
        swf_tag: &swf::DefineBitsLossless,
    ) -> Result<BitmapInfo, Error> {
        let bitmap = ruffle_core::backend::render::decode_define_bits_lossless(swf_tag)?;

        let image = Self::swf_bitmap_to_js_imagedata(&bitmap);

        let handle = BitmapHandle(self.bitmaps.len());
        self.bitmaps.push(BitmapData {
            image: BitmapDataStorage::from_image_data(image),
            width: swf_tag.width.into(),
            height: swf_tag.height.into(),
            data_uri: RefCell::new(None),
        });
        Ok(BitmapInfo {
            handle,
            width: swf_tag.width,
            height: swf_tag.height,
        })
    }

    fn begin_frame(&mut self, clear: Color) {
        // Reset canvas transform in case it was left in a dirty state.
        self.context.reset_transform().unwrap();

        let width = self.canvas.width();
        let height = self.canvas.height();

        if clear.a > 0 {
            let color = format!("rgba({}, {}, {}, {})", clear.r, clear.g, clear.b, clear.a);
            self.context.set_fill_style(&color.into());
            let _ = self.context.set_global_composite_operation("copy");
            self.context
                .fill_rect(0.0, 0.0, width.into(), height.into());
            let _ = self.context.set_global_composite_operation("source-over");
        } else {
            self.context
                .clear_rect(0.0, 0.0, width.into(), height.into());
        }

        self.deactivating_mask = false;
    }

    fn end_frame(&mut self) {
        // Noop
    }

    fn render_bitmap(&mut self, bitmap: BitmapHandle, transform: &Transform, smoothing: bool) {
        if self.deactivating_mask {
            return;
        }

        self.context.set_image_smoothing_enabled(smoothing);

        self.set_transform(&transform.matrix);
        self.set_color_filter(transform);
        if let Some(bitmap) = self.bitmaps.get(bitmap.0) {
            match &bitmap.image {
                BitmapDataStorage::ImageElement(image) => {
                    let _ = self
                        .context
                        .draw_image_with_html_image_element(image, 0.0, 0.0);
                }
                BitmapDataStorage::CanvasElement(canvas, _context) => {
                    let _ = self
                        .context
                        .draw_image_with_html_canvas_element(canvas, 0.0, 0.0);
                }
            }
        }
        self.clear_color_filter();
    }

    fn render_shape(&mut self, shape: ShapeHandle, transform: &Transform) {
        if self.deactivating_mask {
            return;
        }

        self.set_transform(&transform.matrix);
        if let Some(shape) = self.shapes.get(shape.0) {
            for command in shape.0.iter() {
                match command {
                    CanvasDrawCommand::Fill { path, fill_style } => match fill_style {
                        CanvasFillStyle::Color(color) => {
                            let color = color.color_transform(&transform.color_transform);
                            self.context.set_fill_style(&color.0.into());
                            self.context
                                .fill_with_path_2d_and_winding(path, CanvasWindingRule::Evenodd);
                        }
                        CanvasFillStyle::Gradient(gradient) => {
                            self.set_color_filter(&transform);
                            self.context.set_fill_style(gradient);
                            self.context
                                .fill_with_path_2d_and_winding(path, CanvasWindingRule::Evenodd);
                            self.clear_color_filter();
                        }
                        CanvasFillStyle::TransformedGradient(gradient) => {
                            // Canvas has no easy way to draw gradients with an arbitrary transform,
                            // but we can fake it by pushing the gradient's transform to the canvas,
                            // then transforming the path itself by the inverse.
                            self.set_color_filter(&transform);
                            self.context.set_fill_style(&gradient.gradient);
                            let matrix = &gradient.gradient_matrix;
                            self.context
                                .transform(
                                    matrix[0], matrix[1], matrix[2], matrix[3], matrix[4],
                                    matrix[5],
                                )
                                .warn_on_error();
                            let untransformed_path = Path2d::new().unwrap();
                            untransformed_path.add_path_with_transformation(
                                path,
                                gradient.inverse_gradient_matrix.unchecked_ref(),
                            );
                            self.context.fill_with_path_2d_and_winding(
                                &untransformed_path,
                                CanvasWindingRule::Evenodd,
                            );
                            self.context
                                .set_transform(
                                    transform.matrix.a.into(),
                                    transform.matrix.b.into(),
                                    transform.matrix.c.into(),
                                    transform.matrix.d.into(),
                                    transform.matrix.tx.to_pixels(),
                                    transform.matrix.ty.to_pixels(),
                                )
                                .unwrap();
                            self.clear_color_filter();
                        }
                        CanvasFillStyle::Pattern(patt, smoothed) => {
                            self.set_color_filter(&transform);
                            self.context.set_image_smoothing_enabled(*smoothed);
                            self.context.set_fill_style(patt);
                            self.context
                                .fill_with_path_2d_and_winding(path, CanvasWindingRule::Evenodd);
                            self.clear_color_filter();
                        }
                    },
                    CanvasDrawCommand::Stroke {
                        path,
                        line_width,
                        stroke_style,
                        line_cap,
                        line_join,
                        miter_limit,
                    } => {
                        self.context.set_line_cap(line_cap);
                        self.context.set_line_join(line_join);
                        self.context.set_miter_limit(*miter_limit);
                        self.context.set_line_width(*line_width);
                        match stroke_style {
                            CanvasFillStyle::Color(color) => {
                                let color = color.color_transform(&transform.color_transform);
                                self.context.set_stroke_style(&color.0.into());
                                self.context.stroke_with_path(path);
                            }
                            CanvasFillStyle::Gradient(gradient) => {
                                self.set_color_filter(&transform);
                                self.context.set_stroke_style(gradient);
                                self.context.stroke_with_path(path);
                                self.clear_color_filter();
                            }
                            CanvasFillStyle::TransformedGradient(gradient) => {
                                self.set_color_filter(&transform);
                                self.context.set_stroke_style(&gradient.gradient);
                                self.context.stroke_with_path(path);
                                self.context
                                    .set_transform(
                                        transform.matrix.a.into(),
                                        transform.matrix.b.into(),
                                        transform.matrix.c.into(),
                                        transform.matrix.d.into(),
                                        transform.matrix.tx.to_pixels(),
                                        transform.matrix.ty.to_pixels(),
                                    )
                                    .unwrap();
                                self.clear_color_filter();
                            }
                            CanvasFillStyle::Pattern(patt, smoothed) => {
                                self.context.set_image_smoothing_enabled(*smoothed);
                                self.context.set_stroke_style(patt);
                                self.context.stroke_with_path(path);
                                self.clear_color_filter();
                            }
                        };
                    }
                    CanvasDrawCommand::DrawImage {
                        image,
                        x_min,
                        y_min,
                    } => {
                        self.set_color_filter(transform);
                        let _ = self
                            .context
                            .draw_image_with_html_image_element(image, *x_min, *y_min);
                        self.clear_color_filter();
                    }
                }
            }
        }
    }

    fn draw_rect(&mut self, color: Color, matrix: &Matrix) {
        if self.deactivating_mask {
            return;
        }

        self.set_transform(matrix);
        self.clear_color_filter();

        self.context.set_fill_style(
            &format!(
                "rgba({},{},{},{})",
                color.r,
                color.g,
                color.b,
                f32::from(color.a) / 255.0
            )
            .into(),
        );
        self.context.fill_rect(0.0, 0.0, 1.0, 1.0);

        self.clear_color_filter();
    }

    fn push_mask(&mut self) {
        // In the canvas backend, masks are implemented using two render targets.
        // We render the masker clips to the first render target.
        self.push_render_target();
    }
    fn activate_mask(&mut self) {
        // We render the maskee clips to the second render target.
        self.push_render_target();
    }
    fn deactivate_mask(&mut self) {
        self.deactivating_mask = true;
    }
    fn pop_mask(&mut self) {
        self.deactivating_mask = false;

        let (maskee_canvas, maskee_context) = self.pop_render_target();
        let (masker_canvas, _masker_context) = self.pop_render_target();

        // We have to be sure to reset the transforms here so that
        // the texture is drawn starting from the upper-left corner.
        maskee_context.reset_transform().warn_on_error();
        self.context.reset_transform().warn_on_error();

        // We draw the masker onto the maskee using the "destination-in" blend mode.
        // This will filter out pixels where the maskee alpha == 0.
        maskee_context
            .set_global_composite_operation("destination-in")
            .unwrap();

        // Force alpha to 100% for the mask art, because Flash ignores alpha in masks.
        // Otherwise canvas blend modes will draw the masked clip as transparent.
        // TODO: Doesn't work on Safari because it doesn't support context.filter.
        self.color_matrix
            .set_attribute(
                "values",
                "1.0 0 0 0 0 0 1.0 0 0 0 0 0 1.0 0 0 0 0 0 256.0 0",
            )
            .warn_on_error();

        maskee_context.set_filter("url('#_cm')");
        maskee_context
            .draw_image_with_html_canvas_element(&masker_canvas, 0.0, 0.0)
            .unwrap();
        maskee_context
            .set_global_composite_operation("source-over")
            .unwrap();
        maskee_context.set_filter("none");

        // Finally, we draw the finalized masked onto the main canvas.
        self.context.reset_transform().warn_on_error();
        self.context
            .draw_image_with_html_canvas_element(&maskee_canvas, 0.0, 0.0)
            .unwrap();
    }

    fn get_bitmap_pixels(&mut self, bitmap: BitmapHandle) -> Option<Bitmap> {
        let bitmap = &self.bitmaps[bitmap.0];
        bitmap.get_pixels()
    }

    fn register_bitmap_raw(
        &mut self,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Result<BitmapHandle, Error> {
        Ok(self
            .register_bitmap_raw(Bitmap {
                width,
                height,
                data: BitmapFormat::Rgba(rgba),
            })?
            .handle)
    }

    fn update_texture(
        &mut self,
        handle: BitmapHandle,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Result<BitmapHandle, Error> {
        // TODO: Could be optimized to a single put_image_data call
        // in case it is already stored as a canvas+context.
        self.bitmaps[handle.0] = BitmapData {
            image: BitmapDataStorage::from_image_data(
                ImageData::new_with_u8_clamped_array(Clamped(&rgba), width).unwrap(),
            ),
            width,
            height,
            data_uri: RefCell::new(None),
        };

        Ok(handle)
    }
}

#[allow(clippy::cognitive_complexity)]
fn swf_shape_to_svg(
    shape: DistilledShape,
    bitmap_source: &dyn BitmapSource,
    bitmaps: &[BitmapData],
    pixelated_property_value: &str,
) -> ShapeData {
    use fnv::FnvHashSet;
    use ruffle_core::shape_utils::DrawPath;
    use svg::node::element::{
        path::Data, Definitions, Filter, Image, LinearGradient, Path as SvgPath, Pattern,
        RadialGradient, Stop,
    };
    use svg::Document;
    use swf::{FillStyle, LineCapStyle, LineJoinStyle};

    // Some browsers will vomit if you try to load/draw an image with 0 width/height.
    // TODO(Herschel): Might be better to just return None in this case and skip
    // rendering altogether.
    let (width, height) = (
        f32::max(
            (shape.shape_bounds.x_max - shape.shape_bounds.x_min).to_pixels() as f32,
            1.0,
        ),
        f32::max(
            (shape.shape_bounds.y_max - shape.shape_bounds.y_min).to_pixels() as f32,
            1.0,
        ),
    );
    let mut document = Document::new()
        .set("width", width)
        .set("height", height)
        .set(
            "viewBox",
            (
                shape.shape_bounds.x_min.get(),
                shape.shape_bounds.y_min.get(),
                (shape.shape_bounds.x_max - shape.shape_bounds.x_min).get(),
                (shape.shape_bounds.y_max - shape.shape_bounds.y_min).get(),
            ),
        )
        // preserveAspectRatio must be off or Firefox will fudge with the dimensions when we draw an image onto canvas.
        .set("preserveAspectRatio", "none")
        .set("xmlns:xlink", "http://www.w3.org/1999/xlink");

    let width = (shape.shape_bounds.x_max - shape.shape_bounds.x_min).get() as f32;
    let height = (shape.shape_bounds.y_max - shape.shape_bounds.y_min).get() as f32;

    let mut bitmap_defs: FnvHashSet<CharacterId> = FnvHashSet::default();

    let mut defs = Definitions::new();
    let mut num_defs = 0;
    let mut has_linear_rgb_gradient = false;

    let mut svg_paths = Vec::with_capacity(shape.paths.len());
    for path in shape.paths {
        let mut svg_path = SvgPath::new();
        let (style, commands) = match &path {
            DrawPath::Fill { style, commands } => (*style, commands),
            DrawPath::Stroke {
                style, commands, ..
            } => (style.fill_style(), commands),
        };
        let fill = match style {
            FillStyle::Color(Color { r, g, b, a }) => {
                format!("rgba({},{},{},{})", r, g, b, f32::from(*a) / 255.0)
            }
            FillStyle::LinearGradient(gradient) => {
                let shift = Matrix {
                    a: 32768.0 / width,
                    d: 32768.0 / height,
                    tx: swf::Twips::new(-16384),
                    ty: swf::Twips::new(-16384),
                    ..Default::default()
                };
                let gradient_matrix = Matrix::from(gradient.matrix) * shift;

                let mut svg_gradient = LinearGradient::new()
                    .set("id", format!("f{}", num_defs))
                    .set("gradientUnits", "userSpaceOnUse")
                    .set(
                        "gradientTransform",
                        format!(
                            "matrix({} {} {} {} {} {})",
                            gradient_matrix.a,
                            gradient_matrix.b,
                            gradient_matrix.c,
                            gradient_matrix.d,
                            gradient_matrix.tx.get(),
                            gradient_matrix.ty.get()
                        ),
                    );
                svg_gradient = match gradient.spread {
                    GradientSpread::Pad => svg_gradient, // default
                    GradientSpread::Reflect => svg_gradient.set("spreadMethod", "reflect"),
                    GradientSpread::Repeat => svg_gradient.set("spreadMethod", "repeat"),
                };
                if gradient.interpolation == GradientInterpolation::LinearRgb {
                    has_linear_rgb_gradient = true;
                    svg_path = svg_path.set("filter", "url('#_linearrgb')");
                }
                for record in &gradient.records {
                    let color = if gradient.interpolation == GradientInterpolation::LinearRgb {
                        srgb_to_linear(record.color.clone())
                    } else {
                        record.color.clone()
                    };
                    let stop = Stop::new()
                        .set("offset", format!("{}%", f32::from(record.ratio) / 2.55))
                        .set(
                            "stop-color",
                            format!(
                                "rgba({},{},{},{})",
                                color.r,
                                color.g,
                                color.b,
                                f32::from(color.a) / 255.0
                            ),
                        );
                    svg_gradient = svg_gradient.add(stop);
                }
                defs = defs.add(svg_gradient);

                let fill_id = format!("url(#f{})", num_defs);
                num_defs += 1;
                fill_id
            }
            FillStyle::RadialGradient(gradient) => {
                let shift = Matrix {
                    a: 32768.0,
                    d: 32768.0,
                    ..Default::default()
                };
                let gradient_matrix = Matrix::from(gradient.matrix) * shift;

                let mut svg_gradient = RadialGradient::new()
                    .set("id", format!("f{}", num_defs))
                    .set("gradientUnits", "userSpaceOnUse")
                    .set("cx", "0")
                    .set("cy", "0")
                    .set("r", "0.5")
                    .set(
                        "gradientTransform",
                        format!(
                            "matrix({} {} {} {} {} {})",
                            gradient_matrix.a,
                            gradient_matrix.b,
                            gradient_matrix.c,
                            gradient_matrix.d,
                            gradient_matrix.tx.get(),
                            gradient_matrix.ty.get()
                        ),
                    );
                svg_gradient = match gradient.spread {
                    GradientSpread::Pad => svg_gradient, // default
                    GradientSpread::Reflect => svg_gradient.set("spreadMethod", "reflect"),
                    GradientSpread::Repeat => svg_gradient.set("spreadMethod", "repeat"),
                };
                if gradient.interpolation == GradientInterpolation::LinearRgb {
                    has_linear_rgb_gradient = true;
                    svg_path = svg_path.set("filter", "url('#_linearrgb')");
                }
                for record in &gradient.records {
                    let color = if gradient.interpolation == GradientInterpolation::LinearRgb {
                        srgb_to_linear(record.color.clone())
                    } else {
                        record.color.clone()
                    };
                    let stop = Stop::new()
                        .set("offset", format!("{}%", f32::from(record.ratio) / 2.55))
                        .set(
                            "stop-color",
                            format!(
                                "rgba({},{},{},{})",
                                color.r,
                                color.g,
                                color.b,
                                f32::from(color.a) / 255.0
                            ),
                        );
                    svg_gradient = svg_gradient.add(stop);
                }
                defs = defs.add(svg_gradient);

                let fill_id = format!("url(#f{})", num_defs);
                num_defs += 1;
                fill_id
            }
            FillStyle::FocalGradient {
                gradient,
                focal_point,
            } => {
                let shift = Matrix {
                    a: 32768.0,
                    d: 32768.0,
                    ..Default::default()
                };
                let gradient_matrix = Matrix::from(gradient.matrix) * shift;

                let mut svg_gradient = RadialGradient::new()
                    .set("id", format!("f{}", num_defs))
                    .set("fx", focal_point.to_f32() / 2.0)
                    .set("gradientUnits", "userSpaceOnUse")
                    .set("cx", "0")
                    .set("cy", "0")
                    .set("r", "0.5")
                    .set(
                        "gradientTransform",
                        format!(
                            "matrix({} {} {} {} {} {})",
                            gradient_matrix.a,
                            gradient_matrix.b,
                            gradient_matrix.c,
                            gradient_matrix.d,
                            gradient_matrix.tx.get(),
                            gradient_matrix.ty.get()
                        ),
                    );
                svg_gradient = match gradient.spread {
                    GradientSpread::Pad => svg_gradient, // default
                    GradientSpread::Reflect => svg_gradient.set("spreadMethod", "reflect"),
                    GradientSpread::Repeat => svg_gradient.set("spreadMethod", "repeat"),
                };
                if gradient.interpolation == GradientInterpolation::LinearRgb {
                    has_linear_rgb_gradient = true;
                    svg_path = svg_path.set("filter", "url('#_linearrgb')");
                }
                for record in &gradient.records {
                    let color = if gradient.interpolation == GradientInterpolation::LinearRgb {
                        srgb_to_linear(record.color.clone())
                    } else {
                        record.color.clone()
                    };
                    let stop = Stop::new()
                        .set("offset", format!("{}%", f32::from(record.ratio) / 2.55))
                        .set(
                            "stop-color",
                            format!(
                                "rgba({},{},{},{})",
                                color.r,
                                color.g,
                                color.b,
                                f32::from(color.a) / 255.0
                            ),
                        );
                    svg_gradient = svg_gradient.add(stop);
                }
                defs = defs.add(svg_gradient);

                let fill_id = format!("url(#f{})", num_defs);
                num_defs += 1;
                fill_id
            }
            FillStyle::Bitmap {
                id,
                matrix,
                is_smoothed,
                is_repeating,
            } => {
                if let Some(bitmap) = bitmap_source
                    .bitmap(*id)
                    .and_then(|bitmap| bitmaps.get(bitmap.handle.0))
                {
                    if !bitmap_defs.contains(id) {
                        let mut image = Image::new()
                            .set("width", bitmap.width)
                            .set("height", bitmap.height)
                            .set(
                                "xlink:href",
                                bitmap.get_or_compute_data_uri().as_ref().unwrap().clone(),
                            );

                        if !*is_smoothed {
                            image = image.set("image-rendering", pixelated_property_value);
                        }

                        let mut bitmap_pattern = Pattern::new()
                            .set("id", format!("b{}", id))
                            .set("patternUnits", "userSpaceOnUse");

                        if !*is_repeating {
                            bitmap_pattern = bitmap_pattern
                                .set("width", bitmap.width)
                                .set("height", bitmap.height);
                        } else {
                            bitmap_pattern = bitmap_pattern
                                .set("width", bitmap.width)
                                .set("height", bitmap.height)
                                .set("viewBox", format!("0 0 {} {}", bitmap.width, bitmap.height));
                        }

                        bitmap_pattern = bitmap_pattern.add(image);

                        defs = defs.add(bitmap_pattern);
                        bitmap_defs.insert(*id);
                    }
                } else {
                    log::error!("Couldn't fill shape with unknown bitmap {}", id);
                }

                let svg_pattern = Pattern::new()
                    .set("id", format!("f{}", num_defs))
                    .set("xlink:href", format!("#b{}", id))
                    .set(
                        "patternTransform",
                        format!(
                            "matrix({} {} {} {} {} {})",
                            matrix.a,
                            matrix.b,
                            matrix.c,
                            matrix.d,
                            matrix.tx.get(),
                            matrix.ty.get()
                        ),
                    );

                defs = defs.add(svg_pattern);

                let fill_id = format!("url(#f{})", num_defs);
                num_defs += 1;
                fill_id
            }
        };

        let mut data = Data::new();
        for command in commands {
            data = match command {
                DrawCommand::MoveTo { x, y } => data.move_to((x.get(), y.get())),
                DrawCommand::LineTo { x, y } => data.line_to((x.get(), y.get())),
                DrawCommand::CurveTo { x1, y1, x2, y2 } => {
                    data.quadratic_curve_to((x1.get(), y1.get(), x2.get(), y2.get()))
                }
            };
        }

        match path {
            DrawPath::Fill { .. } => {
                svg_path = svg_path
                    .set("fill", fill)
                    .set("fill-rule", "evenodd")
                    .set("d", data);
                svg_paths.push(svg_path);
            }
            DrawPath::Stroke {
                style, is_closed, ..
            } => {
                // Flash always renders strokes with a minimum width of 1 pixel (20 twips).
                // Additionally, many SWFs use the "hairline" stroke setting, which sets the stroke's width
                // to 1 twip. Because of the minimum, this will effectively make the stroke nearly-always render
                // as 1 pixel wide.
                // SVG doesn't have a minimum and can render strokes at fractional widths, so these hairline
                // strokes end up rendering very faintly if we use the actual width of 1 twip.
                // Therefore, we clamp the stroke width to 1 pixel (20 twips). This won't be 100% accurate
                // if the shape is scaled, but it looks much closer to the Flash Player.
                let stroke_width = std::cmp::max(style.width().get(), 20);
                svg_path = svg_path
                    .set("fill", "none")
                    .set("stroke", fill)
                    .set("stroke-width", stroke_width)
                    .set(
                        "stroke-linecap",
                        match style.start_cap() {
                            LineCapStyle::Round => "round",
                            LineCapStyle::Square => "square",
                            LineCapStyle::None => "butt",
                        },
                    )
                    .set(
                        "stroke-linejoin",
                        match style.join_style() {
                            LineJoinStyle::Round => "round",
                            LineJoinStyle::Bevel => "bevel",
                            LineJoinStyle::Miter(_) => "miter",
                        },
                    );

                if let LineJoinStyle::Miter(miter_limit) = style.join_style() {
                    svg_path = svg_path.set("stroke-miterlimit", miter_limit.to_f32());
                }

                if is_closed {
                    data = data.close();
                }

                svg_path = svg_path.set("d", data);
                svg_paths.push(svg_path);
            }
        }
    }

    // If this shape contains a gradient in linear RGB space, add a filter to do the color space adjustment.
    // We have to use a filter because browser don't seem to implement the `color-interpolation` SVG property.
    if has_linear_rgb_gradient {
        // Add a filter to convert from linear space to sRGB space.
        let mut filter = Filter::new()
            .set("id", "_linearrgb")
            .set("color-interpolation-filters", "sRGB");
        let text = svg::node::Text::new(
            r#"
            <feComponentTransfer>
                <feFuncR type="gamma" exponent="0.4545454545"></feFuncR>
                <feFuncG type="gamma" exponent="0.4545454545"></feFuncG>
                <feFuncB type="gamma" exponent="0.4545454545"></feFuncB>
            </feComponentTransfer>
            "#,
        );
        filter = filter.add(text);
        defs = defs.add(filter);
        num_defs += 1;
    }

    if num_defs > 0 {
        document = document.add(defs);
    }

    for svg_path in svg_paths {
        document = document.add(svg_path);
    }

    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
    let svg = document.to_string();
    let svg_encoded = format!(
        "data:image/svg+xml,{}",
        utf8_percent_encode(&svg, NON_ALPHANUMERIC)
    );

    let image = HtmlImageElement::new().unwrap();
    image.set_src(&svg_encoded);

    let mut data = ShapeData(vec![]);
    data.0.push(CanvasDrawCommand::DrawImage {
        image,
        x_min: shape.shape_bounds.x_min.to_pixels(),
        y_min: shape.shape_bounds.y_min.to_pixels(),
    });

    data
}

/// Convert a series of `DrawCommands` to a `Path2d` shape.
///
/// The path can be optionally closed by setting `is_closed` to `true`.
///
/// The resulting path is in the shape's own coordinate space and needs to be
/// transformed to fit within the shape's bounds.
fn draw_commands_to_path2d(commands: &[DrawCommand], is_closed: bool) -> Path2d {
    let path = Path2d::new().unwrap();
    for command in commands {
        match command {
            DrawCommand::MoveTo { x, y } => path.move_to(x.get().into(), y.get().into()),
            DrawCommand::LineTo { x, y } => path.line_to(x.get().into(), y.get().into()),
            DrawCommand::CurveTo { x1, y1, x2, y2 } => path.quadratic_curve_to(
                x1.get().into(),
                y1.get().into(),
                x2.get().into(),
                y2.get().into(),
            ),
        };
    }

    if is_closed {
        path.close_path();
    }

    path
}

fn swf_shape_to_canvas_commands(
    shape: &DistilledShape,
    bitmap_source: &dyn BitmapSource,
    bitmaps: &[BitmapData],
    _pixelated_property_value: &str,
    context: &CanvasRenderingContext2d,
) -> Option<ShapeData> {
    use ruffle_core::shape_utils::DrawPath;
    use swf::{FillStyle, LineCapStyle, LineJoinStyle};

    // Some browsers will vomit if you try to load/draw an image with 0 width/height.
    // TODO(Herschel): Might be better to just return None in this case and skip
    // rendering altogether.
    let (_width, _height) = (
        f32::max(
            (shape.shape_bounds.x_max - shape.shape_bounds.x_min).get() as f32,
            1.0,
        ),
        f32::max(
            (shape.shape_bounds.y_max - shape.shape_bounds.y_min).get() as f32,
            1.0,
        ),
    );

    let mut canvas_data = ShapeData(vec![]);

    let matrix_factory: SvgsvgElement = web_sys::window()
        .expect("window")
        .document()
        .expect("document")
        .create_element_ns(Some("http://www.w3.org/2000/svg"), "svg")
        .expect("create_element on svg")
        .dyn_into::<SvgsvgElement>()
        .expect("an actual SVG element");

    let bounds_viewbox_matrix = matrix_factory.create_svg_matrix();
    bounds_viewbox_matrix.set_a(1.0 / 20.0);
    bounds_viewbox_matrix.set_d(1.0 / 20.0);

    for path in &shape.paths {
        let (style, commands, is_fill, is_closed) = match &path {
            DrawPath::Fill {
                style, commands, ..
            } => (*style, commands, true, false),
            DrawPath::Stroke {
                style,
                commands,
                is_closed,
            } => (style.fill_style(), commands, false, *is_closed),
        };
        let fill_style = match style {
            FillStyle::Color(Color { r, g, b, a }) => CanvasFillStyle::Color(CanvasColor(
                format!("rgba({},{},{},{})", r, g, b, f32::from(*a) / 255.0),
                *r,
                *g,
                *b,
                *a,
            )),
            FillStyle::LinearGradient(gradient) => {
                create_linear_gradient(context, gradient, is_fill).unwrap()
            }
            FillStyle::RadialGradient(gradient) => {
                create_radial_gradient(context, gradient, 0.0, is_fill).unwrap()
            }
            FillStyle::FocalGradient {
                gradient,
                focal_point,
            } => create_radial_gradient(context, gradient, focal_point.to_f64(), is_fill).unwrap(),
            FillStyle::Bitmap {
                id,
                matrix,
                is_smoothed,
                is_repeating,
            } => {
                if let Some(bitmap) = bitmap_source
                    .bitmap(*id)
                    .and_then(|bitmap| bitmaps.get(bitmap.handle.0))
                {
                    let repeat = if !*is_repeating {
                        // NOTE: The WebGL backend does clamping in this case, just like
                        // Flash Player, but CanvasPattern has no such option...
                        "no-repeat"
                    } else {
                        "repeat"
                    };

                    let bitmap_pattern = match &bitmap.image {
                        BitmapDataStorage::ImageElement(elem) => context
                            .create_pattern_with_html_image_element(elem, repeat)
                            .expect("pattern creation success")?,
                        BitmapDataStorage::CanvasElement(canvas, _context) => context
                            .create_pattern_with_html_canvas_element(canvas, repeat)
                            .expect("pattern creation success")?,
                    };

                    let a = *matrix;

                    let matrix = matrix_factory.create_svg_matrix();

                    // The `1.0 / 20.0` in `bounds_viewbox_matrix` does not
                    // affect this, so we have to do it manually here.
                    matrix.set_a(a.a.to_f32() / 20.0);
                    matrix.set_b(a.b.to_f32() / 20.0);
                    matrix.set_c(a.c.to_f32() / 20.0);
                    matrix.set_d(a.d.to_f32() / 20.0);
                    matrix.set_e(a.tx.get() as f32 / 20.0);
                    matrix.set_f(a.ty.get() as f32 / 20.0);

                    bitmap_pattern.set_transform(&matrix);

                    CanvasFillStyle::Pattern(bitmap_pattern, *is_smoothed)
                } else {
                    log::error!("Couldn't fill shape with unknown bitmap {}", id);
                    CanvasFillStyle::Color(CanvasColor("rgba(0,0,0,0)".to_string(), 0, 0, 0, 0))
                }
            }
        };

        let canvas_path = Path2d::new().unwrap();
        canvas_path.add_path_with_transformation(
            &draw_commands_to_path2d(commands, is_closed),
            &bounds_viewbox_matrix,
        );

        match path {
            DrawPath::Fill { .. } => {
                canvas_data.0.push(CanvasDrawCommand::Fill {
                    path: canvas_path,
                    fill_style,
                });
            }
            DrawPath::Stroke { style, .. } => {
                // Flash always renders strokes with a minimum width of 1 pixel (20 twips).
                // Additionally, many SWFs use the "hairline" stroke setting, which sets the stroke's width
                // to 1 twip. Because of the minimum, this will effectively make the stroke nearly-always render
                // as 1 pixel wide.
                // SVG doesn't have a minimum and can render strokes at fractional widths, so these hairline
                // strokes end up rendering very faintly if we use the actual width of 1 twip.
                // Therefore, we clamp the stroke width to 1 pixel (20 twips). This won't be 100% accurate
                // if the shape is scaled, but it looks much closer to the Flash Player.
                let line_width = std::cmp::max(style.width().get(), 20);
                let line_cap = match style.start_cap() {
                    LineCapStyle::Round => "round",
                    LineCapStyle::Square => "square",
                    LineCapStyle::None => "butt",
                };
                let (line_join, miter_limit) = match style.join_style() {
                    LineJoinStyle::Round => ("round", 999_999.0),
                    LineJoinStyle::Bevel => ("bevel", 999_999.0),
                    LineJoinStyle::Miter(ml) => ("miter", ml.to_f32()),
                };
                canvas_data.0.push(CanvasDrawCommand::Stroke {
                    path: canvas_path,
                    line_width: line_width as f64 / 20.0,
                    stroke_style: fill_style,
                    line_cap: line_cap.to_string(),
                    line_join: line_join.to_string(),
                    miter_limit: miter_limit as f64 / 20.0,
                });
            }
        }
    }

    Some(canvas_data)
}

/// Converts an SWF color from sRGB space to linear color space.
pub fn srgb_to_linear(mut color: swf::Color) -> swf::Color {
    fn to_linear_channel(n: u8) -> u8 {
        let mut n = f32::from(n) / 255.0;
        n = if n <= 0.04045 {
            n / 12.92
        } else {
            f32::powf((n + 0.055) / 1.055, 2.4)
        };
        (n.clamp(0.0, 1.0) * 255.0).round() as u8
    }
    color.r = to_linear_channel(color.r);
    color.g = to_linear_channel(color.g);
    color.b = to_linear_channel(color.b);
    color
}

fn create_linear_gradient(
    context: &CanvasRenderingContext2d,
    gradient: &swf::Gradient,
    is_fill: bool,
) -> Result<CanvasFillStyle, JsError> {
    // Canvas linear gradients are configured via the line endpoints, so we only need
    // to transform it if the basis is not orthogonal (skew in the transform).
    let transformed = if is_fill {
        let dot = gradient.matrix.a * gradient.matrix.c + gradient.matrix.b * gradient.matrix.d;
        dot.to_f32().abs() > GRADIENT_TRANSFORM_THRESHOLD
    } else {
        // TODO: Gradient transforms don't work correctly with strokes.
        false
    };
    let create_fn = |matrix: swf::Matrix, gradient_scale: f64| {
        let start = matrix * (swf::Twips::new(-16384), swf::Twips::ZERO);
        let end = matrix * (swf::Twips::new(16384), swf::Twips::ZERO);
        // If we have to scale the gradient due to spread mode, scale the endpoints away from the center.
        let dx = 0.5 * (gradient_scale - 1.0) * (end.0 - start.0).to_pixels();
        let dy = 0.5 * (gradient_scale - 1.0) * (end.1 - start.1).to_pixels();
        Ok(context.create_linear_gradient(
            start.0.to_pixels() - dx,
            start.1.to_pixels() - dy,
            end.0.to_pixels() + dx,
            end.1.to_pixels() + dy,
        ))
    };
    swf_to_canvas_gradient(gradient, transformed, create_fn)
}

fn create_radial_gradient(
    context: &CanvasRenderingContext2d,
    gradient: &swf::Gradient,
    focal_point: f64,
    is_fill: bool,
) -> Result<CanvasFillStyle, JsError> {
    // Canvas radial gradients can not be elliptical or skewed, so transform if there
    // is a non-uniform scale or skew.
    // A scale rotation matrix is always of the form:
    // [[a  b]
    //  [-b a]]
    let transformed = if is_fill {
        (gradient.matrix.a - gradient.matrix.d).to_f32().abs() > GRADIENT_TRANSFORM_THRESHOLD
            || (gradient.matrix.b + gradient.matrix.c).to_f32().abs() > GRADIENT_TRANSFORM_THRESHOLD
    } else {
        // TODO: Gradient transforms don't work correctly with strokes.
        false
    };
    let create_fn = |matrix: swf::Matrix, gradient_scale: f64| {
        let focal_center = matrix
            * (
                swf::Twips::new((focal_point * 16384.0) as i32),
                swf::Twips::ZERO,
            );
        let center = matrix * (swf::Twips::ZERO, swf::Twips::ZERO);
        let end = matrix * (swf::Twips::new(16384), swf::Twips::ZERO);
        let dx = (end.0 - center.0).to_pixels();
        let dy = (end.1 - center.1).to_pixels();
        let radius = (dx * dx + dy * dy).sqrt();
        context
            .create_radial_gradient(
                focal_center.0.to_pixels(),
                focal_center.1.to_pixels(),
                0.0,
                center.0.to_pixels(),
                center.1.to_pixels(),
                // Radius needs to be scaled if gradient spread mode is active.
                radius * gradient_scale,
            )
            .into_js_result()
    };
    swf_to_canvas_gradient(gradient, transformed, create_fn)
}

/// Converts an SWF gradient to a canvas gradient.
///
/// If the SWF gradient has a "simple" transform, this is a direct translation to `CanvasGradient`.
/// If transform is "complex" (skewing or non-uniform scaling), we have to do some trickery and
/// transform the entire path, because canvas does not have a direct way to render a transformed
/// gradient.
fn swf_to_canvas_gradient(
    swf_gradient: &swf::Gradient,
    transformed: bool,
    mut create_gradient_fn: impl FnMut(swf::Matrix, f64) -> Result<CanvasGradient, JsError>,
) -> Result<CanvasFillStyle, JsError> {
    let matrix = if transformed {
        // When we are rendering a complex gradient, the gradient transform is handled later by
        // transforming the path before rendering; so use the indentity matrix here.
        swf::Matrix::scale(swf::Fixed16::from_f64(20.0), swf::Fixed16::from_f64(20.0))
    } else {
        swf_gradient.matrix
    };

    const NUM_REPEATS: f32 = 25.0;
    let gradient_scale = if swf_gradient.spread == swf::GradientSpread::Pad {
        1.0
    } else {
        f64::from(NUM_REPEATS)
    };

    // Canvas does not have support for spread/repeat modes (reflect+repeat), so we have to
    // simulate these repeat modes by duplicating color stops.
    // TODO: We'll hit the edge if the gradient is shrunk way down, but don't think we can do
    // anything better using the current Canvas API. Maybe we could consider the size of the
    // shape here to make sure we fill the area.
    let canvas_gradient = create_gradient_fn(matrix, gradient_scale)?;
    let color_stops: Vec<_> = swf_gradient
        .records
        .iter()
        .map(|record| {
            (
                f32::from(record.ratio) / 255.0,
                format!(
                    "rgba({},{},{},{})",
                    record.color.r,
                    record.color.g,
                    record.color.b,
                    f32::from(record.color.a) / 255.0
                ),
            )
        })
        .collect();

    match swf_gradient.spread {
        swf::GradientSpread::Pad => {
            for stop in color_stops {
                canvas_gradient
                    .add_color_stop(stop.0, &stop.1)
                    .warn_on_error();
            }
        }
        swf::GradientSpread::Reflect => {
            let mut t = 0.0;
            let step = 1.0 / NUM_REPEATS;
            while t < 1.0 {
                // Add the colors forward.
                for stop in &color_stops {
                    canvas_gradient
                        .add_color_stop(t + stop.0 * step, &stop.1)
                        .warn_on_error();
                }
                t += step;
                // Add the colors backward.
                for stop in color_stops.iter().rev() {
                    canvas_gradient
                        .add_color_stop(t + (1.0 - stop.0) * step, &stop.1)
                        .warn_on_error();
                }
                t += step;
            }
        }
        swf::GradientSpread::Repeat => {
            let first_stop = color_stops.first().unwrap();
            let last_stop = color_stops.last().unwrap();
            let mut t = 0.0;
            let step = 1.0 / NUM_REPEATS;
            while t < 1.0 {
                // Duplicate the start/end stops to ensure we don't blend between the seams.
                canvas_gradient
                    .add_color_stop(t, &first_stop.1)
                    .warn_on_error();
                for stop in &color_stops {
                    canvas_gradient
                        .add_color_stop(t + stop.0 * step, &stop.1)
                        .warn_on_error();
                }
                canvas_gradient
                    .add_color_stop(t + step, &last_stop.1)
                    .warn_on_error();
                t += step;
            }
        }
    }

    if transformed {
        // When we render this gradient, we will push the gradient's transform to the canvas,
        // and then transform the path itself by the inverse.
        let matrix = DomMatrix::new_with_array64(
            [
                swf_gradient.matrix.a.to_f64() / 20.0,
                swf_gradient.matrix.b.to_f64() / 20.0,
                swf_gradient.matrix.c.to_f64() / 20.0,
                swf_gradient.matrix.d.to_f64() / 20.0,
                swf_gradient.matrix.tx.to_pixels(),
                swf_gradient.matrix.ty.to_pixels(),
            ]
            .as_mut_slice(),
        )
        .into_js_result()?;
        let inverse_gradient_matrix = matrix.inverse();
        Ok(CanvasFillStyle::TransformedGradient(TransformedGradient {
            gradient: canvas_gradient,
            gradient_matrix: [
                matrix.a(),
                matrix.b(),
                matrix.c(),
                matrix.d(),
                matrix.e(),
                matrix.f(),
            ],
            inverse_gradient_matrix,
        }))
    } else {
        Ok(CanvasFillStyle::Gradient(canvas_gradient))
    }
}
