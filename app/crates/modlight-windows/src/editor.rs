//! What the image editor asks of the system: the cursors a cursor annotation shows
//! (the system's own cursors, as upstream's `WindowsCursorBitmapRenderer`), the desktop
//! picture behind the Wallpaper background, and pictures the `image` crate cannot read
//! (through WIC).

use std::path::{Path, PathBuf};

use image::RgbaImage;

/// How the desktop picture is laid out, in the macOS port's terms (`NSImageScaling`:
/// 0 proportionally down, 1 axes independently, 2 none, 3 proportionally up or down).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WallpaperOptions {
    pub scaling: i64,
    pub allow_clipping: bool,
}

/// The cursor a `CursorType` name stands for, as a picture.
pub fn cursor_image(cursor_type: &str) -> Option<RgbaImage> {
    let _ = cursor_type;
    None
}

/// The desktop picture and its layout.
pub fn desktop_wallpaper() -> Option<(PathBuf, WallpaperOptions)> {
    None
}

/// How Windows lays the desktop picture out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallpaperStyle {
    Fill,
    Fit,
    Stretch,
    Tile,
    Center,
    Span,
}

/// The desktop picture's layout.
pub fn desktop_wallpaper_style() -> Option<WallpaperStyle> {
    None
}

/// A picture file read by WIC (HEIC, JPEG XR and the other codecs Windows has).
pub fn load_image(path: &Path) -> Option<RgbaImage> {
    let _ = path;
    None
}
