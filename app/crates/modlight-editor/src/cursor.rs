//! Cursor images for the cursor annotation (`WindowsCursorBitmapRenderer`).
//!
//! Upstream draws the Windows system cursors and has none elsewhere (its Mac build
//! creates no cursor annotation at all). The port asks the host for the platform's
//! cursor images: the macOS app installs a provider reading `NSCursor`; tests and the
//! QA screens may install their own.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use image::RgbaImage;

use crate::model::CursorType;
use crate::raster::{self, Filter};

pub type CursorProvider = Box<dyn Fn(CursorType) -> Option<RgbaImage> + Send + Sync>;

fn provider() -> &'static RwLock<Option<CursorProvider>> {
    static PROVIDER: OnceLock<RwLock<Option<CursorProvider>>> = OnceLock::new();
    PROVIDER.get_or_init(|| RwLock::new(None))
}

fn cache() -> &'static Mutex<HashMap<CursorType, Option<Arc<RgbaImage>>>> {
    static CACHE: OnceLock<Mutex<HashMap<CursorType, Option<Arc<RgbaImage>>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Installs the host's cursor images (replacing any earlier provider).
pub fn set_provider(f: CursorProvider) {
    *provider().write().unwrap() = Some(f);
    cache().lock().unwrap().clear();
}

/// `CreateAnnotationBitmap`: the cursor at its natural size, or `None` when the host has
/// none for this type.
pub fn annotation_bitmap(cursor: CursorType) -> Option<Arc<RgbaImage>> {
    if let Some(found) = cache().lock().unwrap().get(&cursor) {
        return found.clone();
    }
    let image = provider()
        .read()
        .unwrap()
        .as_ref()
        .and_then(|f| f(cursor))
        .map(Arc::new);
    cache().lock().unwrap().insert(cursor, image.clone());
    image
}

/// `CreatePreviewBitmap`: the cursor fitted into a square with 2 px of margin.
pub fn preview_bitmap(cursor: CursorType, size: u32) -> Option<RgbaImage> {
    let bitmap = annotation_bitmap(cursor)?;
    let canvas = size.max(16);
    let max = (canvas as f32 - 4.0).max(1.0);
    let mut scale = (max / bitmap.width() as f32).min(max / bitmap.height() as f32);
    if !scale.is_finite() || scale <= 0.0 {
        scale = 1.0;
    }
    let w = (bitmap.width() as f32 * scale).round().max(1.0) as u32;
    let h = (bitmap.height() as f32 * scale).round().max(1.0) as u32;
    let scaled = raster::resize(&bitmap, w, h, Filter::Linear);
    let mut out = RgbaImage::new(canvas, canvas);
    let left = ((canvas - w) / 2) as i64;
    let top = ((canvas - h) / 2) as i64;
    image::imageops::overlay(&mut out, &scaled, left, top);
    Some(out)
}
