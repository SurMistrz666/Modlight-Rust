//! Upstream's Windows integration (`StartupManager`, `IntegrationHelpers`), back on
//! Windows: "Start with Windows" (a `Modlight.lnk` shortcut in this user's Startup
//! folder, started with `-silent`), the Explorer context menu "Upload with Modlight" and
//! the "Send to" entry, and the single running instance that receives the files those
//! hand to a second start of the program.
//!
//! The function names are the macOS port's (`in_app_bundle`, `take_service_files`...),
//! so the app calls one API; each says what it means here.
//!
//! Everything is this user's (`HKEY_CURRENT_USER`, the user's own Startup and SendTo
//! folders): nothing needs an administrator. The places are parameters of the inner
//! functions so the tests use a throwaway registry key and folder, never the real ones.
//!
//! # One running copy
//!
//! Upstream's `SingleInstanceManager`: the first start of the program for this user in
//! this session owns a named mutex (`Local\Modlight-<user SID>`) and answers on a named
//! pipe (`\\.\pipe\Modlight-<user SID>-<session>`); a later start finds the mutex, hands
//! its files (or a request to come forward) over the pipe and exits. The pipe takes local
//! clients only and only this user (its DACL names the user alone, and it is owned by the
//! user, which a client checks before it sends anything, so a pipe of the same name set
//! up by somebody else gets nothing). What arrives is data: paths that are absolute and
//! exist are queued for the window to upload, an activation request is noted for the
//! window to read; nothing received is ever run. Messages are capped at 1 MiB.
//!
//! Upstream's own names (`3030612E-...` and `<machine>-<user>-Modlight`) are not reused:
//! its pipe speaks .NET's `BinaryWriter` format and answers nothing, while a start here
//! needs to know whether its files were taken.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, ERROR_SUCCESS, ERROR_TIMEOUT, GENERIC_READ, GENERIC_WRITE, GetLastError,
    HANDLE, INVALID_HANDLE_VALUE, LocalFree, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    EqualSid, GetTokenInformation, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenSessionId, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX, ReadFile, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT, WaitNamedPipeW,
};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_BINARY,
    RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetValueExW,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, GetCurrentProcess, OpenProcessToken, WaitForSingleObject,
};
use windows_sys::Win32::UI::Shell::{FOLDERID_SendTo, FOLDERID_Startup, SHGetKnownFolderPath};

use crate::system::{on_com_thread, shell_open, system_message};
use crate::wide::{from_wide, from_wide_ptr, wide};

/// `StartupState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupState {
    Disabled,
    /// Turned off in Task Manager's Startup apps (`StartupApproved\StartupFolder`).
    DisabledByUser,
    Enabled,
    /// Kept for the macOS API; Windows never waits for an approval.
    RequiresApproval,
    /// Not running from an installed copy: a development build in a cargo target folder.
    NotAvailable,
}

/// The shortcut upstream puts in the Startup and SendTo folders (`"Modlight"` + `.lnk`).
const SHORTCUT_NAME: &str = "Modlight.lnk";
/// What Windows starts the program with at sign-in (`StartupManager`): no window.
const STARTUP_ARGUMENTS: &str = "-silent";
/// Under `HKEY_CURRENT_USER\Software`: where Task Manager notes a Startup folder item it
/// switched off (first byte 3), as upstream reads it.
const STARTUP_APPROVED: &str =
    r"Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder";
/// Under `HKEY_CURRENT_USER\Software\Classes`: upstream's verb key name.
const MENU_KEYS: [&str; 2] = [r"*\shell\Modlight", r"Directory\shell\Modlight"];
/// The real registry root of everything here, under `HKEY_CURRENT_USER`.
const SOFTWARE: &str = "Software";

/// Whether this is a copy that may register itself with Windows (start with Windows,
/// the Explorer menu): any build that does not run from a cargo `target` folder. QA runs
/// are kept away by the app itself (normal mode only), as on a Mac.
pub fn in_app_bundle() -> bool {
    std::env::current_exe().is_ok_and(|exe| !is_development_build(&exe))
}

/// Whether `exe` runs from a cargo target folder: a folder named `target` that cargo
/// marked (`CACHEDIR.TAG`), or that has `debug` or `release` below it on the way to the
/// program.
fn is_development_build(exe: &Path) -> bool {
    exe.ancestors().skip(1).any(|folder| {
        folder
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("target"))
            && (folder.join("CACHEDIR.TAG").is_file()
                || exe.strip_prefix(folder).is_ok_and(|rest| {
                    rest.components().any(|part| {
                        let part = part.as_os_str();
                        part.eq_ignore_ascii_case("debug") || part.eq_ignore_ascii_case("release")
                    })
                }))
    })
}

// ------------------------------------------------------------------ places

/// A known folder of this user (`Environment.GetFolderPath`).
fn known_folder(id: &windows_sys::core::GUID) -> Option<PathBuf> {
    let mut path: *mut u16 = std::ptr::null_mut();
    // SAFETY: `path` receives a string the system allocates; freed below.
    let result = unsafe { SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut path) };
    // SAFETY: on success `path` is a NUL-terminated string; null otherwise.
    let folder = (result >= 0).then(|| unsafe { from_wide_ptr(path) });
    // SAFETY: freeing null is allowed; otherwise the system's allocation, freed once.
    unsafe { windows_sys::Win32::System::Com::CoTaskMemFree(path.cast()) };
    folder.filter(|f| !f.is_empty()).map(PathBuf::from)
}

fn this_program() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| e.to_string())
}

/// Two paths name the same file, as Windows compares names (ignoring case).
fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

/// `"text"` around a path, as a command line wants it.
fn quoted(path: &Path) -> OsString {
    let mut text = OsString::from("\"");
    text.push(path.as_os_str());
    text.push("\"");
    text
}

// ------------------------------------------------------------------ registry

fn wide_os(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

/// Set a string value (`name` `None`: the key's default value), creating the key.
fn registry_set(key: &str, name: Option<&str>, value: &OsStr) -> Result<(), String> {
    let key_name = wide(key);
    let mut handle: HKEY = std::ptr::null_mut();
    // SAFETY: `key_name` is NUL-terminated; the opened key is closed below.
    let result = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key_name.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            std::ptr::null(),
            &mut handle,
            std::ptr::null_mut(),
        )
    };
    if result != ERROR_SUCCESS {
        return Err(system_message(result));
    }
    let name = name.map(wide);
    let data = wide_os(value);
    // SAFETY: `data` holds the string with its NUL, its length given in bytes.
    let result = unsafe {
        RegSetValueExW(
            handle,
            name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
            0,
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        )
    };
    // SAFETY: opened above, closed once.
    unsafe { RegCloseKey(handle) };
    if result != ERROR_SUCCESS {
        return Err(system_message(result));
    }
    Ok(())
}

