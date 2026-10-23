//! Pictures of the screen on Windows (upstream's GDI `Screenshot`, `CaptureHelpers`,
//! `WindowsList`): monitors, the pointer and the window list from User32 and DWM, pixels
//! from GDI, and frames for the recorder.
//!
//! Coordinates are the app's screen points: physical pixels divided by the scale of the
//! primary monitor (`ScreenInfo::scale`, the same for every monitor), which is how egui
//! places windows. On a desktop whose monitors share one scale (the usual case)
//! everything lines up exactly; see [`desktop`] for mixed scales. Pictures are physical
//! pixels. Every call reads physical pixels whatever DPI awareness the calling thread
//! has (see [`desktop::PhysicalPixels`]), so nothing here depends on the process having
//! declared itself DPI aware first.
//!
//! Nothing here is called by QA or automated runs of the app; they use
//! `modlight_core::capture::source::FakeSource`.

pub(crate) mod desktop;
mod frames;
mod gdi;
mod transparent;
pub(crate) mod winlist;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use image::RgbaImage;
use modlight_core::capture::geometry::IRect;
use modlight_core::capture::source::{
    CaptureError, CaptureSource, ScreenInfo, WindowCapture, WindowInfo,
};
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorPos, SW_RESTORE, ShowWindow};

use desktop::{
    MonitorNames, PhysicalPixels, monitors, primary_scale, rect_to_pixels, rect_to_points,
    to_points, virtual_bounds,
};
use winlist::{
    bring_to_front, client_rect, describe, foreground, id_hwnd, is_minimized, is_window, own_pid,
    visible_windows, window_pid, window_rect,
};

pub use frames::ScreenFrames;

/// The Windows desktop, as the capture code sees it.
#[derive(Default)]
pub struct ScreenSource {
    names: MonitorNames,
}

impl ScreenSource {
    pub fn new() -> Arc<ScreenSource> {
        Arc::new(ScreenSource::default())
    }

    /// Kept for the macOS API: nothing on Windows has to be read on the main thread.
    pub fn refresh_on_main_thread(&self) {}
}

impl CaptureSource for ScreenSource {
    /// `Screen.AllScreens`, the primary monitor first; ids are the display numbers.
    fn screens(&self) -> Vec<ScreenInfo> {
        let scale = primary_scale();
        monitors()
            .into_iter()
            .map(|monitor| ScreenInfo {
                id: monitor.id(),
                bounds: rect_to_points(monitor.bounds, scale),
                working_area: rect_to_points(monitor.work, scale),
                scale,
                primary: monitor.primary,
                name: self.names.name(&monitor.device),
            })
            .collect()
    }

    fn cursor_position(&self) -> (i32, i32) {
        let _pixels = PhysicalPixels::enter();
        let mut point = POINT::default();
        // SAFETY: an out-parameter.
        if unsafe { GetCursorPos(&mut point) } == 0 {
            return (0, 0);
        }
        let scale = primary_scale();
        (to_points(point.x, scale), to_points(point.y, scale))
    }

    fn windows(&self) -> Vec<WindowInfo> {
        let _pixels = PhysicalPixels::enter();
        visible_windows(primary_scale())
    }

    /// The foreground window, as upstream's `GetForegroundWindow`. One of this app's own
    /// windows (the region overlay, the main window before it hides) is not an "active
    /// window" to capture or to name a capture after, as on macOS: `None` then.
    fn active_window(&self) -> Option<WindowInfo> {
        let _pixels = PhysicalPixels::enter();
        let hwnd = foreground()?;
        if window_pid(hwnd) == own_pid() {
            return None;
        }
        let rect = window_rect(hwnd)?;
        Some(describe(hwnd, rect, primary_scale(), &mut HashMap::new()))
    }

    /// `Screenshot.CaptureRectangle`: the part of `rect` on the desktop
    /// (`RemoveOutsideScreenArea`), physical pixels.
    fn capture_rect(&self, rect: IRect, cursor: bool) -> Result<RgbaImage, CaptureError> {
        let _pixels = PhysicalPixels::enter();
        let pixels = rect_to_pixels(rect, primary_scale());
        let pixels = pixels.intersect(&virtual_bounds(&monitors()));
        if !pixels.has_area() {
            return Err(CaptureError::Nothing);
        }
        gdi::capture_pixels(pixels, cursor)
    }

    /// `CaptureWindow.Execute`'s picture of one window, read where the window is now:
    /// transparent with or without its shadow (`CaptureWindowTransparent`), else its
    /// client area (`CaptureClientArea`) or its frame as it shows on the desktop.
    fn capture_window(
        &self,
        window: &WindowInfo,
        how: WindowCapture,
    ) -> Result<RgbaImage, CaptureError> {
        let _pixels = PhysicalPixels::enter();
        let hwnd = id_hwnd(window.id);
        if !is_window(hwnd) {
            return Err(CaptureError::Nothing);
        }
        if how.transparent
            && !how.client_area
            && let Some(result) = transparent::capture(hwnd, how.shadow, how.cursor)
        {
            return result;
        }
        let rect = if how.client_area {
            client_rect(hwnd)
        } else {
            window_rect(hwnd)
        }
        .ok_or(CaptureError::Nothing)?;
        let rect = rect.intersect(&virtual_bounds(&monitors()));
        if !rect.has_area() {
            return Err(CaptureError::Nothing);
        }
        gdi::capture_pixels(rect, how.cursor)
    }

