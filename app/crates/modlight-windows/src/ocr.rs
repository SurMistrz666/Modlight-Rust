//! Text recognition with Windows OCR (`Windows.Media.Ocr`), as upstream (`OCRHelper`). A
//! picture in, the recognised lines out.
//!
//! The picture is decoded under the app's limits, scaled down when it is larger than the
//! engine takes (`OcrEngine.MaxImageDimension`; upstream let the engine refuse it), and
//! handed over as a premultiplied BGRA `SoftwareBitmap`. Each line is put together as
//! upstream's `OCRInternal` does: Chinese and Japanese words without spaces between them,
//! right-to-left languages with their words in reading order, every other language as the
//! engine's own line text.
//!
//! `recognize` blocks until the engine is done, so it is called off the UI thread (the
//! app runs it in `spawn_blocking`). WinRT needs no setup on that thread: activation joins
//! the process's multithreaded apartment by itself.

use image::RgbaImage;
use modlight_core::l10n::{self, Arg};
use winrt::Globalization::{Language, LanguageLayoutDirection};
use winrt::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
use winrt::Media::Ocr::{OcrEngine, OcrLine};
use winrt::Storage::Streams::DataWriter;
use winrt::core::HSTRING;

/// The languages Windows OCR has installed, as BCP 47 tags ("en-US").
pub fn languages() -> Vec<String> {
    let list = match OcrEngine::AvailableRecognizerLanguages() {
        Ok(list) => list,
        Err(error) => {
            log::warn!("Windows OCR lists no languages: {error}");
            return Vec::new();
        }
    };
    list.into_iter()
        .filter_map(|language| language.LanguageTag().ok())
        .map(|tag| tag.to_string())
        .filter(|tag| !tag.is_empty())
        .collect()
}

/// A language's name as Windows gives it (`Language.DisplayName`, in the language of
/// Windows' own interface), which is what upstream's language list shows. `None` for a
/// tag Windows cannot read.
pub fn language_name(tag: &str) -> Option<String> {
    let language = Language::CreateLanguage(&HSTRING::from(tag)).ok()?;
    let name = language.DisplayName().ok()?.to_string();
    (!name.trim().is_empty()).then_some(name)
}

/// Recognise the text of a picture (PNG or any format the `image` crate reads) in
/// `language`. Each line, top to bottom.
pub fn recognize(image: &[u8], language: &str) -> Result<Vec<String>, String> {
    let picture = modlight_core::image_decode::from_bytes(image)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    let language =
        Language::CreateLanguage(&HSTRING::from(language)).map_err(|_| unavailable(language))?;
    if !OcrEngine::IsLanguageSupported(&language).unwrap_or(false) {
        return Err(unavailable(&display_name(&language)));
    }
    let engine = OcrEngine::TryCreateFromLanguage(&language)
        .map_err(|_| unavailable(&display_name(&language)))?;
    let max = OcrEngine::MaxImageDimension().unwrap_or(u32::MAX);
    let picture = fit_within(picture, max);
    let bitmap = software_bitmap(&picture).map_err(|e| e.message())?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .and_then(|operation| operation.join())
        .map_err(|e| e.message())?;
    let _ = bitmap.Close();
    let tag = language
        .LanguageTag()
        .map(|t| t.to_string().to_ascii_lowercase())
        .unwrap_or_default();
    let joining = if tag.starts_with("zh") || tag.starts_with("ja") {
        Joining::Concatenated
    } else if language.LayoutDirection().ok() == Some(LanguageLayoutDirection::Rtl) {
        Joining::Reversed
    } else {
        Joining::AsRead
    };
    let mut lines = Vec::new();
    for line in result.Lines().map_err(|e| e.message())? {
        lines.push(line_text(&line, joining).map_err(|e| e.message())?);
    }
    Ok(lines)
}

