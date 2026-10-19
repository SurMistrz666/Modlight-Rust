//! Turning a picture into the bytes that are saved and uploaded
//! (`TaskHelpers.PrepareImage`, `SaveImageAsStream`, `CreateThumbnail`).

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageFormat as Codec, Rgba, RgbaImage};

use crate::tasks::settings::{GifQuality, ImageFormat, PngBitDepth, TaskSettings};

/// `ImageData`: the encoded bytes and the format they ended up in.
#[derive(Clone, Debug)]
pub struct PreparedImage {
    pub bytes: Vec<u8>,
    pub format: ImageFormat,
}

/// `PrepareImage`: the configured format, switching to JPEG when a lossless encoding is
/// over `ImageAutoUseJPEGSize` KB (decimal kilobytes, as upstream multiplies by 1000).
pub fn prepare_image(
    image: &RgbaImage,
    settings: &TaskSettings,
    _strip_png_color_space: bool,
) -> PreparedImage {
    let images = &settings.image_settings;
    let mut prepared = PreparedImage {
        bytes: save_image(
            image,
            images.image_format,
            images.image_png_bit_depth,
            images.image_jpeg_quality,
            images.image_gif_quality,
        ),
        format: images.image_format,
    };
    let limit = images.image_auto_use_jpeg_size.max(0) as usize * 1000;
    if images.image_auto_use_jpeg
        && images.image_format != ImageFormat::JPEG
        && prepared.bytes.len() > limit
    {
        let filled = fill_background(image, Rgba([255, 255, 255, 255]));
        prepared.bytes = if images.image_auto_jpeg_quality {
            jpeg_auto_quality(&filled, limit, 2, 70, 100)
        } else {
            jpeg(&filled, images.image_jpeg_quality)
        };
        prepared.format = ImageFormat::JPEG;
    }
    // A PNG written here carries no gAMA/cHRM/sRGB/iCCP chunks to begin with, so
    // `PNGStripColorSpaceInformation` has nothing to strip.
    prepared
}

/// `SaveImageAsStream`.
pub fn save_image(
    image: &RgbaImage,
    format: ImageFormat,
    png_bit_depth: PngBitDepth,
    jpeg_quality: i64,
    gif_quality: GifQuality,
) -> Vec<u8> {
    match format {
        ImageFormat::PNG => png(image, png_bit_depth),
        ImageFormat::JPEG => jpeg(
            &fill_background(image, Rgba([255, 255, 255, 255])),
            jpeg_quality,
        ),
        ImageFormat::GIF => gif(image, gif_quality),
        ImageFormat::BMP => encode(&DynamicImage::ImageRgba8(image.clone()), Codec::Bmp),
        ImageFormat::TIFF => encode(&DynamicImage::ImageRgba8(image.clone()), Codec::Tiff),
    }
}

fn encode(image: &DynamicImage, codec: Codec) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    if let Err(error) = image.write_to(&mut out, codec) {
        log::warn!("Modlight: the image could not be encoded: {error}");
    }
    out.into_inner()
}

/// `SavePNG`: 32-bit unless asked otherwise; Automatic drops the alpha channel when
/// no pixel uses it.
fn png(image: &RgbaImage, depth: PngBitDepth) -> Vec<u8> {
    let depth = match depth {
        PngBitDepth::Automatic => {
            if is_transparent(image) {
                PngBitDepth::Bit32
            } else {
                PngBitDepth::Bit24
            }
        }
        other => other,
    };
    match depth {
        PngBitDepth::Bit24 => encode(
            &DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(image.clone()).to_rgb8()),
            Codec::Png,
        ),
        _ => encode(&DynamicImage::ImageRgba8(image.clone()), Codec::Png),
    }
}

fn is_transparent(image: &RgbaImage) -> bool {
    image.pixels().any(|p| p.0[3] < 255)
}

/// `SaveJPEG`.
pub fn jpeg(image: &RgbaImage, quality: i64) -> Vec<u8> {
    let rgb = DynamicImage::ImageRgba8(image.clone()).to_rgb8();
    let mut out = Vec::new();
    let quality = quality.clamp(1, 100) as u8;
    if let Err(error) = JpegEncoder::new_with_quality(&mut out, quality).encode_image(&rgb) {
        log::warn!("Modlight: the image could not be encoded as JPEG: {error}");
    }
    out
}

/// `SaveJPEGAutoQuality`: from the top quality down until the bytes fit.
pub fn jpeg_auto_quality(
    image: &RgbaImage,
    size_limit: usize,
    decrement: i64,
    min: i64,
    max: i64,
) -> Vec<u8> {
    let decrement = decrement.clamp(1, 100);
    let min = min.clamp(0, 100);
    let max = max.clamp(0, 100);
    if min >= max {
        return jpeg(image, min);
    }
    let mut quality = max;
    let mut bytes = Vec::new();
    while quality >= min {
        bytes = jpeg(image, quality);
        if bytes.len() <= size_limit {
            break;
        }
        quality -= decrement;
    }
    bytes
}

/// `SaveGIF`. The GIF encoder quantises to 256 colours (NeuQuant); Grayscale converts
/// first. `Bit4` (16 colours) has no counterpart in the encoder and is written as 256.
fn gif(image: &RgbaImage, quality: GifQuality) -> Vec<u8> {
    let source = match quality {
        GifQuality::Grayscale => {
            DynamicImage::ImageLumaA8(DynamicImage::ImageRgba8(image.clone()).to_luma_alpha8())
                .to_rgba8()
        }
        _ => image.clone(),
    };
    encode(&DynamicImage::ImageRgba8(source), Codec::Gif)
}

