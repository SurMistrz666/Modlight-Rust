//! Pixels through GDI (`Screenshot.CaptureRectangleGDI`) and the pointer (`CursorData`).
//!
//! The screen is copied with `BitBlt(SRCCOPY | CAPTUREBLT)` from the screen's device
//! context into a 32-bit top-down DIB section, whose memory is BGRA in rows of
//! `width * 4` bytes: the recorder's frame layout as it is. Coordinates are physical
//! pixels (the caller holds [`PhysicalPixels`](super::desktop::PhysicalPixels)).

use image::RgbaImage;
use modlight_core::capture::geometry::IRect;
use modlight_core::capture::source::CaptureError;
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, GetDC, GetObjectW, HBITMAP,
    HDC, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
};
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CURSOR_SHOWING, CURSORINFO, CopyIcon, DI_NORMAL, DestroyIcon, DrawIconEx, GetCursorInfo,
    GetIconInfo, HICON, ICONINFO,
};

use super::desktop::{monitors, virtual_bounds};
use crate::wide::wide;

/// The screen's device context, released when dropped.
pub(crate) struct ScreenDc(HDC);

impl ScreenDc {
    pub(crate) fn new() -> Option<ScreenDc> {
        // SAFETY: the DC of the whole screen; released in `drop`.
        let dc = unsafe { GetDC(std::ptr::null_mut()) };
        (!dc.is_null()).then_some(ScreenDc(dc))
    }
}

impl Drop for ScreenDc {
    fn drop(&mut self) {
        // SAFETY: the DC `GetDC(NULL)` returned.
        unsafe { ReleaseDC(std::ptr::null_mut(), self.0) };
    }
}

