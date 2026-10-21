//! The desktop's geometry: physical pixels, the app's screen points, and the monitors
//! (`Screen.AllScreens`, `SystemInformation.VirtualScreen`).
//!
//! Win32 answers in physical pixels only for a thread that is per-monitor DPI aware;
//! [`PhysicalPixels`] makes the calling thread so for as long as it lives, whatever the
//! process was started with (winit makes the app's process per-monitor aware v2, test
//! binaries are not). Everything handed to the app is in its screen points: physical
//! pixels divided by the primary monitor's scale ([`primary_scale`]). egui places
//! viewports in logical units of the monitor they are created on, so overlays line up
//! exactly when every monitor has the same scale; with mixed scales a monitor whose
//! scale differs from the primary's is placed off by that ratio (a known limitation).

use std::collections::HashMap;
use std::sync::Mutex;

use modlight_core::capture::geometry::IRect;
use windows_sys::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
    QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    DISPLAY_DEVICEW, EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR,
    MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MONITORINFOEXW, MonitorFromPoint,
};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor,
    MDT_EFFECTIVE_DPI, SetThreadDpiAwarenessContext,
};
use windows_sys::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;
use windows_sys::core::BOOL;

use crate::wide::{from_wide, wide};

/// The calling thread sees physical pixels (per-monitor DPI awareness v2) until this is
/// dropped; then its previous awareness comes back.
pub(crate) struct PhysicalPixels {
    previous: DPI_AWARENESS_CONTEXT,
}

impl PhysicalPixels {
    pub(crate) fn enter() -> PhysicalPixels {
        // SAFETY: a plain call with a predefined context; on a system without it the
        // call fails, returns null and changes nothing.
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        PhysicalPixels { previous }
    }
}

impl Drop for PhysicalPixels {
    fn drop(&mut self) {
        if !self.previous.is_null() {
            // SAFETY: the context this thread had before `enter`.
            unsafe { SetThreadDpiAwarenessContext(self.previous) };
        }
    }
}

/// Pixels per point of a monitor (`dpi / 96`), 1 when it cannot be read.
fn monitor_scale(monitor: HMONITOR) -> f64 {
    let (mut x, mut y) = (0u32, 0u32);
    // SAFETY: a monitor handle from the system and two out-parameters.
    let result = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) };
    if result >= 0 && x > 0 {
        x as f64 / 96.0
    } else {
        1.0
    }
}

/// The app's pixels per point: the primary monitor's scale (`ScreenInfo::scale` of
/// every monitor).
pub(crate) fn primary_scale() -> f64 {
    let _pixels = PhysicalPixels::enter();
    // SAFETY: a plain query; the primary monitor always exists.
    let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    monitor_scale(monitor)
}

/// A length in physical pixels as points.
pub(crate) fn to_points(pixels: i32, scale: f64) -> i32 {
    (pixels as f64 / scale).round() as i32
}

/// A length in points as physical pixels.
pub(crate) fn to_pixels(points: i32, scale: f64) -> i32 {
    (points as f64 * scale).round() as i32
}

/// A physical rectangle in points. Each edge is converted by itself, so rectangles
/// that touch in pixels (two monitors side by side) still touch in points.
pub(crate) fn rect_to_points(rect: IRect, scale: f64) -> IRect {
    let left = to_points(rect.x, scale);
    let top = to_points(rect.y, scale);
    IRect::new(
        left,
        top,
        to_points(rect.right(), scale) - left,
        to_points(rect.bottom(), scale) - top,
    )
}

/// A rectangle in points as physical pixels, edge by edge as [`rect_to_points`].
pub(crate) fn rect_to_pixels(rect: IRect, scale: f64) -> IRect {
    let left = to_pixels(rect.x, scale);
    let top = to_pixels(rect.y, scale);
    IRect::new(
        left,
        top,
        to_pixels(rect.right(), scale) - left,
        to_pixels(rect.bottom(), scale) - top,
    )
}

