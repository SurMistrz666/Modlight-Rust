//! Pixel work shared by the core, the effect annotations and the renderer: conversions
//! between `image::RgbaImage` (straight alpha, the document's form) and tiny-skia's
//! premultiplied `Pixmap`, Skia-like resampling and Skia's Gaussian blur.

use image::{Rgba, RgbaImage};
use tiny_skia::{IntSize, Pixmap, PremultipliedColorU8};

use crate::geom::RectI;

/// The document image as a premultiplied pixmap.
pub fn to_pixmap(image: &RgbaImage) -> Pixmap {
    let mut data = Vec::with_capacity(image.as_raw().len());
    for p in image.pixels() {
        let [r, g, b, a] = p.0;
        if a == 255 {
            data.extend_from_slice(&[r, g, b, a]);
        } else {
            data.extend_from_slice(&[mul(r, a), mul(g, a), mul(b, a), a]);
        }
    }
    let size = IntSize::from_wh(image.width().max(1), image.height().max(1)).unwrap();
    Pixmap::from_vec(data, size).unwrap_or_else(|| Pixmap::new(1, 1).unwrap())
}

/// A premultiplied pixmap back to straight alpha.
pub fn from_pixmap(pixmap: &Pixmap) -> RgbaImage {
    let mut out = Vec::with_capacity(pixmap.data().len());
    for p in pixmap.pixels() {
        let c = p.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    RgbaImage::from_raw(pixmap.width(), pixmap.height(), out).unwrap()
}

fn mul(c: u8, a: u8) -> u8 {
    let v = c as u32 * a as u32 + 128;
    ((v + (v >> 8)) >> 8) as u8
}

/// `SKBitmap.ExtractSubset` (the part of `rect` inside the image).
pub fn crop(image: &RgbaImage, rect: RectI) -> RgbaImage {
    let bounds = RectI::new(0, 0, image.width() as i32, image.height() as i32);
    let r = rect.intersect(&bounds);
    if r.width() <= 0 || r.height() <= 0 {
        return RgbaImage::new(1, 1);
    }
    image::imageops::crop_imm(
        image,
        r.left as u32,
        r.top as u32,
        r.width() as u32,
        r.height() as u32,
    )
    .to_image()
}

/// Sampling as Skia's `SKSamplingOptions` does it when a bitmap is drawn scaled: each
/// destination pixel samples the source at its mapped centre (no prefiltering).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Filter {
    Nearest,
    Linear,
    /// `SKCubicResampler.Mitchell` (B = C = 1/3).
    Mitchell,
    /// `SKCubicResampler.CatmullRom` (B = 0, C = 1/2).
    CatmullRom,
}

fn cubic_weight(x: f32, b: f32, c: f32) -> f32 {
    let x = x.abs();
    if x < 1.0 {
        ((12.0 - 9.0 * b - 6.0 * c) * x * x * x
            + (-18.0 + 12.0 * b + 6.0 * c) * x * x
            + (6.0 - 2.0 * b))
            / 6.0
    } else if x < 2.0 {
        ((-b - 6.0 * c) * x * x * x
            + (6.0 * b + 30.0 * c) * x * x
            + (-12.0 * b - 48.0 * c) * x
            + (8.0 * b + 24.0 * c))
            / 6.0
    } else {
        0.0
    }
}

/// Premultiplied float pixels, for resampling and blurring.
struct Planes {
    width: usize,
    height: usize,
    data: Vec<[f32; 4]>,
}

impl Planes {
    fn from_image(image: &RgbaImage) -> Planes {
        let data = image
            .pixels()
            .map(|p| {
                let a = p.0[3] as f32 / 255.0;
                [
                    p.0[0] as f32 * a,
                    p.0[1] as f32 * a,
                    p.0[2] as f32 * a,
                    p.0[3] as f32,
                ]
            })
            .collect();
        Planes {
            width: image.width() as usize,
            height: image.height() as usize,
            data,
        }
    }

    fn to_image(&self) -> RgbaImage {
        let mut out = RgbaImage::new(self.width as u32, self.height as u32);
        for (i, p) in out.pixels_mut().enumerate() {
            let [r, g, b, a] = self.data[i];
            let a = a.clamp(0.0, 255.0);
            *p = if a <= 0.0 {
                Rgba([0, 0, 0, 0])
            } else {
                let k = 255.0 / a;
                Rgba([
                    (r * k).round().clamp(0.0, 255.0) as u8,
                    (g * k).round().clamp(0.0, 255.0) as u8,
                    (b * k).round().clamp(0.0, 255.0) as u8,
                    a.round() as u8,
                ])
            };
        }
        out
    }

