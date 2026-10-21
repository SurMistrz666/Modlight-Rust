//! Ctrl+V on the main window. egui hears a paste only when the clipboard holds text, but
//! upstream's Ctrl+V uploads whatever is there (a picture, files), so the key itself is
//! watched, with a keyboard hook on the app's own thread (`WH_KEYBOARD`): it sees this
//! app's key presses only and needs no permission.
//!
//! The hook is the thread's, not the system's: Windows calls it on the UI thread when
//! that thread takes a key message out of its queue, for every window of the thread (the
//! main window and the other viewports), and every key goes on to the window unchanged.

use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_V,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HC_ACTION, SetWindowsHookExW, WH_KEYBOARD,
};

static PRESSED: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Start watching (once, on the main thread, when the window opens).
pub fn install() {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    // SAFETY: a hook of this thread only (no module handle), with a procedure that lives
    // as long as the program. It stays installed for the life of the thread.
    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD,
            Some(keyboard_hook),
            std::ptr::null_mut(),
            GetCurrentThreadId(),
        )
    };
    if hook.is_null() {
        log::warn!(
            "Modlight: Ctrl+V cannot be watched: {}",
            crate::system::last_error_message()
        );
        INSTALLED.store(false, Ordering::SeqCst);
    }
}

/// Whether the key is being watched; when it is not, the caller falls back to egui's
/// paste event.
pub fn monitoring() -> bool {
    INSTALLED.load(Ordering::SeqCst)
}

/// Whether Ctrl+V was pressed since the last call (the macOS name is kept).
pub fn take_command_v() -> bool {
    PRESSED.swap(false, Ordering::SeqCst)
}

/// Whether a `WH_KEYBOARD` call is the press of V with Ctrl and without Alt (AltGr is
/// Ctrl+Alt) or Win. `flags` is the hook's `lParam`: bit 31 is set on release, bit 30
/// when the key was already down (a held key repeating, which pastes once only).
fn is_paste(key: usize, flags: isize, ctrl: bool, alt: bool, win: bool) -> bool {
    let released = (flags >> 31) & 1 == 1;
    let repeat = (flags >> 30) & 1 == 1;
    key == VK_V as usize && !released && !repeat && ctrl && !alt && !win
}

fn down(key: u16) -> bool {
    // SAFETY: reads the thread's key state; no pointers.
    unsafe { GetKeyState(key as i32) < 0 }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32
        && wparam == VK_V as usize
        && is_paste(
            wparam,
            lparam,
            down(VK_CONTROL),
            down(VK_MENU),
            down(VK_LWIN) || down(VK_RWIN),
        )
    {
        PRESSED.store(true, Ordering::SeqCst);
    }
    // SAFETY: the event goes on unchanged, as a hook must pass it.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_fresh_ctrl_v_is_a_paste() {
        let press = 0x002F_0001; // scan code of V, repeat count 1
        let repeat = press | (1 << 30);
        let release = press | (1 << 30) | (1 << 31);
        let v = VK_V as usize;
        assert!(is_paste(v, press, true, false, false));
        assert!(!is_paste(v, repeat, true, false, false));
        assert!(!is_paste(v, release, true, false, false));
        assert!(!is_paste(v, press, false, false, false), "no Ctrl");
        assert!(!is_paste(v, press, true, true, false), "AltGr or Ctrl+Alt");
        assert!(!is_paste(v, press, true, false, true), "Win");
        assert!(!is_paste('C' as usize, press, true, false, false));
    }
}