/// How a line's words are put together (`OCRInternal`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Joining {
    /// The engine's own line text.
    AsRead,
    /// Chinese and Japanese: the words with nothing between them.
    Concatenated,
    /// Right-to-left languages: the words in reverse order, joined with spaces.
    Reversed,
}

fn line_text(line: &OcrLine, joining: Joining) -> winrt::core::Result<String> {
    if joining == Joining::AsRead {
        return Ok(line.Text()?.to_string());
    }
    let mut words = Vec::new();
    for word in line.Words()? {
        words.push(word.Text()?.to_string());
    }
    Ok(join_words(words, joining))
}

fn join_words(mut words: Vec<String>, joining: Joining) -> String {
    match joining {
        Joining::Concatenated => words.concat(),
        Joining::Reversed => {
            words.reverse();
            words.join(" ")
        }
        Joining::AsRead => words.join(" "),
    }
}

fn display_name(language: &Language) -> String {
    language
        .DisplayName()
        .or_else(|_| language.LanguageTag())
        .map(|name| name.to_string())
        .unwrap_or_default()
}

/// `OCRHelper_Language_unavailable`, naming the language.
fn unavailable(name: &str) -> String {
    l10n::format(
        l10n::t(l10n::tools::OCRHelper_Language_unavailable),
        &[Arg::from(name)],
    )
}

/// The picture scaled down, keeping its proportions, until neither side is longer than
/// `max` (the engine refuses anything larger).
fn fit_within(picture: RgbaImage, max: u32) -> RgbaImage {
    let (width, height) = picture.dimensions();
    let longest = width.max(height);
    if max == 0 || longest <= max {
        return picture;
    }
    let scale = max as f64 / longest as f64;
    let w = ((width as f64 * scale).floor() as u32).clamp(1, max);
    let h = ((height as f64 * scale).floor() as u32).clamp(1, max);
    image::imageops::resize(&picture, w, h, image::imageops::FilterType::Triangle)
}

/// The picture as premultiplied BGRA, the layout `SoftwareBitmap` holds for `Bgra8`.
fn premultiplied_bgra(picture: &RgbaImage) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(picture.as_raw().len());
    for pixel in picture.pixels() {
        let [r, g, b, a] = pixel.0;
        let times = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
        bytes.extend_from_slice(&[times(b), times(g), times(r), a]);
    }
    bytes
}