    fn get(&self, x: isize, y: isize) -> [f32; 4] {
        let x = x.clamp(0, self.width as isize - 1) as usize;
        let y = y.clamp(0, self.height as isize - 1) as usize;
        self.data[y * self.width + x]
    }
}

/// `SKBitmap.Resize(info, sampling)`.
pub fn resize(image: &RgbaImage, width: u32, height: u32, filter: Filter) -> RgbaImage {
    let width = width.max(1);
    let height = height.max(1);
    if image.width() == 0 || image.height() == 0 {
        return RgbaImage::new(width, height);
    }
    let src = Planes::from_image(image);
    let sx = image.width() as f32 / width as f32;
    let sy = image.height() as f32 / height as f32;
    let mut out = Planes {
        width: width as usize,
        height: height as usize,
        data: vec![[0.0; 4]; (width * height) as usize],
    };
    for y in 0..height as usize {
        let v = (y as f32 + 0.5) * sy;
        for x in 0..width as usize {
            let u = (x as f32 + 0.5) * sx;
            out.data[y * width as usize + x] = sample(&src, u, v, filter);
        }
    }
    out.to_image()
}

fn sample(src: &Planes, u: f32, v: f32, filter: Filter) -> [f32; 4] {
    match filter {
        Filter::Nearest => src.get(u.floor() as isize, v.floor() as isize),
        Filter::Linear => {
            let fx = u - 0.5;
            let fy = v - 0.5;
            let x0 = fx.floor();
            let y0 = fy.floor();
            let tx = fx - x0;
            let ty = fy - y0;
            let (x0, y0) = (x0 as isize, y0 as isize);
            let p00 = src.get(x0, y0);
            let p10 = src.get(x0 + 1, y0);
            let p01 = src.get(x0, y0 + 1);
            let p11 = src.get(x0 + 1, y0 + 1);
            let mut out = [0.0; 4];
            for c in 0..4 {
                let top = p00[c] + (p10[c] - p00[c]) * tx;
                let bottom = p01[c] + (p11[c] - p01[c]) * tx;
                out[c] = top + (bottom - top) * ty;
            }
            out
        }
        Filter::Mitchell | Filter::CatmullRom => {
            let (b, c) = if filter == Filter::Mitchell {
                (1.0 / 3.0, 1.0 / 3.0)
            } else {
                (0.0, 0.5)
            };
            let fx = u - 0.5;
            let fy = v - 0.5;
            let x0 = fx.floor();
            let y0 = fy.floor();
            let tx = fx - x0;
            let ty = fy - y0;
            let mut out = [0.0f32; 4];
            for j in -1..=2isize {
                let wy = cubic_weight(j as f32 - ty, b, c);
                if wy == 0.0 {
                    continue;
                }
                for i in -1..=2isize {
                    let wx = cubic_weight(i as f32 - tx, b, c);
                    if wx == 0.0 {
                        continue;
                    }
                    let p = src.get(x0 as isize + i, y0 as isize + j);
                    for k in 0..4 {
                        out[k] += p[k] * wx * wy;
                    }
                }
            }
            // Cubic overshoot is clamped like Skia clamps premultiplied channels.
            out[3] = out[3].clamp(0.0, 255.0);
            for k in 0..3 {
                out[k] = out[k].clamp(0.0, out[3]);
            }
            out
        }
    }
}

/// `SKImageFilter.CreateBlur(sigma, sigma)` on a premultiplied image with transparent
/// edges (Skia's decal tiling): three box passes per axis, the SVG/Skia approximation of
/// a Gaussian (`window = floor(sigma * 3 * sqrt(2 * pi) / 4 + 0.5)`).
pub fn gaussian_blur(image: &RgbaImage, sigma: f32) -> RgbaImage {
    if sigma <= 0.0 || image.width() == 0 || image.height() == 0 {
        return image.clone();
    }
    let mut planes = Planes::from_image(image);
    blur_planes(&mut planes, sigma);
    planes.to_image()
}

/// The same blur on a premultiplied pixmap, in place (used for drop shadows).
pub fn gaussian_blur_pixmap(pixmap: &mut Pixmap, sigma: f32) {
    if sigma <= 0.0 {
        return;
    }
    let mut planes = Planes {
        width: pixmap.width() as usize,
        height: pixmap.height() as usize,
        data: pixmap
            .pixels()
            .iter()
            .map(|p| {
                [
                    p.red() as f32,
                    p.green() as f32,
                    p.blue() as f32,
                    p.alpha() as f32,
                ]
            })
            .collect(),
    };
    blur_planes(&mut planes, sigma);
    for (dst, src) in pixmap.pixels_mut().iter_mut().zip(&planes.data) {
        let a = src[3].round().clamp(0.0, 255.0) as u8;
        let c = |v: f32| (v.round().clamp(0.0, a as f32)) as u8;
        *dst = PremultipliedColorU8::from_rgba(c(src[0]), c(src[1]), c(src[2]), a)
            .unwrap_or(PremultipliedColorU8::TRANSPARENT);
    }
}

