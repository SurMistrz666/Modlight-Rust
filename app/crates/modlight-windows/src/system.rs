//! What the system says, and the few things the app asks it to do.

pub use crate::windows::{active_window_fullscreen, show_app_windows, visible_frame};

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_CANCELLED, ERROR_FILE_NOT_FOUND, ERROR_FILE_TOO_LARGE,
    ERROR_FILENAME_EXCED_RANGE, ERROR_GEN_FAILURE, ERROR_WRITE_PROTECT, GetLastError, LocalFree,
};
use windows_sys::Win32::Globalization::{
    GetUserDefaultLocaleName, GetUserDefaultUILanguage, GetUserPreferredUILanguages,
    LCIDToLocaleName, MUI_LANGUAGE_NAME,
};
use windows_sys::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    FORMAT_MESSAGE_ALLOCATE_BUFFER, FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS,
    FormatMessageW,
};
use windows_sys::Win32::System::SystemInformation::{
    ComputerNameNetBIOS, GetComputerNameExW, GetSystemWindowsDirectoryW,
};
use windows_sys::Win32::UI::Shell::{
    FO_DELETE, FOF_ALLOWUNDO, FOF_NO_CONNECTED_ELEMENTS, FOF_NOCONFIRMATION, FOF_NOERRORUI,
    FOF_SILENT, ILCreateFromPathW, ILFree, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC,
    SHELLEXECUTEINFOW, SHFILEOPSTRUCTW, SHFileOperationW, SHOpenFolderAndSelectItems,
    ShellExecuteExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SW_SHOWNORMAL, SystemParametersInfoW,
};

use crate::wide::{from_wide, wide};

/// Run `work` on a thread of its own with COM initialised there (a single-threaded
/// apartment, which the shell wants), and wait for its answer. The calling thread's COM
/// state, whatever it is, is never touched. `None` when the thread could not run.
pub(crate) fn on_com_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    std::thread::Builder::new()
        .name("modlight-shell".into())
        .spawn(move || with_com(work))
        .ok()?
        .join()
        .ok()
}

/// The same, without waiting: `true` when the thread started.
fn spawn_on_com_thread(work: impl FnOnce() + Send + 'static) -> bool {
    std::thread::Builder::new()
        .name("modlight-shell".into())
        .spawn(move || with_com(work))
        .is_ok()
}

fn with_com<T>(work: impl FnOnce() -> T) -> T {
    // SAFETY: initialises COM on this thread, balanced below when it succeeded.
    let result = unsafe {
        CoInitializeEx(
            std::ptr::null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        )
    };
    let answer = work();
    if result >= 0 {
        // SAFETY: balances the successful CoInitializeEx above, on the same thread.
        unsafe { CoUninitialize() };
    }
    answer
}

/// The system's own sentence for a Win32 error code, in the user's language.
pub(crate) fn system_message(code: u32) -> String {
    let mut buffer: *mut u16 = std::ptr::null_mut();
    // SAFETY: with FORMAT_MESSAGE_ALLOCATE_BUFFER the system allocates the text and
    // writes its address into `buffer`; it is freed with LocalFree below.
    let length = unsafe {
        FormatMessageW(
            FORMAT_MESSAGE_ALLOCATE_BUFFER
                | FORMAT_MESSAGE_FROM_SYSTEM
                | FORMAT_MESSAGE_IGNORE_INSERTS,
            std::ptr::null(),
            code,
            0,
            (&mut buffer as *mut *mut u16).cast(),
            0,
            std::ptr::null(),
        )
    };
    if length == 0 || buffer.is_null() {
        return format!("Windows error {code}");
    }
    // SAFETY: the system wrote `length` UTF-16 units at `buffer`.
    let text =
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(buffer, length as usize) });
    // SAFETY: `buffer` was allocated by FormatMessageW for us.
    unsafe { LocalFree(buffer.cast()) };
    text.trim().to_string()
}

/// The sentence for the calling thread's last error.
pub(crate) fn last_error_message() -> String {
    // SAFETY: reads the calling thread's last error code.
    system_message(unsafe { GetLastError() })
}

/// Open an http(s) link in the default browser (`URLHelpers.OpenURL`). Anything that is
/// not an absolute http(s) URL is refused rather than handed to the shell.
pub fn open_url(url: &str) -> bool {
    match shell_safe_url(url) {
        Some(url) => shell_open(OsString::from(url), Some("open")),
        None => false,
    }
}

