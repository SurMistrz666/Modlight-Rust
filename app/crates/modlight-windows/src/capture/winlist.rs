//! Top-level windows: the list the capture menu and the region capture use
//! (`WindowsList.GetVisibleWindowsList`, `WindowsRectangleList`), one window's details
//! (`WindowInfo`), and bringing a window to the front (`WindowInfo.Activate`).
//!
//! Rectangles here are physical pixels; the calling thread holds
//! [`PhysicalPixels`](super::desktop::PhysicalPixels) while it reads them.
//!
//! A window's id in the app (`WindowInfo::id`, a `u32`) is its `HWND` truncated to 32
//! bits. Window handles are user handles, which 64-bit Windows keeps within 32
//! significant bits so that 32- and 64-bit programs can exchange them: truncating one
//! and sign-extending it back gives the same handle ("Interprocess Communication
//! Between 32-bit and 64-bit Applications").

use std::collections::HashMap;
use std::path::Path;

use image::RgbaImage;
use modlight_core::capture::geometry::IRect;
use modlight_core::capture::source::WindowInfo;
use windows_sys::Win32::Foundation::{
    CloseHandle, HWND, INVALID_HANDLE_VALUE, LPARAM, POINT, RECT,
};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetClientRect,
    GetForegroundWindow, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsGUIThread, IsIconic, IsWindow, IsWindowVisible, MSG, PM_NOREMOVE,
    PeekMessageW, SetForegroundWindow, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};
use windows_sys::core::BOOL;

use super::desktop::{irect, rect_to_points};
use crate::wide::from_wide;

/// The app's id of a window.
pub(crate) fn hwnd_id(hwnd: HWND) -> u32 {
    hwnd as usize as u32
}

/// The window an app id stands for (sign-extended, see the module notes).
pub(crate) fn id_hwnd(id: u32) -> HWND {
    id as i32 as isize as HWND
}

/// This process's id.
pub(crate) fn own_pid() -> u32 {
    // SAFETY: a plain query.
    unsafe { GetCurrentProcessId() }
}

/// The process that owns a window.
pub(crate) fn window_pid(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    // SAFETY: a window handle (possibly stale: the call then fails) and an out-parameter.
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    pid
}

pub(crate) fn is_window(hwnd: HWND) -> bool {
    // SAFETY: a plain query on any handle value.
    !hwnd.is_null() && unsafe { IsWindow(hwnd) } != 0
}

/// `WindowInfo.ClassName`.
pub(crate) fn class_name(hwnd: HWND) -> String {
    let mut buffer = [0u16; 256];
    // SAFETY: the buffer and its length.
    let length = unsafe { GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    from_wide(&buffer[..length.max(0) as usize])
}

/// `WindowInfo.Text`. For another process's window this reads the caption User32 keeps,
/// without sending it a message, so a hung window cannot block the list.
pub(crate) fn title(hwnd: HWND) -> String {
    // SAFETY: a plain query.
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; length as usize + 1];
    // SAFETY: the buffer and its length.
    let copied = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    from_wide(&buffer[..copied.max(0) as usize])
}

pub(crate) fn style(hwnd: HWND) -> u32 {
    // SAFETY: a plain query.
    unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 }
}

pub(crate) fn ex_style(hwnd: HWND) -> u32 {
    // SAFETY: a plain query.
    unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 }
}

pub(crate) fn is_visible(hwnd: HWND) -> bool {
    // SAFETY: a plain query.
    unsafe { IsWindowVisible(hwnd) != 0 }
}

pub(crate) fn is_minimized(hwnd: HWND) -> bool {
    // SAFETY: a plain query.
    unsafe { IsIconic(hwnd) != 0 }
}

/// `WindowInfo.IsCloaked`: DWM keeps it off screen (another virtual desktop, a suspended
/// store app) although it is "visible".
pub(crate) fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    // SAFETY: the attribute is a DWORD written into `cloaked`.
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            &mut cloaked as *mut u32 as *mut _,
            std::mem::size_of::<u32>() as u32,
        )
    };
    result >= 0 && cloaked != 0
}

/// `GetWindowRect`, physical pixels.
pub(crate) fn outer_rect(hwnd: HWND) -> Option<IRect> {
    let mut rect = RECT::default();
    // SAFETY: an out-parameter.
    (unsafe { GetWindowRect(hwnd, &mut rect) } != 0).then(|| irect(rect))
}

/// `CaptureHelpers.GetWindowRectangle`: what the window looks like on screen, without
/// the invisible resize borders of Windows 10 and later (DWM's extended frame bounds),
/// physical pixels.
pub(crate) fn window_rect(hwnd: HWND) -> Option<IRect> {
    let mut rect = RECT::default();
    // SAFETY: the attribute is a RECT written into `rect`.
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            &mut rect as *mut RECT as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
    };
    let rect = irect(rect);
    if result >= 0 && !rect.is_empty() {
        Some(rect)
    } else {
        outer_rect(hwnd)
    }
}