fn box_passes(sigma: f32) -> Vec<(usize, isize)> {
    let factor = 3.0 * (2.0 * std::f64::consts::PI).sqrt() / 4.0;
    let window = ((sigma as f64 * factor + 0.5).floor() as usize).max(1);
    if window % 2 == 1 {
        let half = (window / 2) as isize;
        vec![(window, half), (window, half), (window, half)]
    } else {
        // Two boxes of `window` centred on the left and right pixel boundaries, then one
        // of `window + 1` centred on the pixel.
        let half = (window / 2) as isize;
        vec![(window, half), (window, half - 1), (window + 1, half)]
    }
}

fn blur_planes(planes: &mut Planes, sigma: f32) {
    let passes = box_passes(sigma);
    let (w, h) = (planes.width, planes.height);
    let mut line = Vec::new();
    let mut out = Vec::new();
    for &(size, left) in &passes {
        for y in 0..h {
            line.clear();
            line.extend_from_slice(&planes.data[y * w..(y + 1) * w]);
            box_line(&line, &mut out, size, left);
            planes.data[y * w..(y + 1) * w].copy_from_slice(&out);
        }
    }
    for &(size, left) in &passes {
        for x in 0..w {
            line.clear();
            line.extend((0..h).map(|y| planes.data[y * w + x]));
            box_line(&line, &mut out, size, left);
            for (y, v) in out.iter().enumerate() {
                planes.data[y * w + x] = *v;
            }
        }
    }
}

/// One box pass over a line with transparent outside: `out[i]` is the mean of
/// `input[i - left .. i - left + size]`.
fn box_line(input: &[[f32; 4]], out: &mut Vec<[f32; 4]>, size: usize, left: isize) {
    let n = input.len() as isize;
    out.clear();
    out.resize(input.len(), [0.0; 4]);
    let mut sum = [0.0f32; 4];
    let first = -left;
    for k in first..first + size as isize {
        if (0..n).contains(&k) {
            for c in 0..4 {
                sum[c] += input[k as usize][c];
            }
        }
    }
    let inv = 1.0 / size as f32;
    for i in 0..n {
        for c in 0..4 {
            out[i as usize][c] = sum[c] * inv;
        }
        let leaving = i - left;
        let entering = i - left + size as isize;
        if (0..n).contains(&leaving) {
            for c in 0..4 {
                sum[c] -= input[leaving as usize][c];
            }
        }
        if (0..n).contains(&entering) {
            for c in 0..4 {
                sum[c] += input[entering as usize][c];
            }
        }
    }
}

/// Pads an image by `pad` pixels on every side with its edge pixels (a clamp shader).
pub fn extend_clamped(
    image: &RgbaImage,
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
) -> RgbaImage {
    let (w, h) = (image.width(), image.height());
    let mut out = RgbaImage::new(w + left + right, h + top + bottom);
    for y in 0..out.height() {
        let sy = (y as i64 - top as i64).clamp(0, h as i64 - 1) as u32;
        for x in 0..out.width() {
            let sx = (x as i64 - left as i64).clamp(0, w as i64 - 1) as u32;
            out.put_pixel(x, y, *image.get_pixel(sx, sy));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premultiply_round_trip_keeps_opaque_pixels() {
        let img = RgbaImage::from_pixel(3, 2, Rgba([10, 200, 30, 255]));
        assert_eq!(from_pixmap(&to_pixmap(&img)), img);
    }

    #[test]
    fn blur_spreads_and_keeps_energy_inside_large_image() {
        let mut img = RgbaImage::from_pixel(41, 41, Rgba([0, 0, 0, 0]));
        img.put_pixel(20, 20, Rgba([255, 255, 255, 255]));
        let out = gaussian_blur(&img, 3.0);
        let total: u32 = out.pixels().map(|p| p.0[3] as u32).sum();
        assert!(total > 200 && total < 300, "{total}");
        assert!(out.get_pixel(20, 20).0[3] < 50);
        assert!(out.get_pixel(23, 20).0[3] > 0);
    }

    #[test]
    fn nearest_upscale_repeats_pixels() {
        let mut img = RgbaImage::new(2, 1);
        img.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        img.put_pixel(1, 0, Rgba([0, 0, 255, 255]));
        let out = resize(&img, 4, 2, Filter::Nearest);
        assert_eq!(out.get_pixel(1, 1).0, [255, 0, 0, 255]);
        assert_eq!(out.get_pixel(2, 0).0, [0, 0, 255, 255]);
    }
}
