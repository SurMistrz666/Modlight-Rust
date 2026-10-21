//! The window itself: a plain top-level Win32 window on the app's main thread (whose
//! winit event loop pumps every message of the thread, this window's and WebView2's
//! callbacks included) with a WebView2 controller filling it.
//!
//! Upstream's form, carried over: 1000 x 760 (scaled for the monitor), at least
//! 560 x 480, centred, in the taskbar, the app's icon and a dark title bar in the dark
//! theme; modeless, nothing on this path blocks the UI thread, every WebView2 call is
//! asynchronous. The navigation's own success flag is ignored, as upstream ignores it:
//! the sign-in is a chain of redirects, and the address and the jar decide.
//!
//! The browser's profile is InPrivate (`ICoreWebView2ControllerOptions`), in a user data
//! folder of its own under the temporary folder. The folder is deleted once the browser
//! has let go of it (`BrowserProcessExited`); one left behind by a crash is swept away
//! the next time a sign-in window opens. The user agent is WebView2's own (Edge's), as
//! upstream left it.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_COLOR, COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC,
    COREWEBVIEW2_PREFERRED_COLOR_SCHEME_DARK, COREWEBVIEW2_PREFERRED_COLOR_SCHEME_LIGHT,
    CreateCoreWebView2EnvironmentWithOptions, GetAvailableCoreWebView2BrowserVersionString,
    ICoreWebView2, ICoreWebView2_2, ICoreWebView2_13, ICoreWebView2Controller,
    ICoreWebView2Controller2, ICoreWebView2Cookie, ICoreWebView2CookieList,
    ICoreWebView2Environment, ICoreWebView2Environment5, ICoreWebView2Environment10,
    ICoreWebView2EnvironmentOptions,
};
use webview2_com::{
    BrowserProcessExitedEventHandler, CoreWebView2EnvironmentOptions,
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    GetCookiesCompletedHandler, NavigationCompletedEventHandler, take_pwstr,
};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::HBRUSH;
use winrt::core::{HSTRING, Interface, PCWSTR, PWSTR};

use super::{CookieData, SignInResult, finish, is_session_domain, is_signed_in_jar};
use crate::wide::wide;

/// How long YouTube is given to settle before the jar is read.
const SETTLE_MS: u32 = 800;
/// The settle timer's id on the window.
const SETTLE_TIMER: usize = 1;
/// Upstream's `Size` and `MinimumSize`, in 96-dpi pixels.
const SIZE: (i32, i32) = (1000, 760);
const MIN_SIZE: (i32, i32) = (560, 480);
const CLASS: &str = "ModlightYouTubeSignIn";
/// The user data folders: `<temp>\Modlight-sign-in-<pid>-<n>-<time>`.
const FOLDER_PREFIX: &str = "Modlight-sign-in-";

/// Whether this is the process's main thread (the one Rust's runtime names "main",
/// where the app's event loop runs). Test threads are named after their tests.
pub fn on_main_thread() -> bool {
    std::thread::current().name() == Some("main")
}

struct Session {
    id: u64,
    hwnd: HWND,
    url: String,
    dark: bool,
    folder: PathBuf,
    brush: HBRUSH,
    environment: Option<ICoreWebView2Environment>,
    controller: Option<ICoreWebView2Controller>,
    webview: Option<ICoreWebView2>,
    /// A controller was asked for (the browser exits once it is closed).
    controller_requested: bool,
    /// The browser has exited already (crashed while the window was open).
    browser_exited: bool,
    /// A harvest is running or has succeeded.
    settled: bool,
    /// The outcome was already reported (the window's destruction must not overwrite it).
    reported: bool,
}

/// A closed window's browser, kept until it has exited and its folder can go.
struct Closing {
    id: u64,
    // Held only so the exit event can still arrive.
    _environment: ICoreWebView2Environment,
    folder: PathBuf,
}