/// `NativeMethods.GetClientRect`: the client area in screen coordinates, physical pixels.
pub(crate) fn client_rect(hwnd: HWND) -> Option<IRect> {
    let mut rect = RECT::default();
    let mut origin = POINT::default();
    // SAFETY: out-parameters.
    let ok =
        unsafe { GetClientRect(hwnd, &mut rect) != 0 && ClientToScreen(hwnd, &mut origin) != 0 };
    ok.then(|| IRect::new(origin.x, origin.y, rect.right, rect.bottom))
}

unsafe extern "system" fn collect_window(hwnd: HWND, data: LPARAM) -> BOOL {
    // SAFETY: `data` is the `Vec` that `top_level_windows` passed in.
    let list = unsafe { &mut *(data as *mut Vec<HWND>) };
    list.push(hwnd);
    1
}

/// Every top-level window, front to back (`EnumWindows` walks the z-order).
pub(crate) fn top_level_windows() -> Vec<HWND> {
    let mut list: Vec<HWND> = Vec::new();
    // SAFETY: the callback only touches `list`, which outlives the call.
    unsafe { EnumWindows(Some(collect_window), &mut list as *mut Vec<HWND> as LPARAM) };
    list
}

/// Console windows report the process attached to them as their owner but belong to
/// the console host: never this app's.
fn is_console(hwnd: HWND) -> bool {
    matches!(
        class_name(hwnd).as_str(),
        "ConsoleWindowClass" | "PseudoConsoleWindow"
    )
}

/// This process's top-level windows, front to back (shown or not).
pub(crate) fn own_windows() -> Vec<HWND> {
    let own = own_pid();
    top_level_windows()
        .into_iter()
        .filter(|&hwnd| window_pid(hwnd) == own && !is_console(hwnd))
        .collect()
}

/// The image file name without its extension (`Process.ProcessName`, upstream's `%pn`).
fn process_name_of_path(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The name of a running process: from its image path, or, for a process this one may
/// not open (a protected service), from the system's process list.
pub(crate) fn process_name(pid: u32) -> String {
    // SAFETY: the handle is closed below; the buffer and its length are passed together.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if !process.is_null() {
            let mut buffer = [0u16; 1024];
            let mut length = buffer.len() as u32;
            let ok = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                buffer.as_mut_ptr(),
                &mut length,
            ) != 0;
            CloseHandle(process);
            if ok {
                return process_name_of_path(&from_wide(&buffer[..length as usize]));
            }
        }
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot.is_null() || snapshot == INVALID_HANDLE_VALUE {
            return String::new();
        }
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut name = String::new();
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more {
            if entry.th32ProcessID == pid {
                name = process_name_of_path(&from_wide(&entry.szExeFile));
                break;
            }
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        name
    }
}

/// The app's description of a window, its rectangle in points.
pub(crate) fn describe(
    hwnd: HWND,
    rect: IRect,
    scale: f64,
    names: &mut HashMap<u32, String>,
) -> WindowInfo {
    let pid = window_pid(hwnd);
    let process_name = names
        .entry(pid)
        .or_insert_with(|| process_name(pid))
        .clone();
    WindowInfo {
        id: hwnd_id(hwnd),
        title: title(hwnd),
        process_name,
        pid: pid as i32,
        rect: rect_to_points(rect, scale),
        on_screen: !is_minimized(hwnd),
    }
}

/// Classes never offered: the desktop and stray buttons (`WindowsList.ignoreList`), the
/// NVIDIA overlay (`WindowsRectangleList.IgnoreClassNameList`).
const IGNORED_CLASSES: [&str; 3] = ["Progman", "Button", "CEF-OSC-WIDGET"];

/// Whether the list shows a window (`WindowsList.IsValidWindow`, with
/// `WindowsRectangleList`'s rule against non-activatable tool windows: overlays of tiling
/// managers and system helpers are never the program the person means).
fn listed(hwnd: HWND) -> Option<IRect> {
    if !is_visible(hwnd) || is_cloaked(hwnd) {
        return None;
    }
    if title(hwnd).is_empty() {
        return None;
    }
    let class = class_name(hwnd);
    if IGNORED_CLASSES
        .iter()
        .any(|ignored| class.eq_ignore_ascii_case(ignored))
    {
        return None;
    }
    let both = WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
    if ex_style(hwnd) & both == both {
        return None;
    }
    window_rect(hwnd).filter(|rect| rect.has_area())
}

/// `WindowsList.GetVisibleWindowsList` without this app's own windows, front to back.
/// Minimised windows stay in the list (the capture menu offers them) with `on_screen`
/// false and the parked rectangle Windows gives them.
pub(crate) fn visible_windows(scale: f64) -> Vec<WindowInfo> {
    let own = own_pid();
    let mut names = HashMap::new();
    top_level_windows()
        .into_iter()
        .filter(|&hwnd| window_pid(hwnd) != own)
        .filter_map(|hwnd| listed(hwnd).map(|rect| describe(hwnd, rect, scale, &mut names)))
        .collect()
}

/// The foreground window (`GetForegroundWindow`), if any.
pub(crate) fn foreground() -> Option<HWND> {
    // SAFETY: a plain query.
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.is_null()).then_some(hwnd)
}

