//! Decoding pictures from bytes this app did not make: thumbnails, previews and the
//! account picture from the server, addresses the person pasted, the clipboard, files
//! opened in the editor or uploaded. Every decode runs under [`image::Limits`], so a
//! small file whose header claims an enormous picture (a PNG saying 30000 x 30000) is
//! refused before anything is allocated instead of taking gigabytes and the app with it.
//!
//! The limits are far above any real screenshot: three 6K displays side by side are
//! 18048 px wide and 240 MB as RGBA, a long scrolling capture 2880 px wide and 32768 px
//! tall is 380 MB.

use std::io::Cursor;
use std::path::Path;

use image::{DynamicImage, ImageFormat, ImageReader, Limits};

/// The widest or tallest picture decoded.
pub const MAX_SIDE: u32 = 32_768;

/// The most memory one still picture's decode may take (the `image` crate's own default).
pub const MAX_ALLOC: u64 = 512 * 1024 * 1024;

/// The most memory one frame of an animation may take while it is decoded: the canvas,
/// the frame and the previous canvas together. A 4K GIF takes about 100 MB.
pub const MAX_ANIMATION_ALLOC: u64 = 128 * 1024 * 1024;

/// The limits for a still picture.
pub fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    limits
}

/// The limits for each frame of an animated GIF or WebP.
pub fn animation_limits() -> Limits {
    let mut limits = limits();
    limits.max_alloc = Some(MAX_ANIMATION_ALLOC);
    limits
}

/// A picture from encoded bytes, its format read from the bytes (`load_from_memory`).
pub fn from_bytes(bytes: &[u8]) -> image::ImageResult<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    reader.limits(limits());
    reader.decode()
}

/// A picture from encoded bytes of a known format (`load_from_memory_with_format`).
pub fn from_bytes_as(bytes: &[u8], format: ImageFormat) -> image::ImageResult<DynamicImage> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits());
    reader.decode()
}

/// A picture file, its format from the extension as `image::open` reads it.
pub fn open(path: &Path) -> image::ImageResult<DynamicImage> {
    let mut reader = ImageReader::open(path)?;
    reader.limits(limits());
    reader.decode()
}

/// Whether a picture of this size may be decoded at all: for decoders that cannot take
/// [`Limits`] (the system's, through `NSImage`), asked before they are.
pub fn fits(width: u64, height: u64) -> bool {
    width > 0
        && height > 0
        && width <= MAX_SIDE as u64
        && height <= MAX_SIDE as u64
        && width.saturating_mul(height).saturating_mul(4) <= MAX_ALLOC
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(format: ImageFormat) -> Vec<u8> {
        let picture = image::RgbImage::from_pixel(16, 16, image::Rgb([200, 30, 30]));
        let mut bytes = Vec::new();
        picture
            .write_to(&mut Cursor::new(&mut bytes), format)
            .unwrap();
        bytes
    }

    /// A real 16 x 16 PNG whose header now says `width` x `height` (CRC fixed): what a
    /// hostile server would send as a thumbnail, a few hundred bytes long.
    pub(crate) fn png_claiming(width: u32, height: u32) -> Vec<u8> {
        let mut png = encoded(ImageFormat::Png);
        // Signature (8), length (4), "IHDR" (4), then width and height.
        assert_eq!(&png[12..16], b"IHDR");
        png[16..20].copy_from_slice(&width.to_be_bytes());
        png[20..24].copy_from_slice(&height.to_be_bytes());
        let crc = crc32(&png[12..29]);
        png[29..33].copy_from_slice(&crc.to_be_bytes());
        png
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for byte in bytes {
            crc ^= *byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// A real 16 x 16 JPEG whose frame header (SOF0) now says `width` x `height`.
    fn jpeg_claiming(width: u16, height: u16) -> Vec<u8> {
        let mut jpeg = encoded(ImageFormat::Jpeg);
        let sof = jpeg
            .windows(2)
            .position(|w| w == [0xff, 0xc0])
            .expect("a baseline frame header");
        // Marker (2), length (2), precision (1), then height and width.
        jpeg[sof + 5..sof + 7].copy_from_slice(&height.to_be_bytes());
        jpeg[sof + 7..sof + 9].copy_from_slice(&width.to_be_bytes());
        jpeg
    }

    fn is_limit(error: &image::ImageError) -> bool {
        matches!(error, image::ImageError::Limits(_))
    }

    #[test]
    fn a_header_claiming_an_enormous_picture_is_refused_by_its_size() {
        // 30000 x 30000 RGBA would be 3.6 GB.
        let error = from_bytes(&png_claiming(30_000, 30_000)).unwrap_err();
        assert!(is_limit(&error), "{error:?}");
        // One side past the limit, however few pixels.
        let error = from_bytes(&png_claiming(MAX_SIDE + 1, 1)).unwrap_err();
        assert!(is_limit(&error), "{error:?}");
        let error = from_bytes_as(&png_claiming(30_000, 30_000), ImageFormat::Png).unwrap_err();
        assert!(is_limit(&error), "{error:?}");

        // 65535 x 65535 is a JPEG's largest: 12.9 GB as RGB.
        let error = from_bytes(&jpeg_claiming(65_535, 65_535)).unwrap_err();
        assert!(is_limit(&error), "{error:?}");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.png");
        std::fs::write(&path, png_claiming(30_000, 30_000)).unwrap();
        assert!(is_limit(&open(&path).unwrap_err()));
        let path = dir.path().join("huge.jpg");
        std::fs::write(&path, jpeg_claiming(40_000, 40_000)).unwrap();
        assert!(is_limit(&open(&path).unwrap_err()));
    }

    #[test]
    fn real_pictures_still_decode() {
        let picture = image::RgbaImage::from_pixel(1920, 1080, image::Rgba([10, 20, 30, 255]));
        let mut png = Vec::new();
        picture
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let decoded = from_bytes(&png).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1920, 1080));
        let mut jpeg = Vec::new();
        DynamicImage::ImageRgba8(picture)
            .to_rgb8()
            .write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg)
            .unwrap();
        assert_eq!(from_bytes(&jpeg).unwrap().width(), 1920);
        // A long scrolling capture is under the limits.
        assert!(fits(2880, 32_768));
        assert!(fits(18_048, 3_384));
        assert!(!fits(30_000, 30_000));
        assert!(!fits(MAX_SIDE as u64 + 1, 1));
        assert!(!fits(0, 10));
    }
}
