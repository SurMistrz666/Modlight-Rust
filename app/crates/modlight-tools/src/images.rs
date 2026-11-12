//! Reading and writing pictures the way the tools do (`ImageHelpers.LoadImage`,
//! `ImageHelpers.SaveImage`): EXIF orientation applied on load while the application
//! setting asks for it, the format chosen by the file's extension on save. Every decode
//! runs under `modlight_core::image_decode`'s limits (pictures come from outside the app:
//! opened, dropped, pasted files).

use std::io::Cursor;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbaImage};

/// `HelpersOptions.RotateImageByExifOrientationData` (`ApplicationConfig`, on by default):
/// whether [`load`] and [`load_bytes`] turn a picture by its EXIF orientation. The app
/// sets it from its settings, as upstream's main form does.
static ROTATE_BY_EXIF: AtomicBool = AtomicBool::new(true);

/// Follow the application's `RotateImageByExifOrientationData`.
pub fn set_rotate_by_exif(rotate: bool) {
    ROTATE_BY_EXIF.store(rotate, Ordering::Relaxed);
}

/// Whether pictures are turned by their EXIF orientation now.
pub fn rotate_by_exif() -> bool {
    ROTATE_BY_EXIF.load(Ordering::Relaxed)
}

/// Load a picture, turned by its EXIF orientation when the setting asks for it.
pub fn load(path: &Path) -> anyhow::Result<RgbaImage> {
    load_turned(path, rotate_by_exif())
}

/// Load a picture, turned by its EXIF orientation only when `rotate` is set.
pub fn load_turned(path: &Path, rotate: bool) -> anyhow::Result<RgbaImage> {
    let reader = ImageReader::open(path)
        .with_context(|| path.display().to_string())?
        .with_guessed_format()?;
    decode(reader, rotate)
}

/// Load a picture from bytes (a PNG from the clipboard, a download).
pub fn load_bytes(bytes: &[u8]) -> anyhow::Result<RgbaImage> {
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    decode(reader, rotate_by_exif())
}

fn decode<R: std::io::BufRead + std::io::Seek>(
    mut reader: ImageReader<R>,
    rotate: bool,
) -> anyhow::Result<RgbaImage> {
    reader.limits(modlight_core::image_decode::limits());
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation().ok().filter(|_| rotate);
    let mut image = DynamicImage::from_decoder(decoder)?;
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    Ok(image.to_rgba8())
}

/// The format a file name asks for (`ImageHelpers.GetImageFormat`); PNG when unknown.
pub fn format_for(path: &Path) -> ImageFormat {
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "jpg" | "jpeg" | "jpe" | "jfif" => ImageFormat::Jpeg,
        "gif" => ImageFormat::Gif,
        "bmp" => ImageFormat::Bmp,
        "tif" | "tiff" => ImageFormat::Tiff,
        "webp" => ImageFormat::WebP,
        "ico" => ImageFormat::Ico,
        _ => ImageFormat::Png,
    }
}

/// Encode for a file name's format (JPEG and BMP lose the alpha channel, as GDI+'s do).
pub fn encode(image: &RgbaImage, format: ImageFormat) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let dynamic = DynamicImage::ImageRgba8(image.clone());
    match format {
        ImageFormat::Jpeg => {
            let rgb = dynamic.to_rgb8();
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 90);
            encoder.encode_image(&rgb)?;
        }
        ImageFormat::Bmp => {
            DynamicImage::ImageRgb8(dynamic.to_rgb8())
                .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Bmp)?;
        }
        other => dynamic.write_to(&mut Cursor::new(&mut bytes), other)?,
    }
    Ok(bytes)
}

/// `ImageHelpers.SaveImage(image, filePath)`.
pub fn save(image: &RgbaImage, path: &Path) -> anyhow::Result<()> {
    let bytes = encode(image, format_for(path))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

/// PNG bytes of a picture.
pub fn png(image: &RgbaImage) -> Vec<u8> {
    encode(image, ImageFormat::Png).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_by_extension() {
        let dir = tempfile::tempdir().unwrap();
        let mut image = RgbaImage::new(4, 2);
        image.put_pixel(3, 1, image::Rgba([10, 20, 30, 255]));
        for name in ["a.png", "a.jpg", "a.bmp", "a.gif", "a.tif", "a.webp"] {
            let path = dir.path().join(name);
            save(&image, &path).unwrap();
            let back = load(&path).unwrap();
            assert_eq!(back.dimensions(), (4, 2), "{name}");
        }
        assert_eq!(format_for(Path::new("x.JPEG")), ImageFormat::Jpeg);
        assert_eq!(format_for(Path::new("x")), ImageFormat::Png);
        let back = load_bytes(&png(&image)).unwrap();
        assert_eq!(back.get_pixel(3, 1).0, [10, 20, 30, 255]);
    }

    /// A 4 x 2 JPEG whose EXIF says "turn 90 degrees clockwise" (orientation 6), as a
    /// phone writes a portrait photo.
    fn jpeg_turned_by_exif() -> Vec<u8> {
        let jpeg = encode(&RgbaImage::new(4, 2), ImageFormat::Jpeg).unwrap();
        // A little-endian TIFF header and one IFD entry: Orientation (0x0112), SHORT, 6.
        let mut tiff: Vec<u8> = b"II*\0".to_vec();
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x0112u16.to_le_bytes());
        tiff.extend_from_slice(&3u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&6u16.to_le_bytes());
        tiff.extend_from_slice(&[0, 0]);
        tiff.extend_from_slice(&0u32.to_le_bytes());
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend_from_slice(&tiff);
        let mut out = jpeg[..2].to_vec();
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&app1);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    /// `ImageHelpers.LoadImage` turns a picture by its EXIF orientation only while
    /// `RotateImageByExifOrientationData` is on; with it off the stored pixels come back
    /// as they are.
    #[test]
    fn exif_orientation_follows_the_setting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("portrait.jpg");
        std::fs::write(&path, jpeg_turned_by_exif()).unwrap();
        assert_eq!(load_turned(&path, true).unwrap().dimensions(), (2, 4));
        assert_eq!(load_turned(&path, false).unwrap().dimensions(), (4, 2));
        // On by default, as upstream.
        assert!(rotate_by_exif());
        assert_eq!(load(&path).unwrap().dimensions(), (2, 4));
    }
}