/// A value's bytes, of the type `flags` asks for.
fn registry_get(key: &str, name: Option<&str>, flags: u32) -> Option<Vec<u8>> {
    let key_name = wide(key);
    let name = name.map(wide);
    let name = name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr());
    let mut size = 0u32;
    // SAFETY: a null buffer asks for the size.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key_name.as_ptr(),
            name,
            flags,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if result != ERROR_SUCCESS {
        return None;
    }
    let mut data = vec![0u8; size as usize + 2];
    let mut size = data.len() as u32;
    // SAFETY: `data` holds `size` bytes.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key_name.as_ptr(),
            name,
            flags,
            std::ptr::null_mut(),
            data.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if result != ERROR_SUCCESS {
        return None;
    }
    data.truncate(size as usize);
    Some(data)
}

fn registry_get_string(key: &str, name: Option<&str>) -> Option<String> {
    let bytes = registry_get(key, name, RRF_RT_REG_SZ)?;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    Some(from_wide(&units))
}

/// Delete a key with everything under it; a key that is not there is fine.
fn registry_delete_tree(key: &str) -> Result<(), String> {
    let key_name = wide(key);
    // SAFETY: `key_name` is NUL-terminated.
    match unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key_name.as_ptr()) } {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
        error => Err(system_message(error)),
    }
}

// ------------------------------------------------------------------ shortcuts

/// What a shortcut starts: its target and arguments (`IShellLinkW`).
#[derive(Debug)]
struct Shortcut {
    target: PathBuf,
    /// Upstream compares the target only; the tests check what was written.
    #[cfg_attr(not(test), allow(dead_code))]
    arguments: String,
}

/// Read a shortcut file, when there is one.
fn read_shortcut(link: &Path) -> Option<Shortcut> {
    if !link.is_file() {
        return None;
    }
    let link = link.to_owned();
    on_com_thread(move || shell_link::read(&link)).flatten()
}

/// `ShortcutHelpers.SetShortcut`: write the shortcut afresh, or delete it.
fn set_shortcut(link: &Path, target: &Path, arguments: &str, create: bool) -> Result<(), String> {
    match std::fs::remove_file(link) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    if !create {
        return Ok(());
    }
    if !target.is_file() {
        return Err(system_message(ERROR_FILE_NOT_FOUND));
    }
    let (link, target, arguments) = (link.to_owned(), target.to_owned(), arguments.to_owned());
    on_com_thread(move || shell_link::write(&link, &target, &arguments))
        .unwrap_or_else(|| Err("the shell did not answer".into()))
}

/// `ShortcutHelpers.CheckShortcut`: the shortcut starts `target`.
fn shortcut_starts(link: &Path, target: &Path) -> bool {
    read_shortcut(link).is_some_and(|shortcut| same_path(&shortcut.target, target))
}

mod shell_link {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::{Path, PathBuf};

    use winrt::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READ,
    };
    use winrt::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use winrt::core::{Interface, PCWSTR};

    use super::Shortcut;
    use crate::wide::{from_wide, wide};

    fn message(error: winrt::core::Error) -> String {
        let text = error.message();
        if text.is_empty() {
            format!("{error}")
        } else {
            text
        }
    }

    /// Call on a thread with COM initialised.
    pub fn read(link: &Path) -> Option<Shortcut> {
        let name = wide(link);
        // SAFETY: COM is initialised on this thread (`on_com_thread`); every string
        // passed is NUL-terminated and outlives its call; the buffers are sized.
        unsafe {
            let shell_link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
            let file: IPersistFile = shell_link.cast().ok()?;
            file.Load(PCWSTR(name.as_ptr()), STGM_READ).ok()?;
            let mut target = vec![0u16; 32_768];
            shell_link
                .GetPath(&mut target, std::ptr::null_mut(), 0)
                .ok()?;
            let mut arguments = vec![0u16; 32_768];
            let arguments = match shell_link.GetArguments(&mut arguments) {
                Ok(()) => from_wide(&arguments),
                Err(_) => String::new(),
            };
            let end = target.iter().position(|&c| c == 0).unwrap_or(target.len());
            let target = OsString::from_wide(&target[..end]);
            (!target.is_empty()).then(|| Shortcut {
                target: PathBuf::from(target),
                arguments,
            })
        }
    }

    /// Call on a thread with COM initialised.
    pub fn write(link: &Path, target: &Path, arguments: &str) -> Result<(), String> {
        let (link, path, arguments) = (wide(link), wide(target), wide(arguments));
        let folder = target.parent().map(wide);
        // SAFETY: as in `read`.
        unsafe {
            let shell_link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(message)?;
            shell_link.SetPath(PCWSTR(path.as_ptr())).map_err(message)?;
            shell_link
                .SetArguments(PCWSTR(arguments.as_ptr()))
                .map_err(message)?;
            if let Some(folder) = &folder {
                shell_link
                    .SetWorkingDirectory(PCWSTR(folder.as_ptr()))
                    .map_err(message)?;
            }
            let file: IPersistFile = shell_link.cast().map_err(message)?;
            file.Save(PCWSTR(link.as_ptr()), true).map_err(message)
        }
    }
}

// ------------------------------------------------------------------ start with Windows

fn startup_state_in(startup_folder: &Path, software: &str, exe: &Path) -> StartupState {
    if !shortcut_starts(&startup_folder.join(SHORTCUT_NAME), exe) {
        return StartupState::Disabled;
    }
    let approved = format!(r"{software}\{STARTUP_APPROVED}");
    match registry_get(&approved, Some(SHORTCUT_NAME), RRF_RT_REG_BINARY) {
        Some(status) if status.first() == Some(&3) => StartupState::DisabledByUser,
        _ => StartupState::Enabled,
    }
}

/// `StartupManager.State` (read): whether the Startup folder's `Modlight.lnk` starts
/// this copy, and whether Task Manager switched it off.
pub fn startup_state() -> StartupState {
    if !in_app_bundle() {
        return StartupState::NotAvailable;
    }
    match (known_folder(&FOLDERID_Startup), this_program()) {
        (Some(folder), Ok(exe)) => startup_state_in(&folder, SOFTWARE, &exe),
        _ => StartupState::NotAvailable,
    }
}