/// A 32-bit top-down DIB section selected into a memory DC.
pub(crate) struct Dib {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u8,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

// SAFETY: the DC and the bitmap belong to this value alone and are used by one thread at
// a time (the owner's).
unsafe impl Send for Dib {}

impl Dib {
    /// A black, fully transparent picture of the given size.
    pub(crate) fn new(width: u32, height: u32) -> Option<Dib> {
        if width == 0 || height == 0 || width > 32768 || height > 32768 {
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                // Negative: rows top to bottom.
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: a memory DC and a DIB section, released in `drop` (or right here when
        // one of them cannot be made).
        unsafe {
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            if dc.is_null() {
                return None;
            }
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(
                dc,
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            if bitmap.is_null() || bits.is_null() {
                if !bitmap.is_null() {
                    DeleteObject(bitmap);
                }
                DeleteDC(dc);
                return None;
            }
            let previous = SelectObject(dc, bitmap);
            Some(Dib {
                dc,
                bitmap,
                previous,
                bits: bits as *mut u8,
                width,
                height,
            })
        }
    }

    pub(crate) fn dc(&self) -> HDC {
        self.dc
    }

    /// The pixels, BGRA, after every GDI call drawing into them has finished.
    pub(crate) fn bgra(&self) -> &[u8] {
        // SAFETY: GDI batches calls per thread; flushing makes the memory current. The
        // section holds width * height * 4 bytes (32 bits per pixel, rows DWORD aligned).
        unsafe {
            GdiFlush();
            std::slice::from_raw_parts(self.bits, (self.width * self.height * 4) as usize)
        }
    }

    pub(crate) fn bgra_mut(&mut self) -> &mut [u8] {
        // SAFETY: as `bgra`; `&mut self` makes the view exclusive.
        unsafe {
            GdiFlush();
            std::slice::from_raw_parts_mut(self.bits, (self.width * self.height * 4) as usize)
        }
    }

    /// Every pixel set to one BGRA value.
    pub(crate) fn fill(&mut self, bgra: [u8; 4]) {
        for pixel in self.bgra_mut().as_chunks_mut::<4>().0 {
            *pixel = bgra;
        }
    }

    /// Copy `part` of the screen (physical pixels) to (x, y) of this picture.
    pub(crate) fn blit_screen(&self, screen: &ScreenDc, part: IRect, x: i32, y: i32) -> bool {
        // SAFETY: two valid DCs; GDI clips to the destination bitmap.
        unsafe {
            BitBlt(
                self.dc,
                x,
                y,
                part.width,
                part.height,
                screen.0,
                part.x,
                part.y,
                SRCCOPY | CAPTUREBLT,
            ) != 0
        }
    }

    /// The pixels as straight RGBA, opaque (the screen has no transparency; GDI leaves
    /// the fourth byte undefined).
    pub(crate) fn to_rgba(&self) -> RgbaImage {
        let mut out = Vec::with_capacity((self.width * self.height * 4) as usize);
        for pixel in self.bgra().as_chunks::<4>().0 {
            out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
        RgbaImage::from_raw(self.width, self.height, out).expect("the DIB's own size")
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        // SAFETY: the objects `new` made, deselected before they are deleted.
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

/// `Screenshot.CaptureRectangleNative`: an area of the desktop in physical pixels, with
/// the pointer drawn in when asked. What lies outside every monitor stays black.
pub(crate) fn capture_pixels(rect: IRect, cursor: bool) -> Result<RgbaImage, CaptureError> {
    let part = rect.intersect(&virtual_bounds(&monitors()));
    if !rect.has_area() || !part.has_area() {
        return Err(CaptureError::Nothing);
    }
    let dib = Dib::new(rect.width as u32, rect.height as u32)
        .ok_or_else(|| CaptureError::Failed("the picture could not be made".into()))?;
    let screen =
        ScreenDc::new().ok_or_else(|| CaptureError::Failed("the screen is not readable".into()))?;
    if !dib.blit_screen(&screen, part, part.x - rect.x, part.y - rect.y) {
        return Err(CaptureError::Failed(
            "the screen could not be copied".into(),
        ));
    }
    if cursor && let Some(cursor) = Cursor::current() {
        cursor.draw(dib.dc(), rect.x, rect.y);
    }
    Ok(dib.to_rgba())
}

/// `CursorData.GetCursorSizeMultiplier`: the accessibility pointer size (1 to 15) as a
/// factor, 1 + (size - 1) / 2.
fn cursor_size_multiplier() -> f64 {
    let key = wide(r"SOFTWARE\Microsoft\Accessibility");
    let value = wide("CursorSize");
    let mut data = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: NUL-terminated names and a DWORD with its size.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut data as *mut u32 as *mut _,
            &mut size,
        )
    };
    if result == 0 && data > 1 {
        1.0 + (data - 1) as f64 * 0.5
    } else {
        1.0
    }
}

/// The pointer as it is now (`CursorData`): a copy of its picture, where it is, its hot
/// spot and the size it is drawn at.
pub(crate) struct Cursor {
    icon: HICON,
    /// Where the pointer points, physical pixels.
    position: POINT,
    hotspot: (i32, i32),
    /// The picture's own size.
    natural: (i32, i32),
    /// The accessibility pointer size (`SizeMultiplier`).
    multiplier: f64,
}

impl Cursor {
    /// The pointer, when it is shown.
    pub(crate) fn current() -> Option<Cursor> {
        let mut info = CURSORINFO {
            cbSize: std::mem::size_of::<CURSORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: a CURSORINFO sized as declared; the copied icon and the bitmaps of its
        // ICONINFO are released here or by `drop`.
        unsafe {
            if GetCursorInfo(&mut info) == 0
                || info.flags & CURSOR_SHOWING == 0
                || info.hCursor.is_null()
            {
                return None;
            }
            let icon = CopyIcon(info.hCursor);
            if icon.is_null() {
                return None;
            }
            let mut icon_info = ICONINFO::default();
            if GetIconInfo(icon, &mut icon_info) == 0 {
                DestroyIcon(icon);
                return None;
            }
            let mut mask = BITMAP::default();
            let mut natural = (32, 32);
            if !icon_info.hbmMask.is_null()
                && GetObjectW(
                    icon_info.hbmMask,
                    std::mem::size_of::<BITMAP>() as i32,
                    &mut mask as *mut BITMAP as *mut _,
                ) != 0
            {
                // A monochrome cursor stacks its AND and XOR masks in one bitmap.
                let height = if icon_info.hbmColor.is_null() {
                    mask.bmHeight / 2
                } else {
                    mask.bmHeight
                };
                natural = (mask.bmWidth.max(1), height.max(1));
            }
            if !icon_info.hbmColor.is_null() {
                DeleteObject(icon_info.hbmColor);
            }
            if !icon_info.hbmMask.is_null() {
                DeleteObject(icon_info.hbmMask);
            }
            Some(Cursor {
                icon,
                position: info.ptScreenPos,
                hotspot: (icon_info.xHotspot as i32, icon_info.yHotspot as i32),
                natural,
                multiplier: cursor_size_multiplier(),
            })
        }
    }

    fn is_default_size(&self) -> bool {
        self.multiplier == 1.0
    }

    /// The size the pointer is drawn at.
    pub(crate) fn size(&self) -> (i32, i32) {
        if self.is_default_size() {
            self.natural
        } else {
            (
                (self.natural.0 as f64 * self.multiplier).round() as i32,
                (self.natural.1 as f64 * self.multiplier).round() as i32,
            )
        }
    }

    /// `CursorData.DrawPosition`: the picture's top left on the desktop, physical pixels.
    pub(crate) fn top_left(&self) -> (i32, i32) {
        let (hx, hy) = if self.is_default_size() {
            self.hotspot
        } else {
            (
                (self.hotspot.0 as f64 * self.multiplier).round() as i32,
                (self.hotspot.1 as f64 * self.multiplier).round() as i32,
            )
        };
        (self.position.x - hx, self.position.y - hy)
    }

    /// `CursorData.DrawCursor`: draw into a DC whose (0, 0) is the desktop's
    /// (`origin_x`, `origin_y`). Inverting pointers (the text beam) invert what is under
    /// them, as on screen.
    pub(crate) fn draw(&self, dc: HDC, origin_x: i32, origin_y: i32) {
        let (x, y) = self.top_left();
        // The natural size is drawn as 0 x 0 ("the picture's own size"), as upstream.
        let (width, height) = if self.is_default_size() {
            (0, 0)
        } else {
            self.size()
        };
        // SAFETY: a valid DC and icon.
        unsafe {
            DrawIconEx(
                dc,
                x - origin_x,
                y - origin_y,
                self.icon,
                width,
                height,
                0,
                std::ptr::null_mut(),
                DI_NORMAL,
            )
        };
    }

    /// The pointer by itself with transparency, for the region capture to draw: drawn
    /// once on black and once on white, the difference is its alpha (the same trick as
    /// `Screenshot.CaptureWindowTransparent`). A pixel that inverts what is under it has
    /// no colour of its own; it is drawn black, as it shows over the usual light page.
    pub(crate) fn image(&self) -> Option<RgbaImage> {
        let (width, height) = self.size();
        let mut black = Dib::new(width as u32, height as u32)?;
        let mut white = Dib::new(width as u32, height as u32)?;
        black.fill([0, 0, 0, 255]);
        white.fill([255, 255, 255, 255]);
        let (x, y) = self.top_left();
        self.draw(black.dc(), x, y);
        self.draw(white.dc(), x, y);
        Some(alpha_from_backgrounds(
            black.bgra(),
            white.bgra(),
            width as u32,
            height as u32,
            AlphaRule::Cursor,
        ))
    }
}

impl Drop for Cursor {
    fn drop(&mut self) {
        // SAFETY: the copy `CopyIcon` made.
        unsafe { DestroyIcon(self.icon) };
    }
}

/// How [`alpha_from_backgrounds`] reads a pixel.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum AlphaRule {
    /// `Screenshot.CreateTransparentImage`: alpha from the red channel, colour from the
    /// black picture.
    Window,
    /// The pointer: alpha from the mean of the channels; inverting pixels black.
    Cursor,
}

/// A picture with transparency from the same thing taken over black and over white
/// (both BGRA): what shows the white through is transparent.
pub(crate) fn alpha_from_backgrounds(
    black: &[u8],
    white: &[u8],
    width: u32,
    height: u32,
    rule: AlphaRule,
) -> RgbaImage {
    let mut out = RgbaImage::new(width, height);
    for ((b, w), pixel) in black
        .as_chunks::<4>()
        .0
        .iter()
        .zip(white.as_chunks::<4>().0)
        .zip(out.pixels_mut())
    {
        let (bb, bg, br) = (b[0] as i32, b[1] as i32, b[2] as i32);
        let (wb, wg, wr) = (w[0] as i32, w[1] as i32, w[2] as i32);
        match rule {
            AlphaRule::Window => {
                // `double alpha = (black.Red - white.Red + 255) / 255.0`.
                let alpha = (br - wr + 255) as f64 / 255.0;
                if alpha >= 1.0 {
                    *pixel = image::Rgba([wr as u8, wg as u8, wb as u8, 255]);
                } else if alpha > 0.0 {
                    let un = |c: i32| (c as f64 / alpha).min(255.0) as u8;
                    *pixel = image::Rgba([un(br), un(bg), un(bb), (255.0 * alpha) as u8]);
                }
            }
            AlphaRule::Cursor => {
                let through = ((wr - br) + (wg - bg) + (wb - bb)) / 3;
                if through < 0 {
                    *pixel = image::Rgba([0, 0, 0, 255]);
                    continue;
                }
                let alpha = 255 - through.min(255);
                if alpha > 0 {
                    let un = |c: i32| ((c * 255 + alpha / 2) / alpha).clamp(0, 255) as u8;
                    *pixel = image::Rgba([un(br), un(bg), un(bb), alpha as u8]);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::desktop::PhysicalPixels;

    #[test]
    fn transparency_from_two_backgrounds() {
        // Opaque red, half transparent blue, fully transparent, an inverting pixel.
        let black = [
            0, 0, 255, 255, 128, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255,
        ];
        let white = [
            0, 0, 255, 255, 255, 127, 127, 255, 255, 255, 255, 255, 0, 0, 0, 255,
        ];
        let cursor = alpha_from_backgrounds(&black, &white, 4, 1, AlphaRule::Cursor);
        assert_eq!(cursor.get_pixel(0, 0).0, [255, 0, 0, 255]);
        let half = cursor.get_pixel(1, 0).0;
        assert_eq!(half[3], 128);
        assert!(half[2] >= 254 && half[0] == 0, "{half:?}");
        assert_eq!(cursor.get_pixel(2, 0).0[3], 0);
        assert_eq!(cursor.get_pixel(3, 0).0, [0, 0, 0, 255]);
        let window = alpha_from_backgrounds(&black, &white, 4, 1, AlphaRule::Window);
        assert_eq!(window.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(window.get_pixel(1, 0).0[3], 128);
        assert_eq!(window.get_pixel(2, 0).0[3], 0);
    }

    #[test]
    fn a_small_area_of_the_screen() {
        let _pixels = PhysicalPixels::enter();
        let picture = capture_pixels(IRect::new(10, 20, 33, 17), false).unwrap();
        assert_eq!(picture.dimensions(), (33, 17));
        assert!(picture.pixels().all(|p| p[3] == 255));
        // Half outside the desktop: the size asked for, the outside black.
        let bounds = virtual_bounds(&monitors());
        let rect = IRect::new(bounds.x - 8, bounds.y, 16, 4);
        let picture = capture_pixels(rect, false).unwrap();
        assert_eq!(picture.dimensions(), (16, 4));
        assert_eq!(picture.get_pixel(0, 0).0, [0, 0, 0, 255]);
        // With the pointer drawn in (wherever it is): the same size.
        let picture = capture_pixels(IRect::new(0, 0, 64, 48), true).unwrap();
        assert_eq!(picture.dimensions(), (64, 48));
        assert_eq!(
            capture_pixels(IRect::new(bounds.right() + 10, 0, 5, 5), false),
            Err(CaptureError::Nothing)
        );
    }

    #[test]
    fn the_pointer_picture() {
        let _pixels = PhysicalPixels::enter();
        // A desktop without a pointer (a remote session, a hidden pointer) has none.
        let Some(cursor) = Cursor::current() else {
            return;
        };
        let (width, height) = cursor.size();
        assert!(width > 0 && height > 0);
        let picture = cursor.image().unwrap();
        assert_eq!(picture.dimensions(), (width as u32, height as u32));
    }
}
