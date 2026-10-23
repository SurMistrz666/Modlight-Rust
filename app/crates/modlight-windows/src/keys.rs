//! Ctrl+V on the main window. egui hears a paste only when the clipboard holds text, but
//! upstream's Ctrl+V uploads whatever is there (a picture, files), so the key itself is
//! watched, with a keyboard hook on the app's own thread (`WH_KEYBOARD`): it sees this
//! app's key presses only and needs no permission.
//!
//! The hook is the thread's, not the system's: Windows calls it on the UI thread when
//! that thread takes a key message out of its queue, for every window of the thread (the
//! main window and the other viewports), and every key goes on to the window unchanged.
//!
//! The same hook hands the shortcuts page the keys egui never reports (Print Screen,
//! Scroll Lock, Pause, the media and browser keys), which upstream binds like any other,
//! and tells it whether a Win key is down (egui has no Win modifier).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, VK_BROWSER_BACK, VK_CANCEL, VK_CONTROL, VK_LAUNCH_APP2, VK_LWIN,
    VK_MENU, VK_PAUSE, VK_RWIN, VK_SCROLL, VK_SHIFT, VK_SNAPSHOT, VK_V,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HC_ACTION, SetWindowsHookExW, WH_KEYBOARD,
};

static PRESSED: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// The last key egui does not report, with the modifiers held when it was pressed.
static SPECIAL: Mutex<Option<SpecialKey>> = Mutex::new(None);

/// A key press egui never reports, as the shortcuts page needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpecialKey {
    /// The virtual-key code (a WinForms `Keys` value).
    pub key: u32,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
}

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

/// The last press of a key egui does not report (Print Screen, Scroll Lock, Pause and
/// Break, the browser, media and launch keys), since the last call.
pub fn take_special_key() -> Option<SpecialKey> {
    SPECIAL.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Whether a Win key is held now.
pub fn win_down() -> bool {
    // SAFETY: reads the keyboard's state; no pointers.
    unsafe { GetAsyncKeyState(VK_LWIN as i32) < 0 || GetAsyncKeyState(VK_RWIN as i32) < 0 }
}

/// Whether `key` is one egui has no `Key` for.
fn is_special(key: usize) -> bool {
    let key = key as u16;
    matches!(key, VK_SNAPSHOT | VK_SCROLL | VK_PAUSE | VK_CANCEL)
        || (VK_BROWSER_BACK..=VK_LAUNCH_APP2).contains(&key)
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

/// Whether a `WH_KEYBOARD` call is a press to report for `key`: a fresh press, or for
/// Print Screen its release (windows get no key-down message for it).
fn is_special_press(key: usize, flags: isize) -> bool {
    let released = (flags >> 31) & 1 == 1;
    let repeat = (flags >> 30) & 1 == 1;
    is_special(key)
        && if key == VK_SNAPSHOT as usize {
            released
        } else {
            !released && !repeat
        }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && is_special_press(wparam, lparam) {
        *SPECIAL.lock().unwrap_or_else(|e| e.into_inner()) = Some(SpecialKey {
            key: wparam as u32,
            ctrl: down(VK_CONTROL),
            shift: down(VK_SHIFT),
            alt: down(VK_MENU),
            win: down(VK_LWIN) || down(VK_RWIN),
        });
    }
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

    #[test]
    fn the_keys_egui_misses_are_reported_once() {
        let press = 0x0001;
        let repeat = press | (1 << 30);
        let release = press | (1 << 30) | (1 << 31);
        // Print Screen arrives as its release only.
        assert!(is_special_press(VK_SNAPSHOT as usize, release));
        assert!(!is_special_press(VK_SNAPSHOT as usize, press));
        for key in [
            VK_SCROLL,
            VK_PAUSE,
            VK_CANCEL,
            VK_BROWSER_BACK,
            VK_LAUNCH_APP2,
        ] {
            assert!(is_special_press(key as usize, press), "{key}");
            assert!(!is_special_press(key as usize, repeat), "{key}");
            assert!(!is_special_press(key as usize, release), "{key}");
        }
        assert!(!is_special_press('A' as usize, press));
        assert!(!is_special_press(VK_V as usize, press));
        assert_eq!(take_special_key(), None);
    }
}