/// `url` as it may be handed to the shell: an absolute http(s) URL without control
/// characters. Browsers are registered as `"browser.exe" ... "%1"`, so a quote, a
/// backslash before the closing quote or a space could end the argument early and add
/// options of the link's choosing; those three are percent-encoded.
fn shell_safe_url(url: &str) -> Option<String> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return None;
    }
    if trimmed.chars().any(char::is_control) {
        return None;
    }
    let mut safe = String::with_capacity(trimmed.len());
    for c in trimmed.chars() {
        match c {
            '"' => safe.push_str("%22"),
            ' ' => safe.push_str("%20"),
            '\\' => safe.push_str("%5C"),
            _ => safe.push(c),
        }
    }
    Some(safe)
}

/// `ShellExecuteEx` on a thread of its own, as upstream's `Process.Start` with
/// `UseShellExecute` (no error dialog of the system's). `verb` `None` is the default
/// verb, what a double click does.
pub(crate) fn shell_open(target: OsString, verb: Option<&'static str>) -> bool {
    spawn_on_com_thread(move || {
        let file = wide(&target);
        let verb = verb.map(wide);
        // SAFETY: an all-zero SHELLEXECUTEINFOW is valid; the fields set below point at
        // NUL-terminated strings that outlive the call.
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
        info.lpVerb = verb.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());
        info.lpFile = file.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        // SAFETY: `info` is initialised as the function requires.
        if unsafe { ShellExecuteExW(&mut info) } == 0 {
            log::warn!(
                "Modlight: the shell could not open {}: {}",
                target.to_string_lossy(),
                last_error_message()
            );
        }
    })
}

/// The Windows folder (`C:\Windows`), where `explorer.exe` is.
fn windows_folder() -> PathBuf {
    let mut buffer = [0u16; 260];
    // SAFETY: the buffer's length is passed with it.
    let length = unsafe { GetSystemWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 || length as usize >= buffer.len() {
        return PathBuf::from(r"C:\Windows");
    }
    PathBuf::from(OsString::from_wide(&buffer[..length as usize]))
}

/// Show a file in Explorer, selected (`OpenFolderWithFile`: `SHOpenFolderAndSelectItems`
/// on the file's ID list). The macOS name is kept. When the shell call fails, Explorer is
/// started with `/select,"<path>"` instead.
pub fn reveal_in_finder(path: &Path) -> bool {
    let Ok(path) = std::path::absolute(path) else {
        return false;
    };
    if !path.exists() {
        return false;
    }
    spawn_on_com_thread(move || {
        if !open_folder_and_select(&path) {
            let mut argument = OsString::from("/select,\"");
            argument.push(path.as_os_str());
            argument.push("\"");
            if let Err(error) = Command::new(windows_folder().join("explorer.exe"))
                .raw_arg(&argument)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                log::warn!("Modlight: Explorer did not start: {error}");
            }
        }
    })
}

fn open_folder_and_select(path: &Path) -> bool {
    let text = wide(path);
    // SAFETY: `text` is NUL-terminated; the ID list is freed below.
    let list = unsafe { ILCreateFromPathW(text.as_ptr()) };
    if list.is_null() {
        return false;
    }
    // SAFETY: `list` is a valid absolute ID list; no children are passed.
    let result = unsafe { SHOpenFolderAndSelectItems(list, 0, std::ptr::null(), 0) };
    // SAFETY: allocated by ILCreateFromPathW above, freed once.
    unsafe { ILFree(list) };
    result >= 0
}

/// Open a file or folder with its program (`FileHelpers.OpenFile`, `OpenFolder`): the
/// shell's default verb, as a double click. Only an absolute path that exists is opened,
/// so a bare name is never looked up in the working folder or on `PATH`; a folder gets
/// its trailing backslash, as upstream gives it, so a program of the same name beside it
/// is never what opens.
pub fn open_path(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    let mut target = path.as_os_str().to_owned();
    if metadata.is_dir()
        && !target
            .encode_wide()
            .last()
            .is_some_and(|c| c == '\\' as u16)
    {
        target.push("\\");
    }
    shell_open(target, None)
}

/// Move a file to the Recycle Bin (`FileHelpers.DeleteFile(path, sendToRecycleBin: true)`,
/// which is Visual Basic's `FileSystem.DeleteFile` with `SendToRecycleBin`:
/// `SHFileOperation` with undo allowed, no question). As there, an item on a drive
/// without a Recycle Bin is deleted for good. The error is the system's own sentence.
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    std::fs::symlink_metadata(&path).map_err(|e| match e.raw_os_error() {
        Some(code) => system_message(code as u32),
        None => e.to_string(),
    })?;
    let mut from: Vec<u16> = strip_verbatim(path.as_os_str()).encode_wide().collect();
    // `SHFileOperation` reads wildcards: a path that has one would delete what it
    // matches. No file name can contain them, so such a path is not a file of ours.
    if from.iter().any(|&c| c == '*' as u16 || c == '?' as u16) {
        return Err(system_message(
            windows_sys::Win32::Foundation::ERROR_INVALID_NAME,
        ));
    }
    // A list of paths, each NUL-terminated, ending with an empty one.
    from.extend([0, 0]);
    on_com_thread(move || recycle(&from)).unwrap_or_else(|| Err(system_message(ERROR_GEN_FAILURE)))
}