/// `StartupManager.State` (write): the Startup folder shortcut to this copy, started
/// with `-silent`, written or deleted.
pub fn set_startup(enabled: bool) -> Result<StartupState, String> {
    if !in_app_bundle() {
        return Err("Modlight is not running from an installed copy".into());
    }
    let folder = known_folder(&FOLDERID_Startup).ok_or("the Startup folder is not known")?;
    set_shortcut(
        &folder.join(SHORTCUT_NAME),
        &this_program()?,
        STARTUP_ARGUMENTS,
        enabled,
    )?;
    Ok(startup_state())
}

/// `StartupManager.OtherCopyStartupTargetPath`: the other copy of Modlight the Startup
/// shortcut starts, when it starts one that is not this. "Off" for this copy then still
/// means Windows starts Modlight at sign-in, and the settings can say which.
pub fn startup_other_copy() -> Option<PathBuf> {
    let folder = known_folder(&FOLDERID_Startup)?;
    let target = read_shortcut(&folder.join(SHORTCUT_NAME))?.target;
    let exe = this_program().ok()?;
    (!same_path(&target, &exe)).then_some(target)
}

/// Open Settings at Startup apps.
pub fn open_login_items_settings() {
    shell_open(OsString::from("ms-settings:startupapps"), None);
}

// ------------------------------------------------------------------ files for the window

/// Files handed over by Explorer (the context menu, "Send to", a second start of the
/// program with files on its command line), waiting for the window.
static RECEIVED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
/// A later start of the program asked for the windows to come forward.
static ACTIVATE: AtomicBool = AtomicBool::new(false);
/// What wakes the window when something arrives from another start.
#[allow(clippy::type_complexity)]
static WAKER: Mutex<Option<Box<dyn Fn() + Send>>> = Mutex::new(None);

/// The files received since the last call.
pub fn take_service_files() -> Vec<PathBuf> {
    std::mem::take(&mut *RECEIVED.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Whether a later start of the program asked for the windows to come forward since the
/// last call (upstream's `ForceActivate` on a start without arguments).
pub fn take_activation_request() -> bool {
    ACTIVATE.swap(false, Ordering::SeqCst)
}

/// What to call when files or an activation request arrive from another start, so the
/// window looks at once (`ctx.request_repaint()`); the pipe is read on a thread of its
/// own.
pub fn set_waker(waker: impl Fn() + Send + 'static) {
    *WAKER.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(waker));
}

fn wake() {
    if let Some(waker) = WAKER.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        waker();
    }
}

