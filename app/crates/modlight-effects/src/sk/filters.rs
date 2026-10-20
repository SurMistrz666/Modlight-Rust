//! Skia's colour filters and image filters on whole bitmaps, as the raster pipeline runs
//! them (highp floats, `to_unorm` rounding half to even as NEON and SSE do).

use super::bitmap::{Bitmap, premul, unpremul};
use super::blend::Blend;
use super::color::C;

/// A pixel in the raster pipeline: premultiplied floats in 0..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct F4 {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

const INV255: f32 = 1.0 / 255.0;

impl F4 {
    /// `load_8888` of a stored (premultiplied) pixel given as its straight colour.
    #[inline]
    pub fn load(c: C) -> F4 {
        let p = premul(c);
        F4 {
            r: p.r as f32 * INV255,
            g: p.g as f32 * INV255,
            b: p.b as f32 * INV255,
            a: p.a as f32 * INV255,
        }
    }

    #[inline]
    pub fn from_straight(c: C) -> F4 {
        F4 {
            r: c.r as f32 * INV255,
            g: c.g as f32 * INV255,
            b: c.b as f32 * INV255,
            a: c.a as f32 * INV255,
        }
    }

    #[inline]
    pub fn unpremul(self) -> F4 {
        let inv = 1.0 / self.a;
        let s = if inv < f32::INFINITY { inv } else { 0.0 };
        F4 {
            r: self.r * s,
            g: self.g * s,
            b: self.b * s,
            a: self.a,
        }
    }

    #[inline]
    pub fn premul(self) -> F4 {
        F4 {
            r: self.r * self.a,
            g: self.g * self.a,
            b: self.b * self.a,
            a: self.a,
        }
    }

    #[inline]
    pub fn clamp01(self) -> F4 {
        F4 {
            r: self.r.clamp(0.0, 1.0),
            g: self.g.clamp(0.0, 1.0),
            b: self.b.clamp(0.0, 1.0),
            a: self.a.clamp(0.0, 1.0),
        }
    }

    /// `store_8888`, read back as a straight colour.
    #[inline]
    pub fn store(self) -> C {
        unpremul(C::new(
            unorm(self.r),
            unorm(self.g),
            unorm(self.b),
            unorm(self.a),
        ))
    }
}

/// `to_unorm(v, 255)`.
#[inline]
pub fn unorm(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round_ties_even() as u8
}

/// `SKColorFilter.CreateColorMatrix` drawn onto a transparent bitmap
/// (`AdjustmentImageEffectBase.ApplyColorMatrix`).
pub fn color_matrix(source: &Bitmap, m: &[f32; 20]) -> Bitmap {
    map_pixels(source, |c| color_matrix_pixel(F4::load(c), m).store())
}

#[inline]
pub fn color_matrix_pixel(p: F4, m: &[f32; 20]) -> F4 {
    let u = p.unpremul();
    let row = |k: usize| {
        u.r.mul_add(
            m[k],
            u.g.mul_add(
                m[k + 1],
                u.b.mul_add(m[k + 2], u.a.mul_add(m[k + 3], m[k + 4])),
            ),
        )
    };
    F4 {
        r: row(0),
        g: row(5),
        b: row(10),
        a: row(15),
    }
    .clamp01()
    .premul()
}

/// `SKColorFilter.CreateTable(null, r, g, b)`: per-channel lookups on unpremultiplied
/// bytes.
pub fn color_table(source: &Bitmap, tr: &[u8; 256], tg: &[u8; 256], tb: &[u8; 256]) -> Bitmap {
    map_pixels(source, |c| {
        let u = F4::load(c).unpremul();
        F4 {
            r: tr[unorm(u.r) as usize] as f32 * INV255,
            g: tg[unorm(u.g) as usize] as f32 * INV255,
            b: tb[unorm(u.b) as usize] as f32 * INV255,
            a: u.a,
        }
        .premul()
        .store()
    })
}

/// `SKColorFilter.CreateBlendMode(color, mode)` on one premultiplied pixel: the pixel is
/// the destination, the colour the source.
#[inline]
pub fn blend_filter_pixel(p: F4, color: C, mode: Blend) -> F4 {
    let s = F4::from_straight(color).premul();
    mode.apply(s, p)
}

fn map_pixels(source: &Bitmap, f: impl Fn(C) -> C) -> Bitmap {
    Bitmap {
        w: source.w,
        h: source.h,
        px: source.px.iter().map(|c| f(*c)).collect(),
    }
}

/// How an image filter reads outside the source (`SKShaderTileMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tile {
    Clamp,
    Repeat,
    Mirror,
    Decal,
}