/// `\\?\C:\x` as `C:\x` and `\\?\UNC\server\x` as `\\server\x`: the shell's file
/// operations do not take the long-path form.
fn strip_verbatim(path: &OsStr) -> OsString {
    let text: Vec<u16> = path.encode_wide().collect();
    let prefix: Vec<u16> = r"\\?\".encode_utf16().collect();
    let unc: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
    if text.starts_with(&unc) {
        let mut out: Vec<u16> = r"\\".encode_utf16().collect();
        out.extend_from_slice(&text[unc.len()..]);
        OsString::from_wide(&out)
    } else if text.starts_with(&prefix) {
        OsString::from_wide(&text[prefix.len()..])
    } else {
        path.to_owned()
    }
}

fn recycle(from: &[u16]) -> Result<(), String> {
    let mut operation = SHFILEOPSTRUCTW {
        hwnd: std::ptr::null_mut(),
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        pTo: std::ptr::null(),
        fFlags: (FOF_ALLOWUNDO
            | FOF_NOCONFIRMATION
            | FOF_SILENT
            | FOF_NOERRORUI
            | FOF_NO_CONNECTED_ELEMENTS) as u16,
        fAnyOperationsAborted: 0,
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: std::ptr::null(),
    };
    // SAFETY: `from` is a double-NUL-terminated list that outlives the call.
    let result = unsafe { SHFileOperationW(&mut operation) };
    if result != 0 {
        return Err(system_message(file_operation_error(result as u32)));
    }
    if operation.fAnyOperationsAborted != 0 {
        return Err(system_message(ERROR_CANCELLED));
    }
    Ok(())
}

/// `SHFileOperation` answers with Win32 codes and with old `DE_` codes of its own, which
/// have no sentence of the system's: those become the nearest Win32 code.
fn file_operation_error(code: u32) -> u32 {
    match code {
        // DE_OPCANCELLED.
        0x75 => ERROR_CANCELLED,
        // DE_ROOTDIR, DE_ACCESSDENIEDSRC.
        0x74 | 0x78 => ERROR_ACCESS_DENIED,
        // DE_PATHTOODEEP, DE_FILENAMETOOLONG, DE_ERROR_MAX.
        0x79 | 0x81 | 0xB7 => ERROR_FILENAME_EXCED_RANGE,
        // DE_INVALIDFILES: the source is not there (any more).
        0x7C => ERROR_FILE_NOT_FOUND,
        // DE_FILE_TOO_LARGE.
        0x85 => ERROR_FILE_TOO_LARGE,
        // Read-only and unformatted CD and DVD media.
        0x82..=0x84 | 0x86..=0x88 => ERROR_WRITE_PROTECT,
        // The rest of the DE_ range and ERRORONDEST.
        0x71..=0x88 | 0x402 | 0x10000 | 0x10074 => ERROR_GEN_FAILURE,
        other => other,
    }
}

/// The computer's name, for the token list ("Modlight on `<name>`"): the NetBIOS name, as
/// upstream's `Environment.MachineName`.
pub fn machine_name() -> String {
    let mut buffer = [0u16; 64];
    let mut size = buffer.len() as u32;
    // SAFETY: `size` holds the buffer's length in UTF-16 units.
    if unsafe { GetComputerNameExW(ComputerNameNetBIOS, buffer.as_mut_ptr(), &mut size) } != 0 {
        let name = from_wide(&buffer[..size as usize]);
        if !name.trim().is_empty() {
            return name;
        }
    }
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".to_string())
}

/// Longest locale name the system writes (`LOCALE_NAME_MAX_LENGTH`).
const LOCALE_NAME_LENGTH: usize = 85;

/// The user's locale (`CurrentCulture`, e.g. `pl-PL`): numbers and dates.
/// `Culture::for_locale` reads `pl-PL` as it reads the Mac's `pl_PL`.
pub fn system_locale() -> String {
    let mut buffer = [0u16; LOCALE_NAME_LENGTH];
    // SAFETY: the buffer's length is passed with it.
    let length = unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), buffer.len() as i32) };
    let name = if length > 0 {
        from_wide(&buffer)
    } else {
        String::new()
    };
    if name.is_empty() {
        "en-US".to_string()
    } else {
        name
    }
}