/// A file or folder that may be queued: absolute, an ordinary drive or network path
/// (not a device), and there.
fn acceptable(path: &Path) -> bool {
    let text: Vec<u16> = path.as_os_str().encode_wide().collect();
    let device = [r"\\.\", r"\\?\", r"\??\"]
        .iter()
        .any(|prefix| text.starts_with(&prefix.encode_utf16().collect::<Vec<_>>()));
    path.is_absolute() && !device && (path.is_file() || path.is_dir())
}

/// Queue files for the window (the first instance's own command line). A relative path
/// is taken from the working folder; only files and folders that exist are queued.
pub fn receive_files(files: Vec<PathBuf>) {
    let files: Vec<PathBuf> = files
        .into_iter()
        .filter_map(|file| std::path::absolute(file).ok())
        .filter(|file| acceptable(file))
        .collect();
    if !files.is_empty() {
        RECEIVED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(files);
        wake();
    }
}

/// What arrives from another start, checked.
fn deliver(request: Request) {
    match request {
        Request::Activate => {
            log::info!("Another start of Modlight asked for the window");
            ACTIVATE.store(true, Ordering::SeqCst);
        }
        Request::Files(files) => {
            log::info!(
                "Another start of Modlight handed over {} file(s)",
                files.len()
            );
            RECEIVED
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend(files);
        }
    }
    wake();
}

/// Start listening for files from later starts of the program. Normal mode only, once,
/// on the main thread: this start becomes the one that runs (it takes the mutex if
/// [`forward_to_running_instance`] or [`activate_running_instance`] has not already).
pub fn install_services_provider() {
    match claim_first_instance() {
        Claim::First => {}
        Claim::Other => {
            log::warn!("Another Modlight is running for this user; later starts go to it")
        }
        Claim::Unknown => log::warn!("Modlight cannot tell whether another copy is running"),
    }
}

/// Windows asks no permission to read the screen; there is no such place to open.
pub fn open_screen_recording_settings() -> bool {
    false
}

/// Kept for the macOS API: there is no system page for the Explorer menu.
pub fn open_services_settings() -> bool {
    false
}

// ------------------------------------------------------------------ Explorer menu

/// Upstream's `ShellExtPath`: `"<exe>" "%1"`.
fn menu_command(exe: &Path) -> OsString {
    let mut command = quoted(exe);
    command.push(" \"%1\"");
    command
}

/// Upstream's `ShellExtIcon`: `"<exe>",0`.
fn menu_icon(exe: &Path) -> OsString {
    let mut icon = quoted(exe);
    icon.push(",0");
    icon
}

fn context_menu_enabled_in(software: &str, exe: &Path) -> bool {
    let expected = menu_command(exe).to_string_lossy().to_lowercase();
    MENU_KEYS.iter().all(|key| {
        registry_get_string(&format!(r"{software}\Classes\{key}\command"), None)
            .is_some_and(|command| command.to_lowercase() == expected)
    })
}

fn set_context_menu_in(
    software: &str,
    exe: &Path,
    text: &str,
    enabled: bool,
) -> Result<(), String> {
    for key in MENU_KEYS {
        registry_delete_tree(&format!(r"{software}\Classes\{key}"))?;
    }
    if !enabled {
        return Ok(());
    }
    for key in MENU_KEYS {
        let key = format!(r"{software}\Classes\{key}");
        registry_set(&key, None, OsStr::new(text))?;
        registry_set(&key, Some("Icon"), &menu_icon(exe))?;
        registry_set(&format!(r"{key}\command"), None, &menu_command(exe))?;
    }
    Ok(())
}

/// `ShellContextMenu`: whether "Upload with Modlight" is in Explorer's context menu of
/// files and folders, starting this copy (`CheckShellContextMenuButton`).
pub fn context_menu_enabled() -> bool {
    in_app_bundle() && this_program().is_ok_and(|exe| context_menu_enabled_in(SOFTWARE, &exe))
}

/// Add or remove "Upload with Modlight" in Explorer's context menu of files and folders
/// (`HKCU\Software\Classes\*\shell\Modlight` and `...\Directory\shell\Modlight`, each
/// with this program's icon and `"<exe>" "%1"`). The entry's text is the app's
/// "Upload with Modlight" in the language in use when it is written.
pub fn set_context_menu(enabled: bool) -> Result<(), String> {
    if !in_app_bundle() {
        return Err("Modlight is not running from an installed copy".into());
    }
    use modlight_core::l10n::{s, t};
    set_context_menu_in(
        SOFTWARE,
        &this_program()?,
        t(s::IntegrationHelpers_UploadWithShareX),
        enabled,
    )
}

/// `SendTo`: whether Modlight is in Explorer's "Send to" menu, starting this copy.
pub fn send_to_enabled() -> bool {
    in_app_bundle()
        && match (known_folder(&FOLDERID_SendTo), this_program()) {
            (Some(folder), Ok(exe)) => shortcut_starts(&folder.join(SHORTCUT_NAME), &exe),
            _ => false,
        }
}

/// Add or remove Modlight in Explorer's "Send to" menu: `Modlight.lnk` to this program in
/// the user's SendTo folder (Explorer hands it the chosen files as arguments).
pub fn set_send_to(enabled: bool) -> Result<(), String> {
    if !in_app_bundle() {
        return Err("Modlight is not running from an installed copy".into());
    }
    let folder = known_folder(&FOLDERID_SendTo).ok_or("the SendTo folder is not known")?;
    set_shortcut(&folder.join(SHORTCUT_NAME), &this_program()?, "", enabled)
}

// ------------------------------------------------------------------ one running copy

/// Hand `files` to a Modlight already running for this user. `true` when this start has
/// nothing left to do and exits: another copy runs (it took the files, or did not answer
/// in time). `false` when this start is the first: it now owns the mutex and takes later
/// starts' files itself. With no files, it only asks whether another copy runs (a
/// `-silent` start).
pub fn forward_to_running_instance(files: &[PathBuf]) -> bool {
    hand_over(claim_first_instance(), || {
        let files: Vec<PathBuf> = files
            .iter()
            .filter_map(|file| std::path::absolute(file).ok())
            .collect();
        send_to_first(&Request::Files(files))
    })
}

/// Bring the windows of a Modlight already running forward. `true` when another copy
/// runs (this start then exits); `false` when this start is the first (see
/// [`forward_to_running_instance`]).
pub fn activate_running_instance() -> bool {
    hand_over(claim_first_instance(), || send_to_first(&Request::Activate))
}

/// `SingleInstanceManager`: whether this start ends after `send` handed its request
/// over. The first start runs. A later one ends whether or not the first answered, as
/// upstream's (`RedirectArgumentsToFirstInstance` swallows a failure, and `Main` runs
/// only in the first instance): two copies would share the settings, the upload queue
/// and the history. A start that could not read the mutex at all (`Claim::Unknown`)
/// runs when nobody answers.
fn hand_over(claim: Claim, send: impl FnOnce() -> bool) -> bool {
    match claim {
        Claim::First => false,
        Claim::Other => {
            if !send() {
                log::warn!("Modlight is already running for this user but did not answer");
            }
            true
        }
        Claim::Unknown => send(),
    }
}

/// What one start says to the first.
#[derive(Clone, Debug, PartialEq)]
enum Request {
    Activate,
    Files(Vec<PathBuf>),
}

const MAGIC: &[u8; 4] = b"MLP1";
const KIND_ACTIVATE: u32 = 1;
const KIND_FILES: u32 = 2;
/// The largest message taken (paths are UTF-16, so about half a million characters).
const MAX_MESSAGE: usize = 1024 * 1024;
const MAX_FILES: usize = 10_000;
/// How long a start waits for the first one's pipe (upstream's `ConnectTimeout`).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long either side waits for the other to read or write.
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// The message: `MLP1`, the kind and the payload's length (little-endian u32 each), then
/// the paths as UTF-16LE, each ending with a NUL (no path can hold one).
fn encode(request: &Request) -> Vec<u8> {
    let (kind, payload): (u32, Vec<u8>) = match request {
        Request::Activate => (KIND_ACTIVATE, Vec::new()),
        Request::Files(files) => (
            KIND_FILES,
            files
                .iter()
                .flat_map(|file| file.as_os_str().encode_wide().chain(std::iter::once(0)))
                .flat_map(u16::to_le_bytes)
                .collect(),
        ),
    };
    let mut message = Vec::with_capacity(12 + payload.len());
    message.extend_from_slice(MAGIC);
    message.extend_from_slice(&kind.to_le_bytes());
    message.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    message.extend_from_slice(&payload);
    message
}

/// The kind and payload length of a message header, when it is one.
fn decode_header(header: &[u8; 12]) -> Option<(u32, usize)> {
    if &header[..4] != MAGIC {
        return None;
    }
    let kind = u32::from_le_bytes(header[4..8].try_into().ok()?);
    let length = u32::from_le_bytes(header[8..12].try_into().ok()?) as usize;
    let valid = match kind {
        KIND_ACTIVATE => length == 0,
        KIND_FILES => length <= MAX_MESSAGE && length.is_multiple_of(2),
        _ => false,
    };
    valid.then_some((kind, length))
}

/// The request in a payload; files that may not be queued are left out.
fn decode_payload(kind: u32, payload: &[u8]) -> Option<Request> {
    if kind == KIND_ACTIVATE {
        return Some(Request::Activate);
    }
    let units: Vec<u16> = payload
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    if units.last().is_some_and(|&c| c != 0) {
        return None;
    }
    let mut files = Vec::new();
    for name in units.split(|&c| c == 0).filter(|name| !name.is_empty()) {
        if files.len() == MAX_FILES {
            return None;
        }
        files.push(PathBuf::from(OsString::from_wide(name)));
    }
    files.retain(|file| acceptable(file));
    Some(Request::Files(files))
}

/// This user, from the process token.
struct User {
    /// The `TOKEN_USER` the SID points into (u64 for its alignment).
    token_user: Vec<u64>,
    session: u32,
}

impl User {
    fn current() -> Option<User> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: opens this process's token for reading; closed below.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return None;
        }
        // SAFETY: `token` was just opened and is owned here.
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let raw = token.as_raw_handle();
        let mut length = 0u32;
        // SAFETY: a null buffer asks for the length.
        unsafe { GetTokenInformation(raw, TokenUser, std::ptr::null_mut(), 0, &mut length) };
        if length == 0 {
            return None;
        }
        let mut token_user = vec![0u64; (length as usize).div_ceil(8)];
        // SAFETY: `token_user` holds at least `length` bytes, aligned for TOKEN_USER.
        if unsafe {
            GetTokenInformation(
                raw,
                TokenUser,
                token_user.as_mut_ptr().cast(),
                length,
                &mut length,
            )
        } == 0
        {
            return None;
        }
        let mut session = 0u32;
        let mut written = 0u32;
        // SAFETY: TokenSessionId writes one u32.
        if unsafe {
            GetTokenInformation(
                raw,
                TokenSessionId,
                (&mut session as *mut u32).cast(),
                4,
                &mut written,
            )
        } == 0
        {
            return None;
        }
        Some(User {
            token_user,
            session,
        })
    }

    fn sid(&self) -> PSID {
        // SAFETY: `token_user` starts with the TOKEN_USER the system wrote.
        unsafe { (*self.token_user.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }

    /// The SID as text (`S-1-5-21-...`).
    fn sid_text(&self) -> Option<String> {
        let mut text: *mut u16 = std::ptr::null_mut();
        // SAFETY: the SID is valid while `self` lives; the text is freed below.
        if unsafe { ConvertSidToStringSidW(self.sid(), &mut text) } == 0 {
            return None;
        }
        // SAFETY: a NUL-terminated string the system allocated.
        let sid = unsafe { from_wide_ptr(text) };
        // SAFETY: allocated by ConvertSidToStringSidW, freed once.
        unsafe { LocalFree(text.cast()) };
        Some(sid)
    }

    fn mutex_name(&self) -> Option<String> {
        Some(format!(r"Local\Modlight-{}", self.sid_text()?))
    }

    fn pipe_name(&self) -> Option<String> {
        Some(format!(
            r"\\.\pipe\Modlight-{}-{}",
            self.sid_text()?,
            self.session
        ))
    }
}

/// Whether this start is the first for this user and session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Claim {
    First,
    Other,
    /// The mutex could not be made (its name taken by an object somebody else made):
    /// the pipe is tried, and with no answer this start runs.
    Unknown,
}

