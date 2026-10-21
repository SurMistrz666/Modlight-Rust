//! Where pictures of the screen come from: the displays, the windows on them, the
//! pointer, and the pixels (`Screenshot`, `CaptureHelpers`, `WindowsList`).
//!
//! The real source is the platform's (`modlight-windows` with upstream's GDI and
//! window APIs, `modlight-macos` with ScreenCaptureKit); automated runs use
//! [`FakeSource`], a desktop drawn from PNG files, so nothing ever asks macOS for the
//! Screen Recording permission.

use std::path::Path;
use std::sync::{Arc, Mutex};

use image::{Rgba, RgbaImage, imageops};
use serde::Deserialize;

use super::geometry::IRect;

/// One display (`Screen`), in the desktop's points.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenInfo {
    pub id: u32,
    pub bounds: IRect,
    /// The part not covered by the menu bar and the Dock (`WorkingArea`).
    pub working_area: IRect,
    /// Pixels per point.
    pub scale: f64,
    pub primary: bool,
    #[serde(default)]
    pub name: String,
}

/// One window (`WindowInfo`), front to back in a list.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowInfo {
    /// `CGWindowID`.
    pub id: u32,
    pub title: String,
    /// The owning app's name (upstream's process name, `%pn`).
    pub process_name: String,
    pub pid: i32,
    pub rect: IRect,
    /// Minimised windows are not on screen (`IsMinimized`).
    pub on_screen: bool,
}

/// What went wrong taking a picture.
#[derive(Clone, Debug, PartialEq)]
pub enum CaptureError {
    /// The system refused (no Screen Recording permission).
    NotPermitted,
    /// Nothing to capture (empty area, window gone).
    Nothing,
    Failed(String),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureError::NotPermitted => f.write_str("screen recording is not permitted"),
            CaptureError::Nothing => f.write_str("nothing to capture"),
            CaptureError::Failed(message) => f.write_str(message),
        }
    }
}

/// How a window is taken (`CaptureTransparent`, `CaptureShadow`, `CaptureClientArea`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindowCapture {
    pub transparent: bool,
    pub shadow: bool,
    pub client_area: bool,
    pub cursor: bool,
}

/// The screen as the capture code sees it.
pub trait CaptureSource: Send + Sync {
    fn screens(&self) -> Vec<ScreenInfo>;
    /// The pointer, in desktop points.
    fn cursor_position(&self) -> (i32, i32);
    /// Visible windows of other apps, front to back.
    fn windows(&self) -> Vec<WindowInfo>;
    /// The frontmost window of the active app (`GetForegroundWindow`).
    fn active_window(&self) -> Option<WindowInfo>;
    /// Pixels of an area of the desktop, at the highest scale of the displays it covers.
    fn capture_rect(&self, rect: IRect, cursor: bool) -> Result<RgbaImage, CaptureError>;
    /// One window by itself.
    fn capture_window(
        &self,
        window: &WindowInfo,
        how: WindowCapture,
    ) -> Result<RgbaImage, CaptureError>;
    /// The pointer's picture and its top-left corner in desktop points.
    fn cursor_image(&self) -> Option<(RgbaImage, (i32, i32))> {
        None
    }
    /// Bring a window to the front before it is taken (`WindowInfo.Activate`).
    fn activate_window(&self, _window: &WindowInfo) {}

    /// `GetScreenBounds`: every display together.
    fn screen_bounds(&self) -> IRect {
        let screens = self.screens();
        let mut iter = screens.iter();
        let Some(first) = iter.next() else {
            return IRect::default();
        };
        iter.fold(first.bounds, |acc, s| acc.union(&s.bounds))
    }

    /// The display under the pointer (`Screen.FromPoint(Cursor.Position)`).
    fn active_screen(&self) -> Option<ScreenInfo> {
        let (x, y) = self.cursor_position();
        let screens = self.screens();
        screens
            .iter()
            .find(|s| s.bounds.contains(x, y))
            .or_else(|| screens.iter().find(|s| s.primary))
            .or_else(|| screens.first())
            .cloned()
    }

    /// The scale a picture of `rect` is taken at: the highest of the displays it covers.
    fn scale_for(&self, rect: IRect) -> f64 {
        self.screens()
            .iter()
            .filter(|s| s.bounds.intersect(&rect).has_area())
            .map(|s| s.scale)
            .fold(0.0_f64, f64::max)
            .max(1.0)
    }
}

