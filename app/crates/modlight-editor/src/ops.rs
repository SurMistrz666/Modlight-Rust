//! The image operations `EditorCore` applies itself (`ImageHelpers` and the manipulation
//! effects it calls): resize, rotate, flip, canvas size, auto crop.

use image::{Rgba, RgbaImage};
use tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};

use crate::raster::{self, Filter};

/// `ResizeImageEffect` (CatmullRom cubic).
pub fn resize(image: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    raster::resize(image, width, height, Filter::CatmullRom)
}

/// `ResizeImageEffect.ResolveTargetSize` with the `LargestDimension` anchor.
pub fn resolve_resize_target(
    source: (u32, u32),
    requested: (i32, i32),
    maintain_aspect_ratio: bool,
) -> (u32, u32) {
    let (sw, sh) = (source.0 as f64, source.1 as f64);
    let width = requested.0.max(0);
    let height = requested.1.max(0);
    if !maintain_aspect_ratio {
        let w = if width > 0 { width as u32 } else { source.0 };
        let h = if height > 0 { height as u32 } else { source.1 };
        return (w, h);
    }
    if width <= 0 && height <= 0 {
        return source;
    }
    if width <= 0 {
        let w = ((height as f64 / sh * sw).round() as i64).max(1) as u32;
        return (w, height as u32);
    }
    if height <= 0 {
        let h = ((width as f64 / sw * sh).round() as i64).max(1) as u32;
        return (width as u32, h);
    }
    let scale = (width as f64 / sw).max(height as f64 / sh);
    (
        ((sw * scale).round() as i64).max(1) as u32,
        ((sh * scale).round() as i64).max(1) as u32,
    )
}

/// `RotateImageEffect` for multiples of 90 degrees: an exact pixel move.
pub fn rotate_orthogonal(image: &RgbaImage, angle: i32) -> RgbaImage {
    let angle = angle.rem_euclid(360);
    match angle {
        90 => image::imageops::rotate90(image),
        180 => image::imageops::rotate180(image),
        270 => image::imageops::rotate270(image),
        _ => image.clone(),
    }
}

/// `RotateImageEffect.Custom(angle, autoResize)`.
pub fn rotate_custom(image: &RgbaImage, angle: f32, auto_resize: bool) -> RgbaImage {
    if angle % 90.0 == 0.0 && auto_resize {
        return rotate_orthogonal(image, angle as i32);
    }
    let (w, h) = (image.width() as f32, image.height() as f32);
    let (out_w, out_h) = if auto_resize {
        let rad = angle.to_radians();
        let (s, c) = rad.sin_cos();
        // The rotated rectangle's bounds (`SKMatrix.MapRect`).
        let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].map(|(x, y)| {
            let dx = x - w / 2.0;
            let dy = y - h / 2.0;
            (w / 2.0 + dx * c - dy * s, h / 2.0 + dx * s + dy * c)
        });
        let min_x = corners.iter().map(|p| p.0).fold(f32::MAX, f32::min);
        let max_x = corners.iter().map(|p| p.0).fold(f32::MIN, f32::max);
        let min_y = corners.iter().map(|p| p.1).fold(f32::MAX, f32::min);
        let max_y = corners.iter().map(|p| p.1).fold(f32::MIN, f32::max);
        ((max_x - min_x).ceil() as u32, (max_y - min_y).ceil() as u32)
    } else {
        (image.width(), image.height())
    };
    let Some(mut target) = Pixmap::new(out_w.max(1), out_h.max(1)) else {
        return image.clone();
    };
    let source = raster::to_pixmap(image);
    let transform = Transform::from_translate(out_w as f32 / 2.0, out_h as f32 / 2.0)
        .pre_rotate(angle)
        .pre_translate(-w / 2.0, -h / 2.0);
    // `DrawBitmap` without a paint samples the nearest pixel.
    target.draw_pixmap(
        0,
        0,
        source.as_ref(),
        &PixmapPaint {
            quality: FilterQuality::Nearest,
            ..PixmapPaint::default()
        },
        transform,
        None,
    );
    raster::from_pixmap(&target)
}

pub fn flip_horizontal(image: &RgbaImage) -> RgbaImage {
    image::imageops::flip_horizontal(image)
}

pub fn flip_vertical(image: &RgbaImage) -> RgbaImage {
    image::imageops::flip_vertical(image)
}

