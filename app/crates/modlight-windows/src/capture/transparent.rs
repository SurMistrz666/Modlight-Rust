//! `Screenshot.CaptureWindowTransparent`: a window by itself, with its rounded corners
//! and (when asked) its shadow, on a transparent background.
//!
//! GDI only sees the composed desktop, so the window is taken three times with a
//! borderless backdrop window of this app placed right behind it: over white, over black
//! and over white again. Where the two backgrounds show through, the difference between
//! the pictures is the transparency. When the two white pictures differ (the window
//! changed meanwhile, an animation) the last one is kept, opaque, as upstream does.
//!
//! The backdrop is a real window on the desktop for a few tens of milliseconds; it is
//! made on the calling thread (a capture worker), never activated, and destroyed before
//! this returns.

use std::sync::OnceLock;
use std::time::Duration;

use image::{Rgba, RgbaImage, imageops};
use modlight_core::capture::geometry::IRect;
use modlight_core::capture::source::CaptureError;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Dwm::{DwmFlush, DwmIsCompositionEnabled};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HDC, InvalidateRect,
    PAINTSTRUCT, UpdateWindow,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetClientRect,
    GetWindowLongPtrW, IsZoomed, MSG, PM_REMOVE, PeekMessageW, RegisterClassExW, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TranslateMessage, WM_ERASEBKGND, WM_PAINT, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_POPUP,
};

use super::desktop::{monitors, virtual_bounds};
use super::gdi::{AlphaRule, Cursor, Dib, ScreenDc, alpha_from_backgrounds};
use super::winlist::{blend_over, window_rect};
use crate::wide::{from_wide, wide};

/// `Screenshot.ShadowOffset`: how far around the window its shadow is taken.
const SHADOW_OFFSET: i32 = 20;

const WHITE: u32 = 0x00FF_FFFF;
const BLACK: u32 = 0x0000_0000;

/// The window class of the backdrop, registered once per process.
fn backdrop_class() -> Option<&'static [u16]> {
    static CLASS: OnceLock<Option<Vec<u16>>> = OnceLock::new();
    CLASS
        .get_or_init(|| {
            let name = wide("ModlightTransparentCaptureBackdrop");
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(backdrop_proc),
                // SAFETY: this module's handle.
                hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
                lpszClassName: name.as_ptr(),
                ..Default::default()
            };
            // SAFETY: a filled class description; the name is copied by the system.
            let registered = unsafe { RegisterClassExW(&class) } != 0
                || unsafe { GetLastError() } == ERROR_CLASS_ALREADY_EXISTS;
            registered.then_some(name)
        })
        .as_deref()
}

/// The backdrop paints itself in the colour kept in its user data.
unsafe extern "system" fn backdrop_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: called by the system for a backdrop window with valid parameters.
    unsafe {
        match message {
            WM_ERASEBKGND => {
                let color = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as u32;
                let mut rect = RECT::default();
                GetClientRect(hwnd, &mut rect);
                let brush = CreateSolidBrush(color);
                FillRect(wparam as HDC, &rect, brush);
                DeleteObject(brush);
                1
            }
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut paint);
                EndPaint(hwnd, &paint);
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

/// Handle the messages waiting for one window of this thread.
fn pump(hwnd: HWND) {
    let mut message = MSG::default();
    // SAFETY: a message loop over one window this thread made.
    unsafe {
        while PeekMessageW(&mut message, hwnd, 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

/// The plain-coloured window behind the one being taken (upstream's borderless `Form`).
struct Backdrop(HWND);

impl Backdrop {
    /// Show it over `rect` right below `target` in the z-order, without activating it.
    fn new(rect: IRect, target: HWND) -> Option<Backdrop> {
        let class = backdrop_class()?;
        let title = wide("");
        // SAFETY: a registered class; the window is destroyed by `drop`.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return None;
        }
        let backdrop = Backdrop(hwnd);
        // SAFETY: plain calls on the window just made.
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, WHITE as isize);
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            if SetWindowPos(
                hwnd,
                target,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            ) == 0
            {
                log::debug!("Transparent capture failed. Reason: SetWindowPos fail.");
                return None;
            }
        }
        backdrop.settle();
        Some(backdrop)
    }

    fn set_color(&self, color: u32) {
        // SAFETY: plain calls on this thread's own window.
        unsafe {
            SetWindowLongPtrW(self.0, GWLP_USERDATA, color as isize);
            InvalidateRect(self.0, std::ptr::null(), 1);
            UpdateWindow(self.0);
        }
        self.settle();
    }

    /// `Application.DoEvents(); Thread.Sleep(10)`, and a wait for the compositor to
    /// show the new colour.
    fn settle(&self) {
        pump(self.0);
        // SAFETY: a plain call; it returns after the next composition.
        unsafe { DwmFlush() };
        std::thread::sleep(Duration::from_millis(10));
    }
}

impl Drop for Backdrop {
    fn drop(&mut self) {
        // SAFETY: the window `new` made on this thread.
        unsafe { DestroyWindow(self.0) };
        pump(self.0);
    }
}

fn dwm_enabled() -> bool {
    let mut enabled = 0;
    // SAFETY: an out-parameter.
    unsafe { DwmIsCompositionEnabled(&mut enabled) >= 0 && enabled != 0 }
}

/// Whether this is Windows 11 or later (build 22000), whose shadows `TrimShadow` treats
/// differently.
fn is_windows_11() -> bool {
    let key = wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let value = wide("CurrentBuildNumber");
    let mut buffer = [0u16; 32];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    // SAFETY: NUL-terminated names and a buffer with its size in bytes.
    let result = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr() as *mut _,
            &mut size,
        )
    };
    result == 0
        && from_wide(&buffer)
            .trim()
            .parse::<u32>()
            .is_ok_and(|build| build >= 22000)
}