/// `CaptureHelpers.EvenRectangleSize`.
pub fn even_size(rect: IRect) -> IRect {
    IRect::new(
        rect.x,
        rect.y,
        rect.width - rect.width % 2,
        rect.height - rect.height % 2,
    )
}

/// A desktop drawn from files (QA and tests): one picture of the whole desktop at a
/// given scale, the displays and the windows on it.
pub struct FakeSource {
    desktop: RgbaImage,
    scale: f64,
    screens: Vec<ScreenInfo>,
    windows: Vec<WindowInfo>,
    cursor: Mutex<(i32, i32)>,
    active: Option<usize>,
    /// Every area taken, for tests.
    pub taken: Mutex<Vec<IRect>>,
    /// Every window taken by itself (its title and how), for tests.
    pub windows_taken: Mutex<Vec<(String, WindowCapture)>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FakeLayout {
    scale: Option<f64>,
    screens: Vec<ScreenInfo>,
    #[serde(default)]
    windows: Vec<WindowInfo>,
    #[serde(default)]
    cursor: Option<(i32, i32)>,
    #[serde(default)]
    active_window: Option<usize>,
}

impl FakeSource {
    pub fn new(
        desktop: RgbaImage,
        scale: f64,
        screens: Vec<ScreenInfo>,
        windows: Vec<WindowInfo>,
    ) -> FakeSource {
        let cursor = screens
            .first()
            .map(|s| {
                (
                    s.bounds.x + s.bounds.width / 2,
                    s.bounds.y + s.bounds.height / 2,
                )
            })
            .unwrap_or_default();
        FakeSource {
            desktop,
            scale,
            screens,
            windows,
            cursor: Mutex::new(cursor),
            active: Some(0),
            taken: Mutex::new(Vec::new()),
            windows_taken: Mutex::new(Vec::new()),
        }
    }

    /// `desktop.png` and `layout.json` from a folder (`<qa-profile>/qa-capture`).
    pub fn from_folder(folder: &Path) -> Option<FakeSource> {
        let desktop = image::open(folder.join("desktop.png")).ok()?.to_rgba8();
        let layout: FakeLayout =
            serde_json::from_slice(&std::fs::read(folder.join("layout.json")).ok()?).ok()?;
        let mut source = FakeSource::new(
            desktop,
            layout.scale.unwrap_or(1.0),
            layout.screens,
            layout.windows,
        );
        if let Some(cursor) = layout.cursor {
            *source.cursor.lock().unwrap() = cursor;
        }
        source.active = layout.active_window.or(source.active);
        Some(source)
    }

