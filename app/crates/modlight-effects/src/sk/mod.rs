//! The pieces of SkiaSharp the effects use, rebuilt for straight-alpha bitmaps:
//! colours, bitmaps with Skia's premultiplied storage, colour and image filters, blurs,
//! blend modes and a canvas.

pub mod bitmap;
pub mod blend;
pub mod blur;
pub mod canvas;
pub mod color;
pub mod filters;
pub mod raster;
pub mod text;

pub use bitmap::{Bitmap, pixel_op, premul, store, unpremul};
pub use blend::Blend;
pub use canvas::{Canvas, Cap, ColorFilter, Join, Paint, Sampling, Shader, Style, rect, rect_xywh};
pub use color::{C, byte, clamp_to_byte, round, roundd};
pub use filters::{F4, Tile};
pub use text::Font;
