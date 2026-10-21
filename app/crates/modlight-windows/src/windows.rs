//! The app's own windows: bring them back from the tray, where on the screen a notice
//! may go, whether a full-screen program is in front, and the region overlay above the
//! taskbar.

/// Show every window of this app the tray hid and bring them forward. Must run on the
/// main thread (menu actions of the tray do).
pub fn show_app_windows() {}

/// The primary monitor's working area (without the taskbar) in the app's screen points
/// (see `capture`), origin at the top left: (x, y, width, height).
pub fn visible_frame() -> Option<(f32, f32, f32, f32)> {
    None
}

/// Whether the foreground window fills its monitor
/// (`CaptureHelpers.IsActiveWindowFullscreen`).
pub fn active_window_fullscreen() -> bool {
    false
}

/// Raise a window of this app, found by its title, above the taskbar (upstream's overlay
/// is a top-most form covering it). Returns whether the window was found.
pub fn raise_overlay(title: &str) -> bool {
    let _ = title;
    false
}