thread_local! {
    static CURRENT: RefCell<Option<Session>> = const { RefCell::new(None) };
    static CLOSING: RefCell<Vec<Closing>> = const { RefCell::new(Vec::new()) };
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// `f` on the open window's state, if there is one with this id. Never panics: callbacks
/// arrive through the message loop, and a panic there would take the app down.
fn session_of<R>(id: u64, f: impl FnOnce(&mut Session) -> R) -> Option<R> {
    CURRENT
        .try_with(|c| {
            let mut slot = c.try_borrow_mut().ok()?;
            slot.as_mut().filter(|s| s.id == id).map(f)
        })
        .ok()
        .flatten()
}

fn session_by_window<R>(hwnd: HWND, f: impl FnOnce(&mut Session) -> R) -> Option<R> {
    CURRENT
        .try_with(|c| {
            let mut slot = c.try_borrow_mut().ok()?;
            slot.as_mut().filter(|s| s.hwnd == hwnd).map(f)
        })
        .ok()
        .flatten()
}

/// The installed runtime's version, or why there is none (`IsRuntimeInstalled`).
fn runtime_version() -> Result<String, String> {
    let mut version = PWSTR::null();
    // SAFETY: the loader writes a CoTaskMem string, which take_pwstr frees.
    unsafe { GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &mut version) }
        .map_err(|e| format!("the WebView2 runtime is not installed ({})", e.message()))?;
    let version = take_pwstr(version);
    if version.is_empty() {
        Err("the WebView2 runtime is not installed".into())
    } else {
        Ok(version)
    }
}

pub fn open(title: &str, url: &str, dark: bool) -> bool {
    // One window at a time: the open one comes forward.
    if let Some(hwnd) = open_window() {
        window::bring_forward(hwnd);
        return true;
    }
    if !url.starts_with("https://") {
        finish(SignInResult::Failed(
            "the sign-in address is not an https address".into(),
        ));
        return false;
    }
    if let Err(reason) = runtime_version() {
        finish(SignInResult::Failed(reason));
        return false;
    }
    if let Err(reason) = window::single_threaded_apartment() {
        finish(SignInResult::Failed(reason));
        return false;
    }
    sweep_folders();
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let folder = std::env::temp_dir().join(format!(
        "{FOLDER_PREFIX}{}-{id}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    if let Err(error) = std::fs::create_dir(&folder) {
        finish(SignInResult::Failed(format!(
            "the browser's folder could not be made: {error}"
        )));
        return false;
    }
    let Some((hwnd, brush)) = window::create(title, dark) else {
        let _ = std::fs::remove_dir_all(&folder);
        finish(SignInResult::Failed("the window could not be made".into()));
        return false;
    };
    CURRENT.with(|c| {
        *c.borrow_mut() = Some(Session {
            id,
            hwnd,
            url: url.to_string(),
            dark,
            folder: folder.clone(),
            brush,
            environment: None,
            controller: None,
            webview: None,
            controller_requested: false,
            browser_exited: false,
            settled: false,
            reported: false,
        })
    });
    if let Err(reason) = start_environment(id, &folder) {
        session_of(id, |s| s.reported = true);
        finish(SignInResult::Failed(reason));
        window::destroy(hwnd);
        return false;
    }
    window::show(hwnd);
    true
}

pub fn close() {
    if let Some(hwnd) = open_window() {
        window::destroy(hwnd);
    }
}

fn open_window() -> Option<HWND> {
    CURRENT
        .try_with(|c| c.try_borrow().ok().and_then(|s| s.as_ref().map(|s| s.hwnd)))
        .ok()
        .flatten()
}

/// The browser failed: the reason is reported and the window closes.
fn fail(id: u64, reason: String) {
    let hwnd = session_of(id, |s| {
        let first = !s.reported;
        s.reported = true;
        first.then_some(s.hwnd)
    })
    .flatten();
    if let Some(hwnd) = hwnd {
        log::warn!("YouTube sign-in: {reason}");
        finish(SignInResult::Failed(reason));
        window::post_close(hwnd);
    }
}

fn start_environment(id: u64, folder: &Path) -> Result<(), String> {
    let options: ICoreWebView2EnvironmentOptions = CoreWebView2EnvironmentOptions::default().into();
    let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
        move |result, environment| {
            environment_ready(id, result, environment);
            Ok(())
        },
    ));
    let folder = HSTRING::from(folder.as_os_str());
    // SAFETY: the loader copies the strings; the handler is reference counted.
    unsafe { CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), &folder, &options, &handler) }
        .map_err(|e| format!("WebView2 did not start ({})", e.message()))
}

