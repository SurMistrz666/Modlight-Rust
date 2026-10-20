//! Synthetic input for the scrolling capture (`InputHelpers.SendMouseWheel`,
//! `SendKeyPress`): CoreGraphics events posted to the session. macOS delivers posted
//! events only for apps the person trusted under Privacy & Security, Accessibility; the
//! first scrolling capture is where that is asked. Never used by automated runs.

use modlight_core::capture::options::ScrollMethod;
use modlight_core::capture::scrolling::Scroller;
use modlight_core::capture::source::WindowInfo;

/// Whether this app may post input (`AXIsProcessTrusted`), without asking.
pub fn input_trusted() -> bool {
    #[cfg(target_os = "macos")]
    {
        #[link(name = "ApplicationServices", kind = "framework")]
        unsafe extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }
        // SAFETY: a plain query of this process's trust.
        unsafe { AXIsProcessTrusted() }
    }
    #[cfg(not(target_os = "macos"))]
    false
}

/// The Mac's scroller.
pub struct MacScroller;

#[cfg(target_os = "macos")]
fn post_key(code: u16) {
    use objc2_core_graphics::{CGEvent, CGEventTapLocation};
    for down in [true, false] {
        if let Some(event) = CGEvent::new_keyboard_event(None, code, down) {
            CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
        }
    }
}

impl Scroller for MacScroller {
    fn activate(&self, window: Option<&WindowInfo>) {
        #[cfg(target_os = "macos")]
        if let Some(window) = window {
            use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
            if let Some(app) =
                NSRunningApplication::runningApplicationWithProcessIdentifier(window.pid)
            {
                app.activateWithOptions(NSApplicationActivationOptions::empty());
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = window;
    }

    fn scroll_to_top(&self) {
        // kVK_Home.
        #[cfg(target_os = "macos")]
        post_key(0x73);
    }

    fn scroll(&self, method: ScrollMethod, amount: i64) {
        #[cfg(target_os = "macos")]
        {
            use objc2_core_graphics::{CGEvent, CGEventTapLocation, CGScrollEventUnit};
            match method {
                // -120 * amount on Windows is `amount` notches; a notch is three lines.
                ScrollMethod::MouseWheel | ScrollMethod::ScrollMessage => {
                    let lines = -(3 * amount.max(1)) as i32;
                    if let Some(event) = CGEvent::new_scroll_wheel_event2(
                        None,
                        CGScrollEventUnit::Line,
                        1,
                        lines,
                        0,
                        0,
                    ) {
                        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
                    }
                }
                // kVK_DownArrow.
                ScrollMethod::DownArrow => {
                    for _ in 0..amount.max(1) {
                        post_key(0x7D);
                    }
                }
                // kVK_PageDown.
                ScrollMethod::PageDown => post_key(0x79),
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (method, amount);
    }
}
