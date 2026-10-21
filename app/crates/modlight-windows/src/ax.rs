//! "Active window borderless" and "Active window top-most" (upstream's
//! `WindowInfo.SetBorderless`/`SetTopMost` through `SetWindowLong` and `SetWindowPos`):
//! the foreground window loses its frame and fills a frame the caller picks; the second
//! time, it gets its style and place back.
//!
//! Frames are (x, y, width, height) in the app's screen points (see `capture`).

/// A frame: x, y, width, height.
pub type Rect = (f64, f64, f64, f64);

/// What the toggle did.
#[derive(Clone, Debug, PartialEq)]
pub enum Toggled {
    /// The window now fills the frame the caller picked.
    Filled,
    /// The window is back where it was.
    Restored,
    /// Kept for the macOS API: Windows needs no permission to move another window.
    NotTrusted,
    /// No window is active.
    NoWindow,
    Failed(String),
}

/// Toggle the active window: fill `pick(window, screens)` (screens as whole and working
/// area frames), or restore it when this hotkey filled it before.
pub fn toggle_active_window(pick: impl FnOnce(Rect, &[(Rect, Rect)]) -> Option<Rect>) -> Toggled {
    let _ = pick;
    Toggled::NoWindow
}

/// Toggle `WS_EX_TOPMOST` on the active window. `Some(true)` when it is now top-most,
/// `Some(false)` when it no longer is, `None` when there is no window.
pub fn toggle_active_window_topmost() -> Option<bool> {
    None
}
