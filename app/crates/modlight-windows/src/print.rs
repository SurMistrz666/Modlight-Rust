//! Printing a picture (`PrintDocument`, `PrintDialog` and `PrintPreviewDialog` upstream):
//! one page through GDI, with or without the print dialog; the preview is the page as a
//! PDF opened in the default viewer. Main thread only; never used by automated runs.
//!
//! The page is measured in points from the top left of the sheet. A printer cannot draw
//! on the whole sheet: its device coordinates start at the corner of the printable area,
//! `PHYSICALOFFSETX/Y` device units into the sheet, so the placement is moved by that
//! much and lands where it was meant to on the paper.
//!
//! The printer named in the settings (`DefaultPrinterOverride`) is chosen before the
//! dialog opens, so the dialog shows it and the person's own choice there wins (upstream
//! put the override over whatever the dialog had chosen).

use std::path::Path;

use image::RgbaImage;

/// The page in points: (width, height) of the default printer's paper.
pub fn page_size() -> (f64, f64) {
    default_printer()
        .and_then(|name| gdi::page_size(&name))
        .unwrap_or_else(locale_paper)
}

/// What a print came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Printed {
    Done,
    /// The dialog was cancelled.
    Cancelled,
    /// The printer named in the settings does not exist; printed on the default one.
    UnknownPrinter,
}

/// Print `picture` at `rect` (points, top left of the page) on one page.
pub fn print(
    picture: &RgbaImage,
    rect: (f64, f64, f64, f64),
    show_panel: bool,
    printer: Option<&str>,
) -> Result<Printed, String> {
    if picture.width() == 0 || picture.height() == 0 {
        return Err("the picture is empty".into());
    }
    let mut outcome = Printed::Done;
    let mut name = default_printer();
    if let Some(wanted) = printer.map(str::trim).filter(|n| !n.is_empty()) {
        if gdi::printer_exists(wanted) {
            name = Some(wanted.to_string());
        } else {
            outcome = Printed::UnknownPrinter;
        }
    }
    let dc = if show_panel {
        match gdi::print_dialog(name.as_deref())? {
            Some(dc) => dc,
            None => return Ok(Printed::Cancelled),
        }
    } else {
        let name = name.ok_or("there is no printer to print on")?;
        gdi::printer_dc(&name)?
    };
    match gdi::print_page(&dc, picture, rect)? {
        gdi::Page::Printed => Ok(outcome),
        // A printer that asks for something of its own (Microsoft Print to PDF's file
        // name) and was told no.
        gdi::Page::Cancelled => Ok(Printed::Cancelled),
    }
}