/// The mutex while this start owns it: until the process ends.
static FIRST: Mutex<Option<OwnedHandle>> = Mutex::new(None);

fn claim_first_instance() -> Claim {
    let mut first = FIRST.lock().unwrap_or_else(|e| e.into_inner());
    if first.is_some() {
        return Claim::First;
    }
    let Some(name) = User::current().and_then(|user| user.mutex_name()) else {
        return Claim::Unknown;
    };
    match create_mutex(&name) {
        Some((handle, false)) => {
            *first = Some(handle);
            drop(first);
            start_server();
            Claim::First
        }
        Some((_, true)) => Claim::Other,
        None => Claim::Unknown,
    }
}

/// A named mutex, and whether it existed before.
fn create_mutex(name: &str) -> Option<(OwnedHandle, bool)> {
    let name = wide(name);
    // SAFETY: `name` is NUL-terminated; the handle is owned by the caller.
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    // SAFETY: reads the calling thread's last error code, right after the call.
    let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if handle.is_null() {
        return None;
    }
    // SAFETY: a fresh handle owned here.
    Some((unsafe { OwnedHandle::from_raw_handle(handle) }, existed))
}

/// The pipe server of this start, once.
fn start_server() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        let Some(user) = User::current() else {
            log::warn!("Modlight cannot listen for later starts: no user");
            return;
        };
        let Some(name) = user.pipe_name() else {
            return;
        };
        let started = std::thread::Builder::new()
            .name("modlight-instance".into())
            .spawn(move || {
                let stop = AtomicBool::new(false);
                if let Err(error) = serve(&name, &user, &stop, &deliver) {
                    log::warn!("Modlight does not listen for later starts: {error}");
                }
            });
        if let Err(error) = started {
            log::warn!("Modlight cannot listen for later starts: {error}");
        }
    });
}

fn send_to_first(request: &Request) -> bool {
    let Some(user) = User::current() else {
        return false;
    };
    let Some(name) = user.pipe_name() else {
        return false;
    };
    let message = encode(request);
    if message.len() > 12 + MAX_MESSAGE {
        log::warn!("Too many files to hand to the running Modlight at once");
        return false;
    }
    send(&name, &user, &message, CONNECT_TIMEOUT)
}

/// A manual-reset event for overlapped I/O.
fn new_event() -> Option<OwnedHandle> {
    // SAFETY: an unnamed event; the handle is owned by the caller.
    let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
    // SAFETY: a fresh handle owned here.
    (!event.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(event) })
}

/// One overlapped operation on `handle`, waited for up to `timeout` (`None`: until
/// `stop` is set, looked at five times a second). On a timeout or a stop the operation
/// is cancelled and its end awaited, so the OVERLAPPED is never left in use.
fn overlapped(
    handle: HANDLE,
    event: &OwnedHandle,
    timeout: Option<Duration>,
    stop: Option<&AtomicBool>,
    start: impl FnOnce(*mut OVERLAPPED) -> i32,
) -> Result<u32, u32> {
    // SAFETY: an all-zero OVERLAPPED with our event is what overlapped I/O takes.
    let mut operation: OVERLAPPED = unsafe { std::mem::zeroed() };
    operation.hEvent = event.as_raw_handle();
    if start(&mut operation) == 0 {
        // SAFETY: reads the calling thread's last error code, right after the call.
        let error = unsafe { GetLastError() };
        if error != ERROR_IO_PENDING {
            return Err(error);
        }
        let deadline = timeout.map(|t| Instant::now() + t);
        loop {
            let slice = match deadline {
                Some(deadline) => deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(200)),
                None => Duration::from_millis(200),
            };
            // SAFETY: waits on our own event.
            if unsafe { WaitForSingleObject(operation.hEvent, slice.as_millis() as u32) }
                == WAIT_OBJECT_0
            {
                break;
            }
            let expired = deadline.is_some_and(|d| Instant::now() >= d);
            let stopped = stop.is_some_and(|s| s.load(Ordering::SeqCst));
            if expired || stopped {
                let mut ignored = 0u32;
                // SAFETY: cancels this operation, then waits for it to end so that
                // `operation` is no longer used by the system when it goes away.
                unsafe {
                    CancelIoEx(handle, &operation);
                    GetOverlappedResult(handle, &operation, &mut ignored, 1);
                }
                return Err(ERROR_TIMEOUT);
            }
        }
    }
    let mut transferred = 0u32;
    // SAFETY: the operation has ended; this reads its result.
    if unsafe { GetOverlappedResult(handle, &operation, &mut transferred, 0) } == 0 {
        // SAFETY: reads the calling thread's last error code.
        return Err(unsafe { GetLastError() });
    }
    Ok(transferred)
}

fn read_exact(handle: HANDLE, event: &OwnedHandle, buffer: &mut [u8], deadline: Instant) -> bool {
    let mut done = 0;
    while done < buffer.len() {
        let left = deadline.saturating_duration_since(Instant::now());
        let chunk = &mut buffer[done..];
        let read = overlapped(handle, event, Some(left), None, |operation| {
            // SAFETY: `chunk` stays alive and untouched until the operation has ended.
            unsafe {
                ReadFile(
                    handle,
                    chunk.as_mut_ptr(),
                    chunk.len().min(u32::MAX as usize) as u32,
                    std::ptr::null_mut(),
                    operation,
                )
            }
        });
        match read {
            Ok(0) | Err(_) => return false,
            Ok(count) => done += count as usize,
        }
    }
    true
}