/// `WindowInfo.Activate` (`SetForegroundWindow`), made to work from a background app: the
/// system lets only the process the person is using change the foreground, so when the
/// plain call is refused, this thread shares the foreground thread's input state for a
/// moment (`AttachThreadInput`) and asks again. No key is pressed for it. Returns whether
/// the window is now in front.
pub(crate) fn bring_to_front(hwnd: HWND) -> bool {
    if !is_window(hwnd) {
        return false;
    }
    // SAFETY: plain calls on a live window; the attachment is undone before returning.
    unsafe {
        if GetForegroundWindow() == hwnd {
            return true;
        }
        if SetForegroundWindow(hwnd) != 0 && GetForegroundWindow() == hwnd {
            return true;
        }
        // Attaching needs a message queue on this thread; a thread without windows gets
        // one from a look at its (empty) queue. A GUI thread has one already and is not
        // made to process messages here.
        if IsGUIThread(0) == 0 {
            let mut message = MSG::default();
            PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
        }
        let theirs = GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut());
        let ours = GetCurrentThreadId();
        let attached = theirs != 0 && theirs != ours && AttachThreadInput(ours, theirs, 1) != 0;
        BringWindowToTop(hwnd);
        SetForegroundWindow(hwnd);
        if attached {
            AttachThreadInput(ours, theirs, 0);
        }
        GetForegroundWindow() == hwnd
    }
}

/// Composite `top` over `base` at (x, y) (source over, straight alpha).
pub(crate) fn blend_over(base: &mut RgbaImage, top: &RgbaImage, x: i64, y: i64) {
    for (tx, ty, pixel) in top.enumerate_pixels() {
        let (bx, by) = (x + tx as i64, y + ty as i64);
        if bx < 0 || by < 0 || bx >= base.width() as i64 || by >= base.height() as i64 {
            continue;
        }
        let a = pixel[3] as u32;
        if a == 0 {
            continue;
        }
        let under = base.get_pixel_mut(bx as u32, by as u32);
        let ua = under[3] as u32;
        // out_a = a + ua (1 - a), colours weighted the same way.
        let out_a = a * 255 + ua * (255 - a);
        if out_a == 0 {
            continue;
        }
        for c in 0..3 {
            let value = (pixel[c] as u32 * a * 255 + under[c] as u32 * ua * (255 - a)) / out_a;
            under[c] = value.min(255) as u8;
        }
        under[3] = (out_a / 255).min(255) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_ids_round_trip() {
        for value in [0x0001_0010isize, 0x7fff_fffe, 0x00a3_1b2c] {
            let hwnd = value as HWND;
            assert_eq!(id_hwnd(hwnd_id(hwnd)), hwnd);
        }
        // A handle with the 32nd bit set comes back sign-extended, as Windows expects.
        let high = 0xffff_ffff_8000_1234u64 as isize as HWND;
        assert_eq!(id_hwnd(hwnd_id(high)), high);
    }

    #[test]
    fn process_names_are_file_stems() {
        assert_eq!(
            process_name_of_path(r"C:\Program Files\Mozilla Firefox\firefox.exe"),
            "firefox"
        );
        assert_eq!(process_name_of_path(r"C:\Tools\my.tool.exe"), "my.tool");
        let own = process_name(own_pid());
        let exe = std::env::current_exe().unwrap();
        assert_eq!(own, exe.file_stem().unwrap().to_string_lossy());
    }

    #[test]
    fn the_list_leaves_this_process_out() {
        let _pixels = super::super::desktop::PhysicalPixels::enter();
        let windows = visible_windows(1.0);
        let own = own_pid() as i32;
        for window in &windows {
            assert_ne!(window.pid, own);
            assert!(!window.title.is_empty());
            assert!(window.rect.has_area());
            assert!(is_window(id_hwnd(window.id)), "{window:?}");
        }
        // The order is the z-order, the same as the system's own walk.
        let order: Vec<u32> = top_level_windows().into_iter().map(hwnd_id).collect();
        let positions: Vec<usize> = windows
            .iter()
            .map(|w| order.iter().position(|&id| id == w.id).unwrap())
            .collect();
        assert!(positions.windows(2).all(|p| p[0] < p[1]));
    }

    #[test]
    fn blending() {
        let mut base = RgbaImage::from_pixel(2, 1, image::Rgba([0, 0, 255, 255]));
        let top = RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 128]));
        blend_over(&mut base, &top, 1, 0);
        assert_eq!(base.get_pixel(0, 0).0, [0, 0, 255, 255]);
        let mixed = base.get_pixel(1, 0).0;
        assert_eq!(mixed[3], 255);
        assert!((126..=130).contains(&mixed[0]) && (125..=129).contains(&mixed[2]));
        // Over a transparent pixel the picture is taken as it is.
        let mut clear = RgbaImage::new(1, 1);
        blend_over(&mut clear, &top, 0, 0);
        assert_eq!(clear.get_pixel(0, 0).0, [255, 0, 0, 128]);
        // Outside the base: nothing happens.
        blend_over(&mut clear, &top, 5, -3);
    }
}