fn environment_ready(
    id: u64,
    result: winrt::core::Result<()>,
    environment: Option<ICoreWebView2Environment>,
) {
    let environment = match (result, environment) {
        (Ok(()), Some(environment)) => environment,
        (Err(error), _) => {
            return fail(id, format!("WebView2 did not start ({})", error.message()));
        }
        (Ok(()), None) => return fail(id, "WebView2 gave no environment".into()),
    };
    // Told when the browser has gone, to delete its folder (or to give up on a browser
    // that died while the window was open).
    if let Ok(watched) = environment.cast::<ICoreWebView2Environment5>() {
        let handler =
            BrowserProcessExitedEventHandler::create(Box::new(move |_environment, _arguments| {
                browser_exited(id);
                Ok(())
            }));
        let mut token = 0i64;
        // SAFETY: a COM call on the thread that owns the environment.
        let _ = unsafe { watched.add_BrowserProcessExited(&handler, &mut token) };
    }
    let Some(hwnd) = session_of(id, |s| {
        s.environment = Some(environment.clone());
        s.controller_requested = true;
        s.hwnd
    }) else {
        // The window closed while the browser was starting; letting go of the
        // environment ends it, and the folder goes with the next sweep.
        return;
    };
    let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
        move |result, controller| {
            controller_ready(id, result, controller);
            Ok(())
        },
    ));
    let parent = winrt::Win32::Foundation::HWND(hwnd);
    // SAFETY: COM calls on the thread that owns the environment and the window.
    let started = unsafe {
        match environment.cast::<ICoreWebView2Environment10>() {
            Ok(environment) => {
                environment
                    .CreateCoreWebView2ControllerOptions()
                    .and_then(|options| {
                        options.SetIsInPrivateModeEnabled(true)?;
                        environment
                            .CreateCoreWebView2ControllerWithOptions(parent, &options, &handler)
                    })
            }
            // A runtime older than InPrivate controllers: the folder is still thrown away.
            Err(_) => environment.CreateCoreWebView2Controller(parent, &handler),
        }
    };
    if let Err(error) = started {
        session_of(id, |s| s.controller_requested = false);
        fail(
            id,
            format!("the browser was not created ({})", error.message()),
        );
    }
}

fn controller_ready(
    id: u64,
    result: winrt::core::Result<()>,
    controller: Option<ICoreWebView2Controller>,
) {
    let controller = match (result, controller) {
        (Ok(()), Some(controller)) => controller,
        (Err(error), _) => {
            return controller_failed(
                id,
                format!("the browser was not created ({})", error.message()),
            );
        }
        (Ok(()), None) => return controller_failed(id, "WebView2 gave no browser".into()),
    };
    let Some((hwnd, dark, url)) = session_of(id, |s| (s.hwnd, s.dark, s.url.clone())) else {
        // The window closed while the browser was being made.
        // SAFETY: closing a controller nothing else holds.
        let _ = unsafe { controller.Close() };
        return;
    };
    // SAFETY: COM calls on the thread that owns the controller; the window is alive.
    let webview = unsafe {
        let _ = controller.SetBounds(window::client_rect(hwnd));
        let _ = controller.SetIsVisible(true);
        if let Ok(controller) = controller.cast::<ICoreWebView2Controller2>() {
            let (r, g, b) = window::background(dark);
            let _ = controller.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR {
                A: 255,
                R: r,
                G: g,
                B: b,
            });
        }
        match controller.CoreWebView2() {
            Ok(webview) => webview,
            Err(error) => {
                let _ = controller.Close();
                session_of(id, |s| s.controller_requested = false);
                return fail(id, format!("the browser has no view ({})", error.message()));
            }
        }
    };
    // SAFETY: as above.
    unsafe {
        // The page follows the app's theme, as WebKit follows the window's appearance.
        if let Ok(profile) = webview.cast::<ICoreWebView2_13>().and_then(|w| w.Profile()) {
            let _ = profile.SetPreferredColorScheme(if dark {
                COREWEBVIEW2_PREFERRED_COLOR_SCHEME_DARK
            } else {
                COREWEBVIEW2_PREFERRED_COLOR_SCHEME_LIGHT
            });
        }
        let handler =
            NavigationCompletedEventHandler::create(Box::new(move |_sender, _arguments| {
                navigation_completed(id);
                Ok(())
            }));
        let mut token = 0i64;
        let _ = webview.add_NavigationCompleted(&handler, &mut token);
    }
    session_of(id, |s| {
        s.controller = Some(controller.clone());
        s.webview = Some(webview.clone());
    });
    // SAFETY: as above.
    unsafe {
        if let Err(error) = webview.Navigate(&HSTRING::from(url.as_str())) {
            return fail(
                id,
                format!("the sign-in page did not load ({})", error.message()),
            );
        }
        if window::is_foreground(hwnd) {
            let _ = controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        }
    }
}