    /// A made-up desktop: two displays side by side, three windows, painted so that
    /// every area of it is recognisable in a crop.
    pub fn sample() -> FakeSource {
        let scale = 1.0;
        let screens = vec![
            ScreenInfo {
                id: 1,
                bounds: IRect::new(0, 0, 1440, 900),
                working_area: IRect::new(0, 25, 1440, 800),
                scale,
                primary: true,
                name: "Built-in Retina Display".into(),
            },
            ScreenInfo {
                id: 2,
                bounds: IRect::new(1440, 0, 1280, 720),
                working_area: IRect::new(1440, 25, 1280, 695),
                scale,
                primary: false,
                name: "External Display".into(),
            },
        ];
        let windows = vec![
            WindowInfo {
                id: 11,
                title: "Notes".into(),
                process_name: "Notes".into(),
                pid: 501,
                rect: IRect::new(160, 120, 640, 420),
                on_screen: true,
            },
            WindowInfo {
                id: 12,
                title: "Finder".into(),
                process_name: "Finder".into(),
                pid: 502,
                rect: IRect::new(520, 300, 700, 460),
                on_screen: true,
            },
            WindowInfo {
                id: 13,
                title: "Terminal".into(),
                process_name: "Terminal".into(),
                pid: 503,
                rect: IRect::new(1560, 100, 900, 500),
                on_screen: true,
            },
        ];
        let width = (2720.0 * scale) as u32;
        let height = (900.0 * scale) as u32;
        let mut desktop = RgbaImage::from_fn(width, height, |x, y| {
            let fx = x as f32 / width as f32;
            let fy = y as f32 / height as f32;
            Rgba([
                (40.0 + 120.0 * fx) as u8,
                (60.0 + 90.0 * fy) as u8,
                (140.0 + 80.0 * (1.0 - fx)) as u8,
                255,
            ])
        });
        // Below the second display there is no screen.
        for y in (720.0 * scale) as u32..height {
            for x in (1440.0 * scale) as u32..width {
                desktop.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        let colors = [
            Rgba([250, 250, 245, 255]),
            Rgba([236, 240, 246, 255]),
            Rgba([30, 32, 36, 255]),
        ];
        for (window, color) in windows.iter().zip(colors).rev() {
            let r = window.rect;
            for y in r.y..r.bottom() {
                for x in r.x..r.right() {
                    let title_bar = y < r.y + 28;
                    let pixel = if title_bar {
                        Rgba([210, 210, 214, 255])
                    } else {
                        color
                    };
                    let (px, py) = ((x as f64 * scale) as u32, (y as f64 * scale) as u32);
                    if px < width && py < height {
                        desktop.put_pixel(px, py, pixel);
                    }
                }
            }
        }
        FakeSource::new(desktop, scale, screens, windows)
    }

    pub fn set_cursor(&self, x: i32, y: i32) {
        *self.cursor.lock().unwrap() = (x, y);
    }

    pub fn desktop(&self) -> &RgbaImage {
        &self.desktop
    }

    fn crop(&self, rect: IRect) -> Result<RgbaImage, CaptureError> {
        let bounds = self.screen_bounds();
        let rect = rect.intersect(&bounds);
        if !rect.has_area() {
            return Err(CaptureError::Nothing);
        }
        self.taken.lock().unwrap().push(rect);
        let s = self.scale;
        let x = ((rect.x - bounds.x) as f64 * s).round() as u32;
        let y = ((rect.y - bounds.y) as f64 * s).round() as u32;
        let w = ((rect.width as f64) * s).round() as u32;
        let h = ((rect.height as f64) * s).round() as u32;
        Ok(imageops::crop_imm(&self.desktop, x, y, w, h).to_image())
    }
}

impl CaptureSource for FakeSource {
    fn screens(&self) -> Vec<ScreenInfo> {
        self.screens.clone()
    }

    fn cursor_position(&self) -> (i32, i32) {
        *self.cursor.lock().unwrap()
    }

    fn windows(&self) -> Vec<WindowInfo> {
        self.windows.clone()
    }

    fn active_window(&self) -> Option<WindowInfo> {
        self.active.and_then(|i| self.windows.get(i)).cloned()
    }

    fn capture_rect(&self, rect: IRect, _cursor: bool) -> Result<RgbaImage, CaptureError> {
        self.crop(rect)
    }

    fn capture_window(
        &self,
        window: &WindowInfo,
        how: WindowCapture,
    ) -> Result<RgbaImage, CaptureError> {
        self.windows_taken
            .lock()
            .unwrap()
            .push((window.title.clone(), how));
        self.crop(window.rect)
    }
}

/// A source the whole app shares (the window, the hotkeys, the recorder).
pub type SharedSource = Arc<dyn CaptureSource>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_desktop_crops_like_the_screen() {
        let source = FakeSource::sample();
        assert_eq!(source.screen_bounds(), IRect::new(0, 0, 2720, 900));
        let picture = source
            .capture_rect(IRect::new(100, 100, 50, 40), false)
            .unwrap();
        assert_eq!(picture.dimensions(), (50, 40));
        // Outside every display: clipped to the desktop.
        let picture = source
            .capture_rect(IRect::new(2700, 880, 100, 100), false)
            .unwrap();
        assert_eq!(picture.dimensions(), (20, 20));
        assert_eq!(
            source.capture_rect(IRect::new(5000, 0, 10, 10), false),
            Err(CaptureError::Nothing)
        );
        source.set_cursor(1500, 50);
        assert_eq!(source.active_screen().unwrap().id, 2);
        assert_eq!(source.scale_for(IRect::new(0, 0, 10, 10)), 1.0);
        let window = source.active_window().unwrap();
        assert_eq!(window.title, "Notes");
        assert_eq!(
            source
                .capture_window(&window, WindowCapture::default())
                .unwrap()
                .dimensions(),
            (640, 420)
        );
        assert_eq!(
            even_size(IRect::new(0, 0, 641, 359)),
            IRect::new(0, 0, 640, 358)
        );
    }
}
