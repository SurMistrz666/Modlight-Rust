//! What the image editor asks of the system: the cursors a cursor annotation shows
//! (the system's own cursors, as upstream's `WindowsCursorBitmapRenderer`), the desktop
//! picture behind the Wallpaper background (`WindowsDesktopWallpaperService`), and
//! pictures the `image` crate cannot read (through WIC).

use std::path::{Path, PathBuf};

use image::RgbaImage;

/// How the desktop picture is laid out, in the macOS port's terms (`NSImageScaling`:
/// 0 proportionally down, 1 axes independently, 2 none, 3 proportionally up or down).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WallpaperOptions {
    pub scaling: i64,
    pub allow_clipping: bool,
}

/// The cursor a `CursorType` name stands for, as a picture.
pub fn cursor_image(cursor_type: &str) -> Option<RgbaImage> {
    cursors::image(cursor_type)
}

/// The desktop picture and its layout.
pub fn desktop_wallpaper() -> Option<(PathBuf, WallpaperOptions)> {
    let path = wallpaper::path()?;
    let style = desktop_wallpaper_style().unwrap_or(WallpaperStyle::Fill);
    Some((path, style.options()))
}

/// A picture file read by WIC (HEIC, JPEG XR and the other codecs Windows has).
pub fn load_image(path: &Path) -> Option<RgbaImage> {
    match wic::load(path) {
        Ok(image) => Some(image),
        Err(error) => {
            log::warn!("Modlight: WIC could not open {}: {error}", path.display());
            None
        }
    }
}

/// Windows' own layout of the desktop picture (Settings, Personalisation, Background,
/// "Choose a fit"), from `HKCU\Control Panel\Desktop`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallpaperStyle {
    Fill,
    Fit,
    Stretch,
    Tile,
    Center,
    Span,
}

impl WallpaperStyle {
    /// From the registry's `WallpaperStyle` and `TileWallpaper` strings: "10" Fill,
    /// "6" Fit, "2" Stretch, "22" Span, "0" Center, or Tile when `TileWallpaper` is "1";
    /// anything else is Fill (the layout upstream always reported).
    pub fn from_registry(style: &str, tile: &str) -> WallpaperStyle {
        match (style.trim(), tile.trim()) {
            (_, "1") => WallpaperStyle::Tile,
            ("0", _) => WallpaperStyle::Center,
            ("2", _) => WallpaperStyle::Stretch,
            ("6", _) => WallpaperStyle::Fit,
            ("22", _) => WallpaperStyle::Span,
            _ => WallpaperStyle::Fill,
        }
    }

    /// The layout in the macOS port's terms, which the editor maps on to its own
    /// (none = Center, axes independently = Stretch, up or down with clipping = Fill,
    /// without = Fit). Two have no such term and take the nearest: Tile is the picture
    /// at its own size (Center), Span is one screen's share of a picture spread over all
    /// of them (Fill).
    pub fn options(self) -> WallpaperOptions {
        let (scaling, allow_clipping) = match self {
            WallpaperStyle::Fill | WallpaperStyle::Span => (3, true),
            WallpaperStyle::Fit => (3, false),
            WallpaperStyle::Stretch => (1, false),
            WallpaperStyle::Center | WallpaperStyle::Tile => (2, false),
        };
        WallpaperOptions {
            scaling,
            allow_clipping,
        }
    }
}

/// Windows' layout of the desktop picture, for a caller that can show all six.
pub fn desktop_wallpaper_style() -> Option<WallpaperStyle> {
    let style = wallpaper::desktop_string("WallpaperStyle")?;
    let tile = wallpaper::desktop_string("TileWallpaper").unwrap_or_default();
    Some(WallpaperStyle::from_registry(&style, &tile))
}