/// No browser came of the request. With the window still open it gives up; with the
/// window gone, nothing will ever end this environment's browser, so it is let go of
/// now (and its folder with it, or with the next sweep).
fn controller_failed(id: u64, reason: String) {
    if session_of(id, |s| s.controller_requested = false).is_some() {
        fail(id, reason);
    } else if let Some(closing) = take_closing(id) {
        remove_folder(&closing.folder);
    }
}

/// `CheckAsync`, up to the wait: on the far side of the sign-in, the harvest is claimed
/// (navigations arrive in bursts) and starts after the settle time.
fn navigation_completed(id: u64) {
    let Some(webview) = session_of(id, |s| {
        if s.settled || s.reported {
            None
        } else {
            s.webview.clone()
        }
    })
    .flatten() else {
        return;
    };
    let mut source = PWSTR::null();
    // SAFETY: WebView2 writes a CoTaskMem string, which take_pwstr frees.
    if unsafe { webview.Source(&mut source) }.is_err() {
        return;
    }
    let address = take_pwstr(source);
    if !super::is_signed_in_page(&address) {
        return;
    }
    if let Some(hwnd) = session_of(id, |s| {
        s.settled = true;
        s.hwnd
    }) {
        window::start_settle_timer(hwnd);
    }
}

/// After the settle time: every cookie of the profile is read (it has only ever been to
/// Google's sign-in and to YouTube, whose session spans .google.com and .youtube.com).
fn harvest(hwnd: HWND) {
    let Some((id, webview)) = session_by_window(hwnd, |s| (s.id, s.webview.clone())) else {
        return;
    };
    let Some(webview) = webview else {
        session_of(id, |s| s.settled = false);
        return;
    };
    let handler = GetCookiesCompletedHandler::create(Box::new(move |result, list| {
        cookies_read(id, result, list);
        Ok(())
    }));
    // SAFETY: COM calls on the thread that owns the view; a null address asks for all.
    let asked = unsafe {
        webview
            .cast::<ICoreWebView2_2>()
            .and_then(|w| w.CookieManager())
            .and_then(|manager| manager.GetCookies(PCWSTR::null(), &handler))
    };
    if asked.is_err() {
        session_of(id, |s| s.settled = false);
    }
}

fn cookies_read(id: u64, result: winrt::core::Result<()>, list: Option<ICoreWebView2CookieList>) {
    if session_of(id, |s| s.reported) != Some(false) {
        return;
    }
    let jar = match (result, list) {
        (Ok(()), Some(list)) => read_jar(&list),
        _ => Vec::new(),
    };
    let names: Vec<String> = jar.iter().map(|c| c.name.clone()).collect();
    if jar.is_empty() || !is_signed_in_jar(&names) {
        // On youtube.com but not signed in (the sign-in redirects through here on its
        // way in): the person carries on.
        session_of(id, |s| s.settled = false);
        return;
    }
    let Some(hwnd) = session_of(id, |s| {
        s.reported = true;
        s.hwnd
    }) else {
        return;
    };
    finish(SignInResult::Captured(jar));
    window::post_close(hwnd);
}

