//! Global hotkeys (`RegisterHotKey` in upstream's `HotkeyForm`), through the
//! `global-hotkey` crate, which registers exactly that. WinForms key codes are Windows
//! virtual-key codes, so a binding means the key it names. Normal mode only: QA never
//! registers anything.
//!
//! `global-hotkey` names keys by their `Code`, and a few keys a Windows keyboard has have
//! none (`Oem8`, the key left of 1 on a British keyboard and right of the right Shift on
//! a French one; the two extra keys of a Brazilian ABNT keyboard). Those are registered
//! the way upstream registers every key: `RegisterHotKey` with the virtual-key code, on a
//! message-only window of this crate's. Both kinds arrive in [`SystemHotkeys::pressed`].
//!
//! Ctrl+Pause reaches Windows as Ctrl+Break (`VK_CANCEL`), so a binding of Pause with
//! Control (or of WinForms' `Cancel`, what typing Ctrl+Break stores) registers that.

use std::sync::Mutex;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use modlight_core::hotkeys::{HotkeyBackend, HotkeyInfo, Keys};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HWND_MESSAGE, RegisterClassExW, WM_HOTKEY,
    WNDCLASSEXW,
};

use crate::wide::wide;

/// The `Code` of a WinForms key code (a Windows virtual-key code), when `global-hotkey`
/// can register the key. `control`: whether the binding holds Control (Ctrl+Break).
pub fn code_for(key: u32, control: bool) -> Option<Code> {
    use Code::*;
    let letters = [
        KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO,
        KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
    ];
    let digits = [
        Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
    ];
    let numpad = [
        Numpad0, Numpad1, Numpad2, Numpad3, Numpad4, Numpad5, Numpad6, Numpad7, Numpad8, Numpad9,
    ];
    let functions = [
        F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F20,
        F21, F22, F23, F24,
    ];
    Some(match key {
        65..=90 => letters[(key - 65) as usize],
        48..=57 => digits[(key - 48) as usize],
        96..=105 => numpad[(key - 96) as usize],
        112..=135 => functions[(key - 112) as usize],
        // `Cancel` is Ctrl+Break; `global-hotkey` registers Pause with Control as that.
        3 if control => Pause,
        8 => Backspace,
        9 => Tab,
        12 => NumpadClear,
        13 => Enter,
        19 => Pause,
        20 => CapsLock,
        21 => KanaMode,
        27 => Escape,
        28 => Convert,
        29 => NonConvert,
        32 => Space,
        33 => PageUp,
        34 => PageDown,
        35 => End,
        36 => Home,
        37 => ArrowLeft,
        38 => ArrowUp,
        39 => ArrowRight,
        40 => ArrowDown,
        44 => PrintScreen,
        45 => Insert,
        46 => Delete,
        47 => Help,
        93 => ContextMenu,
        95 => Sleep,
        106 => NumpadMultiply,
        107 => NumpadAdd,
        108 => NumpadComma,
        109 => NumpadSubtract,
        110 => NumpadDecimal,
        111 => NumpadDivide,
        144 => NumLock,
        145 => ScrollLock,
        146 => NumpadEqual,
        166 => BrowserBack,
        167 => BrowserForward,
        168 => BrowserRefresh,
        169 => BrowserStop,
        170 => BrowserSearch,
        171 => BrowserFavorites,
        172 => BrowserHome,
        173 => AudioVolumeMute,
        174 => AudioVolumeDown,
        175 => AudioVolumeUp,
        176 => MediaTrackNext,
        177 => MediaTrackPrevious,
        178 => MediaStop,
        179 => MediaPlayPause,
        180 => LaunchMail,
        181 => MediaSelect,
        182 => LaunchApp1,
        183 => LaunchApp2,
        186 => Semicolon,
        187 => Equal,
        188 => Comma,
        189 => Minus,
        190 => Period,
        191 => Slash,
        192 => Backquote,
        219 => BracketLeft,
        220 => Backslash,
        221 => BracketRight,
        222 => Quote,
        226 => IntlBackslash,
        250 => MediaPlay,
        _ => return None,
    })
}

/// Whether a key without a `Code` may still be registered by its virtual-key code: not a
/// mouse button and not a modifier key on its own.
fn raw_key(key: u32) -> bool {
    (1..=254).contains(&key)
        && !matches!(key, 1 | 2 | 4 | 5 | 6 | 16 | 17 | 18 | 91 | 92 | 160..=165)
}

/// The `global-hotkey` registration a binding becomes, when its key has a `Code`.
pub fn hotkey_for(info: HotkeyInfo) -> Option<HotKey> {
    let code = code_for(info.key_code(), info.hotkey.has(Keys::CONTROL))?;
    let mut mods = Modifiers::empty();
    if info.hotkey.has(Keys::CONTROL) {
        mods |= Modifiers::CONTROL;
    }
    if info.hotkey.has(Keys::SHIFT) {
        mods |= Modifiers::SHIFT;
    }
    if info.hotkey.has(Keys::ALT) {
        mods |= Modifiers::ALT;
    }
    if info.win {
        // `global-hotkey` registers META (and SUPER, which it turns into META) as MOD_WIN.
        mods |= Modifiers::META;
    }
    Some(HotKey::new(Some(mods), code))
}

