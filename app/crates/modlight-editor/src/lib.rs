//! The image editor of Modlight App for macOS: a port of upstream's
//! `ShareX.ImageEditor` (Avalonia + SkiaSharp) to Rust, egui and tiny-skia.
//!
//! - `model`, `geom`, `color`: the annotations of `Core/Annotations`.
//! - `core`, `history`: `Core/Editor/EditorCore` and `Core/History`.
//! - `text`, `render`: Avalonia's visuals of the annotations, rasterised with tiny-skia,
//!   for the screen and for the exported image.
//! - `effects`: the effect registry the image effects library plugs into.
//! - `ui`: the editor window (`Presentation/**`) drawn with egui.

pub mod annot_effects;
pub mod color;
pub mod core;
pub mod cursor;
pub mod editor;
pub mod effects;
pub mod emoji;
pub mod frame;
pub mod geom;
pub mod history;
pub mod model;
pub mod ops;
pub mod options;
pub mod qa;
pub mod raster;
pub mod render;
pub mod text;
pub mod ui;