/// The cookies of the list that belong to the session (YouTube's and Google's).
fn read_jar(list: &ICoreWebView2CookieList) -> Vec<CookieData> {
    let mut count = 0u32;
    // SAFETY: COM calls on a list WebView2 handed over for this call.
    if unsafe { list.Count(&mut count) }.is_err() {
        return Vec::new();
    }
    (0..count)
        .filter_map(|i| unsafe { list.GetValueAtIndex(i) }.ok())
        .filter_map(|cookie| cookie_data(&cookie))
        .filter(|cookie| is_session_domain(&cookie.domain))
        .collect()
}

fn cookie_data(cookie: &ICoreWebView2Cookie) -> Option<CookieData> {
    let text = |read: &dyn Fn(*mut PWSTR) -> winrt::core::Result<()>| -> Option<String> {
        let mut value = PWSTR::null();
        read(&mut value).ok()?;
        Some(take_pwstr(value))
    };
    // SAFETY: property reads of a cookie WebView2 handed over; every string is freed by
    // take_pwstr.
    unsafe {
        let name = text(&|p| cookie.Name(p))?;
        let value = text(&|p| cookie.Value(p))?;
        let domain = text(&|p| cookie.Domain(p))?;
        let path = text(&|p| cookie.Path(p)).unwrap_or_else(|| "/".into());
        let mut secure = winrt::core::BOOL(0);
        let _ = cookie.IsSecure(&mut secure);
        let mut session = winrt::core::BOOL(1);
        let _ = cookie.IsSession(&mut session);
        let mut expires = 0f64;
        let _ = cookie.Expires(&mut expires);
        Some(CookieData {
            domain,
            path,
            name,
            value,
            secure: secure.as_bool(),
            expires_unix: expiry(session.as_bool(), expires),
        })
    }
}

/// A cookie's expiry as seconds since the epoch, or zero for "session" (`Expiry`): the
/// session flag first, since the date of a session cookie is a sentinel.
pub fn expiry(session: bool, expires: f64) -> i64 {
    if session || !expires.is_finite() || expires <= 0.0 {
        0
    } else {
        expires as i64
    }
}

/// The browser exited: its folder is deleted, or, while its window is still open, the
/// window gives up.
fn browser_exited(id: u64) {
    if session_of(id, |s| s.browser_exited = true).is_some() {
        fail(id, "the browser stopped".into());
        return;
    }
    if let Some(closing) = take_closing(id) {
        remove_folder(&closing.folder);
    }
}

fn take_closing(id: u64) -> Option<Closing> {
    CLOSING
        .try_with(|c| {
            let mut list = c.try_borrow_mut().ok()?;
            let at = list.iter().position(|e| e.id == id)?;
            Some(list.remove(at))
        })
        .ok()
        .flatten()
}

fn remove_folder(folder: &Path) {
    if let Err(error) = std::fs::remove_dir_all(folder) {
        log::info!("YouTube sign-in: the browser's folder stays until the next sweep ({error})");
    }
}

/// The window is being destroyed: the outcome is reported (closed by the person =
/// abandoned) and the browser is closed; its folder goes once it has exited.
fn window_destroyed(hwnd: HWND) {
    let session = CURRENT
        .try_with(|c| {
            let mut slot = c.try_borrow_mut().ok()?;
            if slot.as_ref().is_some_and(|s| s.hwnd == hwnd) {
                slot.take()
            } else {
                None
            }
        })
        .ok()
        .flatten();
    let Some(session) = session else {
        return;
    };
    if !session.reported {
        finish(SignInResult::Abandoned);
    }
    // SAFETY: COM calls on the thread that owns the browser.
    unsafe {
        if let Some(controller) = &session.controller {
            let _ = controller.Close();
        }
    }
    window::delete_brush(session.brush);
    match session.environment {
        Some(environment) if session.controller_requested && !session.browser_exited => {
            let _ = CLOSING.try_with(|c| {
                if let Ok(mut list) = c.try_borrow_mut() {
                    list.push(Closing {
                        id: session.id,
                        _environment: environment,
                        folder: session.folder,
                    });
                }
            });
        }
        // No browser is running (any more) for this folder.
        _ => remove_folder(&session.folder),
    }
}

