//! Global hotkeys (`RegisterHotKey` in upstream's `HotkeyForm`), through the
//! `global-hotkey` crate, which registers exactly that. WinForms key codes are Windows
//! virtual-key codes, so a binding means the key it names. Normal mode only: QA never
//! registers anything.

use modlight_core::hotkeys::{HotkeyBackend, HotkeyInfo};

/// The Windows registration. Create it on the main thread.
pub struct SystemHotkeys {}

impl SystemHotkeys {
    pub fn new() -> SystemHotkeys {
        SystemHotkeys {}
    }

    /// Ids of the hotkeys pressed since the last call.
    pub fn pressed() -> Vec<u32> {
        Vec::new()
    }
}

impl Default for SystemHotkeys {
    fn default() -> Self {
        SystemHotkeys::new()
    }
}

impl HotkeyBackend for SystemHotkeys {
    fn register(&mut self, info: HotkeyInfo) -> Result<u32, String> {
        let _ = info;
        Err("not implemented yet".into())
    }

    fn unregister(&mut self, id: u32) {
        let _ = id;
    }
}
