//! Where the app keeps its files (`Program.PersonalFolder`) and, on Windows, the shell's
//! known folders behind .NET's `Environment.GetFolderPath`.
//!
//! The personal folder holds `ApplicationConfig.json`, `HotkeysConfig.json`,
//! `Queue.json`, `Queue/`, `History.db`, `Logs/`, `Tools/`, `Models/`, `ImageEffects/`
//! and `Screenshots/`. On Windows it is upstream's `Documents\Modlight`
//! (`Program.DefaultPersonalFolder`: `MyDocuments` joined with `PersonalFolderName`). On
//! macOS it is `~/Library/Application Support/Modlight`, the place a Mac app keeps such
//! files (`docs/MACOS.md`). Upstream's other ways to move it (`-portable`, a `Portable`
//! file or `PersonalPath.cfg` beside the exe, the registry's `PersonalPath`) are not
//! ported: the folder is fixed on both systems.

use std::path::PathBuf;

/// `Program.PersonalFolderName`: the folder's own name, kept apart from the product's
/// name upstream so a rename never moves anybody's data.
pub const PERSONAL_FOLDER_NAME: &str = crate::APP_NAME;

/// `Program.PersonalFolder` (its default, `DefaultPersonalFolder`): `Documents\Modlight`
/// on Windows, `~/Library/Application Support/Modlight` on macOS.
///
/// On Windows the Documents folder is the shell's (`FOLDERID_Documents`), so a folder
/// the person moved (to another drive, into OneDrive) is followed as .NET follows it.
/// When the shell has none, .NET's `GetFolderPath` answers "" and upstream's
/// `Path.Combine` then gives a bare "Modlight", a folder under the working folder; the
/// same happens here.
pub fn personal_folder() -> PathBuf {
    #[cfg(windows)]
    {
        let documents = known::path(&known::DOCUMENTS).unwrap_or_default();
        PathBuf::from(documents).join(PERSONAL_FOLDER_NAME)
    }
    #[cfg(not(windows))]
    {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(PERSONAL_FOLDER_NAME)
    }
}

/// The shell's known folders (`SHGetKnownFolderPath`) and the system folder, the calls
/// .NET's `Environment.GetFolderPath` makes on Windows.
#[cfg(windows)]
pub(crate) mod known {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    use windows_sys::Win32::UI::Shell::SHGetKnownFolderPath;
    use windows_sys::core::{GUID, PWSTR};

    pub use windows_sys::Win32::UI::Shell::FOLDERID_Documents as DOCUMENTS;

    /// The known folder's path, as `SHGetKnownFolderPath(id, KF_FLAG_DEFAULT)` gives it
    /// (`SpecialFolderOption.None`); `None` when the shell has no such folder here (a
    /// virtual folder such as the printers, or one that does not exist).
    pub fn path(id: &GUID) -> Option<String> {
        let mut raw: PWSTR = std::ptr::null_mut();
        // SAFETY: `id` is a valid GUID; the shell allocates the string, which is read
        // up to its terminator and freed with `CoTaskMemFree` whatever the result.
        let result = unsafe { SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut raw) };
        let text = if result >= 0 && !raw.is_null() {
            // SAFETY: on success `raw` points at a NUL-terminated UTF-16 string.
            Some(unsafe { from_wide_nul(raw) })
        } else {
            None
        };
        if !raw.is_null() {
            // SAFETY: the buffer came from the shell's allocator.
            unsafe { CoTaskMemFree(raw as *const core::ffi::c_void) };
        }
        text
    }

    /// `Environment.SystemDirectory` (`GetSystemDirectoryW`), "C:\Windows\system32".
    pub fn system_directory() -> String {
        let mut buffer = vec![0u16; 260];
        loop {
            // SAFETY: the buffer's length is passed with it.
            let length =
                unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
            if length == 0 {
                return String::new();
            }
            if length < buffer.len() {
                return String::from_utf16_lossy(&buffer[..length]);
            }
            buffer.resize(length + 1, 0);
        }
    }

    /// A NUL-terminated UTF-16 string as Rust text.
    ///
    /// # Safety
    /// `raw` must point at a readable, NUL-terminated UTF-16 string.
    pub unsafe fn from_wide_nul(raw: *const u16) -> String {
        let mut length = 0;
        // SAFETY: the caller promises a terminator.
        while unsafe { *raw.add(length) } != 0 {
            length += 1;
        }
        // SAFETY: `length` units were just read.
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(raw, length) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn the_personal_folder_is_documents_modlight() {
        let folder = personal_folder();
        assert_eq!(folder.file_name().unwrap(), "Modlight");
        let documents = known::path(&known::DOCUMENTS).expect("a Documents folder");
        assert_eq!(folder, std::path::Path::new(&documents).join("Modlight"));
        assert!(folder.is_absolute());
        let system = known::system_directory();
        assert!(
            system.to_ascii_lowercase().ends_with("system32"),
            "{system}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_personal_folder_is_in_application_support() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            personal_folder(),
            std::path::Path::new(&home).join("Library/Application Support/Modlight")
        );
    }
}
