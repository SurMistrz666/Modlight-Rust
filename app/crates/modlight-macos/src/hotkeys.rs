//! Global hotkeys on a Mac (`RegisterHotKey` in upstream's `HotkeyForm`): the
//! `global-hotkey` crate, which registers through Carbon's `RegisterEventHotKey`. That
//! needs no permission (unlike a keyboard event tap) and sees only the combinations that
//! were registered. Normal mode only: QA never registers anything.
//!
//! WinForms key codes map onto the Mac keyboard as MACOS.md describes: Control is
//! Command, Alt is Option, Win is Control; Print Screen, Scroll Lock and Pause are F13,
//! F14 and F15.

use modlight_core::hotkeys::{HotkeyBackend, HotkeyInfo, Keys};

#[cfg(target_os = "macos")]
use global_hotkey::hotkey::{Code, HotKey, Modifiers};

/// The `Code` of a WinForms key code, when the Mac keyboard has the key.
#[cfg(target_os = "macos")]
pub fn code_for(key: u32) -> Option<Code> {
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
    ];
    Some(match key {
        65..=90 => letters[(key - 65) as usize],
        48..=57 => digits[(key - 48) as usize],
        96..=105 => numpad[(key - 96) as usize],
        112..=131 => functions[(key - 112) as usize],
        8 => Backspace,
        9 => Tab,
        13 => Enter,
        27 => Escape,
        32 => Space,
        33 => PageUp,
        34 => PageDown,
        35 => End,
        36 => Home,
        37 => ArrowLeft,
        38 => ArrowUp,
        39 => ArrowRight,
        40 => ArrowDown,
        45 => Help,
        46 => Delete,
        47 => Help,
        // The keys in Print Screen's, Scroll Lock's and Pause's place on Apple keyboards.
        44 => F13,
        145 => F14,
        19 => F15,
        106 => NumpadMultiply,
        107 => NumpadAdd,
        109 => NumpadSubtract,
        110 => NumpadDecimal,
        111 => NumpadDivide,
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
        _ => return None,
    })
}

/// The registration a binding becomes.
#[cfg(target_os = "macos")]
pub fn hotkey_for(info: HotkeyInfo) -> Option<HotKey> {
    let code = code_for(info.key_code())?;
    let mut mods = Modifiers::empty();
    if info.hotkey.has(Keys::CONTROL) {
        mods |= Modifiers::META;
    }
    if info.hotkey.has(Keys::SHIFT) {
        mods |= Modifiers::SHIFT;
    }
    if info.hotkey.has(Keys::ALT) {
        mods |= Modifiers::ALT;
    }
    if info.win {
        mods |= Modifiers::CONTROL;
    }
    Some(HotKey::new(Some(mods), code))
}

/// The Mac's registration. Create it on the main thread.
pub struct MacHotkeys {
    #[cfg(target_os = "macos")]
    manager: Option<global_hotkey::GlobalHotKeyManager>,
    #[cfg(target_os = "macos")]
    registered: Vec<HotKey>,
}

// SAFETY: the manager is created and used on the main thread only (the app's frame).
unsafe impl Send for MacHotkeys {}

impl MacHotkeys {
    pub fn new() -> MacHotkeys {
        #[cfg(target_os = "macos")]
        {
            let manager = match global_hotkey::GlobalHotKeyManager::new() {
                Ok(manager) => Some(manager),
                Err(error) => {
                    log::warn!("Hotkeys are not available: {error}");
                    None
                }
            };
            MacHotkeys {
                manager,
                registered: Vec::new(),
            }
        }
        #[cfg(not(target_os = "macos"))]
        MacHotkeys {}
    }

    /// Ids of the hotkeys pressed since the last call.
    pub fn pressed() -> Vec<u32> {
        #[cfg(target_os = "macos")]
        {
            let mut ids = Vec::new();
            while let Ok(event) = global_hotkey::GlobalHotKeyEvent::receiver().try_recv() {
                if event.state == global_hotkey::HotKeyState::Pressed {
                    ids.push(event.id);
                }
            }
            ids
        }
        #[cfg(not(target_os = "macos"))]
        Vec::new()
    }
}

impl Default for MacHotkeys {
    fn default() -> Self {
        MacHotkeys::new()
    }
}

impl HotkeyBackend for MacHotkeys {
    fn register(&mut self, info: HotkeyInfo) -> Result<u32, String> {
        #[cfg(target_os = "macos")]
        {
            let manager = self.manager.as_ref().ok_or("no hotkey manager")?;
            let hotkey = hotkey_for(info).ok_or("this key is not on a Mac keyboard")?;
            manager.register(hotkey).map_err(|e| e.to_string())?;
            self.registered.push(hotkey);
            Ok(hotkey.id())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = info;
            Err("hotkeys need macOS".into())
        }
    }

    fn unregister(&mut self, id: u32) {
        #[cfg(target_os = "macos")]
        if let Some(index) = self.registered.iter().position(|h| h.id() == id) {
            let hotkey = self.registered.remove(index);
            if let Some(manager) = &self.manager {
                let _ = manager.unregister(hotkey);
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = id;
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn windows_keys_map_onto_the_mac_keyboard() {
        assert_eq!(code_for(44), Some(Code::F13));
        assert_eq!(code_for(50), Some(Code::Digit2));
        assert_eq!(code_for(192), Some(Code::Backquote));
        assert_eq!(code_for(173), None, "no volume keys");
        let hotkey = hotkey_for(HotkeyInfo::new(Keys::CONTROL | Keys::SHIFT | 50)).unwrap();
        assert_eq!(hotkey.mods, Modifiers::META | Modifiers::SHIFT);
        let win = hotkey_for(HotkeyInfo {
            hotkey: Keys(Keys::ALT | 65),
            win: true,
        })
        .unwrap();
        assert_eq!(win.mods, Modifiers::ALT | Modifiers::CONTROL);
    }
}
