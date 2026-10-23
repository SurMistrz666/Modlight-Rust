//! Synthetic input for the scrolling capture, as upstream's `ScrollingCaptureManager`
//! does it: the window brought to the front (`WindowInfo.Activate`), keys and the wheel
//! through `SendInput` (`InputHelpers.SendKeyPress`, `SendMouseWheel`), and
//! `WM_VSCROLL` sent to the window itself (`ScrollMethod.ScrollMessage`, and with Home
//! for "scroll to top"). Never used by automated runs.

use std::sync::atomic::{AtomicIsize, Ordering};

use modlight_core::capture::options::ScrollMethod;
use modlight_core::capture::scrolling::Scroller;
use modlight_core::capture::source::WindowInfo;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP, MOUSEEVENTF_WHEEL,
    MOUSEINPUT, SendInput, VIRTUAL_KEY, VK_DOWN, VK_HOME, VK_NEXT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SB_LINEDOWN, SB_TOP, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_VSCROLL,
};

use crate::capture::winlist::{bring_to_front, id_hwnd, is_window};

/// Kept for the macOS API: Windows lets any app send input to the desktop it runs on.
pub fn input_trusted() -> bool {
    true
}

/// The window `activate` was given (`selectedWindow`), for the messages sent to it.
/// One scrolling capture runs at a time.
static SELECTED: AtomicIsize = AtomicIsize::new(0);

/// How long a window may take to answer a scroll message before it is given up on
/// (upstream's `SendMessage` would wait for a hung window forever).
const MESSAGE_TIMEOUT_MS: u32 = 1000;

fn selected() -> Option<HWND> {
    let hwnd = SELECTED.load(Ordering::SeqCst) as HWND;
    is_window(hwnd).then_some(hwnd)
}

fn send(inputs: &[INPUT]) {
    // SAFETY: a slice of initialised INPUT structures and their size.
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
}

fn key(code: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: code,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                ..Default::default()
            },
        },
    }
}

/// `InputHelpers.SendKeyPress`: the key down and up, by virtual key code alone.
fn press(code: VIRTUAL_KEY) {
    send(&[key(code, false), key(code, true)]);
}

/// `InputHelpers.SendMouseWheel(delta)`: one wheel event of `delta` (120 a notch,
/// negative down) where the pointer is.
fn wheel(delta: i32) {
    send(&[INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                mouseData: delta as u32,
                dwFlags: MOUSEEVENTF_WHEEL,
                ..Default::default()
            },
        },
    }]);
}

/// `NativeMethods.SendMessage(handle, WM_VSCROLL, command, 0)`.
fn vscroll(hwnd: HWND, command: i32) {
    let mut result = 0usize;
    // SAFETY: a plain message with no pointers in it; the call gives up on a hung window.
    unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_VSCROLL,
            command as usize,
            0,
            SMTO_ABORTIFHUNG,
            MESSAGE_TIMEOUT_MS,
            &mut result,
        )
    };
}

/// The wheel delta of one scroll step (`-120 * ScrollAmount`).
fn wheel_delta(amount: i64) -> i32 {
    amount
        .saturating_mul(-120)
        .clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// The Windows scroller.
pub struct SystemScroller;

impl Scroller for SystemScroller {
    /// `selectedWindow.Activate()`: `SetForegroundWindow`, made to work from a
    /// background app (see `capture::winlist::bring_to_front`).
    fn activate(&self, window: Option<&WindowInfo>) {
        let hwnd = window.map(|w| id_hwnd(w.id)).filter(|&h| is_window(h));
        SELECTED.store(
            hwnd.map(|h| h as isize).unwrap_or_default(),
            Ordering::SeqCst,
        );
        if let Some(hwnd) = hwnd {
            bring_to_front(hwnd);
        }
    }

    /// `AutoScrollTop`: Home, and `SB_TOP` sent to the window.
    fn scroll_to_top(&self) {
        press(VK_HOME);
        if let Some(hwnd) = selected() {
            vscroll(hwnd, SB_TOP);
        }
    }

    fn scroll(&self, method: ScrollMethod, amount: i64) {
        match method {
            ScrollMethod::MouseWheel => wheel(wheel_delta(amount)),
            ScrollMethod::DownArrow => {
                for _ in 0..amount {
                    press(VK_DOWN);
                }
            }
            ScrollMethod::PageDown => press(VK_NEXT),
            ScrollMethod::ScrollMessage => {
                if let Some(hwnd) = selected() {
                    for _ in 0..amount {
                        vscroll(hwnd, SB_LINEDOWN);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_steps() {
        assert_eq!(wheel_delta(1), -120);
        assert_eq!(wheel_delta(3), -360);
        assert_eq!(wheel_delta(0), 0);
        assert_eq!(wheel_delta(i64::MAX), i32::MIN);
        // The wheel's data field carries the signed delta as its bits.
        assert_eq!(wheel_delta(2) as u32, (-240i32) as u32);
    }

    #[test]
    fn no_window_selected() {
        // Without a window nothing is activated and no message has anywhere to go.
        SystemScroller.activate(None);
        assert!(selected().is_none());
        // Scrolling by message with no window selected sends nothing.
        SystemScroller.scroll(ScrollMethod::ScrollMessage, 3);
    }

    /// Presses Home and sends the wheel to whatever is under the pointer: run by hand
    /// with a scrollable window in front, `cargo test -p modlight-windows -- --ignored
    /// scrolls_the_front_window`.
    #[test]
    #[ignore = "sends input to the desktop"]
    fn scrolls_the_front_window() {
        SystemScroller.scroll_to_top();
        SystemScroller.scroll(ScrollMethod::MouseWheel, 2);
        SystemScroller.scroll(ScrollMethod::DownArrow, 2);
        SystemScroller.scroll(ScrollMethod::PageDown, 1);
    }
}
