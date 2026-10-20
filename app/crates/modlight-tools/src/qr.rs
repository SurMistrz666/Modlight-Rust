//! The QR code tool (`TaskHelpers.GenerateQRCode`, `BarcodeScan`, `SaveQRCodeAsync`,
//! `QRCodeViewModel`) on rxing, a Rust port of the ZXing library upstream uses
//! (ZXing.Net): the same encoder, the same rendering of modules into pixels, the same
//! readers (every format, try harder, inverted, rotated).

use image::{Rgba, RgbaImage};
use rxing::common::BitMatrix;
use rxing::{
    BarcodeFormat, BinaryBitmap, DecodeHints, EncodeHints, Luma8LuminanceSource, MultiFormatWriter,
    MultiUseMultiFormatReader, Writer,
};

/// `CheckQRCodeContent`: what a QR code can hold in UTF-8.
pub const MAXIMUM_CONTENT_BYTES: usize = 2952;

/// `CheckQRCodeContent`.
pub fn check_content(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAXIMUM_CONTENT_BYTES
}

/// `QRCodeViewModel.EffectiveSize`: the size box, or the preview's room, at least 64.
pub fn effective_size(size_setting: i64, available: i64) -> i64 {
    (if size_setting > 0 {
        size_setting
    } else {
        available
    })
    .max(64)
}

/// The module matrix of `text` at `size` x `size` pixels, with the quiet zone `margin`
/// (`QrCodeEncodingOptions`: UTF-8, error correction L).
pub fn matrix(text: &str, size: u32, margin: Option<u32>) -> Result<BitMatrix, String> {
    let mut hints = EncodeHints {
        CharacterSet: Some("UTF-8".into()),
        ..EncodeHints::default()
    };
    if let Some(margin) = margin {
        hints.Margin = Some(margin.to_string());
    }
    MultiFormatWriter
        .encode_with_hints(
            text,
            &BarcodeFormat::QR_CODE,
            size as i32,
            size as i32,
            &hints,
        )
        .map_err(|e| e.to_string())
}

/// `GenerateQRCode(text, size)`: black modules on white, margin 1; `None` for content a
/// QR code cannot hold.
pub fn generate(text: &str, size: u32) -> Option<RgbaImage> {
    if !check_content(text) {
        return None;
    }
    let matrix = matrix(text, size, Some(1)).ok()?;
    Some(render(&matrix))
}

/// `BitmapRenderer`: one pixel per matrix cell.
pub fn render(matrix: &BitMatrix) -> RgbaImage {
    let (w, h) = (matrix.getWidth(), matrix.getHeight());
    let mut image = RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255]));
    for y in 0..h {
        for x in 0..w {
            if matrix.get(x, y) {
                image.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
    }
    image
}

/// `BarcodeWriterSvg` (no margin given: ZXing's default quiet zone of 4): the matrix as
/// rectangles, one per run of dark cells in a row.
pub fn svg(text: &str, size: u32) -> Result<String, String> {
    let matrix = matrix(text, size, None)?;
    let (w, h) = (matrix.getWidth(), matrix.getHeight());
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" standalone=\"no\"?>");
    out.push_str("<!-- Created with ZXing.Net (http://zxingnet.codeplex.com/) -->");
    out.push_str("<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">");
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.2\" baseProfile=\"tiny\" shape-rendering=\"crispEdges\" viewBox=\"0 0 {w} {h}\" viewport-fill=\"rgb(255,255,255)\" viewport-fill-opacity=\"1\" fill=\"rgb(0,0,0)\" fill-opacity=\"1\" style=\"background-color:rgb(255,255,255);background-color:rgba(255,255,255,1);\">"
    ));
    for y in 0..h {
        let mut x = 0;
        while x < w {
            if !matrix.get(x, y) {
                x += 1;
                continue;
            }
            let start = x;
            while x < w && matrix.get(x, y) {
                x += 1;
            }
            out.push_str(&format!(
                "<rect x=\"{start}\" y=\"{y}\" width=\"{}\" height=\"1\"/>",
                x - start
            ));
        }
    }
    out.push_str("</svg>");
    Ok(out)
}

/// ZXing.Net's `BitmapLuminanceSource`: weighted RGB, transparent pixels count as white.
pub fn luminance(image: &RgbaImage) -> Vec<u8> {
    image
        .pixels()
        .map(|p| {
            let [r, g, b, a] = p.0;
            let luma = (19562u32 * r as u32 + 38550 * g as u32 + 7424 * b as u32) >> 16;
            let a = a as u32;
            (((luma * a) >> 8) + ((255 * (255 - a)) >> 8)).min(255) as u8
        })
        .collect()
}

