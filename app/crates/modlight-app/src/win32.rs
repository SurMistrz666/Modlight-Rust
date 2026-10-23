//! The few things the window asks of Windows itself, beside `modlight-windows`: the
//! parent's console for a GUI program started from one, the taskbar button's progress
//! (`TaskbarManager`), the tray icon's double-click time, whether ShareX is running
//! (`HotkeyProblemViewModel`), starting a downloaded installer (`DownloaderWindow`), and a
//! wake-up for hotkeys that arrive while the window is idle.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Shell::{ITaskbarList3, TBPF_NOPROGRESS, TaskbarList};
use windows::core::PCWSTR;

/// A NUL-terminated UTF-16 copy of `text`.
fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

/// A Windows GUI program has no console of its own. Started from one (`--qa-shot`,
/// `--help`), it writes into its parent's, unless its output already goes somewhere
/// (a pipe or a file).
pub fn attach_parent_console() {
    use windows::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE,
    };
    // SAFETY: plain Win32 calls without pointers.
    unsafe {
        let redirected = GetStdHandle(STD_OUTPUT_HANDLE).is_ok_and(|h| !h.is_invalid());
        if !redirected {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

/// The native handle of the window eframe made, when there is one (none headlessly).
pub fn window_handle(cc: &eframe::CreationContext<'_>) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match cc.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

/// `TaskbarManager`: the average progress on the taskbar button (`SetProgressValue`), or
/// none (`SetProgressState(NoProgress)`), through `ITaskbarList3`.
pub struct TaskbarProgress {
    hwnd: isize,
    list: Option<ITaskbarList3>,
    /// The list could not be made (no shell): stop asking.
    unavailable: bool,
    /// What the button shows now: a percentage, or nothing.
    shown: Option<i32>,
}

impl TaskbarProgress {
    pub fn new(hwnd: isize) -> TaskbarProgress {
        TaskbarProgress {
            hwnd,
            list: None,
            unavailable: false,
            shown: None,
        }
    }

    fn list(&mut self) -> Option<&ITaskbarList3> {
        if self.list.is_none() && !self.unavailable {
            // SAFETY: COM is initialised on the UI thread (winit's OLE drag and drop).
            let made: windows::core::Result<ITaskbarList3> = unsafe {
                CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)
                    .and_then(|list: ITaskbarList3| list.HrInit().map(|()| list))
            };
            match made {
                Ok(list) => self.list = Some(list),
                Err(error) => {
                    log::warn!("Modlight: the taskbar progress is not available: {error}");
                    self.unavailable = true;
                }
            }
        }
        self.list.as_ref()
    }

    /// Show `progress` (0 to 100) on the button, or take the progress off (`None`).
    pub fn show(&mut self, progress: Option<i32>) {
        if progress == self.shown {
            return;
        }
        let hwnd = HWND(self.hwnd as *mut core::ffi::c_void);
        let Some(list) = self.list() else {
            return;
        };
        // SAFETY: the handle is the app's own main window, alive for the app's lifetime.
        let result = unsafe {
            match progress {
                Some(value) => list.SetProgressValue(hwnd, value.clamp(0, 100) as u64, 100),
                None => list.SetProgressState(hwnd, TBPF_NOPROGRESS),
            }
        };
        if let Err(error) = result {
            log::warn!("Modlight: the taskbar progress could not be set: {error}");
        }
        self.shown = progress;
    }
}

/// `SystemInformation.DoubleClickTime`.
pub fn double_click_time() -> Duration {
    // SAFETY: no arguments.
    let ms = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() };
    Duration::from_millis(u64::from(ms.clamp(100, 5000)))
}

/// `HotkeyProblemViewModel.IsShareXRunning`: ShareX's single-instance mutex exists.
pub fn sharex_running() -> bool {
    use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
    let name = wide(std::ffi::OsStr::new("82E6AC09-0FEF-4390-AD9F-0DD3F5561EFC"));
    // SAFETY: `name` is NUL-terminated and outlives the call; the handle is closed.
    unsafe {
        match OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, PCWSTR(name.as_ptr())) {
            Ok(handle) => {
                let _ = CloseHandle(handle);
                true
            }
            Err(_) => false,
        }
    }
}

/// What starting the installer did (`InstallerLaunch`).
#[derive(Clone, Debug, PartialEq)]
pub enum InstallerStart {
    Started,
    /// The elevation prompt was declined.
    Cancelled,
    Failed(String),
}

/// `DownloaderWindow.StartInstaller`: the verified installer with `/UPDATE`, through the
/// shell so it can ask for elevation itself.
pub fn start_installer(path: &Path) -> InstallerStart {
    use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let file = wide(path.as_os_str());
    let parameters = wide(std::ffi::OsStr::new("/UPDATE"));
    let verb = wide(std::ffi::OsStr::new("open"));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: every string is NUL-terminated and lives until the call returns.
    match unsafe { ShellExecuteExW(&mut info) } {
        Ok(()) => InstallerStart::Started,
        Err(error) if error.code() == ERROR_CANCELLED.to_hresult() => InstallerStart::Cancelled,
        Err(error) => InstallerStart::Failed(error.message()),
    }
}

/// Wake the window whenever a global hotkey is waiting: the app takes the hotkeys once a
/// frame, and an idle window has no frames until something asks for one. Normal mode
/// only; the thread lives as long as the app.
pub fn wake_for_hotkeys(ctx: egui::Context) {
    let spawned = std::thread::Builder::new()
        .name("modlight-hotkey-wake".into())
        .spawn(move || {
            let waiting = global_hotkey::GlobalHotKeyEvent::receiver();
            loop {
                if !waiting.is_empty() {
                    ctx.request_repaint();
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        });
    if let Err(error) = spawned {
        log::warn!("Modlight: the hotkey wake-up could not start: {error}");
    }
}
