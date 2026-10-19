//! What the system says, and the few things the app asks it to do.

pub use crate::windows::{active_window_fullscreen, show_app_windows, visible_frame};

use std::path::Path;
use std::process::{Command, Stdio};

/// Open an http(s) link in the default browser (`URLHelpers.OpenURL`). Anything that is
/// not an absolute http(s) URL is refused rather than handed to `open`.
pub fn open_url(url: &str) -> bool {
    let trimmed = url.trim();
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return false;
    }
    Command::new("/usr/bin/open")
        .arg(trimmed)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

/// Show a file in Finder (`OpenFolderWithFile`).
pub fn reveal_in_finder(path: &Path) -> bool {
    Command::new("/usr/bin/open")
        .arg("-R")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

/// Open a file or folder with its app (`FileHelpers.OpenFile`, `OpenFolder`). The path
/// goes after `--`, so a name that starts with a dash is never read as an option.
pub fn open_path(path: &Path) -> bool {
    Command::new("/usr/bin/open")
        .arg("--")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

/// Move a file to the Trash (`FileHelpers.DeleteFile(path, sendToRecycleBin: true)`).
/// The error is the system's own sentence.
#[cfg(target_os = "macos")]
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};

    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, None)
        .map_err(|error| error.localizedDescription().to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

fn defaults_read(domain: &str, key: &str) -> Option<String> {
    let output = Command::new("/usr/bin/defaults")
        .args(["read", domain, key])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_string())
}

/// The machine's name, for the token list ("Modlight on `<name>`").
pub fn machine_name() -> String {
    if let Ok(output) = Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        && output.status.success()
        && let Ok(name) = String::from_utf8(output.stdout)
        && !name.trim().is_empty()
    {
        return name.trim().to_string();
    }
    std::env::var("HOST")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Mac".to_string())
}

/// The system locale identifier (`AppleLocale`, e.g. `en_PL`): numbers and dates.
pub fn system_locale() -> String {
    defaults_read("-g", "AppleLocale").unwrap_or_else(|| "en_US".to_string())
}

/// The first preferred language (`AppleLanguages`, e.g. `pl-PL`): the interface
/// language when the setting is Automatic.
pub fn preferred_language() -> String {
    defaults_read("-g", "AppleLanguages")
        .and_then(|list| {
            list.split(['(', ')', ',', '"', '\n'])
                .map(str::trim)
                .find(|item| !item.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "en".to_string())
}

/// Whether the system asks for less motion (the counterpart of Windows' client-area
/// animations switch, `ClientAreaAnimation.Enabled`).
pub fn reduce_motion() -> bool {
    defaults_read("com.apple.universalaccess", "reduceMotion").as_deref() == Some("1")
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_web_links_are_opened() {
        assert!(!super::open_url("file:///etc/passwd"));
        assert!(!super::open_url("-a Calculator"));
        assert!(!super::open_url("javascript:alert(1)"));
    }
}