fn write_all(handle: HANDLE, event: &OwnedHandle, buffer: &[u8], deadline: Instant) -> bool {
    let mut done = 0;
    while done < buffer.len() {
        let left = deadline.saturating_duration_since(Instant::now());
        let chunk = &buffer[done..];
        let written = overlapped(handle, event, Some(left), None, |operation| {
            // SAFETY: `chunk` stays alive until the operation has ended.
            unsafe {
                WriteFile(
                    handle,
                    chunk.as_ptr(),
                    chunk.len().min(u32::MAX as usize) as u32,
                    std::ptr::null_mut(),
                    operation,
                )
            }
        });
        match written {
            Ok(0) | Err(_) => return false,
            Ok(count) => done += count as usize,
        }
    }
    true
}

/// Whether the object behind `handle` is owned by `user`.
fn owned_by(handle: HANDLE, user: &User) -> bool {
    let mut owner: PSID = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: asks for the owner only; the descriptor it points into is freed below.
    let result = unsafe {
        GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if result != ERROR_SUCCESS {
        return false;
    }
    // SAFETY: both SIDs are valid here.
    let same = !owner.is_null() && unsafe { EqualSid(owner, user.sid()) } != 0;
    // SAFETY: allocated by GetSecurityInfo, freed once.
    unsafe { LocalFree(descriptor) };
    same
}

/// Send `message` to the pipe `name` and wait for its answer. The pipe must be owned by
/// `user`; it may identify this process but not act as it (`SECURITY_IDENTIFICATION`).
fn send(name: &str, user: &User, message: &[u8], connect_timeout: Duration) -> bool {
    let pipe_name = wide(name);
    let deadline = Instant::now() + connect_timeout;
    let pipe = loop {
        // SAFETY: `pipe_name` is NUL-terminated; the handle is owned below.
        let handle = unsafe {
            CreateFileW(
                pipe_name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                std::ptr::null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE {
            // SAFETY: a fresh handle owned here.
            break unsafe { OwnedHandle::from_raw_handle(handle) };
        }
        // SAFETY: reads the calling thread's last error code, right after the call.
        let error = unsafe { GetLastError() };
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        match error {
            // The first start answers another one; wait for its turn.
            // SAFETY: `pipe_name` is NUL-terminated.
            ERROR_PIPE_BUSY => unsafe {
                WaitNamedPipeW(pipe_name.as_ptr(), left.as_millis().max(1) as u32);
            },
            // The first start is still starting.
            ERROR_FILE_NOT_FOUND => std::thread::sleep(left.min(Duration::from_millis(50))),
            _ => return false,
        }
    };
    let handle = pipe.as_raw_handle();
    if !owned_by(handle, user) {
        log::warn!("The pipe of the running Modlight is not this user's; nothing was sent");
        return false;
    }
    let Some(event) = new_event() else {
        return false;
    };
    let deadline = Instant::now() + IO_TIMEOUT;
    let mut answer = [0u8; 1];
    write_all(handle, &event, message, deadline)
        && read_exact(handle, &event, &mut answer, deadline)
        && answer[0] == 1
}

/// The first instance's side: answer one start after another on the pipe `name` until
/// `stop` is set, handing what arrives to `deliver`. A pipe of this name that already
/// exists (another copy's, or somebody else's) is an error: this one never shares it.
fn serve(
    name: &str,
    user: &User,
    stop: &AtomicBool,
    deliver: &dyn Fn(Request),
) -> Result<(), String> {
    let sid = user.sid_text().ok_or("no user SID")?;
    let pipe = create_pipe(name, &sid)?;
    let handle = pipe.as_raw_handle();
    let event = new_event().ok_or_else(|| system_message(unsafe { GetLastError() }))?;
    while !stop.load(Ordering::SeqCst) {
        let connected = overlapped(handle, &event, None, Some(stop), |operation| {
            // SAFETY: `operation` lives until the connection is made or cancelled.
            unsafe { ConnectNamedPipe(handle, operation) }
        });
        match connected {
            Ok(_) | Err(ERROR_PIPE_CONNECTED) => answer(handle, &event, deliver),
            Err(ERROR_TIMEOUT) => break,
            Err(error) => {
                log::warn!("Modlight's pipe: {}", system_message(error));
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        // SAFETY: ends this client's connection; the pipe waits for the next one.
        unsafe { DisconnectNamedPipe(handle) };
    }
    Ok(())
}

/// The pipe, local clients only, this user only (`O:` and the one ACE are the user's).
fn create_pipe(name: &str, sid: &str) -> Result<OwnedHandle, String> {
    let sddl = wide(format!("O:{sid}D:P(A;;GA;;;{sid})"));
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `sddl` is NUL-terminated; the descriptor is freed below.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(crate::system::last_error_message());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let pipe_name = wide(name);
    // SAFETY: every pointer is valid for the call; one instance, reused for each client.
    let pipe = unsafe {
        CreateNamedPipeW(
            pipe_name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            64 * 1024,
            64 * 1024,
            0,
            &attributes,
        )
    };
    // SAFETY: reads the calling thread's last error code, right after the call.
    let error = unsafe { GetLastError() };
    // SAFETY: allocated by the conversion above, freed once; the pipe keeps its copy.
    unsafe { LocalFree(descriptor) };
    if pipe == INVALID_HANDLE_VALUE {
        return Err(system_message(error));
    }
    // SAFETY: a fresh handle owned here.
    Ok(unsafe { OwnedHandle::from_raw_handle(pipe) })
}

/// One client: read its message, answer 1 when it was a message of ours (0 otherwise),
/// and give it a moment to read the answer before the connection ends (ending it drops
/// what was not read yet).
fn answer(handle: HANDLE, event: &OwnedHandle, deliver: &dyn Fn(Request)) {
    let deadline = Instant::now() + IO_TIMEOUT;
    let mut header = [0u8; 12];
    if !read_exact(handle, event, &mut header, deadline) {
        return;
    }
    let request = decode_header(&header).and_then(|(kind, length)| {
        let mut payload = vec![0u8; length];
        read_exact(handle, event, &mut payload, deadline)
            .then(|| decode_payload(kind, &payload))
            .flatten()
    });
    let taken = request.is_some();
    if let Some(request) = request {
        deliver(request);
    }
    if write_all(handle, event, &[taken as u8], deadline) {
        // The client closes once it has the answer; this read ends then.
        let mut rest = [0u8; 1];
        let _ = read_exact(
            handle,
            event,
            &mut rest,
            Instant::now() + Duration::from_secs(1),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::mpsc;

    fn unique(prefix: &str) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{prefix}-{}-{nanos}", std::process::id())
    }

    /// A throwaway `HKCU\Software\ModlightTest-...` that is deleted with all it holds.
    struct ThrowawayKey(String);

    impl ThrowawayKey {
        fn new() -> ThrowawayKey {
            ThrowawayKey(format!(r"Software\{}", unique("ModlightTest")))
        }
    }

    impl Drop for ThrowawayKey {
        fn drop(&mut self) {
            use windows_sys::Win32::System::Registry::{KEY_READ, RegOpenKeyExW};
            registry_delete_tree(&self.0).unwrap();
            let name = wide(&self.0);
            let mut handle: HKEY = std::ptr::null_mut();
            // SAFETY: `name` is NUL-terminated; a key that opened is closed again.
            let opened = unsafe {
                RegOpenKeyExW(HKEY_CURRENT_USER, name.as_ptr(), 0, KEY_READ, &mut handle)
            };
            if opened == ERROR_SUCCESS {
                unsafe { RegCloseKey(handle) };
            }
            assert_eq!(opened, ERROR_FILE_NOT_FOUND, "the test key is gone");
        }
    }

    #[test]
    fn a_test_binary_is_not_an_installed_copy() {
        assert!(!in_app_bundle());
        assert_eq!(startup_state(), StartupState::NotAvailable);
        assert!(set_startup(true).is_err());
        assert!(set_context_menu(true).is_err());
        assert!(set_send_to(true).is_err());
        assert!(!context_menu_enabled());
        assert!(!send_to_enabled());
        assert!(!open_services_settings());
    }

    #[test]
    fn development_builds_are_recognised() {
        assert!(is_development_build(Path::new(
            r"D:\WORK\Modlight\app\target\release\Modlight.exe"
        )));
        assert!(is_development_build(Path::new(
            r"D:\x\target\x86_64-pc-windows-msvc\debug\deps\modlight.exe"
        )));
        assert!(!is_development_build(Path::new(
            r"C:\Program Files\Modlight\Modlight.exe"
        )));
        assert!(!is_development_build(Path::new(
            r"C:\Users\x\AppData\Local\Programs\Modlight\Modlight.exe"
        )));
    }

    #[test]
    fn the_context_menu_is_upstreams_keys() {
        let root = ThrowawayKey::new();
        let exe = Path::new(r"C:\Program Files\Modlight\Modlight.exe");
        assert!(!context_menu_enabled_in(&root.0, exe));
        set_context_menu_in(&root.0, exe, "Prześlij za pomocą Modlight", true).unwrap();
        assert!(context_menu_enabled_in(&root.0, exe));
        for key in MENU_KEYS {
            let key = format!(r"{}\Classes\{key}", root.0);
            assert_eq!(
                registry_get_string(&key, None).as_deref(),
                Some("Prześlij za pomocą Modlight")
            );
            assert_eq!(
                registry_get_string(&key, Some("Icon")).as_deref(),
                Some(r#""C:\Program Files\Modlight\Modlight.exe",0"#)
            );
            assert_eq!(
                registry_get_string(&format!(r"{key}\command"), None).as_deref(),
                Some(r#""C:\Program Files\Modlight\Modlight.exe" "%1""#)
            );
        }
        // Another copy's entry is not this copy's.
        assert!(!context_menu_enabled_in(
            &root.0,
            Path::new(r"D:\Other\Modlight.exe")
        ));
        set_context_menu_in(&root.0, exe, "Upload with Modlight", false).unwrap();
        assert!(!context_menu_enabled_in(&root.0, exe));
        assert!(
            registry_get_string(&format!(r"{}\Classes\{}", root.0, MENU_KEYS[0]), None).is_none()
        );
    }

    #[test]
    fn the_startup_shortcut_and_task_managers_switch() {
        let root = ThrowawayKey::new();
        let folder = tempfile::tempdir().unwrap();
        let programs = tempfile::tempdir().unwrap();
        let exe = programs.path().join("Modlight.exe");
        let other = programs.path().join("Other copy.exe");
        std::fs::write(&exe, b"not a program").unwrap();
        std::fs::write(&other, b"not a program").unwrap();
        let link = folder.path().join(SHORTCUT_NAME);

        assert_eq!(
            startup_state_in(folder.path(), &root.0, &exe),
            StartupState::Disabled
        );
        set_shortcut(&link, &exe, STARTUP_ARGUMENTS, true).unwrap();
        let shortcut = read_shortcut(&link).unwrap();
        assert!(same_path(&shortcut.target, &exe), "{shortcut:?}");
        assert_eq!(shortcut.arguments, "-silent");
        assert_eq!(
            startup_state_in(folder.path(), &root.0, &exe),
            StartupState::Enabled
        );

        // Task Manager's Startup apps: 3 is off, 2 is on.
        let approved = format!(r"{}\{STARTUP_APPROVED}", root.0);
        set_binary(
            &approved,
            SHORTCUT_NAME,
            &[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        assert_eq!(
            startup_state_in(folder.path(), &root.0, &exe),
            StartupState::DisabledByUser
        );
        set_binary(
            &approved,
            SHORTCUT_NAME,
            &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        assert_eq!(
            startup_state_in(folder.path(), &root.0, &exe),
            StartupState::Enabled
        );

        // A shortcut to another copy is off for this one.
        set_shortcut(&link, &other, STARTUP_ARGUMENTS, true).unwrap();
        assert_eq!(
            startup_state_in(folder.path(), &root.0, &exe),
            StartupState::Disabled
        );
        set_shortcut(&link, &exe, STARTUP_ARGUMENTS, false).unwrap();
        assert!(!link.exists());
        assert_eq!(
            startup_state_in(folder.path(), &root.0, &exe),
            StartupState::Disabled
        );
        // Deleting what is not there is fine; a missing program is not linked to.
        set_shortcut(&link, &exe, "", false).unwrap();
        assert!(set_shortcut(&link, &programs.path().join("gone.exe"), "", true).is_err());
    }

    #[test]
    fn the_send_to_shortcut() {
        let folder = tempfile::tempdir().unwrap();
        let programs = tempfile::tempdir().unwrap();
        let exe = programs.path().join("Modlight.exe");
        std::fs::write(&exe, b"not a program").unwrap();
        let link = folder.path().join(SHORTCUT_NAME);
        set_shortcut(&link, &exe, "", true).unwrap();
        assert!(shortcut_starts(&link, &exe));
        assert_eq!(read_shortcut(&link).unwrap().arguments, "");
        set_shortcut(&link, &exe, "", false).unwrap();
        assert!(!shortcut_starts(&link, &exe));
    }

    fn set_binary(key: &str, name: &str, value: &[u8]) {
        use windows_sys::Win32::System::Registry::REG_BINARY;
        let key_name = wide(key);
        let mut handle: HKEY = std::ptr::null_mut();
        unsafe {
            assert_eq!(
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    key_name.as_ptr(),
                    0,
                    std::ptr::null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_WRITE,
                    std::ptr::null(),
                    &mut handle,
                    std::ptr::null_mut(),
                ),
                ERROR_SUCCESS
            );
            let name = wide(name);
            assert_eq!(
                RegSetValueExW(
                    handle,
                    name.as_ptr(),
                    0,
                    REG_BINARY,
                    value.as_ptr(),
                    value.len() as u32
                ),
                ERROR_SUCCESS
            );
            RegCloseKey(handle);
        }
    }

    #[test]
    fn messages_carry_any_path_and_refuse_the_rest() {
        let odd = PathBuf::from(OsString::from_wide(&[
            'C' as u16,
            ':' as u16,
            '\\' as u16,
            0xD800,
            'x' as u16,
        ]));
        let request = Request::Files(vec![PathBuf::from(r"C:\Zdjęcie ż.png"), odd.clone()]);
        let message = encode(&request);
        let header: [u8; 12] = message[..12].try_into().unwrap();
        let (kind, length) = decode_header(&header).unwrap();
        assert_eq!(length, message.len() - 12);
        let units: Vec<u16> = message[12..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u16::from_le_bytes(*p))
            .collect();
        let names: Vec<&[u16]> = units.split(|&c| c == 0).filter(|n| !n.is_empty()).collect();
        assert_eq!(names[1], odd.as_os_str().encode_wide().collect::<Vec<_>>());
        // Neither exists, so neither is queued.
        assert_eq!(
            decode_payload(kind, &message[12..]),
            Some(Request::Files(Vec::new()))
        );

        let activate = encode(&Request::Activate);
        assert_eq!(
            decode_header(&activate[..12].try_into().unwrap()),
            Some((KIND_ACTIVATE, 0))
        );
        let mut wrong = activate.clone();
        wrong[0] = b'X';
        assert_eq!(decode_header(&wrong[..12].try_into().unwrap()), None);
        let mut huge = encode(&Request::Files(Vec::new()));
        huge[8..12].copy_from_slice(&(MAX_MESSAGE as u32 + 2).to_le_bytes());
        assert_eq!(decode_header(&huge[..12].try_into().unwrap()), None);
        // A payload that does not end with a NUL is cut short: refused.
        assert_eq!(decode_payload(KIND_FILES, &[b'C', 0]), None);
    }

    #[test]
    fn only_paths_that_exist_are_accepted() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("picture.png");
        std::fs::write(&file, b"x").unwrap();
        assert!(acceptable(&file));
        assert!(acceptable(folder.path()));
        assert!(!acceptable(Path::new("picture.png")));
        assert!(!acceptable(&folder.path().join("missing.png")));
        assert!(!acceptable(Path::new(r"\\.\PhysicalDrive0")));
        assert!(!acceptable(Path::new(r"\\.\pipe\x")));
    }

    #[test]
    fn a_later_start_never_runs_beside_the_first() {
        // The first start runs, and sends nothing.
        let sent = std::cell::Cell::new(false);
        assert!(!hand_over(Claim::First, || sent.replace(true)));
        assert!(!sent.get());
        // A later one ends whether the first took its files or never answered
        // (`RedirectArgumentsToFirstInstance` swallows the failure, and `Main` runs only
        // in the first instance): two copies would share the settings, the upload queue
        // and the history.
        assert!(hand_over(Claim::Other, || true));
        assert!(hand_over(Claim::Other, || false));
        // Only a start that could not read the mutex at all runs when nobody answers.
        assert!(hand_over(Claim::Unknown, || true));
        assert!(!hand_over(Claim::Unknown, || false));
    }

    #[test]
    fn a_second_mutex_of_the_same_name_finds_the_first() {
        let name = format!(r"Local\{}", unique("ModlightTest"));
        let (first, existed) = create_mutex(&name).unwrap();
        assert!(!existed);
        let (_second, existed) = create_mutex(&name).unwrap();
        assert!(existed);
        drop(first);
    }

    #[test]
    fn files_and_activation_go_through_the_pipe() {
        let user = User::current().unwrap();
        let name = format!(r"\\.\pipe\{}", unique("ModlightTest"));
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, received) = mpsc::channel();
        let server = {
            let (name, stop) = (name.clone(), stop.clone());
            std::thread::spawn(move || {
                let user = User::current().unwrap();
                let sender = Mutex::new(sender);
                serve(&name, &user, &stop, &|request| {
                    sender.lock().unwrap().send(request).unwrap();
                })
            })
        };
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("Zdjęcie.png");
        std::fs::write(&file, b"x").unwrap();
        let files = vec![file.clone(), folder.path().join("missing.png")];
        let wait = Duration::from_secs(5);

        // The client waits for the pipe to appear.
        assert!(send(&name, &user, &encode(&Request::Files(files)), wait));
        assert_eq!(
            received.recv_timeout(wait).unwrap(),
            Request::Files(vec![file])
        );
        assert!(send(&name, &user, &encode(&Request::Activate), wait));
        assert_eq!(received.recv_timeout(wait).unwrap(), Request::Activate);
        // Something that is not a message is answered with a no, and the server goes on.
        assert!(!send(&name, &user, b"GET / HTTP/1.1\r\n\r\n", wait));
        assert!(send(&name, &user, &encode(&Request::Activate), wait));
        assert_eq!(received.recv_timeout(wait).unwrap(), Request::Activate);
        // Nobody else may serve the same name while this server does.
        let sid = user.sid_text().unwrap();
        assert!(create_pipe(&name, &sid).is_err());

        stop.store(true, Ordering::SeqCst);
        server.join().unwrap().unwrap();
        // No server: the client gives up when its time is over.
        let started = Instant::now();
        assert!(!send(
            &name,
            &user,
            &encode(&Request::Activate),
            Duration::from_millis(300)
        ));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn own_command_line_files_are_queued_when_they_exist() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        receive_files(vec![file.clone(), folder.path().join("missing.txt")]);
        assert_eq!(take_service_files(), vec![file]);
        assert!(take_service_files().is_empty());
        assert!(!take_activation_request());
    }
}