    /// `CursorData`: the pointer's picture (physical pixels, transparent) and its top
    /// left in points, for the region capture to draw as an object of its own.
    fn cursor_image(&self) -> Option<(RgbaImage, (i32, i32))> {
        let _pixels = PhysicalPixels::enter();
        let cursor = gdi::Cursor::current()?;
        let picture = cursor.image()?;
        let (x, y) = cursor.top_left();
        let scale = primary_scale();
        Some((picture, (to_points(x, scale), to_points(y, scale))))
    }

    /// `CaptureWindow.Execute`: a minimised window is restored (and given 250 ms to
    /// draw), a window that is not the active one is brought to the front (100 ms).
    fn activate_window(&self, window: &WindowInfo) {
        let hwnd = id_hwnd(window.id);
        if !is_window(hwnd) {
            return;
        }
        if is_minimized(hwnd) {
            // SAFETY: a plain call on a live window.
            unsafe { ShowWindow(hwnd, SW_RESTORE) };
            std::thread::sleep(Duration::from_millis(250));
        }
        if foreground() != Some(hwnd) {
            bring_to_front(hwnd);
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_in_points() {
        let source = ScreenSource::new();
        let screens = source.screens();
        assert!(!screens.is_empty());
        assert_eq!(screens.iter().filter(|s| s.primary).count(), 1);
        let primary = &screens[0];
        assert!(primary.primary);
        assert_eq!((primary.bounds.x, primary.bounds.y), (0, 0));
        let scale = primary.scale;
        for screen in &screens {
            assert_eq!(screen.scale, scale);
            assert!(screen.bounds.contains_rect(&screen.working_area));
            assert!(!screen.name.is_empty());
        }
        // Ids are stable and distinct.
        let again: Vec<u32> = source.screens().iter().map(|s| s.id).collect();
        assert_eq!(again, screens.iter().map(|s| s.id).collect::<Vec<_>>());
        let mut ids = again.clone();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), screens.len());
        // The points of the primary monitor are its pixels over its scale.
        let pixels = monitors().into_iter().find(|m| m.primary).unwrap().bounds;
        assert_eq!(primary.bounds, rect_to_points(pixels, scale));
        let (x, y) = source.cursor_position();
        let bounds = source.screen_bounds();
        assert!(
            x >= bounds.x - 1 && x <= bounds.right() + 1,
            "{x} outside {bounds:?}"
        );
        assert!(y >= bounds.y - 1 && y <= bounds.bottom() + 1);
    }

    #[test]
    fn a_small_capture_has_the_physical_size() {
        let source = ScreenSource::new();
        let scale = source.screens()[0].scale;
        let rect = IRect::new(40, 30, 50, 20);
        let picture = source.capture_rect(rect, false).unwrap();
        let pixels = rect_to_pixels(rect, scale);
        assert_eq!(
            picture.dimensions(),
            (pixels.width as u32, pixels.height as u32)
        );
        assert!(picture.pixels().all(|p| p[3] == 255));
        // With the pointer, and partly off the desktop: clipped to it.
        let bounds = source.screen_bounds();
        let picture = source
            .capture_rect(IRect::new(bounds.right() - 10, 0, 30, 10), true)
            .unwrap();
        assert_eq!(picture.height(), to_pixels_len(10, scale));
        assert!(picture.width() <= to_pixels_len(10, scale) + 1);
        assert_eq!(
            source.capture_rect(IRect::new(bounds.right() + 5, 0, 10, 10), false),
            Err(CaptureError::Nothing)
        );
    }

    fn to_pixels_len(points: i32, scale: f64) -> u32 {
        desktop::to_pixels(points, scale) as u32
    }

    #[test]
    fn windows_of_other_apps_only() {
        let source = ScreenSource::new();
        let own = std::process::id() as i32;
        for window in source.windows() {
            assert_ne!(window.pid, own);
            assert!(
                !window.process_name.is_empty() || window.pid == 0,
                "{window:?}"
            );
        }
        if let Some(active) = source.active_window() {
            assert_ne!(active.pid, own);
        }
    }

    #[test]
    fn a_gone_window_is_nothing() {
        let source = ScreenSource::new();
        let gone = WindowInfo {
            id: 0,
            ..Default::default()
        };
        assert_eq!(
            source.capture_window(&gone, WindowCapture::default()),
            Err(CaptureError::Nothing)
        );
        // Activating it does nothing (and touches no window).
        source.activate_window(&gone);
    }

    #[test]
    fn the_pointer_picture_in_points() {
        let source = ScreenSource::new();
        if let Some((picture, (x, y))) = source.cursor_image() {
            assert!(picture.width() > 0 && picture.height() > 0);
            let bounds = source.screen_bounds();
            // The picture's corner is near the pointer, which is on the desktop.
            assert!(x > bounds.x - 200 && x < bounds.right() + 200);
            assert!(y > bounds.y - 200 && y < bounds.bottom() + 200);
        }
    }

    /// Takes the foreground window by itself, transparent, with a short-lived backdrop
    /// window behind it. Run by hand with a window of your own in front:
    /// `cargo test -p modlight-windows -- --ignored --nocapture transparent_window`.
    #[test]
    #[ignore = "puts a window on the desktop behind the foreground window"]
    fn transparent_window() {
        let source = ScreenSource::new();
        let window = source.active_window().expect("a foreground window");
        let how = WindowCapture {
            transparent: true,
            shadow: true,
            client_area: false,
            cursor: false,
        };
        let picture = source.capture_window(&window, how).unwrap();
        let out = std::env::temp_dir().join("modlight-transparent-window.png");
        picture.save(&out).unwrap();
        println!("{} -> {}", window.title, out.display());
        assert!(picture.pixels().any(|p| p[3] < 255));
    }
}
