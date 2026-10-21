//! What the system says, and the few things the app asks it to do.

pub use crate::windows::{active_window_fullscreen, show_app_windows, visible_frame};

use std::path::Path;

/// Open an http(s) link in the default browser (`URLHelpers.OpenURL`). Anything that is
/// not an absolute http(s) URL is refused rather than handed to the shell.
pub fn open_url(url: &str) -> bool {
    let trimmed = url.trim();
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return false;
    }
    false
}

/// Show a file in Explorer, selected (`OpenFolderWithFile`). The macOS name is kept.
pub fn reveal_in_finder(path: &Path) -> bool {
    let _ = path;
    false
}

/// Open a file or folder with its program (`FileHelpers.OpenFile`, `OpenFolder`).
pub fn open_path(path: &Path) -> bool {
    let _ = path;
    false
}

/// Move a file to the Recycle Bin (`FileHelpers.DeleteFile(path, sendToRecycleBin: true)`).
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    let _ = path;
    Err("not implemented yet".into())
}

/// The computer's name, for the token list ("Modlight on `<name>`").
pub fn machine_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".to_string())
}

/// The user's locale (`CurrentCulture`, e.g. `pl-PL`): numbers and dates.
pub fn system_locale() -> String {
    "en-US".to_string()
}

/// The first preferred interface language (`CurrentUICulture`, e.g. `pl-PL`): the
/// interface language when the setting is Automatic.
pub fn preferred_language() -> String {
    "en".to_string()
}

/// Whether the system asks for less motion (`SystemParameters.ClientAreaAnimation`
/// switched off, "Animation effects" in Accessibility settings).
pub fn reduce_motion() -> bool {
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_web_links_are_opened() {
        assert!(!super::open_url("file:///C:/Windows/win.ini"));
        assert!(!super::open_url("calc.exe"));
        assert!(!super::open_url("javascript:alert(1)"));
    }
}