/// The system cursors, read as upstream's renderer reads them: the handle `LoadCursorW`
/// gives for the kind, its colour and mask bitmaps (`GetIconInfo`) read back with
/// `GetDIBits`. A cursor with an alpha channel keeps it; a colour cursor without one is
/// opaque where its mask is clear; a black-and-white cursor is its two masks (black,
/// white, clear). The pixels such cursors invert on screen are drawn black, as they show
/// over the light backgrounds screenshots mostly have.
mod cursors {
    use image::RgbaImage;
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
        DeleteObject, GetDIBits, GetObjectW, HBITMAP, HDC,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetIconInfo, HCURSOR, ICONINFO, IDC_APPSTARTING, IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_HELP,
        IDC_IBEAM, IDC_NO, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE,
        IDC_UPARROW, IDC_WAIT, LoadCursorW,
    };
    use windows_sys::core::PCWSTR;

    /// The resource of a kind (`FallbackCursorIds`), and the standard cursor that stands
    /// in when the system has no such resource. The no-move and pan kinds are the
    /// autoscroll cursors user32 carries (32652 up and down, 32653 left and right, 32654
    /// to 32662 the rest). Upstream drew WinForms' own cursors first, so each kind looks
    /// as WinForms draws it: no-move horizontal is the left-right one (upstream's fallback
    /// table has the two swapped), and the splitters, which WinForms draws as resize
    /// arrows between two bars, are the system's resize arrows.
    fn resource(cursor_type: &str) -> Option<(u16, PCWSTR)> {
        let id = |c: PCWSTR| c as usize as u16;
        Some(match cursor_type {
            "AppStarting" => (id(IDC_APPSTARTING), IDC_APPSTARTING),
            "Arrow" | "Default" => (id(IDC_ARROW), IDC_ARROW),
            "Cross" => (id(IDC_CROSS), IDC_CROSS),
            "Hand" => (id(IDC_HAND), IDC_HAND),
            "Help" => (id(IDC_HELP), IDC_HELP),
            "IBeam" => (id(IDC_IBEAM), IDC_IBEAM),
            "No" => (id(IDC_NO), IDC_NO),
            "SizeAll" => (id(IDC_SIZEALL), IDC_SIZEALL),
            "SizeNESW" => (id(IDC_SIZENESW), IDC_SIZENESW),
            "SizeNS" => (id(IDC_SIZENS), IDC_SIZENS),
            "SizeNWSE" => (id(IDC_SIZENWSE), IDC_SIZENWSE),
            "SizeWE" => (id(IDC_SIZEWE), IDC_SIZEWE),
            "UpArrow" => (id(IDC_UPARROW), IDC_UPARROW),
            "WaitCursor" => (id(IDC_WAIT), IDC_WAIT),
            "HSplit" => (id(IDC_SIZENS), IDC_SIZENS),
            "VSplit" => (id(IDC_SIZEWE), IDC_SIZEWE),
            "NoMoveVert" => (32652, IDC_SIZENS),
            "NoMoveHoriz" => (32653, IDC_SIZEWE),
            "NoMove2D" => (32654, IDC_SIZEALL),
            "PanNorth" => (32655, IDC_SIZENS),
            "PanSouth" => (32656, IDC_SIZENS),
            "PanWest" => (32657, IDC_SIZEWE),
            "PanEast" => (32658, IDC_SIZEWE),
            "PanNW" => (32659, IDC_SIZENWSE),
            "PanNE" => (32660, IDC_SIZENESW),
            "PanSW" => (32661, IDC_SIZENESW),
            "PanSE" => (32662, IDC_SIZENWSE),
            _ => return None,
        })
    }

    fn load(id: PCWSTR) -> HCURSOR {
        // SAFETY: a predefined cursor of the system, shared and never destroyed.
        unsafe { LoadCursorW(std::ptr::null_mut(), id) }
    }

    pub fn image(cursor_type: &str) -> Option<RgbaImage> {
        let (id, standard) = resource(cursor_type)?;
        let mut cursor = load(id as usize as PCWSTR);
        if cursor.is_null() {
            cursor = load(standard);
        }
        if cursor.is_null() {
            return None;
        }
        render(cursor)
    }

    /// The two bitmaps `GetIconInfo` hands out, deleted when dropped.
    struct Bitmaps(ICONINFO);

    impl Drop for Bitmaps {
        fn drop(&mut self) {
            // SAFETY: copies made for us by GetIconInfo, deleted once.
            unsafe {
                if !self.0.hbmColor.is_null() {
                    DeleteObject(self.0.hbmColor);
                }
                if !self.0.hbmMask.is_null() {
                    DeleteObject(self.0.hbmMask);
                }
            }
        }
    }

    struct MemoryDc(HDC);

    impl Drop for MemoryDc {
        fn drop(&mut self) {
            // SAFETY: a DC created here, deleted once.
            unsafe { DeleteDC(self.0) };
        }
    }

    fn size(bitmap: HBITMAP) -> Option<(u32, u32)> {
        // SAFETY: a zeroed BITMAP receives the description of a live bitmap.
        let mut info: BITMAP = unsafe { std::mem::zeroed() };
        let read = unsafe {
            GetObjectW(
                bitmap,
                size_of::<BITMAP>() as i32,
                &mut info as *mut BITMAP as *mut _,
            )
        };
        let (w, h) = (info.bmWidth.unsigned_abs(), info.bmHeight.unsigned_abs());
        (read != 0 && w > 0 && h > 0 && w <= 1024 && h <= 2048).then_some((w, h))
    }

    /// A bitmap's pixels as top-down 32-bit BGRA rows (a 1-bit mask comes back black
    /// and white).
    fn pixels(dc: &MemoryDc, bitmap: HBITMAP, width: u32, height: u32) -> Option<Vec<u8>> {
        // SAFETY: a zeroed BITMAPINFO with its header filled in; the buffer holds the
        // rows asked for.
        unsafe {
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..info.bmiHeader
            };
            let mut bits = vec![0u8; width as usize * height as usize * 4];
            let lines = GetDIBits(
                dc.0,
                bitmap,
                0,
                height,
                bits.as_mut_ptr().cast(),
                &mut info,
                DIB_RGB_COLORS,
            );
            (lines == height as i32).then_some(bits)
        }
    }

    fn render(cursor: HCURSOR) -> Option<RgbaImage> {
        // SAFETY: a zeroed ICONINFO receives the cursor's bitmaps.
        let mut info: ICONINFO = unsafe { std::mem::zeroed() };
        if unsafe { GetIconInfo(cursor, &mut info) } == 0 {
            return None;
        }
        let bitmaps = Bitmaps(info);
        // SAFETY: a memory DC compatible with the screen.
        let dc = MemoryDc(unsafe { CreateCompatibleDC(std::ptr::null_mut()) });
        if dc.0.is_null() {
            return None;
        }
        if !bitmaps.0.hbmColor.is_null() {
            let (width, height) = size(bitmaps.0.hbmColor)?;
            let color = pixels(&dc, bitmaps.0.hbmColor, width, height)?;
            let mask = if bitmaps.0.hbmMask.is_null() {
                None
            } else {
                pixels(&dc, bitmaps.0.hbmMask, width, height)
            };
            Some(from_color(width, height, &color, mask.as_deref()))
        } else if !bitmaps.0.hbmMask.is_null() {
            let (width, double) = size(bitmaps.0.hbmMask)?;
            let height = double / 2;
            if height == 0 {
                return None;
            }
            let masks = pixels(&dc, bitmaps.0.hbmMask, width, height * 2)?;
            let (and, xor) = masks.split_at(width as usize * height as usize * 4);
            Some(from_masks(width, height, and, xor))
        } else {
            None
        }
    }

    /// A colour cursor: its own alpha when it has one, else opaque where the AND mask
    /// is clear.
    pub fn from_color(width: u32, height: u32, color: &[u8], mask: Option<&[u8]>) -> RgbaImage {
        let has_alpha = color.as_chunks::<4>().0.iter().any(|p| p[3] != 0);
        let mut image = RgbaImage::new(width, height);
        for (i, pixel) in image.pixels_mut().enumerate() {
            let p = &color[i * 4..i * 4 + 4];
            let (b, g, r) = (p[0], p[1], p[2]);
            *pixel = if has_alpha {
                image::Rgba([r, g, b, p[3]])
            } else {
                let transparent = mask.is_some_and(|m| m[i * 4] != 0);
                match (transparent, r | g | b != 0) {
                    (false, _) => image::Rgba([r, g, b, 255]),
                    // Inverts the screen.
                    (true, true) => image::Rgba([0, 0, 0, 255]),
                    (true, false) => image::Rgba([0, 0, 0, 0]),
                }
            };
        }
        image
    }

    /// A black-and-white cursor from its AND and XOR masks.
    pub fn from_masks(width: u32, height: u32, and: &[u8], xor: &[u8]) -> RgbaImage {
        let mut image = RgbaImage::new(width, height);
        for (i, pixel) in image.pixels_mut().enumerate() {
            let a = and[i * 4] != 0;
            let x = xor[i * 4] != 0;
            *pixel = match (a, x) {
                (false, false) => image::Rgba([0, 0, 0, 255]),
                (false, true) => image::Rgba([255, 255, 255, 255]),
                (true, false) => image::Rgba([0, 0, 0, 0]),
                // Inverts the screen.
                (true, true) => image::Rgba([0, 0, 0, 255]),
            };
        }
        image
    }
}