/// `RegisterHotKey`'s modifiers for a binding (`HotkeyInfo.ModifiersEnum`), with
/// MOD_NOREPEAT as `global-hotkey` sets it.
fn raw_modifiers(info: HotkeyInfo) -> u32 {
    let mut mods = MOD_NOREPEAT;
    if info.hotkey.has(Keys::CONTROL) {
        mods |= MOD_CONTROL;
    }
    if info.hotkey.has(Keys::SHIFT) {
        mods |= MOD_SHIFT;
    }
    if info.hotkey.has(Keys::ALT) {
        mods |= MOD_ALT;
    }
    if info.win {
        mods |= MOD_WIN;
    }
    mods
}

/// Ids of the keys registered by their virtual-key code that were pressed.
static RAW_PRESSED: Mutex<Vec<u32>> = Mutex::new(Vec::new());
/// Ids of those keys: the application range of `RegisterHotKey` ends at 0xBFFF.
const RAW_IDS: std::ops::RangeInclusive<u32> = 0xB000..=0xBFFF;

/// The message-only window the keys without a `Code` are registered on. It is never
/// shown; `WM_HOTKEY` reaches it through the UI thread's message loop.
struct RawWindow {
    hwnd: HWND,
}

impl RawWindow {
    fn new() -> Option<RawWindow> {
        let class = wide("ModlightHotkeys");
        // SAFETY: the module handle of this program; no pointers kept.
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let description = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(raw_window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..Default::default()
        };
        // SAFETY: a valid class description; registering it twice only fails, which is
        // fine (the class from the first time is used).
        unsafe { RegisterClassExW(&description) };
        // SAFETY: a message-only window (parent HWND_MESSAGE) of the class above.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        (!hwnd.is_null()).then_some(RawWindow { hwnd })
    }
}

impl Drop for RawWindow {
    fn drop(&mut self) {
        // SAFETY: the window was created by this thread and is destroyed once.
        unsafe { DestroyWindow(self.hwnd) };
    }
}

unsafe extern "system" fn raw_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_HOTKEY {
        RAW_PRESSED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(wparam as u32);
    }
    // SAFETY: every message goes on to the default handling.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// The Windows registration. Create it on the main thread.
pub struct SystemHotkeys {
    manager: Option<global_hotkey::GlobalHotKeyManager>,
    registered: Vec<HotKey>,
    raw_window: Option<RawWindow>,
    raw_registered: Vec<u32>,
}

// SAFETY: the manager and the window are created and used on the main thread only (the
// app's frame); the value only moves into the app's state there.
unsafe impl Send for SystemHotkeys {}

impl SystemHotkeys {
    pub fn new() -> SystemHotkeys {
        let manager = match global_hotkey::GlobalHotKeyManager::new() {
            Ok(manager) => Some(manager),
            Err(error) => {
                log::warn!("Hotkeys are not available: {error}");
                None
            }
        };
        SystemHotkeys {
            manager,
            registered: Vec::new(),
            raw_window: None,
            raw_registered: Vec::new(),
        }
    }

    /// Ids of the hotkeys pressed since the last call.
    pub fn pressed() -> Vec<u32> {
        let mut ids = Vec::new();
        while let Ok(event) = global_hotkey::GlobalHotKeyEvent::receiver().try_recv() {
            if event.state == global_hotkey::HotKeyState::Pressed {
                ids.push(event.id);
            }
        }
        ids.append(&mut RAW_PRESSED.lock().unwrap_or_else(|e| e.into_inner()));
        ids
    }

    /// Register a key `global-hotkey` has no `Code` for, by its virtual-key code.
    fn register_raw(&mut self, info: HotkeyInfo) -> Result<u32, String> {
        if self.raw_window.is_none() {
            self.raw_window = RawWindow::new();
        }
        let hwnd = self.raw_window.as_ref().ok_or("no hotkey window")?.hwnd;
        let id = RAW_IDS
            .clone()
            .find(|id| {
                !self.raw_registered.contains(id) && !self.registered.iter().any(|h| h.id() == *id)
            })
            .ok_or("too many hotkeys")?;
        // SAFETY: `hwnd` is this thread's window; the id is free.
        if unsafe { RegisterHotKey(hwnd, id as i32, raw_modifiers(info), info.key_code()) } == 0 {
            return Err(crate::system::last_error_message());
        }
        self.raw_registered.push(id);
        Ok(id)
    }
}

impl Default for SystemHotkeys {
    fn default() -> Self {
        SystemHotkeys::new()
    }
}

impl HotkeyBackend for SystemHotkeys {
    fn register(&mut self, info: HotkeyInfo) -> Result<u32, String> {
        let Some(hotkey) = hotkey_for(info) else {
            if raw_key(info.key_code()) {
                return self.register_raw(info);
            }
            return Err("this key cannot be a hotkey".into());
        };
        let manager = self.manager.as_ref().ok_or("no hotkey manager")?;
        manager.register(hotkey).map_err(|e| e.to_string())?;
        self.registered.push(hotkey);
        Ok(hotkey.id())
    }

