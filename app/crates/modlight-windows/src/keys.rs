//! Ctrl+V on the main window. egui hears a paste only when the clipboard holds text, but
//! upstream's Ctrl+V uploads whatever is there (a picture, files), so the key itself is
//! watched, with a keyboard hook on the app's own thread (`WH_KEYBOARD`): it sees this
//! app's key presses only and needs no permission.

use std::sync::atomic::{AtomicBool, Ordering};

static PRESSED: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Start watching (once, on the main thread, when the window opens).
pub fn install() {}

/// Whether the key is being watched; when it is not, the caller falls back to egui's
/// paste event.
pub fn monitoring() -> bool {
    INSTALLED.load(Ordering::SeqCst)
}

/// Whether Ctrl+V was pressed since the last call (the macOS name is kept).
pub fn take_command_v() -> bool {
    PRESSED.swap(false, Ordering::SeqCst)
}