/// Folders of earlier sign-in windows that nothing holds any more: those of processes
/// that have ended, and this process's own ones no window or closing browser uses.
fn sweep_folders() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    let in_use: Vec<PathBuf> = CLOSING
        .try_with(|c| {
            c.try_borrow()
                .map(|list| list.iter().map(|e| e.folder.clone()).collect())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let me = std::process::id();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = folder_process(&name.to_string_lossy()) else {
            continue;
        };
        let path = entry.path();
        let stale = if pid == me {
            !in_use.contains(&path)
        } else {
            !window::process_is_running(pid)
        };
        if stale && entry.file_type().is_ok_and(|t| t.is_dir()) {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// The process a sign-in folder's name belongs to (`Modlight-sign-in-<pid>-...`).
pub fn folder_process(name: &str) -> Option<u32> {
    name.strip_prefix(FOLDER_PREFIX)?
        .split('-')
        .next()?
        .parse()
        .ok()
}

/// The Win32 side: the window class, the window and its messages.
mod window {
    use windows_sys::Win32::Foundation::{
        CloseHandle, HWND, LPARAM, LRESULT, RECT, STILL_ACTIVE, WPARAM,
    };
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
    use windows_sys::Win32::Graphics::Gdi::{
        CreateSolidBrush, DeleteObject, FillRect, GetMonitorInfoW, HBRUSH, HDC,
        MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CopyIcon, CreateWindowExW, DefWindowProcW,
        DestroyIcon, DestroyWindow, GCLP_HICON, GCLP_HICONSM, GetClassLongPtrW, GetClientRect,
        GetForegroundWindow, HICON, ICON_BIG, ICON_SMALL, IDC_ARROW, IsIconic, IsWindow, KillTimer,
        LoadCursorW, MINMAXINFO, PostMessageW, RegisterClassExW, SW_RESTORE, SW_SHOW,
        SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetForegroundWindow, SetTimer, SetWindowPos,
        ShowWindow, WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_GETICON,
        WM_GETMINMAXINFO, WM_MOVE, WM_SETFOCUS, WM_SETICON, WM_SIZE, WM_TIMER, WNDCLASSEXW,
        WS_OVERLAPPEDWINDOW,
    };

    use super::{MIN_SIZE, SETTLE_MS, SETTLE_TIMER, SIZE, wide};

    /// The main thread is a single-threaded COM apartment, as WebView2 needs (winit
    /// makes it one for drag and drop; joined here when it is not yet anything).
    pub fn single_threaded_apartment() -> Result<(), String> {
        // SAFETY: plain COM initialisation of this thread, kept for its lifetime.
        let result = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        if result >= 0 {
            Ok(())
        } else {
            Err(format!(
                "the main thread is not a single-threaded COM apartment ({result:#x})"
            ))
        }
    }

    pub fn background(dark: bool) -> (u8, u8, u8) {
        if dark { (32, 32, 32) } else { (255, 255, 255) }
    }

    fn scale(value: i32, dpi: u32) -> i32 {
        (value as i64 * dpi as i64 / 96) as i32
    }

    fn register_class() -> bool {
        static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *REGISTERED.get_or_init(|| {
            let name = wide(super::CLASS);
            // SAFETY: a class of this module with a static name buffer (leaked once).
            unsafe {
                let class = WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(window_proc),
                    hInstance: GetModuleHandleW(std::ptr::null()),
                    hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                    lpszClassName: Box::leak(name.into_boxed_slice()).as_ptr(),
                    ..std::mem::zeroed()
                };
                RegisterClassExW(&class) != 0
            }
        })
    }

    /// The window, hidden, sized and centred on the app's monitor, with its title, the
    /// app's icon and the theme's title bar.
    pub fn create(title: &str, dark: bool) -> Option<(HWND, HBRUSH)> {
        if !register_class() {
            return None;
        }
        let class = wide(super::CLASS);
        let title = wide(title);
        // SAFETY: Win32 calls on the main thread with NUL-terminated strings; every
        // handle made here is owned by the window or the session.
        unsafe {
            let app = GetActiveWindow();
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return None;
            }
            let monitor = MonitorFromWindow(
                if app.is_null() { hwnd } else { app },
                MONITOR_DEFAULTTONEAREST,
            );
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
            if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) < 0 {
                dpi_x = 96;
            }
            if GetMonitorInfoW(monitor, &mut info) != 0 {
                let work = info.rcWork;
                let (work_w, work_h) = (work.right - work.left, work.bottom - work.top);
                let w = scale(SIZE.0, dpi_x).min(work_w);
                let h = scale(SIZE.1, dpi_x).min(work_h);
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    work.left + (work_w - w) / 2,
                    work.top + (work_h - h) / 2,
                    w,
                    h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            let dark_bar: i32 = dark as i32;
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                &dark_bar as *const i32 as *const _,
                size_of::<i32>() as u32,
            );
            if !app.is_null() {
                copy_icon(app, hwnd, ICON_BIG, GCLP_HICON);
                copy_icon(app, hwnd, ICON_SMALL, GCLP_HICONSM);
            }
            let (r, g, b) = background(dark);
            let brush = CreateSolidBrush(r as u32 | (g as u32) << 8 | (b as u32) << 16);
            Some((hwnd, brush))
        }
    }

    /// The app window's icon, copied onto this window (destroyed with it).
    unsafe fn copy_icon(from: HWND, to: HWND, kind: u32, class_index: i32) {
        // SAFETY: plain message and class queries of the app's own window.
        unsafe {
            let mut icon = SendMessageW(from, WM_GETICON, kind as WPARAM, 0) as HICON;
            if icon.is_null() {
                icon = GetClassLongPtrW(from, class_index) as HICON;
            }
            if !icon.is_null() {
                let copy = CopyIcon(icon);
                if !copy.is_null() {
                    SendMessageW(to, WM_SETICON, kind as WPARAM, copy as LPARAM);
                }
            }
        }
    }

    unsafe fn destroy_icons(hwnd: HWND) {
        // SAFETY: the copies made in copy_icon, owned by this window.
        unsafe {
            for kind in [ICON_BIG, ICON_SMALL] {
                let icon = SendMessageW(hwnd, WM_GETICON, kind as WPARAM, 0) as HICON;
                if !icon.is_null() {
                    DestroyIcon(icon);
                }
            }
        }
    }

    pub fn show(hwnd: HWND) {
        // SAFETY: a window of this thread.
        unsafe {
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
        }
    }

    pub fn bring_forward(hwnd: HWND) {
        // SAFETY: a window of this thread.
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd);
        }
    }

    pub fn is_foreground(hwnd: HWND) -> bool {
        // SAFETY: plain query.
        unsafe { GetForegroundWindow() == hwnd }
    }

    pub fn destroy(hwnd: HWND) {
        // SAFETY: a window of this thread; destroying it runs WM_DESTROY synchronously.
        unsafe {
            if IsWindow(hwnd) != 0 {
                DestroyWindow(hwnd);
            }
        }
    }

    /// Closed from the message loop, never from inside a WebView2 callback.
    pub fn post_close(hwnd: HWND) {
        // SAFETY: posting to a window of this thread.
        unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
    }

    pub fn start_settle_timer(hwnd: HWND) {
        // SAFETY: a timer on a window of this thread, killed when it fires.
        unsafe { SetTimer(hwnd, SETTLE_TIMER, SETTLE_MS, None) };
    }

    pub fn delete_brush(brush: HBRUSH) {
        if !brush.is_null() {
            // SAFETY: the session's own brush, deleted once.
            unsafe { DeleteObject(brush) };
        }
    }

    pub fn client_rect(hwnd: HWND) -> winrt::Win32::Foundation::RECT {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: a window of this thread.
        unsafe { GetClientRect(hwnd, &mut rect) };
        winrt::Win32::Foundation::RECT {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }

    pub fn process_is_running(pid: u32) -> bool {
        // SAFETY: a handle opened and closed here.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return false;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(process, &mut code);
            CloseHandle(process);
            ok == 0 || code == STILL_ACTIVE as u32
        }
    }

    fn fill(hwnd: HWND, dc: HDC) -> bool {
        let Some(brush) = super::session_by_window(hwnd, |s| s.brush) else {
            return false;
        };
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: painting the window's own DC during WM_ERASEBKGND.
        unsafe {
            GetClientRect(hwnd, &mut rect);
            FillRect(dc, &rect, brush);
        }
        true
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_SIZE | WM_MOVE => {
                super::layout(hwnd, message == WM_SIZE);
                0
            }
            WM_SETFOCUS => {
                super::focus(hwnd);
                0
            }
            WM_GETMINMAXINFO => {
                // SAFETY: the system passes a MINMAXINFO for the window.
                unsafe {
                    let dpi = match GetDpiForWindow(hwnd) {
                        0 => 96,
                        dpi => dpi,
                    };
                    let info = &mut *(lparam as *mut MINMAXINFO);
                    info.ptMinTrackSize.x = scale(MIN_SIZE.0, dpi);
                    info.ptMinTrackSize.y = scale(MIN_SIZE.1, dpi);
                }
                0
            }
            WM_DPICHANGED => {
                // SAFETY: the system passes the suggested rectangle for the new DPI.
                unsafe {
                    let suggested = &*(lparam as *const RECT);
                    SetWindowPos(
                        hwnd,
                        std::ptr::null_mut(),
                        suggested.left,
                        suggested.top,
                        suggested.right - suggested.left,
                        suggested.bottom - suggested.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                0
            }
            WM_ERASEBKGND if fill(hwnd, wparam as HDC) => 1,
            WM_TIMER if wparam == SETTLE_TIMER => {
                // SAFETY: the timer started on this window.
                unsafe { KillTimer(hwnd, SETTLE_TIMER) };
                super::harvest(hwnd);
                0
            }
            WM_DESTROY => {
                // SAFETY: the window's own timer and icons.
                unsafe {
                    KillTimer(hwnd, SETTLE_TIMER);
                }
                super::window_destroyed(hwnd);
                // SAFETY: as above.
                unsafe { destroy_icons(hwnd) };
                0
            }
            // SAFETY: everything else as Windows does it.
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }
}

/// The browser follows the window: its bounds on a resize, its position on a move.
fn layout(hwnd: HWND, resized: bool) {
    let Some(controller) = session_by_window(hwnd, |s| s.controller.clone()).flatten() else {
        return;
    };
    // SAFETY: COM calls on the thread that owns the controller.
    unsafe {
        if resized {
            let _ = controller.SetBounds(window::client_rect(hwnd));
        } else {
            let _ = controller.NotifyParentWindowPositionChanged();
        }
    }
}

fn focus(hwnd: HWND) {
    let Some(controller) = session_by_window(hwnd, |s| s.controller.clone()).flatten() else {
        return;
    };
    // SAFETY: a COM call on the thread that owns the controller.
    let _ = unsafe { controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_cookie_has_no_expiry() {
        assert_eq!(expiry(true, 1_900_000_000.0), 0);
        assert_eq!(expiry(false, 1_900_000_000.5), 1_900_000_000);
        assert_eq!(expiry(false, -1.0), 0);
        assert_eq!(expiry(false, f64::NAN), 0);
    }

    #[test]
    fn sign_in_folders_are_known_by_their_process() {
        assert_eq!(
            folder_process("Modlight-sign-in-4242-1-1761000000000"),
            Some(4242)
        );
        assert_eq!(folder_process("Modlight-sign-in-x-1"), None);
        assert_eq!(folder_process("Other-4242-1"), None);
    }

    #[test]
    fn a_process_that_has_ended_is_not_running() {
        assert!(window::process_is_running(std::process::id()));
        // Process ids are multiples of four; this one is never handed out.
        assert!(!window::process_is_running(u32::MAX - 2));
    }
}