/// The desktop picture: `SystemParametersInfoW(SPI_GETDESKWALLPAPER)`, and when that is
/// empty, the path inside `TranscodedImageCache` (as upstream).
mod wallpaper {
    use std::path::PathBuf;

    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegGetValueW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETDESKWALLPAPER, SystemParametersInfoW,
    };

    use crate::wide::{from_wide, wide};

    const DESKTOP: &str = r"Control Panel\Desktop";

    pub fn path() -> Option<PathBuf> {
        let mut buffer = vec![0u16; 32_768];
        // SAFETY: the buffer holds the number of characters passed.
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETDESKWALLPAPER,
                buffer.len() as u32,
                buffer.as_mut_ptr().cast(),
                0,
            )
        };
        if ok != 0 {
            let path = from_wide(&buffer);
            if !path.trim().is_empty() {
                // A path that is set but gone is no wallpaper (upstream gives up too).
                let path = PathBuf::from(path);
                return path.is_file().then_some(path);
            }
        }
        let cache = desktop_value("TranscodedImageCache", RRF_RT_REG_BINARY)?;
        let path = PathBuf::from(path_in_cache(&cache)?);
        path.is_file().then_some(path)
    }

    /// A value of `HKCU\Control Panel\Desktop` as raw bytes.
    fn desktop_value(name: &str, kind: u32) -> Option<Vec<u8>> {
        let key = wide(DESKTOP);
        let value = wide(name);
        let mut size = 0u32;
        // SAFETY: a size query, then a read into a buffer of that size.
        unsafe {
            let status = RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                kind,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            );
            if status != 0 || size == 0 || size > 1 << 20 {
                return None;
            }
            let mut data = vec![0u8; size as usize];
            let status = RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                kind,
                std::ptr::null_mut(),
                data.as_mut_ptr().cast(),
                &mut size,
            );
            if status != 0 {
                return None;
            }
            data.truncate(size as usize);
            Some(data)
        }
    }

    /// A string value of `HKCU\Control Panel\Desktop`.
    pub fn desktop_string(name: &str) -> Option<String> {
        let data = desktop_value(name, RRF_RT_REG_SZ)?;
        let units: Vec<u16> = data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        Some(from_wide(&units))
    }

    /// `TryExtractWallpaperPathFromTranscodedCache`: the first drive (`C:\`) or UNC
    /// (`\\`) path in the UTF-16 blob, up to its NUL.
    pub fn path_in_cache(cache: &[u8]) -> Option<String> {
        let text: Vec<u16> = cache
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        let backslash = b'\\' as u16;
        let start = (0..text.len().saturating_sub(2)).find(|&i| {
            let letter = char::from_u32(text[i] as u32).is_some_and(|c| c.is_alphabetic());
            (letter && text[i + 1] == b':' as u16 && text[i + 2] == backslash)
                || (text[i] == backslash && text[i + 1] == backslash)
        })?;
        let path = from_wide(&text[start..]);
        let path = path.trim();
        (!path.is_empty()).then(|| path.to_string())
    }
}

