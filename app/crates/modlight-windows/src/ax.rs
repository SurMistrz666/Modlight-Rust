//! "Active window borderless" and "Active window top-most" (upstream's
//! `BorderlessWindowManager` and `WindowInfo.TopMost`, through `SetWindowLong` and
//! `SetWindowPos`): the foreground window loses its caption and frame and fills a frame
//! the caller picks; the second time, it gets its styles and place back.
//!
//! Frames are (x, y, width, height) in the app's screen points (see `capture`).
//! Windows of a program running as administrator cannot be changed by a program that is
//! not (User Interface Privilege Isolation); the toggle then reports a failure.

use std::collections::HashMap;
use std::sync::Mutex;

use modlight_core::capture::geometry::IRect;
use windows_sys::Win32::Foundation::{GetLastError, HWND, SetLastError};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetWindowPlacement, HWND_NOTOPMOST, HWND_TOP, HWND_TOPMOST, SW_RESTORE,
    SW_SHOWMAXIMIZED, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER,
    SetWindowLongPtrW, SetWindowPlacement, SetWindowPos, ShowWindow, WINDOWPLACEMENT, WS_CAPTION,
    WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_STATICEDGE, WS_EX_TOPMOST, WS_MAXIMIZEBOX,
    WS_SYSMENU, WS_THICKFRAME,
};

use crate::capture::desktop::{PhysicalPixels, monitors, primary_scale};
use crate::capture::winlist::{
    class_name, ex_style, foreground, is_minimized, is_window, outer_rect, style,
};

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

/// `BorderlessWindowInfo`: what a window was before this hotkey changed it.
#[derive(Clone, Copy)]
struct Saved {
    style: u32,
    ex_style: u32,
    /// `GetWindowRect`, physical pixels.
    rect: IRect,
    /// Its placement, to give a maximised window its state and normal place back.
    placement: WINDOWPLACEMENT,
}

/// `BorderlessWindowManager.BorderlessWindows`, by window handle.
static BORDERLESS: Mutex<Option<HashMap<isize, Saved>>> = Mutex::new(None);

/// The desktop and the taskbar are no program's window to resize.
const SHELL_CLASSES: [&str; 4] = [
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
];

fn active() -> Option<HWND> {
    let hwnd = foreground()?;
    let class = class_name(hwnd);
    (!SHELL_CLASSES
        .iter()
        .any(|shell| class.eq_ignore_ascii_case(shell)))
    .then_some(hwnd)
}

fn to_frame(rect: IRect, scale: f64) -> Rect {
    (
        rect.x as f64 / scale,
        rect.y as f64 / scale,
        rect.width as f64 / scale,
        rect.height as f64 / scale,
    )
}

fn to_pixels(frame: Rect, scale: f64) -> IRect {
    let left = (frame.0 * scale).round() as i32;
    let top = (frame.1 * scale).round() as i32;
    let right = ((frame.0 + frame.2) * scale).round() as i32;
    let bottom = ((frame.1 + frame.3) * scale).round() as i32;
    IRect::new(left, top, right - left, bottom - top)
}

