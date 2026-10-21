//! Pictures of the screen on Windows (upstream's GDI `Screenshot`, `CaptureHelpers`):
//! monitors, the pointer and the window list from User32, pixels from GDI, and frames
//! for the recorder.
//!
//! Coordinates are the app's screen points: physical pixels divided by the scale of the
//! primary monitor (`ScreenInfo::scale`), which is how egui places windows. On a desktop
//! whose monitors share one scale (the usual case) everything lines up exactly.

use std::sync::Arc;

use image::RgbaImage;
use modlight_core::capture::geometry::IRect;
use modlight_core::capture::source::{
    CaptureError, CaptureSource, ScreenInfo, WindowCapture, WindowInfo,
};
use modlight_core::recording::session::FrameSource;

/// The Windows desktop, as the capture code sees it.
#[derive(Default)]
pub struct ScreenSource {}

impl ScreenSource {
    pub fn new() -> Arc<ScreenSource> {
        Arc::new(ScreenSource::default())
    }

    /// Kept for the macOS API: nothing on Windows has to be read on the main thread.
    pub fn refresh_on_main_thread(&self) {}
}

impl CaptureSource for ScreenSource {
    fn screens(&self) -> Vec<ScreenInfo> {
        Vec::new()
    }

    fn cursor_position(&self) -> (i32, i32) {
        (0, 0)
    }

    fn windows(&self) -> Vec<WindowInfo> {
        Vec::new()
    }

    fn active_window(&self) -> Option<WindowInfo> {
        None
    }

    fn capture_rect(&self, rect: IRect, cursor: bool) -> Result<RgbaImage, CaptureError> {
        let _ = (rect, cursor);
        Err(CaptureError::Failed("not implemented yet".into()))
    }

    fn capture_window(
        &self,
        window: &WindowInfo,
        how: WindowCapture,
    ) -> Result<RgbaImage, CaptureError> {
        let _ = (window, how);
        Err(CaptureError::Failed("not implemented yet".into()))
    }

    fn activate_window(&self, window: &WindowInfo) {
        let _ = window;
    }
}

/// The recorder's frames: the region's pixels at the requested rate, tightly packed
/// BGRA, without this app's own windows.
pub struct ScreenFrames {
    region: IRect,
    cursor: bool,
    fps: i64,
    size: (u32, u32),
}

impl ScreenFrames {
    pub fn new(region: IRect, cursor: bool, fps: i64) -> ScreenFrames {
        let even = |v: i32| ((v.max(2) as u32) / 2 * 2).max(2);
        ScreenFrames {
            region,
            cursor,
            fps,
            size: (even(region.width), even(region.height)),
        }
    }
}

impl FrameSource for ScreenFrames {
    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn start(&mut self) -> Result<(), String> {
        let _ = (self.region, self.cursor, self.fps);
        Err("not implemented yet".into())
    }

    fn latest(&mut self) -> Option<Arc<Vec<u8>>> {
        None
    }

    fn stop(&mut self) {}
}
