//! Synthetic input for the scrolling capture (`InputHelpers.SendMouseWheel`,
//! `SendKeyPress`): `SendInput`. Never used by automated runs.

use modlight_core::capture::options::ScrollMethod;
use modlight_core::capture::scrolling::Scroller;
use modlight_core::capture::source::WindowInfo;

/// Kept for the macOS API: Windows lets any app send input to the desktop it runs on.
pub fn input_trusted() -> bool {
    true
}

/// The Windows scroller.
pub struct SystemScroller;

impl Scroller for SystemScroller {
    fn activate(&self, window: Option<&WindowInfo>) {
        let _ = window;
    }

    fn scroll_to_top(&self) {}

    fn scroll(&self, method: ScrollMethod, amount: i64) {
        let _ = (method, amount);
    }
}