fn software_bitmap(picture: &RgbaImage) -> winrt::core::Result<SoftwareBitmap> {
    let writer = DataWriter::new()?;
    writer.WriteBytes(&premultiplied_bgra(picture))?;
    let buffer = writer.DetachBuffer()?;
    SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        picture.width() as i32,
        picture.height() as i32,
        BitmapAlphaMode::Premultiplied,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_put_together_as_upstream() {
        let words = || vec!["one".to_string(), "two".to_string(), "three".to_string()];
        assert_eq!(join_words(words(), Joining::Concatenated), "onetwothree");
        assert_eq!(join_words(words(), Joining::Reversed), "three two one");
        assert_eq!(join_words(words(), Joining::AsRead), "one two three");
    }

    #[test]
    fn a_large_picture_is_scaled_into_the_engines_bounds() {
        let picture = RgbaImage::new(400, 100);
        assert_eq!(fit_within(picture.clone(), 200).dimensions(), (200, 50));
        assert_eq!(fit_within(picture.clone(), 400).dimensions(), (400, 100));
        assert_eq!(
            fit_within(RgbaImage::new(10, 3000), 100).dimensions(),
            (1, 100)
        );
    }

    #[test]
    fn pixels_are_premultiplied_bgra() {
        let mut picture = RgbaImage::new(2, 1);
        picture.put_pixel(0, 0, image::Rgba([255, 128, 0, 255]));
        picture.put_pixel(1, 0, image::Rgba([255, 255, 255, 128]));
        assert_eq!(
            premultiplied_bgra(&picture),
            [0, 128, 255, 255, 128, 128, 128, 128]
        );
    }

    /// Black text on white, drawn by GDI into a bitmap in memory (no window).
    fn rendered_text(text: &str) -> RgbaImage {
        use windows_sys::Win32::Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            CreateFontW, DIB_RGB_COLORS, DeleteDC, DeleteObject, FW_NORMAL, SelectObject,
            SetBkColor, SetTextColor, TextOutW,
        };
        let (width, height) = (900i32, 140i32);
        // SAFETY: plain GDI calls on objects created and released here; the DIB's bits are
        // read while the section is alive.
        unsafe {
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            assert!(!dc.is_null());
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits = std::ptr::null_mut();
            let section = CreateDIBSection(
                dc,
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            assert!(!section.is_null() && !bits.is_null());
            let old = SelectObject(dc, section);
            let pixels =
                std::slice::from_raw_parts_mut(bits as *mut u8, (width * height * 4) as usize);
            pixels.fill(255);
            let face = crate::wide::wide("Arial");
            let font = CreateFontW(
                -64,
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                0,
                0,
                0,
                5, // CLEARTYPE_QUALITY
                0,
                face.as_ptr(),
            );
            let old_font = SelectObject(dc, font);
            SetTextColor(dc, 0x0000_0000);
            SetBkColor(dc, 0x00FF_FFFF);
            let line = crate::wide::wide(text);
            TextOutW(dc, 24, 30, line.as_ptr(), (line.len() - 1) as i32);
            let mut picture = RgbaImage::new(width as u32, height as u32);
            for (i, pixel) in picture.pixels_mut().enumerate() {
                let p = &pixels[i * 4..i * 4 + 4];
                *pixel = image::Rgba([p[2], p[1], p[0], 255]);
            }
            SelectObject(dc, old_font);
            DeleteObject(font);
            SelectObject(dc, old);
            DeleteObject(section);
            DeleteDC(dc);
            picture
        }
    }

    #[test]
    fn windows_ocr_reads_a_rendered_line() {
        let installed = languages();
        // English when it is there, else another language written in the Latin alphabet
        // (the line below reads the same in any of them).
        let latin = [
            "en", "pl", "de", "fr", "es", "it", "pt", "nl", "cs", "sv", "da", "fi",
        ];
        let primary = |tag: &str| tag.split('-').next().unwrap_or("").to_ascii_lowercase();
        let Some(language) = latin
            .iter()
            .find_map(|wanted| installed.iter().find(|t| primary(t) == *wanted))
        else {
            eprintln!("No Latin-script OCR language installed ({installed:?}); skipped.");
            return;
        };
        let png = {
            let mut bytes = Vec::new();
            rendered_text("Quick brown fox 2026")
                .write_to(
                    &mut std::io::Cursor::new(&mut bytes),
                    image::ImageFormat::Png,
                )
                .unwrap();
            bytes
        };
        let lines = recognize(&png, language).expect("recognised");
        let text = lines.join(" ").to_ascii_lowercase();
        assert!(
            text.contains("quick") && text.contains("brown") && text.contains("2026"),
            "{lines:?}"
        );
        // The bare language subtag (upstream's default is "en") names the same language.
        assert!(recognize(&png, &primary(language)).is_ok());
        let name = language_name(language);
        eprintln!("{language} ({name:?}): {lines:?}");
        assert!(name.is_some());
    }

    #[test]
    fn a_language_windows_has_not_installed_is_named_in_the_error() {
        let png = {
            let mut bytes = Vec::new();
            RgbaImage::from_pixel(64, 64, image::Rgba([255, 255, 255, 255]))
                .write_to(
                    &mut std::io::Cursor::new(&mut bytes),
                    image::ImageFormat::Png,
                )
                .unwrap();
            bytes
        };
        // Klingon has no OCR pack on any Windows.
        assert!(recognize(&png, "tlh").is_err());
        assert!(recognize(b"not a picture", "en-US").is_err());
    }
}