/// Write the page as a PDF (the preview).
pub fn page_pdf(
    picture: &RgbaImage,
    rect: (f64, f64, f64, f64),
    path: &Path,
) -> Result<(), String> {
    let bytes = pdf::page(picture, rect, page_size())?;
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

/// The default printer's name (`GetDefaultPrinterW`).
fn default_printer() -> Option<String> {
    use windows_sys::Win32::Graphics::Printing::GetDefaultPrinterW;
    let mut length = 0u32;
    // SAFETY: a null buffer asks for the length only.
    unsafe { GetDefaultPrinterW(std::ptr::null_mut(), &mut length) };
    if length == 0 {
        return None;
    }
    let mut buffer = vec![0u16; length as usize];
    // SAFETY: the buffer holds `length` characters.
    let ok = unsafe { GetDefaultPrinterW(buffer.as_mut_ptr(), &mut length) };
    let name = crate::wide::from_wide(&buffer);
    (ok != 0 && !name.is_empty()).then_some(name)
}

/// The paper of the person's region when there is no printer at all
/// (`LOCALE_IPAPERSIZE`): A4 where that is the paper, US Letter otherwise.
fn locale_paper() -> (f64, f64) {
    use windows_sys::Win32::Globalization::{
        GetLocaleInfoEx, LOCALE_IPAPERSIZE, LOCALE_RETURN_NUMBER,
    };
    let mut value = 0u32;
    // SAFETY: LOCALE_RETURN_NUMBER writes one u32 (two UTF-16 units) into the buffer.
    let written = unsafe {
        GetLocaleInfoEx(
            std::ptr::null(),
            LOCALE_IPAPERSIZE | LOCALE_RETURN_NUMBER,
            &mut value as *mut u32 as *mut u16,
            2,
        )
    };
    let millimetres = |w: f64, h: f64| (w * 72.0 / 25.4, h * 72.0 / 25.4);
    match (written, value) {
        (0, _) => (612.0, 792.0),
        (_, 5) => (612.0, 1008.0),
        (_, 8) => millimetres(297.0, 420.0),
        (_, 9) => millimetres(210.0, 297.0),
        _ => (612.0, 792.0),
    }
}

/// The printer side: device contexts, the print dialog and the page itself.
mod gdi {
    use image::RgbaImage;
    use windows_sys::Win32::Foundation::{ERROR_CANCELLED, GetLastError, HGLOBAL};
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDCW, DIB_RGB_COLORS, DeleteDC, GetDeviceCaps,
        HALFTONE, HDC, LOGPIXELSX, LOGPIXELSY, PHYSICALHEIGHT, PHYSICALOFFSETX, PHYSICALOFFSETY,
        PHYSICALWIDTH, SRCCOPY, SetBrushOrgEx, SetStretchBltMode, StretchDIBits,
    };
    use windows_sys::Win32::Graphics::Printing::{ClosePrinter, OpenPrinterW, PRINTER_HANDLE};
    use windows_sys::Win32::Storage::Xps::{
        AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage,
    };
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    use windows_sys::Win32::UI::Controls::Dialogs::{
        CommDlgExtendedError, DEVNAMES, PD_NOCURRENTPAGE, PD_NOPAGENUMS, PD_NOSELECTION,
        PD_RESULT_PRINT, PD_RETURNDC, PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGEXW, PRINTDLGW,
        PrintDlgExW, PrintDlgW, START_PAGE_GENERAL,
    };

    use crate::wide::wide;

    /// A device context this module created; deleted when dropped.
    pub struct Dc(HDC);

    impl Drop for Dc {
        fn drop(&mut self) {
            // SAFETY: a DC created by CreateDCW or the print dialog, deleted once.
            unsafe { DeleteDC(self.0) };
        }
    }

    pub enum Page {
        Printed,
        Cancelled,
    }

    pub fn printer_exists(name: &str) -> bool {
        let name = wide(name);
        let mut handle = PRINTER_HANDLE::default();
        // SAFETY: a NUL-terminated name and an out pointer; the handle is closed here.
        unsafe {
            if OpenPrinterW(name.as_ptr(), &mut handle, std::ptr::null()) == 0 {
                return false;
            }
            ClosePrinter(handle);
        }
        true
    }

    pub fn printer_dc(name: &str) -> Result<Dc, String> {
        let name = wide(name);
        // SAFETY: a NUL-terminated device name; the printer's own default settings.
        let dc = unsafe {
            CreateDCW(
                std::ptr::null(),
                name.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if dc.is_null() {
            Err("the printer could not be opened".into())
        } else {
            Ok(Dc(dc))
        }
    }

    fn caps(dc: &Dc, index: u32) -> i32 {
        // SAFETY: a live DC.
        unsafe { GetDeviceCaps(dc.0, index as i32) }
    }

    /// The sheet of this printer in points.
    pub fn page_size(name: &str) -> Option<(f64, f64)> {
        let dc = printer_dc(name).ok()?;
        let (dpi_x, dpi_y) = (caps(&dc, LOGPIXELSX), caps(&dc, LOGPIXELSY));
        let (width, height) = (caps(&dc, PHYSICALWIDTH), caps(&dc, PHYSICALHEIGHT));
        if dpi_x <= 0 || dpi_y <= 0 || width <= 0 || height <= 0 {
            return None;
        }
        Some((
            width as f64 * 72.0 / dpi_x as f64,
            height as f64 * 72.0 / dpi_y as f64,
        ))
    }

    /// A `DEVNAMES` block naming `printer`, so the dialog opens on it.
    fn dev_names(printer: &str) -> HGLOBAL {
        let driver: Vec<u16> = "winspool\0".encode_utf16().collect();
        let device = wide(printer);
        let header = (size_of::<DEVNAMES>() / 2) as u16;
        let units = header as usize + driver.len() + device.len() + 1;
        // SAFETY: a fresh zeroed block of `units` UTF-16 units, written while locked.
        unsafe {
            let block = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, units * 2);
            if block.is_null() {
                return block;
            }
            let base = GlobalLock(block) as *mut u16;
            if base.is_null() {
                return block;
            }
            let names = DEVNAMES {
                wDriverOffset: header,
                wDeviceOffset: header + driver.len() as u16,
                wOutputOffset: header + (driver.len() + device.len()) as u16,
                wDefault: 0,
            };
            std::ptr::write_unaligned(base as *mut DEVNAMES, names);
            let mut at = base.add(header as usize);
            for unit in driver.iter().chain(device.iter()) {
                *at = *unit;
                at = at.add(1);
            }
            GlobalUnlock(block);
            block
        }
    }

    fn free(block: HGLOBAL) {
        if !block.is_null() {
            // SAFETY: a block the dialog handed back (or we made), freed once.
            unsafe { windows_sys::Win32::Foundation::GlobalFree(block) };
        }
    }

    /// The window the dialog belongs to: the app's active window on this thread, or its
    /// foreground window when that is one of this process's.
    fn owner() -> windows_sys::Win32::Foundation::HWND {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId,
        };
        // SAFETY: plain queries.
        unsafe {
            let active = GetActiveWindow();
            if !active.is_null() {
                return active;
            }
            let foreground = GetForegroundWindow();
            let mut process = 0u32;
            GetWindowThreadProcessId(foreground, &mut process);
            if process == std::process::id() {
                foreground
            } else {
                std::ptr::null_mut()
            }
        }
    }

    /// The print dialog (`PrintDialog` with `UseEXDialog`), opened on `printer`. The
    /// chosen printer's DC, or `None` when it was cancelled.
    pub fn print_dialog(printer: Option<&str>) -> Result<Option<Dc>, String> {
        let owner = owner();
        if !owner.is_null() {
            // SAFETY: a zeroed PRINTDLGEXW is valid input once its size and flags are set.
            let mut dialog: PRINTDLGEXW = unsafe { std::mem::zeroed() };
            dialog.lStructSize = size_of::<PRINTDLGEXW>() as u32;
            dialog.hwndOwner = owner;
            dialog.hDevNames = printer.map(dev_names).unwrap_or(std::ptr::null_mut());
            dialog.Flags = PD_RETURNDC
                | PD_NOPAGENUMS
                | PD_NOSELECTION
                | PD_NOCURRENTPAGE
                | PD_USEDEVMODECOPIESANDCOLLATE;
            dialog.nMinPage = 1;
            dialog.nMaxPage = 1;
            dialog.nCopies = 1;
            dialog.nStartPage = START_PAGE_GENERAL;
            // SAFETY: the structure is filled in as the function documents.
            let result = unsafe { PrintDlgExW(&mut dialog) };
            free(dialog.hDevMode);
            free(dialog.hDevNames);
            if result >= 0 {
                let dc = (!dialog.hDC.is_null()).then_some(Dc(dialog.hDC));
                return Ok(if dialog.dwResultAction == PD_RESULT_PRINT {
                    if dc.is_none() {
                        return Err("the print dialog gave no printer".into());
                    }
                    dc
                } else {
                    None
                });
            }
            log::warn!("The print dialog failed ({result:#x}); the classic one opens instead.");
        }
        // The classic dialog: it needs no owner window.
        // SAFETY: a zeroed PRINTDLGW is valid input once its size and flags are set.
        let mut dialog: PRINTDLGW = unsafe { std::mem::zeroed() };
        dialog.lStructSize = size_of::<PRINTDLGW>() as u32;
        dialog.hwndOwner = owner;
        dialog.hDevNames = printer.map(dev_names).unwrap_or(std::ptr::null_mut());
        dialog.Flags = PD_RETURNDC | PD_NOPAGENUMS | PD_NOSELECTION | PD_USEDEVMODECOPIESANDCOLLATE;
        dialog.nMinPage = 1;
        dialog.nMaxPage = 1;
        dialog.nCopies = 1;
        // SAFETY: the structure is filled in as the function documents.
        let ok = unsafe { PrintDlgW(&mut dialog) };
        free(dialog.hDevMode);
        free(dialog.hDevNames);
        if ok == 0 {
            // SAFETY: plain query.
            let error = unsafe { CommDlgExtendedError() };
            return if error == 0 {
                Ok(None)
            } else {
                Err(format!("the print dialog failed ({error:#x})"))
            };
        }
        if dialog.hDC.is_null() {
            return Err("the print dialog gave no printer".into());
        }
        Ok(Some(Dc(dialog.hDC)))
    }

    /// Where the placement (points from the sheet's top left) lands in device units.
    pub fn device_rect(
        rect: (f64, f64, f64, f64),
        dpi: (i32, i32),
        offset: (i32, i32),
    ) -> (i32, i32, i32, i32) {
        let sx = dpi.0 as f64 / 72.0;
        let sy = dpi.1 as f64 / 72.0;
        let left = (rect.0 * sx).round() as i32 - offset.0;
        let top = (rect.1 * sy).round() as i32 - offset.1;
        let width = ((rect.2 * sx).round() as i32).max(1);
        let height = ((rect.3 * sy).round() as i32).max(1);
        (left, top, width, height)
    }

    /// The picture over white paper, as the BGRX rows of a top-down 32-bit DIB.
    pub fn on_paper(picture: &RgbaImage) -> Vec<u8> {
        let mut bits = Vec::with_capacity(picture.as_raw().len());
        for pixel in picture.pixels() {
            let [r, g, b, a] = pixel.0;
            let over_white =
                |c: u8| ((c as u32 * a as u32 + 255 * (255 - a as u32) + 127) / 255) as u8;
            bits.extend_from_slice(&[over_white(b), over_white(g), over_white(r), 0]);
        }
        bits
    }

    /// One document of one page with the picture on it (`PrintDocument.Print`).
    pub fn print_page(
        dc: &Dc,
        picture: &RgbaImage,
        rect: (f64, f64, f64, f64),
    ) -> Result<Page, String> {
        let dpi = (caps(dc, LOGPIXELSX), caps(dc, LOGPIXELSY));
        if dpi.0 <= 0 || dpi.1 <= 0 {
            return Err("the printer gave no resolution".into());
        }
        let offset = (caps(dc, PHYSICALOFFSETX), caps(dc, PHYSICALOFFSETY));
        let (x, y, w, h) = device_rect(rect, dpi, offset);
        let bits = on_paper(picture);
        // SAFETY: a zeroed BITMAPINFO with its header filled in describes `bits`.
        let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: picture.width() as i32,
            biHeight: -(picture.height() as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..info.bmiHeader
        };
        let name = wide("Modlight");
        let document = DOCINFOW {
            cbSize: size_of::<DOCINFOW>() as i32,
            lpszDocName: name.as_ptr(),
            lpszOutput: std::ptr::null(),
            lpszDatatype: std::ptr::null(),
            fwType: 0,
        };
        // SAFETY: GDI calls on a live printer DC; `bits` and `info` outlive the calls.
        unsafe {
            if StartDocW(dc.0, &document) <= 0 {
                return if GetLastError() == ERROR_CANCELLED {
                    Ok(Page::Cancelled)
                } else {
                    Err("the printer did not start the document".into())
                };
            }
            if StartPage(dc.0) <= 0 {
                AbortDoc(dc.0);
                return Err("the printer did not start the page".into());
            }
            SetStretchBltMode(dc.0, HALFTONE);
            SetBrushOrgEx(dc.0, 0, 0, std::ptr::null_mut());
            let lines = StretchDIBits(
                dc.0,
                x,
                y,
                w,
                h,
                0,
                0,
                picture.width() as i32,
                picture.height() as i32,
                bits.as_ptr().cast(),
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
            if lines == 0 || lines == -1 {
                AbortDoc(dc.0);
                return Err("the printer did not take the picture".into());
            }
            if EndPage(dc.0) <= 0 {
                AbortDoc(dc.0);
                return Err("the printer did not finish the page".into());
            }
            if EndDoc(dc.0) <= 0 {
                return Err("the printer did not finish the document".into());
            }
        }
        Ok(Page::Printed)
    }
}

/// The page as a one-page PDF 1.4, written by hand: the picture as an RGB image
/// (Flate-compressed) with its alpha as a soft mask when it has any, drawn at the
/// placement on a page the size of the paper.
mod pdf {
    use std::io::Write;

    use image::RgbaImage;

    /// A number as PDF writes it: at most four decimals, no exponent, no trailing zeros.
    pub fn number(value: f64) -> String {
        let value = if value.is_finite() { value } else { 0.0 };
        let text = format!("{value:.4}");
        let text = text.trim_end_matches('0').trim_end_matches('.');
        match text {
            "-0" | "" => "0".to_string(),
            _ => text.to_string(),
        }
    }

    fn deflate(bytes: &[u8]) -> Result<Vec<u8>, String> {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).map_err(|e| e.to_string())?;
        encoder.finish().map_err(|e| e.to_string())
    }

    struct Writer {
        bytes: Vec<u8>,
        offsets: Vec<usize>,
    }

    impl Writer {
        fn object(&mut self, body: &[u8]) {
            self.offsets.push(self.bytes.len());
            let number = self.offsets.len();
            self.bytes
                .extend_from_slice(format!("{number} 0 obj\n").as_bytes());
            self.bytes.extend_from_slice(body);
            self.bytes.extend_from_slice(b"\nendobj\n");
        }

        fn stream(&mut self, dictionary: &str, data: &[u8]) {
            let opening = if dictionary.is_empty() {
                "<<".to_string()
            } else {
                format!("<< {dictionary}")
            };
            let mut body = format!("{opening} /Length {} >>\nstream\n", data.len()).into_bytes();
            body.extend_from_slice(data);
            body.extend_from_slice(b"\nendstream");
            self.object(&body);
        }
    }

    pub fn page(
        picture: &RgbaImage,
        rect: (f64, f64, f64, f64),
        paper: (f64, f64),
    ) -> Result<Vec<u8>, String> {
        let (width, height) = picture.dimensions();
        if width == 0 || height == 0 {
            return Err("the picture is empty".into());
        }
        let (paper_w, paper_h) = paper;
        let translucent = picture.pixels().any(|p| p.0[3] != 255);
        let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
        let mut alpha = Vec::new();
        for pixel in picture.pixels() {
            rgb.extend_from_slice(&pixel.0[..3]);
            if translucent {
                alpha.push(pixel.0[3]);
            }
        }
        let mut pdf = Writer {
            bytes: b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec(),
            offsets: Vec::new(),
        };
        pdf.object(b"<< /Type /Catalog /Pages 2 0 R >>");
        pdf.object(b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
        pdf.object(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] \
                 /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>",
                number(paper_w),
                number(paper_h)
            )
            .as_bytes(),
        );
        // PDF's origin is the bottom left of the page; the placement's is the top left.
        let content = format!(
            "q {} 0 0 {} {} {} cm /Im0 Do Q",
            number(rect.2),
            number(rect.3),
            number(rect.0),
            number(paper_h - rect.1 - rect.3)
        );
        pdf.stream("", content.as_bytes());
        let image = format!(
            "/Type /XObject /Subtype /Image /Width {width} /Height {height} \
             /ColorSpace /DeviceRGB /BitsPerComponent 8 /Interpolate true \
             /Filter /FlateDecode{}",
            if translucent { " /SMask 6 0 R" } else { "" }
        );
        pdf.stream(&image, &deflate(&rgb)?);
        if translucent {
            let mask = format!(
                "/Type /XObject /Subtype /Image /Width {width} /Height {height} \
                 /ColorSpace /DeviceGray /BitsPerComponent 8 /Interpolate true \
                 /Filter /FlateDecode"
            );
            pdf.stream(&mask, &deflate(&alpha)?);
        }
        let xref = pdf.bytes.len();
        let count = pdf.offsets.len() + 1;
        let mut table = format!("xref\n0 {count}\n0000000000 65535 f \n");
        for offset in &pdf.offsets {
            table.push_str(&format!("{offset:010} 00000 n \n"));
        }
        table.push_str(&format!(
            "trailer\n<< /Size {count} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
        ));
        pdf.bytes.extend_from_slice(table.as_bytes());
        Ok(pdf.bytes)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
        haystack[from..]
            .windows(needle.len())
            .position(|w| w == needle)
            .map(|at| at + from)
    }

    /// The objects the cross-reference table points at, checked to be where it says.
    fn objects(pdf: &[u8]) -> Vec<usize> {
        assert!(pdf.starts_with(b"%PDF-1.4\n"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        let start = find(pdf, b"startxref\n", 0).expect("startxref") + b"startxref\n".len();
        let end = find(pdf, b"\n", start).unwrap();
        let xref: usize = std::str::from_utf8(&pdf[start..end])
            .unwrap()
            .parse()
            .unwrap();
        assert!(pdf[xref..].starts_with(b"xref\n0 "));
        let table = std::str::from_utf8(&pdf[xref..]).unwrap();
        let mut lines = table.lines().skip(1);
        let count: usize = lines
            .next()
            .unwrap()
            .split(' ')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(lines.next(), Some("0000000000 65535 f "));
        let mut offsets = Vec::new();
        for number in 1..count {
            let entry = lines.next().unwrap();
            // Every entry is exactly 20 bytes with its line end.
            assert_eq!(entry.len(), 19, "{entry:?}");
            let offset: usize = entry[..10].parse().unwrap();
            assert!(entry.ends_with(" 00000 n "));
            assert!(
                pdf[offset..].starts_with(format!("{number} 0 obj\n").as_bytes()),
                "object {number} is not at {offset}"
            );
            offsets.push(offset);
        }
        assert_eq!(lines.next(), Some("trailer"));
        assert!(table.contains(&format!("/Size {count} /Root 1 0 R")));
        offsets
    }

    /// The decompressed stream of the object at `offset`.
    fn stream(pdf: &[u8], offset: usize) -> Vec<u8> {
        let header_end = find(pdf, b"stream\n", offset).unwrap();
        let dictionary = std::str::from_utf8(&pdf[offset..header_end]).unwrap();
        let length: usize = dictionary
            .split("/Length ")
            .nth(1)
            .unwrap()
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let data = &pdf[header_end + 7..header_end + 7 + length];
        assert!(pdf[header_end + 7 + length..].starts_with(b"\nendstream\nendobj\n"));
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(data)
            .read_to_end(&mut out)
            .unwrap();
        out
    }

    #[test]
    fn the_preview_is_a_well_formed_page_of_the_papers_size() {
        let mut picture = RgbaImage::from_pixel(30, 20, image::Rgba([10, 20, 30, 255]));
        picture.put_pixel(0, 0, image::Rgba([200, 100, 50, 255]));
        let a4 = (595.2756, 841.8898);
        let pdf = pdf::page(&picture, (36.0, 72.0, 300.0, 200.0), a4).unwrap();
        let offsets = objects(&pdf);
        assert_eq!(offsets.len(), 5, "no soft mask for an opaque picture");
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/MediaBox [0 0 595.2756 841.8898]"));
        // 72 pt from the top of the page is 841.8898 - 72 - 200 from its bottom.
        assert!(text.contains("q 300 0 0 200 36 569.8898 cm /Im0 Do Q"));
        assert!(!text.contains("/SMask"));
        let rgb = stream(&pdf, offsets[4]);
        assert_eq!(rgb.len(), 30 * 20 * 3);
        assert_eq!(&rgb[..6], &[200, 100, 50, 10, 20, 30]);
    }

    #[test]
    fn transparency_becomes_a_soft_mask() {
        let mut picture = RgbaImage::from_pixel(4, 3, image::Rgba([0, 0, 255, 255]));
        picture.put_pixel(3, 2, image::Rgba([0, 0, 0, 0]));
        let pdf = pdf::page(&picture, (0.0, 0.0, 612.0, 792.0), (612.0, 792.0)).unwrap();
        let offsets = objects(&pdf);
        assert_eq!(offsets.len(), 6);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/SMask 6 0 R"));
        assert!(text.contains("/MediaBox [0 0 612 792]"));
        let mask = stream(&pdf, offsets[5]);
        assert_eq!(mask.len(), 12);
        assert_eq!(mask[11], 0);
        assert!(mask[..11].iter().all(|&a| a == 255));
    }

    #[test]
    fn page_pdf_writes_the_file_and_refuses_an_empty_picture() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("preview.pdf");
        let picture = RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]));
        page_pdf(&picture, (10.0, 10.0, 100.0, 100.0), &path).unwrap();
        let pdf = std::fs::read(&path).unwrap();
        objects(&pdf);
        let (w, h) = page_size();
        assert!(String::from_utf8_lossy(&pdf).contains(&format!(
            "/MediaBox [0 0 {} {}]",
            pdf::number(w),
            pdf::number(h)
        )));
        assert!(page_pdf(&RgbaImage::new(0, 0), (0.0, 0.0, 1.0, 1.0), &path).is_err());
    }

    #[test]
    fn numbers_are_written_plainly() {
        assert_eq!(pdf::number(612.0), "612");
        assert_eq!(pdf::number(595.27559), "595.2756");
        assert_eq!(pdf::number(-0.00001), "0");
        assert_eq!(pdf::number(0.5), "0.5");
        assert_eq!(pdf::number(f64::NAN), "0");
        assert_eq!(pdf::number(1e-9), "0");
    }

    #[test]
    fn the_paper_is_a_real_sheet() {
        let (w, h) = page_size();
        // Anything from a label to A0.
        assert!(
            (100.0..3400.0).contains(&w) && (100.0..3400.0).contains(&h),
            "{w} x {h}"
        );
    }

    #[test]
    fn the_placement_lands_on_the_sheet_past_the_printers_margin() {
        // 600 dpi, a printable area starting 0.25 in into the sheet.
        let rect = gdi::device_rect((72.0, 36.0, 144.0, 72.0), (600, 600), (150, 150));
        assert_eq!(rect, (450, 150, 1200, 600));
        let rect = gdi::device_rect((0.0, 0.0, 0.01, 0.01), (300, 300), (0, 0));
        assert_eq!((rect.2, rect.3), (1, 1));
    }

    #[test]
    fn transparency_prints_as_paper() {
        let mut picture = RgbaImage::new(2, 1);
        picture.put_pixel(0, 0, image::Rgba([255, 0, 0, 0]));
        picture.put_pixel(1, 0, image::Rgba([0, 0, 255, 255]));
        assert_eq!(gdi::on_paper(&picture), [255, 255, 255, 0, 255, 0, 0, 0]);
    }

    #[test]
    fn an_unknown_printer_is_unknown() {
        assert!(!gdi::printer_exists(
            "Modlight test printer that does not exist"
        ));
    }
}