/// Pictures through the Windows Imaging Component: the decoder for the file, its first
/// frame, converted to 32-bit RGBA with straight alpha. The frame's size is checked
/// against the app's limits before a single pixel is decoded.
mod wic {
    use std::path::Path;

    use image::RgbaImage;
    use winrt::Win32::Foundation::{GENERIC_READ, RPC_E_CHANGED_MODE};
    use winrt::Win32::Graphics::Imaging::{
        CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICImagingFactory, IWICPalette,
        WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
    };
    use winrt::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use winrt::core::{HSTRING, IUnknown};

    /// COM on this thread for the length of a decode: joined here when the thread had
    /// none, left as it was when it already had an apartment.
    struct Apartment(bool);

    impl Apartment {
        fn enter() -> Result<Apartment, String> {
            // SAFETY: plain COM initialisation, undone in Drop when it succeeded.
            let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if result.is_ok() {
                Ok(Apartment(true))
            } else if result == RPC_E_CHANGED_MODE {
                Ok(Apartment(false))
            } else {
                Err(result.message())
            }
        }
    }

    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: balances the successful CoInitializeEx above.
                unsafe { CoUninitialize() };
            }
        }
    }

    pub fn load(path: &Path) -> Result<RgbaImage, String> {
        let _apartment = Apartment::enter()?;
        let name = HSTRING::from(path.as_os_str());
        // SAFETY: WIC objects used on this thread while COM is initialised; the buffer
        // is exactly the size CopyPixels is told.
        unsafe {
            let factory: IWICImagingFactory = CoCreateInstance(
                &CLSID_WICImagingFactory,
                None::<&IUnknown>,
                CLSCTX_INPROC_SERVER,
            )
            .map_err(|e| e.message())?;
            let decoder = factory
                .CreateDecoderFromFilename(
                    &name,
                    None,
                    GENERIC_READ,
                    WICDecodeMetadataCacheOnDemand,
                )
                .map_err(|e| e.message())?;
            let frame = decoder.GetFrame(0).map_err(|e| e.message())?;
            let (mut width, mut height) = (0u32, 0u32);
            frame
                .GetSize(&mut width, &mut height)
                .map_err(|e| e.message())?;
            if !modlight_core::image_decode::fits(width as u64, height as u64) {
                return Err(format!(
                    "a {width} x {height} picture is larger than the app opens"
                ));
            }
            let converter = factory.CreateFormatConverter().map_err(|e| e.message())?;
            converter
                .Initialize(
                    &frame,
                    &GUID_WICPixelFormat32bppRGBA,
                    WICBitmapDitherTypeNone,
                    None::<&IWICPalette>,
                    0.0,
                    WICBitmapPaletteTypeCustom,
                )
                .map_err(|e| e.message())?;
            let stride = width * 4;
            let mut pixels = vec![0u8; stride as usize * height as usize];
            converter
                .CopyPixels(std::ptr::null(), stride, &mut pixels)
                .map_err(|e| e.message())?;
            RgbaImage::from_raw(width, height, pixels)
                .ok_or_else(|| "the decoded picture has the wrong size".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [&str; 28] = [
        "AppStarting",
        "Arrow",
        "Cross",
        "Default",
        "Hand",
        "Help",
        "HSplit",
        "IBeam",
        "No",
        "NoMove2D",
        "NoMoveHoriz",
        "NoMoveVert",
        "PanEast",
        "PanNE",
        "PanNorth",
        "PanNW",
        "PanSE",
        "PanSouth",
        "PanSW",
        "PanWest",
        "SizeAll",
        "SizeNESW",
        "SizeNS",
        "SizeNWSE",
        "SizeWE",
        "UpArrow",
        "VSplit",
        "WaitCursor",
    ];

    #[test]
    fn every_cursor_kind_is_a_picture_with_something_on_it() {
        for kind in KINDS {
            let image = cursor_image(kind).unwrap_or_else(|| panic!("{kind}: no picture"));
            assert!(image.width() >= 16 && image.height() >= 16, "{kind}");
            let opaque = image.pixels().filter(|p| p.0[3] == 255).count();
            assert!(opaque > 10, "{kind}: {opaque} opaque pixels");
            let clear = image.pixels().filter(|p| p.0[3] == 0).count();
            assert!(clear > 0, "{kind}: no clear pixels");
        }
        assert!(cursor_image("Nonsense").is_none());
    }

    #[test]
    fn black_and_white_cursors_are_their_masks() {
        // AND, XOR as 32-bit rows: clear, black, white, inverting.
        let and = [255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 0];
        let xor = [0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 0, 255, 255, 255, 0];
        let image = cursors::from_masks(4, 1, &and, &xor);
        let alphas: Vec<u8> = image.pixels().map(|p| p.0[3]).collect();
        assert_eq!(alphas, [0, 255, 255, 255]);
        assert_eq!(image.get_pixel(2, 0).0, [255, 255, 255, 255]);
        assert_eq!(image.get_pixel(3, 0).0, [0, 0, 0, 255]);
    }

    #[test]
    fn colour_cursors_keep_their_alpha_or_use_their_mask() {
        let with_alpha = [10, 20, 30, 128, 0, 0, 0, 0];
        let image = cursors::from_color(2, 1, &with_alpha, None);
        assert_eq!(image.get_pixel(0, 0).0, [30, 20, 10, 128]);
        assert_eq!(image.get_pixel(1, 0).0, [0, 0, 0, 0]);
        let no_alpha = [10, 20, 30, 0, 0, 0, 0, 0];
        let mask = [0, 0, 0, 0, 255, 255, 255, 0];
        let image = cursors::from_color(2, 1, &no_alpha, Some(&mask));
        assert_eq!(image.get_pixel(0, 0).0, [30, 20, 10, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [0, 0, 0, 0]);
    }

    #[test]
    fn the_wallpaper_styles_map_onto_the_editors_layouts() {
        let style = WallpaperStyle::from_registry;
        assert_eq!(style("10", "0"), WallpaperStyle::Fill);
        assert_eq!(style("6", "0"), WallpaperStyle::Fit);
        assert_eq!(style("2", "0"), WallpaperStyle::Stretch);
        assert_eq!(style("0", "1"), WallpaperStyle::Tile);
        assert_eq!(style("0", "0"), WallpaperStyle::Center);
        assert_eq!(style("22", "0"), WallpaperStyle::Span);
        assert_eq!(style("", ""), WallpaperStyle::Fill);
        let options = |scaling, allow_clipping| WallpaperOptions {
            scaling,
            allow_clipping,
        };
        assert_eq!(WallpaperStyle::Fill.options(), options(3, true));
        assert_eq!(WallpaperStyle::Fit.options(), options(3, false));
        assert_eq!(WallpaperStyle::Stretch.options(), options(1, false));
        assert_eq!(WallpaperStyle::Center.options(), options(2, false));
        assert_eq!(WallpaperStyle::Tile.options(), options(2, false));
        assert_eq!(WallpaperStyle::Span.options(), options(3, true));
    }

    #[test]
    fn the_cached_wallpaper_path_is_found_in_its_blob() {
        let mut blob = vec![0x7a, 0x00, 0x01, 0x00, 0x10, 0x00, 0x00, 0x00];
        for unit in "C:\\Users\\x\\Pictures\\sea.jpg\0junk".encode_utf16() {
            blob.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(
            wallpaper::path_in_cache(&blob).as_deref(),
            Some("C:\\Users\\x\\Pictures\\sea.jpg")
        );
        let mut unc = Vec::new();
        for unit in "\u{1}\\\\server\\share\\a.png\0".encode_utf16() {
            unc.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(
            wallpaper::path_in_cache(&unc).as_deref(),
            Some("\\\\server\\share\\a.png")
        );
        assert_eq!(wallpaper::path_in_cache(&[1, 0, 2, 0]), None);
    }

    #[test]
    fn the_desktop_wallpaper_is_a_file_when_there_is_one() {
        // Reads the owner's settings, changes nothing.
        if let Some((path, _)) = desktop_wallpaper() {
            assert!(path.is_file(), "{}", path.display());
        }
    }

    #[test]
    fn wic_decodes_a_picture_file() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("zdjęcie.png");
        let mut picture = RgbaImage::from_pixel(5, 3, image::Rgba([10, 200, 30, 255]));
        picture.put_pixel(4, 2, image::Rgba([250, 0, 0, 64]));
        picture.save(&path).unwrap();
        let read = load_image(&path).expect("decoded");
        assert_eq!(read, picture);
        // Off a worker thread with no apartment of its own, too.
        let again = std::thread::spawn(move || load_image(&path))
            .join()
            .unwrap();
        assert_eq!(again.as_ref(), Some(&picture));
        assert!(load_image(&folder.path().join("missing.png")).is_none());
    }

    #[test]
    fn wic_refuses_a_picture_beyond_the_limits_before_decoding_it() {
        // A PNG header claiming 40000 x 40000 pixels and no pixel data.
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("huge.png");
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = b"IHDR".to_vec();
        ihdr.extend_from_slice(&40_000u32.to_be_bytes());
        ihdr.extend_from_slice(&40_000u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(&ihdr);
        png.extend_from_slice(&crc32(&ihdr).to_be_bytes());
        png.extend_from_slice(&0u32.to_be_bytes());
        png.extend_from_slice(b"IEND");
        png.extend_from_slice(&crc32(b"IEND").to_be_bytes());
        std::fs::write(&path, &png).unwrap();
        assert!(load_image(&path).is_none());
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &byte in bytes {
            crc ^= byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
}