fn decode_once(luma: &[u8], w: u32, h: u32, qr_only: bool) -> Vec<String> {
    let Ok(source) = Luma8LuminanceSource::new(luma.to_vec(), w, h) else {
        return Vec::new();
    };
    let mut hints = DecodeHints {
        TryHarder: Some(true),
        ..DecodeHints::default()
    };
    if qr_only {
        hints.PossibleFormats = Some([BarcodeFormat::QR_CODE].into_iter().collect());
    }
    let mut reader =
        rxing::multi::GenericMultipleBarcodeReader::new(MultiUseMultiFormatReader::default());
    let mut bitmap = BinaryBitmap::new(rxing::common::HybridBinarizer::new(source));
    use rxing::multi::MultipleBarcodeReader;
    reader
        .decode_multiple_with_hints(&mut bitmap, &hints)
        .map(|results| {
            results
                .into_iter()
                .map(|r| r.getText().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// `BarcodeScan(bitmap, scanQRCodeOnly)`: `AutoRotate`, `TryHarder`, `TryInverted`,
/// `DecodeMultiple`. Every code found, in reading order; empty when there is none.
pub fn scan(image: &RgbaImage, qr_only: bool) -> Vec<String> {
    let (mut w, mut h) = image.dimensions();
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let mut luma = luminance(image);
    for _rotation in 0..4 {
        let found = decode_once(&luma, w, h, qr_only);
        if !found.is_empty() {
            return found;
        }
        let inverted: Vec<u8> = luma.iter().map(|v| 255 - v).collect();
        let found = decode_once(&inverted, w, h, qr_only);
        if !found.is_empty() {
            return found;
        }
        // `rotateCounterClockwise`.
        let mut rotated = vec![0u8; luma.len()];
        for y in 0..h {
            for x in 0..w {
                let (nx, ny) = (y, w - 1 - x);
                rotated[(ny * h + nx) as usize] = luma[(y * w + x) as usize];
            }
        }
        luma = rotated;
        std::mem::swap(&mut w, &mut h);
    }
    Vec::new()
}

/// `ApplyScanResultsAsync`: the texts, a blank line between them.
pub fn join_results(results: &[String]) -> String {
    results.join("\n\n")
}

/// `GetSuggestedFileName`: the first 80 characters, file-name safe, ".png". The
/// characters replaced are .NET's `Path.GetInvalidFileNameChars` on Windows, so a name
/// suggested here is one a Windows install would suggest too.
pub fn suggested_file_name(text: &str) -> String {
    let invalid = |c: char| {
        matches!(c, '"' | '<' | '>' | '|' | ':' | '*' | '?' | '\\' | '/') || (c as u32) < 32
    };
    let name: String = text
        .chars()
        .take(80)
        .map(|c| if invalid(c) { '_' } else { c })
        .collect();
    let name = name.trim();
    if name.is_empty() {
        "qr-code.png".into()
    } else {
        format!("{name}.png")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_limits() {
        assert!(!check_content(""));
        assert!(check_content("a"));
        assert!(check_content(&"a".repeat(2952)));
        assert!(!check_content(&"a".repeat(2953)));
        // Bytes, not characters.
        assert!(!check_content(&"ą".repeat(1477)));
        assert_eq!(effective_size(0, 30), 64);
        assert_eq!(effective_size(0, 380), 380);
        assert_eq!(effective_size(512, 380), 512);
    }

    #[test]
    fn generated_codes_read_back() {
        for text in [
            "https://files.example.com/alex/q8",
            "Zażółć gęślą jaźń",
            &"0123456789".repeat(40),
        ] {
            let image = generate(text, 400).expect("a code");
            assert!(image.width() >= 400);
            assert_eq!(scan(&image, false), vec![text.to_string()]);
            assert_eq!(scan(&image, true), vec![text.to_string()]);
        }
    }

    #[test]
    fn rendering_follows_zxing() {
        // "Hi" is a version 1 code: 21 modules + 1 quiet module each side = 23; at 100
        // pixels the multiple is 4 (92 px) and the padding (100 - 84) / 2 = 8.
        let image = generate("Hi", 100).unwrap();
        assert_eq!(image.dimensions(), (100, 100));
        assert_eq!(image.get_pixel(7, 7).0, [255, 255, 255, 255]);
        assert_eq!(image.get_pixel(8, 8).0, [0, 0, 0, 255]);
        assert_eq!(image.get_pixel(8 + 21 * 4 - 1, 8).0, [0, 0, 0, 255]);
        assert_eq!(image.get_pixel(8 + 21 * 4, 8).0, [255, 255, 255, 255]);
        // Smaller than the code: the code's own size.
        let image = generate("Hi", 10).unwrap();
        assert_eq!(image.dimensions(), (23, 23));
    }

    #[test]
    fn inverted_and_rotated_codes() {
        let mut image = generate("inverted", 200).unwrap();
        for p in image.pixels_mut() {
            p.0 = [255 - p.0[0], 255 - p.0[1], 255 - p.0[2], 255];
        }
        assert_eq!(scan(&image, false), vec!["inverted".to_string()]);
        let rotated = image::imageops::rotate90(&generate("turned", 200).unwrap());
        assert_eq!(scan(&rotated, false), vec!["turned".to_string()]);
    }

    #[test]
    fn several_codes_in_one_picture() {
        let a = generate("first", 160).unwrap();
        let b = generate("second", 160).unwrap();
        let mut canvas = RgbaImage::from_pixel(400, 200, Rgba([255, 255, 255, 255]));
        image::imageops::overlay(&mut canvas, &a, 10, 20);
        image::imageops::overlay(&mut canvas, &b, 220, 20);
        let mut found = scan(&canvas, false);
        found.sort();
        assert_eq!(found, vec!["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn nothing_found() {
        let blank = RgbaImage::from_pixel(50, 50, Rgba([255, 255, 255, 255]));
        assert!(scan(&blank, false).is_empty());
        // Transparent counts as white.
        let clear = RgbaImage::new(30, 30);
        assert!(luminance(&clear).iter().all(|v| *v == 254 || *v == 255));
    }

    #[test]
    fn svg_output() {
        let svg = svg("Hi", 29).unwrap();
        assert!(svg.starts_with("<?xml version=\"1.0\" standalone=\"no\"?>"));
        assert!(svg.contains("viewBox=\"0 0 29 29\""));
        assert!(svg.contains("<rect x=\"4\" y=\"4\" width=\"7\" height=\"1\"/>"));
        assert!(svg.ends_with("</svg>"));
    }

    #[test]
    fn file_names() {
        assert_eq!(suggested_file_name("https://a/b"), "https___a_b.png");
        assert_eq!(suggested_file_name("  "), "qr-code.png");
        assert_eq!(suggested_file_name(&"x".repeat(100)).len(), 84);
    }
}