/// `ImageHelpers.ResizeCanvas`: padding on each side filled with a colour.
pub fn resize_canvas(
    image: &RgbaImage,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    background: crate::color::Color,
) -> Option<RgbaImage> {
    let width = image.width() as i32 + left + right;
    let height = image.height() as i32 + top + bottom;
    if width <= 0 || height <= 0 {
        return None;
    }
    let fill = Rgba([background.r, background.g, background.b, background.a]);
    let mut out = RgbaImage::from_pixel(width as u32, height as u32, fill);
    for y in 0..image.height() as i32 {
        let ty = y + top;
        if ty < 0 || ty >= height {
            continue;
        }
        for x in 0..image.width() as i32 {
            let tx = x + left;
            if tx < 0 || tx >= width {
                continue;
            }
            let src = *image.get_pixel(x as u32, y as u32);
            // `DrawBitmap` blends the image over the background (SrcOver).
            let dst = out.get_pixel_mut(tx as u32, ty as u32);
            *dst = blend_over(src, *dst);
        }
    }
    Some(out)
}

fn blend_over(src: Rgba<u8>, dst: Rgba<u8>) -> Rgba<u8> {
    let sa = src.0[3] as f32 / 255.0;
    if sa >= 1.0 {
        return src;
    }
    let da = dst.0[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return Rgba([0, 0, 0, 0]);
    }
    let mut out = [0u8; 4];
    for ((o, s), d) in out.iter_mut().zip(src.0).zip(dst.0).take(3) {
        let v = (s as f32 * sa + d as f32 * da * (1.0 - sa)) / out_a;
        *o = v.round().clamp(0.0, 255.0) as u8;
    }
    out[3] = (out_a * 255.0).round() as u8;
    Rgba(out)
}

/// `ImageHelpers.ColorsMatch`.
pub fn colors_match(a: Rgba<u8>, b: Rgba<u8>, tolerance: i32) -> bool {
    (0..4).all(|i| (a.0[i] as i32 - b.0[i] as i32).abs() <= tolerance)
}

/// The content bounds `AutoCrop` finds: pixels that do not match `color`.
/// `None` when every pixel matches.
pub fn content_bounds(
    image: &RgbaImage,
    color: Rgba<u8>,
    tolerance: i32,
) -> Option<(u32, u32, u32, u32)> {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0, 0);
    let mut found = false;
    for (x, y, p) in image.enumerate_pixels() {
        if !colors_match(*p, color, tolerance) {
            found = true;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    found.then_some((min_x, min_y, max_x, max_y))
}

/// `AutoCropImageEffect(color, tolerance)`.
pub fn auto_crop(image: &RgbaImage, color: Rgba<u8>, tolerance: i32) -> RgbaImage {
    match content_bounds(image, color, tolerance) {
        Some((min_x, min_y, max_x, max_y)) => {
            image::imageops::crop_imm(image, min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
                .to_image()
        }
        None => RgbaImage::new(1, 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_target_keeps_the_aspect_ratio_on_the_largest_scale() {
        assert_eq!(resolve_resize_target((200, 100), (100, 0), true), (100, 50));
        assert_eq!(
            resolve_resize_target((200, 100), (100, 100), true),
            (200, 100)
        );
        assert_eq!(resolve_resize_target((200, 100), (0, 30), false), (200, 30));
    }

    #[test]
    fn custom_rotation_grows_the_canvas() {
        let img = RgbaImage::from_pixel(100, 50, Rgba([255, 0, 0, 255]));
        let out = rotate_custom(&img, 45.0, true);
        assert_eq!((out.width(), out.height()), (107, 107));
        let clipped = rotate_custom(&img, 45.0, false);
        assert_eq!((clipped.width(), clipped.height()), (100, 50));
        assert_eq!(rotate_custom(&img, 90.0, true).dimensions(), (50, 100));
    }

    #[test]
    fn auto_crop_finds_content() {
        let mut img = RgbaImage::from_pixel(10, 10, Rgba([255, 255, 255, 255]));
        img.put_pixel(3, 4, Rgba([0, 0, 0, 255]));
        img.put_pixel(6, 7, Rgba([0, 0, 0, 255]));
        let out = auto_crop(&img, Rgba([255, 255, 255, 255]), 10);
        assert_eq!(out.dimensions(), (4, 4));
    }
}