/// A Win32 rectangle as an `IRect`.
pub(crate) fn irect(rect: RECT) -> IRect {
    IRect::new(
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
    )
}

/// One monitor in physical pixels.
#[derive(Clone, Debug)]
pub(crate) struct Monitor {
    /// The GDI device name (`\\.\DISPLAY1`): stable while the app runs.
    pub device: String,
    pub bounds: IRect,
    /// Without the taskbar and docked app bars (`WorkingArea`).
    pub work: IRect,
    pub primary: bool,
}

impl Monitor {
    /// The id the app keys its overlays by: the display's number (`\\.\DISPLAY2` is 2),
    /// or a hash of an unusual device name.
    pub(crate) fn id(&self) -> u32 {
        display_number(&self.device).unwrap_or_else(|| {
            // FNV-1a: stable for the same name, never confused with a display number.
            let hash = self.device.bytes().fold(0x811c_9dc5u32, |hash, byte| {
                (hash ^ byte as u32).wrapping_mul(0x0100_0193)
            });
            hash | 0x8000_0000
        })
    }
}

/// The number at the end of `\\.\DISPLAYn`.
fn display_number(device: &str) -> Option<u32> {
    let digits: String = device
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse().ok().filter(|n| *n > 0 && *n < 0x8000_0000)
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _dc: HDC,
    _clip: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: `data` is the `Vec` that `monitors` passed in, alive for the whole call.
    let list = unsafe { &mut *(data as *mut Vec<Monitor>) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: a monitor handed to the callback and a MONITORINFOEXW sized as declared.
    if unsafe {
        GetMonitorInfoW(
            monitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
    } != 0
    {
        list.push(Monitor {
            device: from_wide(&info.szDevice),
            bounds: irect(info.monitorInfo.rcMonitor),
            work: irect(info.monitorInfo.rcWork),
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
    }
    1
}

/// Every monitor in physical pixels, the primary one first and the others in the
/// system's order (`Screen.AllScreens`).
pub(crate) fn monitors() -> Vec<Monitor> {
    let _pixels = PhysicalPixels::enter();
    let mut list: Vec<Monitor> = Vec::new();
    // SAFETY: the callback only touches `list`, which outlives the call.
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect_monitor),
            &mut list as *mut Vec<Monitor> as LPARAM,
        )
    };
    list.sort_by_key(|m| !m.primary);
    list
}

/// `SystemInformation.VirtualScreen`: every monitor together, physical pixels.
pub(crate) fn virtual_bounds(monitors: &[Monitor]) -> IRect {
    let mut iter = monitors.iter();
    let Some(first) = iter.next() else {
        return IRect::default();
    };
    iter.fold(first.bounds, |acc, m| acc.union(&m.bounds))
}

/// The monitors' readable names by GDI device name: the monitor's own name
/// ("DELL U2720Q", from DisplayConfig), else the adapter's description of it
/// ("Generic PnP Monitor"), else the device name. Asked once per device and kept.
#[derive(Default)]
pub(crate) struct MonitorNames {
    known: Mutex<HashMap<String, String>>,
}

impl MonitorNames {
    pub(crate) fn name(&self, device: &str) -> String {
        let mut known = self.known.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(name) = known.get(device) {
            return name.clone();
        }
        let friendly = friendly_names();
        for (gdi, name) in &friendly {
            known.entry(gdi.clone()).or_insert_with(|| name.clone());
        }
        let name = friendly
            .get(device)
            .cloned()
            .or_else(|| adapter_monitor_name(device))
            .unwrap_or_else(|| device.trim_start_matches(r"\\.\").to_string());
        known.insert(device.to_string(), name.clone());
        name
    }
}

/// GDI device name to the monitor's friendly name, for every active display path.
fn friendly_names() -> HashMap<String, String> {
    let mut names = HashMap::new();
    let (mut path_count, mut mode_count) = (0u32, 0u32);
    // SAFETY: two out-parameters.
    let sized = unsafe {
        GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
    };
    if sized != ERROR_SUCCESS || path_count == 0 {
        return names;
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
    // SAFETY: buffers of the sizes the system asked for; the counts are written back.
    let queried = unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            std::ptr::null_mut(),
        )
    };
    if queried != ERROR_SUCCESS {
        return names;
    }
    for path in paths.iter().take(path_count as usize) {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        source.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        source.header.adapterId = path.sourceInfo.adapterId;
        source.header.id = path.sourceInfo.id;
        let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
        target.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
        target.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
        target.header.adapterId = path.targetInfo.adapterId;
        target.header.id = path.targetInfo.id;
        // SAFETY: request packets whose headers name their own type and size.
        let ok = unsafe {
            DisplayConfigGetDeviceInfo(&mut source.header) == 0
                && DisplayConfigGetDeviceInfo(&mut target.header) == 0
        };
        if !ok {
            continue;
        }
        let name = from_wide(&target.monitorFriendlyDeviceName);
        if !name.trim().is_empty() {
            names.insert(from_wide(&source.viewGdiDeviceName), name);
        }
    }
    names
}

/// The adapter's description of the monitor on a GDI device (`EnumDisplayDevices`).
fn adapter_monitor_name(device: &str) -> Option<String> {
    let mut info = DISPLAY_DEVICEW {
        cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
        ..Default::default()
    };
    let device = wide(device);
    // SAFETY: a NUL-terminated device name and a structure sized as declared.
    let found = unsafe { EnumDisplayDevicesW(device.as_ptr(), 0, &mut info, 0) } != 0;
    let name = from_wide(&info.DeviceString);
    (found && !name.trim().is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_and_pixels() {
        assert_eq!(to_points(2560, 1.25), 2048);
        assert_eq!(to_pixels(2048, 1.25), 2560);
        assert_eq!(to_points(-1920, 1.5), -1280);
        // Points survive a round trip at any scale of at least 1.
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0, 2.25, 3.0] {
            for points in -3000..3000 {
                assert_eq!(to_points(to_pixels(points, scale), scale), points);
            }
        }
        // Edges are converted one by one: neighbours keep touching.
        let left = rect_to_points(IRect::new(0, 0, 1707, 1080), 1.5);
        let right = rect_to_points(IRect::new(1707, 0, 1707, 1080), 1.5);
        assert_eq!(left.right(), right.x);
        assert_eq!(
            rect_to_pixels(IRect::new(10, 20, 100, 50), 1.25),
            IRect::new(13, 25, 125, 63)
        );
        assert_eq!(
            rect_to_points(IRect::new(-2560, 0, 2560, 1440), 1.0),
            IRect::new(-2560, 0, 2560, 1440)
        );
    }

    #[test]
    fn display_ids() {
        assert_eq!(display_number(r"\\.\DISPLAY1"), Some(1));
        assert_eq!(display_number(r"\\.\DISPLAY12"), Some(12));
        assert_eq!(display_number(r"\\.\DISPLAY"), None);
        let odd = Monitor {
            device: "WinDisc".into(),
            bounds: IRect::default(),
            work: IRect::default(),
            primary: false,
        };
        assert!(odd.id() >= 0x8000_0000);
        assert_eq!(odd.id(), odd.clone().id());
    }

    #[test]
    fn the_monitors_have_one_primary() {
        let monitors = monitors();
        assert!(!monitors.is_empty());
        assert_eq!(monitors.iter().filter(|m| m.primary).count(), 1);
        assert!(monitors[0].primary);
        // The primary monitor's top left is the desktop's origin.
        assert_eq!((monitors[0].bounds.x, monitors[0].bounds.y), (0, 0));
        for monitor in &monitors {
            assert!(monitor.bounds.has_area());
            assert!(monitor.bounds.contains_rect(&monitor.work));
        }
        let scale = primary_scale();
        assert!((1.0..=5.0).contains(&scale), "{scale}");
        let names = MonitorNames::default();
        for monitor in &monitors {
            assert!(!names.name(&monitor.device).is_empty());
        }
    }
}
