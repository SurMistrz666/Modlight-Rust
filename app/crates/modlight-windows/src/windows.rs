//! The app's own windows: bring them back from the tray, where on the screen a notice
//! may go, whether a full-screen program is in front, and the region overlay above the
//! taskbar.

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    HWND_TOPMOST, SW_RESTORE, SW_SHOW, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos,
    ShowWindow,
};

use crate::capture::desktop::{PhysicalPixels, monitors, primary_scale, rect_to_points};
use crate::capture::winlist::{
    bring_to_front, class_name, foreground, is_minimized, is_visible, own_windows, title,
    window_rect,
};

/// The window class winit gives every window it makes (egui's viewports), unless the
/// app names another.
const WINIT_CLASS: &str = "Window Class";

/// This app's windows: winit's, with a title (winit's own helper windows and the tray's
/// message window have other classes and no title).
fn app_windows() -> Vec<HWND> {
    own_windows()
        .into_iter()
        .filter(|&hwnd| class_name(hwnd) == WINIT_CLASS && !title(hwnd).is_empty())
        .collect()
}

/// Show every window of this app the tray hid (and restore the minimised ones), then
/// bring the front one forward. Must run on the main thread (menu actions of the tray
/// do): a hidden window is never repainted, so the app could not act on the command
/// that follows while all its windows are hidden.
///
/// The windows are shown behind winit's back: winit still believes a window it hid is
/// hidden until the app shows it again (`ViewportCommand::Visible(true)`), and ignores
/// a `Visible(false)` until then. The app's "show" commands send that.
pub fn show_app_windows() {
    let windows = app_windows();
    for &hwnd in &windows {
        // SAFETY: plain calls on this app's own windows, on their thread.
        unsafe {
            if !is_visible(hwnd) {
                ShowWindow(hwnd, SW_SHOW);
            }
            if is_minimized(hwnd) {
                ShowWindow(hwnd, SW_RESTORE);
            }
        }
    }
    if let Some(&front) = windows.first() {
        bring_to_front(front);
    }
}

/// The primary monitor's working area (without the taskbar) in the app's screen points
/// (see `capture`), origin at the top left: (x, y, width, height).
pub fn visible_frame() -> Option<(f32, f32, f32, f32)> {
    let primary = monitors().into_iter().find(|m| m.primary)?;
    let area = rect_to_points(primary.work, primary_scale());
    Some((
        area.x as f32,
        area.y as f32,
        area.width as f32,
        area.height as f32,
    ))
}

/// `CaptureHelpers.IsActiveWindowFullscreen`: the foreground window (not the desktop)
/// covers the whole monitor it is mostly on.
pub fn active_window_fullscreen() -> bool {
    let _pixels = PhysicalPixels::enter();
    let Some(hwnd) = foreground() else {
        return false;
    };
    let class = class_name(hwnd);
    if ["Progman", "WorkerW"]
        .iter()
        .any(|ignored| class.eq_ignore_ascii_case(ignored))
    {
        return false;
    }
    let Some(rect) = window_rect(hwnd) else {
        return false;
    };
    // `Screen.FromRectangle`: the monitor with the largest part of the window.
    let screens = monitors();
    let Some(monitor) = screens.iter().max_by_key(|m| {
        let part = m.bounds.intersect(&rect);
        part.width as i64 * part.height as i64
    }) else {
        return false;
    };
    rect.contains_rect(&monitor.bounds)
}

/// Raise a window of this app, found by its title, above the taskbar (upstream's overlay
/// is a top-most form covering it): top-most, shown and brought to the front, which is
/// what lets a window covering a monitor go over the taskbar. Returns whether the window
/// was found.
pub fn raise_overlay(title_text: &str) -> bool {
    let Some(hwnd) = own_windows()
        .into_iter()
        .find(|&hwnd| title(hwnd) == title_text)
    else {
        return false;
    };
    // SAFETY: a plain call on this app's own window.
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        )
    };
    bring_to_front(hwnd);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_working_area_is_on_the_primary_monitor() {
        let (x, y, width, height) = visible_frame().unwrap();
        assert!(width > 0.0 && height > 0.0);
        let primary = monitors().into_iter().find(|m| m.primary).unwrap();
        let bounds = rect_to_points(primary.bounds, primary_scale());
        assert!(x >= bounds.x as f32 && y >= bounds.y as f32);
        assert!(x + width <= bounds.right() as f32 && y + height <= bounds.bottom() as f32);
    }

    #[test]
    fn no_window_of_a_test_is_found() {
        // A test has no windows of its own: nothing is raised, nothing shown.
        assert!(app_windows().is_empty());
        assert!(!raise_overlay("Modlight - Region capture 1"));
        // Reads the foreground window only.
        let _ = active_window_fullscreen();
    }
}