/// `ImageHelpers.FillBackground`: the picture over a solid colour.
pub fn fill_background(image: &RgbaImage, color: Rgba<u8>) -> RgbaImage {
    let mut out = RgbaImage::from_pixel(image.width(), image.height(), color);
    for (x, y, p) in image.enumerate_pixels() {
        let a = p.0[3] as u32;
        if a == 255 {
            out.put_pixel(x, y, *p);
        } else if a > 0 {
            let back = out.get_pixel(x, y).0;
            let mix = |f: u8, b: u8| ((f as u32 * a + b as u32 * (255 - a)) / 255) as u8;
            out.put_pixel(
                x,
                y,
                Rgba([
                    mix(p.0[0], back[0]),
                    mix(p.0[1], back[1]),
                    mix(p.0[2], back[2]),
                    255,
                ]),
            );
        }
    }
    out
}

/// `CreateThumbnail`'s picture: resized by `Resize(width, height)` (a zero side keeps
/// the aspect ratio), over white, as JPEG quality 90.
pub fn thumbnail_jpeg(image: &RgbaImage, width: i64, height: i64) -> Option<Vec<u8>> {
    let (w, h) = (image.width() as f64, image.height() as f64);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let (tw, th) = match (width.max(0) as f64, height.max(0) as f64) {
        (0.0, 0.0) => return None,
        (tw, 0.0) => (tw, (h * tw / w).round()),
        (0.0, th) => ((w * th / h).round(), th),
        (tw, th) => (tw, th),
    };
    let resized = image::imageops::resize(
        image,
        tw.max(1.0) as u32,
        th.max(1.0) as u32,
        image::imageops::FilterType::Lanczos3,
    );
    Some(jpeg(
        &fill_background(&resized, Rgba([255, 255, 255, 255])),
        90,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: u32, h: u32, alpha: u8) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            Rgba([(x * 7) as u8, (y * 13) as u8, 90, alpha])
        })
    }

    #[test]
    fn png_by_default_and_jpeg_when_too_big() {
        let mut settings = TaskSettings::default();
        let small = prepare_image(&picture(20, 10, 255), &settings, false);
        assert_eq!(small.format, ImageFormat::PNG);
        assert_eq!(&small.bytes[1..4], b"PNG");

        settings.image_settings.image_auto_use_jpeg_size = 0;
        let big = prepare_image(&picture(64, 64, 255), &settings, false);
        assert_eq!(big.format, ImageFormat::JPEG);
        assert_eq!(&big.bytes[..2], &[0xFF, 0xD8]);

        settings.image_settings.image_auto_use_jpeg = false;
        let kept = prepare_image(&picture(64, 64, 255), &settings, false);
        assert_eq!(kept.format, ImageFormat::PNG);
    }

    #[test]
    fn every_format_encodes_and_reads_back() {
        let image = picture(16, 8, 200);
        for format in ImageFormat::ALL {
            let bytes = save_image(
                &image,
                *format,
                PngBitDepth::Automatic,
                80,
                GifQuality::Default,
            );
            let back =
                image::load_from_memory(&bytes).unwrap_or_else(|e| panic!("{format:?}: {e}"));
            assert_eq!((back.width(), back.height()), (16, 8), "{format:?}");
        }
        assert_eq!(ImageFormat::TIFF.extension(), "tif");
    }

    #[test]
    fn automatic_png_drops_an_unused_alpha_channel() {
        let opaque = png(&picture(4, 4, 255), PngBitDepth::Automatic);
        assert_eq!(
            image::load_from_memory(&opaque).unwrap().color(),
            image::ColorType::Rgb8
        );
        let clear = png(&picture(4, 4, 10), PngBitDepth::Automatic);
        assert_eq!(
            image::load_from_memory(&clear).unwrap().color(),
            image::ColorType::Rgba8
        );
    }

    #[test]
    fn background_fill_blends_over_white() {
        let image = RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 0]));
        assert_eq!(
            fill_background(&image, Rgba([255, 255, 255, 255]))
                .get_pixel(0, 0)
                .0,
            [255, 255, 255, 255]
        );
        let half = RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 128]));
        let p = fill_background(&half, Rgba([255, 255, 255, 255]))
            .get_pixel(0, 0)
            .0;
        assert!((126..=128).contains(&p[0]), "{p:?}");
    }

    #[test]
    fn thumbnails_keep_the_aspect_ratio() {
        let bytes = thumbnail_jpeg(&picture(400, 200, 255), 200, 0).unwrap();
        let thumb = image::load_from_memory(&bytes).unwrap();
        assert_eq!((thumb.width(), thumb.height()), (200, 100));
        assert!(thumbnail_jpeg(&picture(4, 4, 255), 0, 0).is_none());
    }

    #[test]
    fn auto_quality_steps_down_until_it_fits() {
        let image = picture(64, 64, 255);
        let best = jpeg(&image, 100).len();
        let fitted = jpeg_auto_quality(&image, best - 1, 2, 70, 100);
        assert!(fitted.len() < best);
    }
}