/// `SKImageFilter.CreateMatrixConvolution` drawn onto a bitmap of the source's size
/// (`SkMatrixConvolutionImageFilter`'s raster path).
#[allow(clippy::too_many_arguments)]
pub fn matrix_convolution(
    source: &Bitmap,
    kw: i32,
    kh: i32,
    kernel: &[f32],
    gain: f32,
    bias: f32,
    offset: (i32, i32),
    tile: Tile,
    convolve_alpha: bool,
) -> Bitmap {
    let (w, h) = (source.w, source.h);
    // The filter reads premultiplied pixels, unpremultiplied first unless alpha is
    // convolved too.
    let src: Vec<C> = if convolve_alpha {
        source.px.iter().map(|c| premul(*c)).collect()
    } else {
        source.px.clone()
    };
    let fetch = |x: i32, y: i32| -> C {
        let (x, y) = match tile {
            Tile::Clamp => (x.clamp(0, w - 1), y.clamp(0, h - 1)),
            Tile::Repeat => (x.rem_euclid(w), y.rem_euclid(h)),
            Tile::Mirror => (mirror(x, w), mirror(y, h)),
            Tile::Decal => {
                if x < 0 || y < 0 || x >= w || y >= h {
                    return C::ZERO;
                }
                (x, y)
            }
        };
        src[(y * w + x) as usize]
    };
    let mut out = Vec::with_capacity(source.px.len());
    for y in 0..h {
        for x in 0..w {
            let (mut sa, mut sr, mut sg, mut sb) = (0f32, 0f32, 0f32, 0f32);
            for cy in 0..kh {
                for cx in 0..kw {
                    let s = fetch(x + cx - offset.0, y + cy - offset.1);
                    let k = kernel[(cy * kw + cx) as usize];
                    if convolve_alpha {
                        sa += s.a as f32 * k;
                    }
                    sr += s.r as f32 * k;
                    sg += s.g as f32 * k;
                    sb += s.b as f32 * k;
                }
            }
            let a = if convolve_alpha {
                ((sa * gain + bias).floor() as i32).clamp(0, 255)
            } else {
                255
            };
            let r = ((sr * gain + bias).floor() as i32).clamp(0, a);
            let g = ((sg * gain + bias).floor() as i32).clamp(0, a);
            let b = ((sb * gain + bias).floor() as i32).clamp(0, a);
            let c = if convolve_alpha {
                unpremul(C::new(r as u8, g as u8, b as u8, a as u8))
            } else {
                let a = fetch(x, y).a;
                // SkPreMultiplyARGB then read back.
                unpremul(premul(C::new(r as u8, g as u8, b as u8, a)))
            };
            out.push(c);
        }
    }
    Bitmap { w, h, px: out }
}

fn mirror(v: i32, n: i32) -> i32 {
    if n <= 1 {
        return 0;
    }
    let period = 2 * n;
    let m = v.rem_euclid(period);
    if m < n { m } else { period - 1 - m }
}

/// `SKImageFilter.CreateDilate(rx, ry)` (or erode): the per-channel maximum (minimum) of
/// premultiplied pixels over the box, transparent outside.
pub fn morphology(source: &Bitmap, rx: i32, ry: i32, dilate: bool) -> Bitmap {
    let (w, h) = (source.w, source.h);
    let src: Vec<C> = source.px.iter().map(|c| premul(*c)).collect();
    let pick = |a: u8, b: u8| if dilate { a.max(b) } else { a.min(b) };
    let init = if dilate { 0u8 } else { 255u8 };
    let pass = |input: &[C], horizontal: bool, r: i32| -> Vec<C> {
        if r <= 0 {
            return input.to_vec();
        }
        let mut out = vec![C::ZERO; input.len()];
        for y in 0..h {
            for x in 0..w {
                let mut acc = [init; 4];
                for k in -r..=r {
                    let (sx, sy) = if horizontal { (x + k, y) } else { (x, y + k) };
                    if sx < 0 || sy < 0 || sx >= w || sy >= h {
                        // Skia clamps the box to the bounds.
                        continue;
                    }
                    let p = input[(sy * w + sx) as usize];
                    acc = [
                        pick(acc[0], p.r),
                        pick(acc[1], p.g),
                        pick(acc[2], p.b),
                        pick(acc[3], p.a),
                    ];
                }
                out[(y * w + x) as usize] = C::new(acc[0], acc[1], acc[2], acc[3]);
            }
        }
        out
    };
    let a = pass(&src, true, rx);
    let b = pass(&a, false, ry);
    Bitmap {
        w,
        h,
        px: b.into_iter().map(unpremul).collect(),
    }
}