fn grab(screen: &ScreenDc, rect: IRect) -> Result<Dib, CaptureError> {
    let dib = Dib::new(rect.width as u32, rect.height as u32)
        .ok_or_else(|| CaptureError::Failed("the picture could not be made".into()))?;
    let part = rect.intersect(&virtual_bounds(&monitors()));
    if part.has_area() {
        dib.blit_screen(screen, part, part.x - rect.x, part.y - rect.y);
    }
    Ok(dib)
}

/// The colours of two BGRA pictures are the same (the fourth byte is GDI's own).
fn same_colors(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len()
        && a.as_chunks::<4>()
            .0
            .iter()
            .zip(b.as_chunks::<4>().0)
            .all(|(p, q)| p[..3] == q[..3])
}

/// `Screenshot.CaptureWindowTransparent(handle)`: physical pixels, the window as it is
/// in front of the desktop now. `None` when the backdrop could not be put behind it
/// (upstream then takes the window's area as it is).
pub(crate) fn capture(
    hwnd: HWND,
    shadow: bool,
    cursor: bool,
) -> Option<Result<RgbaImage, CaptureError>> {
    let mut rect = window_rect(hwnd)?;
    // SAFETY: a plain query.
    if shadow && unsafe { IsZoomed(hwnd) } == 0 && dwm_enabled() {
        rect = IRect::new(
            rect.x - SHADOW_OFFSET,
            rect.y - SHADOW_OFFSET,
            rect.width + 2 * SHADOW_OFFSET,
            rect.height + 2 * SHADOW_OFFSET,
        );
        let screens = monitors()
            .into_iter()
            .map(|m| m.bounds)
            .filter(|b| b.intersect(&rect).has_area())
            .reduce(|a, b| a.union(&b))
            .unwrap_or_default();
        rect = rect.intersect(&screens);
    }
    if !rect.has_area() {
        return Some(Err(CaptureError::Nothing));
    }
    let cursor = if cursor { Cursor::current() } else { None };
    let Some(screen) = ScreenDc::new() else {
        return Some(Err(CaptureError::Failed(
            "the screen is not readable".into(),
        )));
    };
    let backdrop = Backdrop::new(rect, hwnd)?;
    let pictures = (|| {
        let white = grab(&screen, rect)?;
        backdrop.set_color(BLACK);
        let black = grab(&screen, rect)?;
        backdrop.set_color(WHITE);
        let white2 = grab(&screen, rect)?;
        Ok::<_, CaptureError>((white, black, white2))
    })();
    drop(backdrop);
    let (white, black, white2) = match pictures {
        Ok(pictures) => pictures,
        Err(error) => return Some(Err(error)),
    };
    let transparent = same_colors(white.bgra(), white2.bgra());
    let mut image = if transparent {
        alpha_from_backgrounds(
            black.bgra(),
            white.bgra(),
            rect.width as u32,
            rect.height as u32,
            AlphaRule::Window,
        )
    } else {
        log::debug!("Transparent capture failed. Reason: Images not equal.");
        white2.to_rgba()
    };
    if let Some(cursor) = cursor
        && let Some(picture) = cursor.image()
    {
        let (x, y) = cursor.top_left();
        blend_over(
            &mut image,
            &picture,
            (x - rect.x) as i64,
            (y - rect.y) as i64,
        );
    }
    if transparent {
        image = auto_crop(image);
        if !shadow {
            trim_shadow(&mut image, is_windows_11());
        }
    }
    Some(Ok(image))
}

/// `ColorBgra.Bgra`.
fn bgra_value(pixel: &Rgba<u8>) -> u32 {
    pixel[2] as u32 | (pixel[1] as u32) << 8 | (pixel[0] as u32) << 16 | (pixel[3] as u32) << 24
}