    fn unregister(&mut self, id: u32) {
        if let Some(index) = self.registered.iter().position(|h| h.id() == id) {
            let hotkey = self.registered.remove(index);
            if let Some(manager) = &self.manager {
                let _ = manager.unregister(hotkey);
            }
        } else if let Some(index) = self.raw_registered.iter().position(|r| *r == id) {
            self.raw_registered.remove(index);
            if let Some(window) = &self.raw_window {
                // SAFETY: the id was registered on this window.
                unsafe { UnregisterHotKey(window.hwnd, id as i32) };
            }
        }
    }
}

impl Drop for SystemHotkeys {
    fn drop(&mut self) {
        for id in std::mem::take(&mut self.raw_registered) {
            if let Some(window) = &self.raw_window {
                // SAFETY: the id was registered on this window.
                unsafe { UnregisterHotKey(window.hwnd, id as i32) };
            }
        }
        if let Some(manager) = &self.manager {
            let _ = manager.unregister_all(&self.registered);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modlight_core::hotkeys::KEY_NAMES;

    #[test]
    fn windows_keys_map_onto_codes() {
        assert_eq!(code_for(44, false), Some(Code::PrintScreen));
        assert_eq!(code_for(145, false), Some(Code::ScrollLock));
        assert_eq!(code_for(19, false), Some(Code::Pause));
        assert_eq!(code_for(3, true), Some(Code::Pause), "Ctrl+Break");
        assert_eq!(code_for(3, false), None);
        assert_eq!(code_for(50, false), Some(Code::Digit2));
        assert_eq!(code_for(65, false), Some(Code::KeyA));
        assert_eq!(code_for(105, false), Some(Code::Numpad9));
        assert_eq!(code_for(135, false), Some(Code::F24));
        assert_eq!(code_for(192, false), Some(Code::Backquote));
        assert_eq!(code_for(226, false), Some(Code::IntlBackslash));
        assert_eq!(code_for(173, false), Some(Code::AudioVolumeMute));
        assert_eq!(code_for(179, false), Some(Code::MediaPlayPause));
        assert_eq!(
            code_for(223, false),
            None,
            "Oem8 goes by its virtual-key code"
        );
        assert!(raw_key(223));
        assert!(!raw_key(1), "a mouse button");
        assert!(!raw_key(17), "Control alone");

        let hotkey = hotkey_for(HotkeyInfo::new(Keys::CONTROL | Keys::SHIFT | 50)).unwrap();
        assert_eq!(hotkey.mods, Modifiers::CONTROL | Modifiers::SHIFT);
        assert_eq!(hotkey.key, Code::Digit2);
        let win = hotkey_for(HotkeyInfo {
            hotkey: Keys(Keys::ALT | 44),
            win: true,
        })
        .unwrap();
        assert_eq!(win.mods, Modifiers::ALT | Modifiers::META);
        assert_eq!(
            raw_modifiers(HotkeyInfo {
                hotkey: Keys(Keys::CONTROL | Keys::ALT | 223),
                win: true,
            }),
            MOD_NOREPEAT | MOD_CONTROL | MOD_ALT | MOD_WIN
        );
    }

    #[test]
    fn every_key_of_a_windows_keyboard_can_be_registered() {
        for (name, code) in KEY_NAMES {
            // Modifier keys on their own and mouse buttons are never hotkeys.
            if matches!(*name, "None" | "Shift" | "Control" | "Alt") || !raw_key(*code) {
                continue;
            }
            // Without a `Code`: LineFeed, Select, Print, Execute (keys no keyboard has
            // any more) and Oem8, registered by their virtual-key code.
            assert!(
                code_for(*code, true).is_some() || matches!(code, 10 | 41 | 42 | 43 | 223),
                "{name} ({code}) has no Code"
            );
        }
    }

    /// Registers a combination nobody uses (Ctrl+Alt+Shift+Win+F24, and the same with
    /// Oem8) and takes it back at once.
    #[test]
    fn an_unlikely_hotkey_is_registered_and_released() {
        let mut hotkeys = SystemHotkeys::new();
        let all = Keys::CONTROL | Keys::ALT | Keys::SHIFT;
        let f24 = HotkeyInfo {
            hotkey: Keys(all | 135),
            win: true,
        };
        let id = hotkeys.register(f24).unwrap();
        hotkeys.unregister(id);
        assert!(hotkeys.registered.is_empty());
        let oem8 = HotkeyInfo {
            hotkey: Keys(all | 223),
            win: true,
        };
        let id = hotkeys.register(oem8).unwrap();
        assert!(RAW_IDS.contains(&id));
        hotkeys.unregister(id);
        assert!(hotkeys.raw_registered.is_empty());
        assert!(
            hotkeys.register(HotkeyInfo::new(1)).is_err(),
            "a mouse button"
        );
    }
}
