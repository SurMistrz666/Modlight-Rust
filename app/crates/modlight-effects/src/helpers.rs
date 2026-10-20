//! `Core/ImageEffects/Helpers`: `ProceduralEffectHelper`, `AnalogEffectHelper` and
//! `DistortionEffectHelper`.

use crate::sk::{Bitmap, C, Canvas, Paint, Sampling, Tile, round};

// ---------- ProceduralEffectHelper ----------

pub fn clamp_int(v: i32, min: i32, max: i32) -> i32 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

#[inline]
pub fn clamp01(v: f32) -> f32 {
    if v <= 0.0 {
        0.0
    } else if v >= 1.0 {
        1.0
    } else {
        v
    }
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn smooth_step(edge0: f32, edge1: f32, value: f32) -> f32 {
    if (edge1 - edge0).abs() < 0.000001 {
        return if value >= edge1 { 1.0 } else { 0.0 };
    }
    let t = clamp01((value - edge0) / (edge1 - edge0));
    t * t * (3.0 - 2.0 * t)
}

/// `ProceduralEffectHelper.Hash01`.
pub fn hash01(x: i32, y: i32, seed: i32) -> f32 {
    let mut h: u32 = 2166136261;
    h = (h ^ x as u32).wrapping_mul(16777619);
    h = (h ^ y as u32).wrapping_mul(16777619);
    h = (h ^ seed as u32).wrapping_mul(16777619);
    h ^= h >> 13;
    h = h.wrapping_mul(1274126177);
    h ^= h >> 16;
    (h & 0x00FF_FFFF) as f32 / 16777215.0
}

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

pub fn value_noise(x: f32, y: f32, seed: i32) -> f32 {
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let x1 = x0.wrapping_add(1);
    let y1 = y0.wrapping_add(1);
    let tx = fade(x - x0 as f32);
    let ty = fade(y - y0 as f32);
    let v00 = hash01(x0, y0, seed);
    let v10 = hash01(x1, y0, seed);
    let v01 = hash01(x0, y1, seed);
    let v11 = hash01(x1, y1, seed);
    let vx0 = lerp(v00, v10, tx);
    let vx1 = lerp(v01, v11, tx);
    lerp(vx0, vx1, ty)
}

pub fn fractal_noise(x: f32, y: f32, octaves: i32, lacunarity: f32, gain: f32, seed: i32) -> f32 {
    let mut value = 0.0;
    let mut amplitude = 1.0;
    let mut frequency = 1.0;
    let mut normalization = 0.0;
    for i in 0..octaves {
        value += value_noise(x * frequency, y * frequency, seed.wrapping_add(i * 1619)) * amplitude;
        normalization += amplitude;
        frequency *= lacunarity;
        amplitude *= gain;
    }
    if normalization > 0.0 {
        value / normalization
    } else {
        0.0
    }
}

/// `ProceduralEffectHelper.BilinearSample` (clamped to the edges).
pub fn bilinear_sample(pixels: &[C], width: i32, height: i32, x: f32, y: f32) -> C {
    let right = width - 1;
    let bottom = height - 1;
    let x = x.clamp(0.0, right as f32);
    let y = y.clamp(0.0, bottom as f32);
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let x1 = if x0 < right { x0 + 1 } else { right };
    let y1 = if y0 < bottom { y0 + 1 } else { bottom };
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let c00 = pixels[(y0 * width + x0) as usize];
    let c10 = pixels[(y0 * width + x1) as usize];
    let c01 = pixels[(y1 * width + x0) as usize];
    let c11 = pixels[(y1 * width + x1) as usize];
    let r0 = lerp(c00.r as f32, c10.r as f32, tx);
    let g0 = lerp(c00.g as f32, c10.g as f32, tx);
    let b0 = lerp(c00.b as f32, c10.b as f32, tx);
    let a0 = lerp(c00.a as f32, c10.a as f32, tx);
    let r1 = lerp(c01.r as f32, c11.r as f32, tx);
    let g1 = lerp(c01.g as f32, c11.g as f32, tx);
    let b1 = lerp(c01.b as f32, c11.b as f32, tx);
    let a1 = lerp(c01.a as f32, c11.a as f32, tx);
    C::new(
        clamp_to_byte(lerp(r0, r1, ty)),
        clamp_to_byte(lerp(g0, g1, ty)),
        clamp_to_byte(lerp(b0, b1, ty)),
        clamp_to_byte(lerp(a0, a1, ty)),
    )
}

pub use crate::sk::clamp_to_byte;

// ---------- AnalogEffectHelper ----------

/// `AnalogEffectHelper.CreateBlurredClamp`: blur with the edges extended (clamp), not
/// faded.
pub fn blurred_clamp(source: &Bitmap, radius: f32) -> Bitmap {
    if radius <= 0.01 {
        return source.clone();
    }
    let padding = 2.max((radius * 2.0).ceil() as i32);
    let ew = source.w + padding * 2;
    let eh = source.h + padding * 2;
    // Clamp shader translated by the padding.
    let mut expanded = Bitmap::new(ew, eh);
    for y in 0..eh {
        let sy = (y - padding).clamp(0, source.h - 1);
        for x in 0..ew {
            let sx = (x - padding).clamp(0, source.w - 1);
            expanded.px[(y * ew + x) as usize] = source.get(sx, sy);
        }
    }
    let sigma = 0.001f32.max(radius / 3.0);
    let blurred = blur_bitmap(&expanded, sigma, sigma);
    let mut out = Bitmap::new(source.w, source.h);
    for y in 0..source.h {
        for x in 0..source.w {
            out.px[(y * source.w + x) as usize] = blurred.get(x + padding, y + padding);
        }
    }
    out
}

/// `canvas.DrawBitmap(source, 0, 0, new SKPaint { ImageFilter = CreateBlur(sx, sy) })` on
/// a transparent canvas of the same size.
pub fn blur_bitmap(source: &Bitmap, sx: f32, sy: f32) -> Bitmap {
    let mut canvas = Canvas::new(source.w, source.h);
    canvas.draw_bitmap(source, 0.0, 0.0, &Paint::default().with_image_blur(sx, sy));
    canvas.into_bitmap()
}

pub fn luminance(c: C) -> f32 {
    0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32
}

pub fn luminance01(c: C) -> f32 {
    luminance(c) / 255.0
}

pub fn lerp_color(from: C, to: C, t: f32) -> C {
    let a = clamp01(t);
    C::new(
        clamp_to_byte(lerp(from.r as f32, to.r as f32, a)),
        clamp_to_byte(lerp(from.g as f32, to.g as f32, a)),
        clamp_to_byte(lerp(from.b as f32, to.b as f32, a)),
        clamp_to_byte(lerp(from.a as f32, to.a as f32, a)),
    )
}

pub fn apply_contrast(value: f32, contrast: f32) -> f32 {
    clamp01((value - 0.5) * contrast + 0.5)
}

pub fn screen(base: f32, blend: f32) -> f32 {
    let b = clamp01(base);
    let s = clamp01(blend);
    1.0 - (1.0 - b) * (1.0 - s)
}

pub fn overlay(base: f32, blend: f32) -> f32 {
    let b = clamp01(base);
    let s = clamp01(blend);
    if b < 0.5 {
        2.0 * b * s
    } else {
        1.0 - 2.0 * (1.0 - b) * (1.0 - s)
    }
}

/// `AnalogEffectHelper.FromHsl`: hue wrapped, saturation and lightness clamped.
pub fn from_hsl(mut h: f32, s: f32, l: f32, alpha: u8) -> C {
    while h < 0.0 {
        h += 360.0;
    }
    while h >= 360.0 {
        h -= 360.0;
    }
    C::from_hsl(h, s.clamp(0.0, 100.0), l.clamp(0.0, 100.0), alpha)
}

// ---------- DistortionEffectHelper ----------

pub fn percentage_to_x(width: i32, percentage: f32) -> f32 {
    (percentage.clamp(0.0, 100.0) / 100.0) * 0.max(width - 1) as f32
}

pub fn percentage_to_y(height: i32, percentage: f32) -> f32 {
    (percentage.clamp(0.0, 100.0) / 100.0) * 0.max(height - 1) as f32
}

pub fn sample_transparent(pixels: &[C], width: i32, height: i32, x: f32, y: f32) -> C {
    if x < 0.0 || x > (width - 1) as f32 || y < 0.0 || y > (height - 1) as f32 {
        return C::TRANSPARENT;
    }
    bilinear_sample(pixels, width, height, x, y)
}

pub fn blend(from: C, to: C, amount: f32) -> C {
    lerp_color(from, to, amount)
}

pub fn multiply_rgb(c: C, factor: f32) -> C {
    let v = factor.max(0.0);
    C::new(
        clamp_to_byte(c.r as f32 * v),
        clamp_to_byte(c.g as f32 * v),
        clamp_to_byte(c.b as f32 * v),
        c.a,
    )
}

pub fn wrap_angle(radians: f32) -> f32 {
    let two_pi = std::f32::consts::PI * 2.0;
    let mut r = radians % two_pi;
    if r < 0.0 {
        r += two_pi;
    }
    r
}

/// `MathF.Round` re-exported for effect code.
pub fn mround(v: f32) -> f32 {
    round(v)
}

/// `SKBitmap.Resize(info, sampling)`.
pub fn resize(source: &Bitmap, w: i32, h: i32, sampling: Sampling) -> Bitmap {
    crate::sk::canvas::resize(source, w, h, sampling)
}

/// Bilinear sample of a whole bitmap with a tile mode (`SKShader` sampling).
pub fn sample(source: &Bitmap, u: f32, v: f32, sampling: Sampling, tile: Tile) -> C {
    crate::sk::canvas::sample_bitmap(source, u, v, sampling, tile)
}

/// The `HsvToRgb(h, s, v)` many effects carry privately: `h` in 0..1, no wrapping.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    if s <= 0.0001 {
        return (v, v, v);
    }
    let scaled = h * 6.0;
    let sector = scaled.floor() as i32;
    let f = scaled - sector as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match sector {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}
