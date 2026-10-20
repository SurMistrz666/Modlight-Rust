//! Moving another app's window through the Accessibility API, for "Active window
//! borderless": the active app's focused window is sized to a frame the caller picks and,
//! the second time, put back where it was. macOS asks the person to allow Modlight under
//! Privacy & Security, Accessibility the first time; automated runs never come here.
//!
//! Frames are (x, y, width, height) in points, origin at the top left of the main display.

/// A frame: x, y, width, height.
pub type Rect = (f64, f64, f64, f64);

/// What the toggle did.
#[derive(Clone, Debug, PartialEq)]
pub enum Toggled {
    /// The window now fills the frame the caller picked.
    Filled,
    /// The window is back where it was.
    Restored,
    /// Modlight is not allowed to control other apps yet (the system was asked).
    NotTrusted,
    /// No app or no window is active.
    NoWindow,
    Failed(String),
}

/// Toggle the active window: fill `pick(window, screens)` (screens as whole and visible
/// frames), or restore it when this hotkey filled it before. Main thread only.
pub fn toggle_active_window(pick: impl FnOnce(Rect, &[(Rect, Rect)]) -> Option<Rect>) -> Toggled {
    #[cfg(target_os = "macos")]
    {
        mac::toggle(pick)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pick;
        Toggled::NoWindow
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;
    use std::sync::Mutex;

    use objc2_app_kit::{NSScreen, NSWorkspace};
    use objc2_core_foundation::CFString;
    use objc2_foundation::{MainThreadMarker, NSDictionary, NSNumber, NSString};

    use super::{Rect, Toggled};

    type Ref = *const c_void;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Point {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Size {
        width: f64,
        height: f64,
    }

    const POINT: u32 = 1; // kAXValueCGPointType
    const SIZE: u32 = 2; // kAXValueCGSizeType

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: Ref) -> bool;
        fn AXUIElementCreateApplication(pid: i32) -> Ref;
        fn AXUIElementCopyAttributeValue(element: Ref, attribute: Ref, value: *mut Ref) -> i32;
        fn AXUIElementSetAttributeValue(element: Ref, attribute: Ref, value: Ref) -> i32;
        fn AXValueCreate(kind: u32, value: *const c_void) -> Ref;
        fn AXValueGetValue(value: Ref, kind: u32, out: *mut c_void) -> bool;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: Ref);
        fn CFEqual(a: Ref, b: Ref) -> u8;
    }

    /// A window this hotkey filled: its element (retained) and where it was.
    struct Remembered {
        element: usize,
        frame: Rect,
    }

    static FILLED: Mutex<Vec<Remembered>> = Mutex::new(Vec::new());

    fn attribute(name: &str) -> objc2_core_foundation::CFRetained<CFString> {
        CFString::from_str(name)
    }

    fn as_ref(s: &CFString) -> Ref {
        s as *const CFString as Ref
    }

    /// Copy an attribute (owned: the caller releases it).
    unsafe fn copy(element: Ref, name: &str) -> Option<Ref> {
        let key = attribute(name);
        let mut value: Ref = std::ptr::null();
        // SAFETY: a valid element and attribute; `value` receives a +1 reference.
        let error = unsafe { AXUIElementCopyAttributeValue(element, as_ref(&key), &mut value) };
        (error == 0 && !value.is_null()).then_some(value)
    }

    unsafe fn frame_of(window: Ref) -> Option<Rect> {
        // SAFETY: AXValues of the documented types, released after reading.
        unsafe {
            let position = copy(window, "AXPosition")?;
            let mut point = Point::default();
            let ok = AXValueGetValue(position, POINT, &mut point as *mut Point as *mut c_void);
            CFRelease(position);
            let size_value = copy(window, "AXSize")?;
            let mut size = Size::default();
            let ok2 = AXValueGetValue(size_value, SIZE, &mut size as *mut Size as *mut c_void);
            CFRelease(size_value);
            (ok && ok2).then_some((point.x, point.y, size.width, size.height))
        }
    }

    unsafe fn set_frame(window: Ref, frame: Rect) -> Result<(), String> {
        let point = Point {
            x: frame.0,
            y: frame.1,
        };
        let size = Size {
            width: frame.2,
            height: frame.3,
        };
        // SAFETY: AXValues made here, released after use. Position, size, position again:
        // a window crossing to a smaller screen is clamped by its old size otherwise.
        unsafe {
            let position = AXValueCreate(POINT, &point as *const Point as *const c_void);
            let size_value = AXValueCreate(SIZE, &size as *const Size as *const c_void);
            let position_key = attribute("AXPosition");
            let size_key = attribute("AXSize");
            let a = AXUIElementSetAttributeValue(window, as_ref(&position_key), position);
            let b = AXUIElementSetAttributeValue(window, as_ref(&size_key), size_value);
            let c = AXUIElementSetAttributeValue(window, as_ref(&position_key), position);
            CFRelease(position);
            CFRelease(size_value);
            if a != 0 && b != 0 && c != 0 {
                return Err(format!("the window did not move (AXError {b})"));
            }
        }
        Ok(())
    }

    /// The screens as (whole, visible) frames, top-left origin.
    fn screens(mtm: MainThreadMarker) -> Vec<(Rect, Rect)> {
        let list = NSScreen::screens(mtm);
        let Some(primary) = list.firstObject() else {
            return Vec::new();
        };
        let top = primary.frame().size.height;
        let flip = |f: objc2_foundation::NSRect| {
            (
                f.origin.x,
                top - (f.origin.y + f.size.height),
                f.size.width,
                f.size.height,
            )
        };
        list.iter()
            .map(|s| (flip(s.frame()), flip(s.visibleFrame())))
            .collect()
    }

    pub fn toggle(pick: impl FnOnce(Rect, &[(Rect, Rect)]) -> Option<Rect>) -> Toggled {
        let Some(mtm) = MainThreadMarker::new() else {
            return Toggled::Failed("not on the main thread".into());
        };
        // SAFETY: a plain query.
        if !unsafe { AXIsProcessTrusted() } {
            // Ask once, with the system's own prompt.
            let key = NSString::from_str("AXTrustedCheckOptionPrompt");
            let yes = NSNumber::numberWithBool(true);
            let options = NSDictionary::from_slices(&[&*key], &[&*yes]);
            // SAFETY: an NSDictionary is a CFDictionary (toll-free bridged).
            unsafe {
                AXIsProcessTrustedWithOptions(
                    &*options as *const NSDictionary<NSString, NSNumber> as Ref,
                )
            };
            return Toggled::NotTrusted;
        }
        let Some(app) = NSWorkspace::sharedWorkspace().frontmostApplication() else {
            return Toggled::NoWindow;
        };
        let pid = app.processIdentifier();
        // SAFETY: AX elements created or copied here are released on every path.
        unsafe {
            let application = AXUIElementCreateApplication(pid);
            if application.is_null() {
                return Toggled::NoWindow;
            }
            let window = copy(application, "AXFocusedWindow");
            CFRelease(application);
            let Some(window) = window else {
                return Toggled::NoWindow;
            };
            // `if (window.IsMinimized) window.Restore()`.
            if let Some(minimized) = copy(window, "AXMinimized") {
                let is = (*(minimized as *const NSNumber)).boolValue();
                CFRelease(minimized);
                if is {
                    let key = attribute("AXMinimized");
                    let no = NSNumber::numberWithBool(false);
                    AXUIElementSetAttributeValue(
                        window,
                        as_ref(&key),
                        &*no as *const NSNumber as Ref,
                    );
                }
            }
            let mut filled = FILLED.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(index) = filled
                .iter()
                .position(|r| CFEqual(r.element as Ref, window) != 0)
            {
                let remembered = filled.remove(index);
                let result = set_frame(window, remembered.frame);
                CFRelease(remembered.element as Ref);
                CFRelease(window);
                return match result {
                    Ok(()) => Toggled::Restored,
                    Err(error) => Toggled::Failed(error),
                };
            }
            let Some(current) = frame_of(window) else {
                CFRelease(window);
                return Toggled::Failed("the window has no position".into());
            };
            let Some(target) = pick(current, &screens(mtm)) else {
                CFRelease(window);
                return Toggled::NoWindow;
            };
            match set_frame(window, target) {
                Ok(()) => {
                    // Kept (with the reference copied above) until it is put back.
                    filled.push(Remembered {
                        element: window as usize,
                        frame: current,
                    });
                    Toggled::Filled
                }
                Err(error) => {
                    CFRelease(window);
                    Toggled::Failed(error)
                }
            }
        }
    }
}