/// `ImageHelpers.FindAutoCropRectangle(bmp)`: the part that differs from the top-left
/// corner (left and top edges) and from the bottom-right corner (right and bottom
/// edges); a transparent corner matches any transparent pixel. (x, y, width, height).
fn auto_crop_rect(image: &RgbaImage) -> (u32, u32, u32, u32) {
    let (width, height) = image.dimensions();
    let mut crop = (0u32, 0u32, width, height);
    if width == 0 || height == 0 {
        return crop;
    }
    let key = |pixel: &Rgba<u8>| {
        let mask = if pixel[3] == 0 {
            0xFF00_0000
        } else {
            0xFFFF_FFFF
        };
        (bgra_value(pixel) & mask, mask)
    };
    let (check, mask) = key(image.get_pixel(0, 0));
    let differs =
        |x: u32, y: u32, check: u32, mask: u32| bgra_value(image.get_pixel(x, y)) & mask != check;
    match (0..width).find(|&x| (0..height).any(|y| differs(x, y, check, mask))) {
        Some(x) => {
            crop.0 = x;
            crop.2 -= x;
        }
        // Every pixel the same colour.
        None => return crop,
    }
    if let Some(y) = (0..height).find(|&y| (0..width).any(|x| differs(x, y, check, mask))) {
        crop.1 = y;
        crop.3 -= y;
    }
    let (check, mask) = key(image.get_pixel(width - 1, height - 1));
    if let Some(x) = (0..width)
        .rev()
        .find(|&x| (0..height).any(|y| differs(x, y, check, mask)))
    {
        crop.2 = x + 1 - crop.0;
    }
    if let Some(y) = (0..height)
        .rev()
        .find(|&y| (0..width).any(|x| differs(x, y, check, mask)))
    {
        crop.3 = y + 1 - crop.1;
    }
    crop
}

/// `ImageHelpers.AutoCropImage`.
fn auto_crop(image: RgbaImage) -> RgbaImage {
    let (x, y, width, height) = auto_crop_rect(&image);
    if (x, y, width, height) == (0, 0, image.width(), image.height()) || width == 0 || height == 0 {
        return image;
    }
    imageops::crop_imm(&image, x, y, width, height).to_image()
}

/// `Screenshot.TrimShadow`: without the shadow, the faint pixels of the shadow left in
/// the rounded corners are cleared (10 pixels into each corner, up to the first solid
/// one). Windows 11's larger corners use lower thresholds.
fn trim_shadow(image: &mut RgbaImage, windows_11: bool) {
    const CORNER: u32 = 10;
    let (width, height) = image.dimensions();
    let mut threshold = 200u8;
    let clear_row = |image: &mut RgbaImage, y: u32, threshold: u8| {
        for x in 0..CORNER.min(width) {
            if image.get_pixel(x, y)[3] < threshold {
                image.put_pixel(x, y, Rgba([0, 0, 0, 0]));
            } else {
                break;
            }
        }
        for x in (width.saturating_sub(CORNER)..width).rev() {
            if image.get_pixel(x, y)[3] < threshold {
                image.put_pixel(x, y, Rgba([0, 0, 0, 0]));
            } else {
                break;
            }
        }
    };
    for i in 0..CORNER.min(height) {
        if windows_11 {
            threshold = 75;
        }
        clear_row(image, i, threshold);
        if windows_11 {
            threshold = 123;
        }
        clear_row(image, height - i - 1, threshold);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_to_what_is_not_background() {
        let mut image = RgbaImage::new(20, 10);
        for y in 3..7 {
            for x in 5..12 {
                image.put_pixel(x, y, Rgba([200, 10, 10, 255]));
            }
        }
        assert_eq!(auto_crop_rect(&image), (5, 3, 7, 4));
        assert_eq!(auto_crop(image).dimensions(), (7, 4));
        // All one colour: nothing to crop.
        let plain = RgbaImage::from_pixel(4, 4, Rgba([1, 2, 3, 255]));
        assert_eq!(auto_crop_rect(&plain), (0, 0, 4, 4));
        // Transparent pixels of any colour count as background.
        let mut faint = RgbaImage::from_pixel(6, 6, Rgba([9, 9, 9, 0]));
        faint.put_pixel(2, 2, Rgba([0, 0, 0, 255]));
        faint.put_pixel(0, 5, Rgba([50, 0, 0, 0]));
        assert_eq!(auto_crop_rect(&faint), (2, 2, 1, 1));
    }

    #[test]
    fn faint_corners_are_cleared() {
        let mut image = RgbaImage::from_pixel(30, 30, Rgba([10, 20, 30, 255]));
        image.put_pixel(0, 0, Rgba([10, 20, 30, 100]));
        image.put_pixel(1, 0, Rgba([10, 20, 30, 150]));
        image.put_pixel(29, 29, Rgba([10, 20, 30, 100]));
        let mut older = image.clone();
        trim_shadow(&mut older, false);
        assert_eq!(older.get_pixel(0, 0)[3], 0);
        assert_eq!(older.get_pixel(1, 0)[3], 0);
        assert_eq!(older.get_pixel(29, 29)[3], 0);
        trim_shadow(&mut image, true);
        // Windows 11, top: below 75 only; the first solid pixel stops the row.
        assert_eq!(image.get_pixel(0, 0)[3], 100);
        // Bottom: below 123.
        assert_eq!(image.get_pixel(29, 29)[3], 0);
        // Read from the registry; both answers are fine here.
        let _ = is_windows_11();
    }

    #[test]
    fn colours_compare_without_gdi_alpha() {
        assert!(same_colors(&[1, 2, 3, 0], &[1, 2, 3, 255]));
        assert!(!same_colors(&[1, 2, 3, 0], &[1, 2, 4, 0]));
    }
}