/// The first preferred interface language (`CurrentUICulture`, e.g. `pl-PL`): the
/// interface language when the setting is Automatic.
pub fn preferred_language() -> String {
    preferred_ui_languages()
        .into_iter()
        .next()
        .or_else(default_ui_language)
        .unwrap_or_else(|| "en".to_string())
}

/// `GetUserPreferredUILanguages`: the user's interface languages, most wanted first.
fn preferred_ui_languages() -> Vec<String> {
    let mut count = 0u32;
    let mut length = 0u32;
    // SAFETY: a null buffer asks for the length in UTF-16 units.
    if unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            std::ptr::null_mut(),
            &mut length,
        )
    } == 0
        || length == 0
    {
        return Vec::new();
    }
    let mut buffer = vec![0u16; length as usize];
    // SAFETY: `buffer` holds `length` units, as the call above asked for.
    if unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            buffer.as_mut_ptr(),
            &mut length,
        )
    } == 0
    {
        return Vec::new();
    }
    // A list of NUL-terminated names ending with an empty one.
    buffer
        .split(|&c| c == 0)
        .filter(|name| !name.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

/// `GetUserDefaultUILanguage` as a tag.
fn default_ui_language() -> Option<String> {
    // SAFETY: no arguments.
    let lcid = unsafe { GetUserDefaultUILanguage() };
    let mut buffer = [0u16; LOCALE_NAME_LENGTH];
    // SAFETY: the buffer's length is passed with it.
    let length =
        unsafe { LCIDToLocaleName(lcid as u32, buffer.as_mut_ptr(), buffer.len() as i32, 0) };
    (length > 0)
        .then(|| from_wide(&buffer))
        .filter(|name| !name.is_empty())
}

/// Whether the system asks for less motion (`SystemParameters.ClientAreaAnimation`
/// switched off, "Animation effects" in Accessibility settings). When it cannot be read,
/// animations stay on, as upstream's `ClientAreaAnimation.Enabled` decides.
pub fn reduce_motion() -> bool {
    let mut enabled: i32 = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL into `enabled`.
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        )
    };
    read != 0 && enabled == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_links_are_opened() {
        assert!(!super::open_url("file:///C:/Windows/win.ini"));
        assert!(!super::open_url("calc.exe"));
        assert!(!super::open_url("javascript:alert(1)"));
        assert!(!super::open_url("ms-settings:startupapps"));
        assert!(!super::open_url("https://example.com/\r\nx"));
    }

    #[test]
    fn links_cannot_end_the_browsers_argument() {
        assert_eq!(
            shell_safe_url(" https://example.com/a b\"c\\ ").as_deref(),
            Some("https://example.com/a%20b%22c%5C")
        );
        assert_eq!(
            shell_safe_url("http://127.0.0.1:8420/x?y=1&z=%41").as_deref(),
            Some("http://127.0.0.1:8420/x?y=1&z=%41")
        );
        assert_eq!(shell_safe_url("https://a/\tb"), None);
    }

    #[test]
    fn only_absolute_existing_paths_are_opened() {
        assert!(!open_path(Path::new("notepad.exe")));
        assert!(!open_path(Path::new(
            r"C:\no such folder of modlight\x.txt"
        )));
        assert!(!reveal_in_finder(Path::new(
            r"C:\no such folder of modlight\x.txt"
        )));
    }

    #[test]
    fn a_missing_file_is_not_trashed() {
        let error = move_to_trash(Path::new(r"C:\no such folder of modlight\x.txt")).unwrap_err();
        assert!(!error.is_empty());
        assert!(!error.contains("os error"), "{error}");
    }

    #[test]
    fn verbatim_paths_lose_their_prefix() {
        assert_eq!(strip_verbatim(OsStr::new(r"\\?\C:\a\b.txt")), r"C:\a\b.txt");
        assert_eq!(
            strip_verbatim(OsStr::new(r"\\?\UNC\srv\share\b")),
            r"\\srv\share\b"
        );
        assert_eq!(strip_verbatim(OsStr::new(r"C:\a")), r"C:\a");
    }

    /// Puts a small file into this user's Recycle Bin, so it is run by hand only.
    #[test]
    #[ignore]
    fn a_file_goes_to_the_recycle_bin() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("modlight-trash-test.txt");
        std::fs::write(&file, b"Modlight test file, safe to delete").unwrap();
        move_to_trash(&file).unwrap();
        assert!(!file.exists());
    }

    #[test]
    fn the_system_answers() {
        assert!(!machine_name().is_empty());
        let locale = system_locale();
        assert!(locale.len() >= 2 && !locale.contains('_'), "{locale}");
        let language = preferred_language();
        assert!(language.len() >= 2, "{language}");
        let _ = reduce_motion();
        assert!(!system_message(ERROR_ACCESS_DENIED).is_empty());
    }
}
