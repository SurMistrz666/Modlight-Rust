//! Reading and writing pictures the way the tools do (`ImageHelpers.LoadImage`,
//! `ImageHelpers.SaveImage`): EXIF orientation applied on load, the format chosen by the
//! file's extension on save. Every decode runs under `modlight_core::image_decode`'s
//! limits (pictures come from outside the app: opened, dropped, pasted files).

use std::io::Cursor;
use std::path::Path;

use anyhow::Context;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbaImage};

/// Load a picture, turned by its EXIF orientation.
pub fn load(path: &Path) -> anyhow::Result<RgbaImage> {
    let mut reader = ImageReader::open(path)
        .with_context(|| path.display().to_string())?
        .with_guessed_format()?;
    reader.limits(modlight_core::image_decode::limits());
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation().ok();
    let mut image = DynamicImage::from_decoder(decoder)?;
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    Ok(image.to_rgba8())
}

/// Load a picture from bytes (a PNG from the clipboard, a download).
pub fn load_bytes(bytes: &[u8]) -> anyhow::Result<RgbaImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    reader.limits(modlight_core::image_decode::limits());
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation().ok();
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
}