/// `SetWindowLong`, with its failure told apart from a previous value of 0.
fn set_long(hwnd: HWND, index: i32, value: u32) -> Result<(), String> {
    // SAFETY: plain calls; the last error is cleared first so that a 0 result can be
    // read correctly.
    unsafe {
        SetLastError(0);
        if SetWindowLongPtrW(hwnd, index, value as i32 as isize) == 0 && GetLastError() != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    Ok(())
}

fn set_pos(hwnd: HWND, rect: IRect, flags: u32) -> Result<(), String> {
    // SAFETY: a plain call.
    let ok = unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOP,
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            flags,
        )
    } != 0;
    if ok {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

const FRAME_FLAGS: u32 = SWP_FRAMECHANGED | SWP_NOOWNERZORDER | SWP_NOZORDER;

/// `BorderlessWindowManager.RestoreWindow`.
fn restore(hwnd: HWND, saved: &Saved) -> Result<(), String> {
    set_long(hwnd, GWL_STYLE, saved.style)?;
    set_long(hwnd, GWL_EXSTYLE, saved.ex_style)?;
    set_pos(hwnd, saved.rect, FRAME_FLAGS)?;
    if saved.placement.showCmd == SW_SHOWMAXIMIZED as u32 {
        // SAFETY: the placement read from this window before.
        unsafe { SetWindowPlacement(hwnd, &saved.placement) };
    }
    Ok(())
}

/// `BorderlessWindowManager.MakeWindowBorderless`, to the frame `pick` chooses.
fn make_borderless(
    hwnd: HWND,
    pick: impl FnOnce(Rect, &[(Rect, Rect)]) -> Option<Rect>,
) -> Result<Option<Saved>, String> {
    let rect = outer_rect(hwnd).ok_or("the window has no position")?;
    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    // SAFETY: a WINDOWPLACEMENT with its length set.
    unsafe { GetWindowPlacement(hwnd, &mut placement) };
    let saved = Saved {
        style: style(hwnd),
        ex_style: ex_style(hwnd),
        rect,
        placement,
    };
    let scale = primary_scale();
    let screens: Vec<(Rect, Rect)> = monitors()
        .into_iter()
        .map(|m| (to_frame(m.bounds, scale), to_frame(m.work, scale)))
        .collect();
    let Some(target) = pick(to_frame(rect, scale), &screens) else {
        return Ok(None);
    };
    let target = to_pixels(target, scale);
    let mut flags = FRAME_FLAGS;
    if !target.has_area() {
        flags |= SWP_NOMOVE | SWP_NOSIZE;
    }
    set_long(
        hwnd,
        GWL_STYLE,
        saved.style & !(WS_CAPTION | WS_MAXIMIZEBOX | WS_SYSMENU | WS_THICKFRAME),
    )?;
    let result = set_long(
        hwnd,
        GWL_EXSTYLE,
        saved.ex_style & !(WS_EX_CLIENTEDGE | WS_EX_DLGMODALFRAME | WS_EX_STATICEDGE),
    )
    .and_then(|()| set_pos(hwnd, target, flags));
    if let Err(error) = result {
        let _ = restore(hwnd, &saved);
        return Err(error);
    }
    Ok(Some(saved))
}

/// Toggle the active window: fill `pick(window, screens)` (screens as whole and working
/// area frames), or restore it when this hotkey filled it before
/// (`BorderlessWindowManager.ToggleBorderlessWindow`). A minimised window is restored
/// first, as upstream does.
pub fn toggle_active_window(pick: impl FnOnce(Rect, &[(Rect, Rect)]) -> Option<Rect>) -> Toggled {
    let _pixels = PhysicalPixels::enter();
    let Some(hwnd) = active() else {
        return Toggled::NoWindow;
    };
    if is_minimized(hwnd) {
        // SAFETY: a plain call on a live window.
        unsafe { ShowWindow(hwnd, SW_RESTORE) };
    }
    let mut guard = BORDERLESS.lock().unwrap_or_else(|e| e.into_inner());
    let filled = guard.get_or_insert_with(HashMap::new);
    // Windows closed since are forgotten (their handles may be used again).
    filled.retain(|&handle, _| is_window(handle as HWND));
    if let Some(saved) = filled.remove(&(hwnd as isize)) {
        return match restore(hwnd, &saved) {
            Ok(()) => Toggled::Restored,
            Err(error) => Toggled::Failed(error),
        };
    }
    match make_borderless(hwnd, pick) {
        Ok(Some(saved)) => {
            filled.insert(hwnd as isize, saved);
            Toggled::Filled
        }
        Ok(None) => Toggled::NoWindow,
        Err(error) => Toggled::Failed(error),
    }
}

/// Toggle `WS_EX_TOPMOST` on the active window (`windowInfo.TopMost = !windowInfo.TopMost`:
/// `SetWindowPos(HWND_TOPMOST / HWND_NOTOPMOST)`). `Some(true)` when it is now top-most,
/// `Some(false)` when it no longer is, `None` when there is no window.
pub fn toggle_active_window_topmost() -> Option<bool> {
    let hwnd = active()?;
    let topmost = ex_style(hwnd) & WS_EX_TOPMOST != 0;
    let after = if topmost {
        HWND_NOTOPMOST
    } else {
        HWND_TOPMOST
    };
    // SAFETY: a plain call on a live window.
    unsafe { SetWindowPos(hwnd, after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE) };
    Some(ex_style(hwnd) & WS_EX_TOPMOST != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_and_pixels() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let rect = IRect::new(-1920, 37, 1280, 1003);
            assert_eq!(to_pixels(to_frame(rect, scale), scale), rect);
        }
        assert_eq!(
            to_frame(IRect::new(0, 0, 250, 100), 1.25),
            (0.0, 0.0, 200.0, 80.0)
        );
    }

    /// Takes the frame off the foreground window and puts it back: run by hand with a
    /// window of your own in front,
    /// `cargo test -p modlight-windows -- --ignored --test-threads=1 borderless_and_back`.
    #[test]
    #[ignore = "restyles and moves the foreground window"]
    fn borderless_and_back() {
        let screens = std::cell::RefCell::new(Vec::new());
        let filled = toggle_active_window(|window, list| {
            screens.borrow_mut().extend_from_slice(list);
            Some(window)
        });
        assert_eq!(filled, Toggled::Filled);
        assert!(!screens.borrow().is_empty());
        assert_eq!(toggle_active_window(|_, _| None), Toggled::Restored);
        let first = toggle_active_window_topmost().unwrap();
        assert_eq!(toggle_active_window_topmost(), Some(!first));
    }
}
