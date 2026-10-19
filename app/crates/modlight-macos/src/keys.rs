//! Cmd+V on the main window. egui hears a paste only when the clipboard holds text, but
//! upstream's Ctrl+V uploads whatever is there (a picture, files), so the key itself is
//! watched with an AppKit local monitor. A local monitor sees this app's own key events
//! only and needs no permission.

use std::sync::atomic::{AtomicBool, Ordering};

static PRESSED: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Start watching (once, on the main thread, when the window opens).
pub fn install() {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    #[cfg(target_os = "macos")]
    mac::install();
}

/// Whether the key is being watched; when it is not, the caller falls back to egui's
/// paste event.
pub fn monitoring() -> bool {
    INSTALLED.load(Ordering::SeqCst)
}

/// Whether Cmd+V was pressed since the last call.
pub fn take_command_v() -> bool {
    PRESSED.swap(false, Ordering::SeqCst)
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ptr::NonNull;
    use std::sync::atomic::Ordering;

    use block2::RcBlock;
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags};

    pub fn install() {
        let block = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit hands the handler a valid event for the call's duration.
            let key = unsafe { event.as_ref() };
            let command = key.modifierFlags().contains(NSEventModifierFlags::Command);
            let is_v = key
                .charactersIgnoringModifiers()
                .is_some_and(|c| c.to_string().eq_ignore_ascii_case("v"));
            if command && is_v {
                super::PRESSED.store(true, Ordering::SeqCst);
            }
            // The event goes on to the window unchanged.
            event.as_ptr()
        });
        // SAFETY: the handler returns the event it was given, which is valid.
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block)
        };
        // The monitor lives as long as the app.
        std::mem::forget(monitor);
    }
}
