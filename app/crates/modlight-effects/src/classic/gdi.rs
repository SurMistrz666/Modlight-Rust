//! The picture operations of `ShareX.HelpersLib` the classic effects call
//! (`ImageHelpers`, `ColorMatrixManager`, `ConvolutionMatrixManager`, `GradientInfo`).
//!
//! Pixel loops (`UnsafeBitmap`) are ported as written. What upstream leaves to GDI+
//! (`Graphics.DrawImage` with high-quality bicubic, anti-aliased paths, gradient
//! brushes, `DrawString`) is drawn with this crate's Skia port: same geometry, but
//! GDI+'s own filtering and anti-aliasing are not reproduced to the pixel (MACOS.md).

use std::sync::Arc;

use tiny_skia::{PathBuilder, Transform};

use super::model::{ANCHOR_ALL, Gradient, Padding};
use crate::dotnet::DotNetRandom;
use crate::sk::{Bitmap, Blend, C, Canvas, Cap, Paint, Sampling, Shader, Style, Tile, rect};

pub const TOP: u32 = 1;
pub const BOTTOM: u32 = 2;
pub const LEFT: u32 = 4;
pub const RIGHT: u32 = 8;

/// `RandomFast.Next(min, max)`: both ends included.
pub fn rand_incl(rng: &mut DotNetRandom, min: i32, max: i32) -> i32 {
    if min >= max {
        return min;
    }
    rng.next_range(min, max.saturating_add(1))
}

// ---------- colour matrices ----------

/// A GDI+ `ColorMatrix` (`[row][column]`, applied to `[r g b a 1]`).
pub type Matrix = [[f32; 5]; 5];

pub const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 0.0, 1.0],
];

const RW: f32 = 0.212671;
const GW: f32 = 0.715160;
const BW: f32 = 0.072169;

pub fn m_inverse() -> Matrix {
    [
        [-1.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, -1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, -1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0],
        [1.0, 1.0, 1.0, 0.0, 1.0],
    ]
}

pub fn m_alpha(value: f32, add: f32) -> Matrix {
    let mut m = IDENTITY;
    m[3][3] = value;
    m[4][3] = add;
    m
}

pub fn m_brightness(value: f32) -> Matrix {
    let mut m = IDENTITY;
    m[4][0] = value;
    m[4][1] = value;
    m[4][2] = value;
    m
}

pub fn m_contrast(value: f32) -> Matrix {
    let mut m = IDENTITY;
    m[0][0] = value;
    m[1][1] = value;
    m[2][2] = value;
    m
}

pub fn m_black_white() -> Matrix {
    [
        [1.5, 1.5, 1.5, 0.0, 0.0],
        [1.5, 1.5, 1.5, 0.0, 0.0],
        [1.5, 1.5, 1.5, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0],
        [-1.0, -1.0, -1.0, 0.0, 1.0],
    ]
}

pub fn m_polaroid() -> Matrix {
    [
        [1.438, -0.062, -0.062, 0.0, 0.0],
        [-0.122, 1.378, -0.122, 0.0, 0.0],
        [-0.016, -0.016, 1.483, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0],
        [-0.03, 0.05, -0.02, 0.0, 1.0],
    ]
}

pub fn m_grayscale(v: f32) -> Matrix {
    [
        [RW * v, RW * v, RW * v, 0.0, 0.0],
        [GW * v, GW * v, GW * v, 0.0, 0.0],
        [BW * v, BW * v, BW * v, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 1.0],
    ]
}

pub fn m_sepia(v: f32) -> Matrix {
    [
        [0.393 * v, 0.349 * v, 0.272 * v, 0.0, 0.0],
        [0.769 * v, 0.686 * v, 0.534 * v, 0.0, 0.0],
        [0.189 * v, 0.168 * v, 0.131 * v, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 1.0],
    ]
}

pub fn m_hue(angle: f32) -> Matrix {
    let a = angle * (std::f64::consts::PI / 180.0) as f32;
    let c = (a as f64).cos() as f32;
    let s = (a as f64).sin() as f32;
    [
        [
            (RW + c * (1.0 - RW)) + s * -RW,
            (RW + c * -RW) + s * 0.143,
            (RW + c * -RW) + s * -(1.0 - RW),
            0.0,
            0.0,
        ],
        [
            (GW + c * -GW) + s * -GW,
            (GW + c * (1.0 - GW)) + s * 0.14,
            (GW + c * -GW) + s * GW,
            0.0,
            0.0,
        ],
        [
            (BW + c * -BW) + s * (1.0 - BW),
            (BW + c * -BW) + s * -0.283,
            (BW + c * (1.0 - BW)) + s * BW,
            0.0,
            0.0,
        ],
        [0.0, 0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 1.0],
    ]
}

pub fn m_saturation(v: f32) -> Matrix {
    let i = 1.0 - v;
    [
        [i * RW + v, i * RW, i * RW, 0.0, 0.0],
        [i * GW, i * GW + v, i * GW, 0.0, 0.0],
        [i * BW, i * BW, i * BW + v, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 1.0],
    ]
}

pub fn m_colorize(color: C, v: f32) -> Matrix {
    let (r, g, b) = (
        color.r as f32 / 255.0,
        color.g as f32 / 255.0,
        color.b as f32 / 255.0,
    );
    let inv = 1.0 - v;
    [
        [inv + v * r * RW, v * g * RW, v * b * RW, 0.0, 0.0],
        [v * r * GW, inv + v * g * GW, v * b * GW, 0.0, 0.0],
        [v * r * BW, v * g * BW, inv + v * b * BW, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 1.0],
    ]
}

/// `ColorMatrixManager.Mask`: every pixel the colour, its alpha scaled.
pub fn m_mask(opacity: f32, color: C) -> Matrix {
    [
        [0.0; 5],
        [0.0; 5],
        [0.0; 5],
        [0.0, 0.0, 0.0, color.a as f32 / 255.0 * opacity, 0.0],
        [
            color.r as f32 / 255.0,
            color.g as f32 / 255.0,
            color.b as f32 / 255.0,
            0.0,
            1.0,
        ],
    ]
}

fn unit(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// `ColorMatrix.Apply`: GDI+ `ImageAttributes.SetColorMatrix` on each pixel.
pub fn color_matrix(src: &Bitmap, m: &Matrix) -> Bitmap {
    let px = src
        .px
        .iter()
        .map(|c| {
            let v = [
                c.r as f32 / 255.0,
                c.g as f32 / 255.0,
                c.b as f32 / 255.0,
                c.a as f32 / 255.0,
                1.0,
            ];
            let col = |j: usize| (0..5).map(|i| v[i] * m[i][j]).sum::<f32>();
            C::new(unit(col(0)), unit(col(1)), unit(col(2)), unit(col(3)))
        })
        .collect();
    Bitmap::from_pixels(src.w, src.h, px)
}

/// `ChangeGamma`: `ImageAttributes.SetGamma` (each colour to the power of the value).
pub fn gamma(src: &Bitmap, value: f32) -> Bitmap {
    let g = value.clamp(0.1, 5.0);
    let table: Vec<u8> = (0..256).map(|i| unit((i as f32 / 255.0).powf(g))).collect();
    let px = src
        .px
        .iter()
        .map(|c| {
            C::new(
                table[c.r as usize],
                table[c.g as usize],
                table[c.b as usize],
                c.a,
            )
        })
        .collect();
    Bitmap::from_pixels(src.w, src.h, px)
}

// ---------- convolution ----------

/// `ConvolutionMatrix`.
pub struct Kernel {
    pub rows: Vec<Vec<f64>>,
    pub offset: u8,
    pub consider_alpha: bool,
}

impl Kernel {
    pub fn new(h: usize, w: usize) -> Kernel {
        Kernel {
            rows: vec![vec![0.0; w]; h],
            offset: 0,
            consider_alpha: false,
        }
    }

    pub fn filled(value: f64) -> Kernel {
        let mut k = Kernel::new(3, 3);
        for r in &mut k.rows {
            r.fill(value);
        }
        k
    }
}

pub fn k_smooth() -> Kernel {
    let factor = 1.0 + 8.0;
    let mut k = Kernel::filled(1.0 / factor);
    k.rows[1][1] = 1.0 / factor;
    k
}

pub fn k_mean_removal() -> Kernel {
    let factor = 9.0 - 8.0;
    let mut k = Kernel::filled(-1.0 / factor);
    k.rows[1][1] = 9.0 / factor;
    k
}

pub fn k_sharpen() -> Kernel {
    let factor = 11.0 - 8.0;
    let mut k = Kernel::filled(0.0);
    k.rows[1][1] = 11.0 / factor;
    for (y, x) in [(1, 0), (0, 1), (2, 1), (1, 2)] {
        k.rows[y][x] = -2.0 / factor;
    }
    k
}

pub fn k_emboss() -> Kernel {
    let mut k = Kernel::filled(-1.0);
    k.rows[1][1] = 4.0;
    for (y, x) in [(1, 0), (0, 1), (2, 1), (1, 2)] {
        k.rows[y][x] = 0.0;
    }
    k.offset = 127;
    k
}

pub fn k_edge_detect() -> Kernel {
    let mut k = Kernel::new(3, 3);
    k.rows[0] = vec![-1.0; 3];
    k.rows[2] = vec![1.0; 3];
    k.offset = 127;
    k
}

/// `ConvolutionMatrixManager.Apply`: edges clamped, values clamped and truncated.
pub fn convolve(src: &Bitmap, k: &Kernel) -> Bitmap {
    let (w, h) = (src.w, src.h);
    let kh = k.rows.len() as i32;
    let kw = k.rows.first().map_or(0, |r| r.len()) as i32;
    let (ox, oy) = ((kw - 1) / 2, (kh - 1) / 2);
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let (mut r, mut g, mut b, mut a) = (0f64, 0f64, 0f64, 0f64);
            for fy in 0..kh {
                let sy = (y + fy - oy).clamp(0, h - 1);
                for fx in 0..kw {
                    let sx = (x + fx - ox).clamp(0, w - 1);
                    let c = src.get(sx, sy);
                    let m = k.rows[fy as usize][fx as usize];
                    r += m * c.r as f64;
                    g += m * c.g as f64;
                    b += m * c.b as f64;
                    if k.consider_alpha {
                        a += m * c.a as f64;
                    }
                }
            }
            let fin = |v: f64| (v + k.offset as f64).clamp(0.0, 255.0) as u8;
            let alpha = if k.consider_alpha {
                fin(a)
            } else {
                src.get(x, y).a
            };
            px.push(C::new(fin(r), fin(g), fin(b), alpha));
        }
    }
    Bitmap::from_pixels(w, h, px)
}

/// `ImageHelpers.GaussianBlur`: two passes of a normalised Gaussian, alpha included.
pub fn gaussian_blur(src: &Bitmap, radius: i32) -> Bitmap {
    let size = (radius * 2 + 1) as usize;
    let sigma = radius as f64 / 3.0;
    let g = |x: f64| {
        let left = 1.0 / ((2.0 * std::f64::consts::PI).sqrt() * sigma);
        left * (-x * x / (2.0 * sigma.powi(2))).exp()
    };
    let mid = (size as f64 - 1.0) / 2.0;
    let mut weights: Vec<f64> = (0..size).map(|x| g(x as f64 - mid) * g(0.0)).collect();
    let sum: f64 = weights.iter().sum();
    for w in &mut weights {
        *w /= sum;
    }
    let mut horizontal = Kernel::new(1, size);
    horizontal.rows[0] = weights.clone();
    horizontal.consider_alpha = true;
    let mut vertical = Kernel::new(size, 1);
    for (i, w) in weights.iter().enumerate() {
        vertical.rows[i][0] = *w;
    }
    vertical.consider_alpha = true;
    convolve(&convolve(src, &horizontal), &vertical)
}

// ---------- box blur, pixelate, colour depth ----------

/// `ImageHelpers.BoxBlur`: two horizontal and two vertical running-sum passes. Empty
/// pixels (all zero) count as hits but add nothing.
pub fn box_blur(bmp: &mut Bitmap, mut range: i32) {
    if range <= 1 {
        return;
    }
    if range % 2 == 0 {
        range += 1;
    }
    for _ in 0..2 {
        box_pass(bmp, range, true);
        box_pass(bmp, range, false);
    }
}

fn box_pass(bmp: &mut Bitmap, range: i32, horizontal: bool) {
    let (w, h) = (bmp.w, bmp.h);
    let half = range / 2;
    let (outer, inner) = if horizontal { (h, w) } else { (w, h) };
    let idx = |o: i32, i: i32| -> usize {
        if horizontal {
            (o * w + i) as usize
        } else {
            (i * w + o) as usize
        }
    };
    let mut line = vec![C::ZERO; inner as usize];
    for o in 0..outer {
        let (mut hits, mut r, mut g, mut b, mut a) = (0i32, 0i32, 0i32, 0i32, 0i32);
        for i in -half..inner {
            let old = i - half - 1;
            if old >= 0 {
                let c = bmp.px[idx(o, old)];
                if c != C::ZERO {
                    r -= c.r as i32;
                    g -= c.g as i32;
                    b -= c.b as i32;
                    a -= c.a as i32;
                }
                hits -= 1;
            }
            let new = i + half;
            if new < inner {
                let c = bmp.px[idx(o, new)];
                if c != C::ZERO {
                    r += c.r as i32;
                    g += c.g as i32;
                    b += c.b as i32;
                    a += c.a as i32;
                }
                hits += 1;
            }
            if i >= 0 && hits > 0 {
                line[i as usize] = C::new(
                    (r / hits) as u8,
                    (g / hits) as u8,
                    (b / hits) as u8,
                    (a / hits) as u8,
                );
            }
        }
        for i in 0..inner {
            bmp.px[idx(o, i)] = line[i as usize];
        }
    }
}

/// `ImageHelpers.Pixelate`: alpha-weighted block averages, then an optional grid.
pub fn pixelate(bmp: &mut Bitmap, size: i32, border: i32, border_color: C) {
    if size <= 1 {
        return;
    }
    let (w, h) = (bmp.w, bmp.h);
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            let xl = (x + size).min(w);
            let yl = (y + size).min(h);
            let count = ((xl - x) * (yl - y)) as f32;
            let (mut r, mut g, mut b, mut a, mut wc) = (0f32, 0f32, 0f32, 0f32, 0f32);
            for y2 in y..yl {
                for x2 in x..xl {
                    let c = bmp.get(x2, y2);
                    let pw = c.a as f32 / 255.0;
                    r += c.r as f32 * pw;
                    g += c.g as f32 * pw;
                    b += c.b as f32 * pw;
                    a += c.a as f32 * pw;
                    wc += pw;
                }
            }
            let avg = C::new(
                (b_or_zero(r, wc)) as u8,
                (b_or_zero(g, wc)) as u8,
                (b_or_zero(b, wc)) as u8,
                (a / count) as u8,
            );
            for y2 in y..yl {
                for x2 in x..xl {
                    bmp.px[(y2 * w + x2) as usize] = avg;
                }
            }
            x += size;
        }
        y += size;
    }
    if border > 0 && border_color.a > 0 {
        // A `size` x `size` texture with an inset border, tiled from the origin.
        let inset = |t: i32| t < border || t >= size - border;
        let mut canvas = Canvas::from_bitmap(bmp);
        let mut tile = Bitmap::new(size, size);
        for ty in 0..size {
            for tx in 0..size {
                if inset(tx) || inset(ty) {
                    tile.px[(ty * size + tx) as usize] = border_color;
                }
            }
        }
        let paint = Paint::default().with_shader(Shader::Image {
            bitmap: Arc::new(tile),
            tile_x: Tile::Repeat,
            tile_y: Tile::Repeat,
            matrix: Transform::identity(),
            sampling: Sampling::Nearest,
        });
        canvas.draw_rect(rect(0.0, 0.0, w as f32, h as f32), &paint);
        *bmp = canvas.into_bitmap();
    }
}

/// `(byte)(sum / weight)` with .NET's float-to-byte of NaN (0) for empty blocks.
fn b_or_zero(sum: f32, weight: f32) -> f32 {
    let v = sum / weight;
    if v.is_finite() { v } else { 0.0 }
}

/// `ImageHelpers.ColorDepth`.
pub fn color_depth(bmp: &mut Bitmap, bits: i32) {
    if !(1..=8).contains(&bits) {
        return;
    }
    let colors = 2f64.powi(bits);
    let interval = 255.0 / (colors - 1.0);
    let remap = |c: u8| -> u8 {
        ((c as f64 / interval).round_ties_even() * interval).round_ties_even() as u8
    };
    for c in bmp.px.iter_mut() {
        *c = C::new(remap(c.r), remap(c.g), remap(c.b), c.a);
    }
}

// ---------- colours ----------

fn perceived_brightness(c: C) -> i32 {
    ((c.r as f64 * c.r as f64 * 0.299)
        + (c.g as f64 * c.g as f64 * 0.587)
        + (c.b as f64 * c.b as f64 * 0.114))
        .sqrt() as i32
}

/// `ImageHelpers.SelectiveColor`: each pixel takes the palette colour nearest in
/// perceived brightness.
pub fn selective_color(bmp: &mut Bitmap, light: C, dark: C, palette: i32) {
    let palette = palette.max(2);
    let mut colors: Vec<(i32, C)> = Vec::new();
    for i in 0..palette {
        let t = i as f32 / (palette - 1) as f32;
        let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as i32;
        let c = C::rgb(
            lerp(light.r, dark.r).clamp(0, 255) as u8,
            lerp(light.g, dark.g).clamp(0, 255) as u8,
            lerp(light.b, dark.b).clamp(0, 255) as u8,
        );
        let pb = perceived_brightness(c);
        if !colors.iter().any(|(k, _)| *k == pb) {
            colors.push((pb, c));
        }
    }
    for c in bmp.px.iter_mut() {
        let pb = perceived_brightness(*c);
        // `Aggregate`: keeps the current one unless the next is strictly closer.
        let mut best = colors[0];
        for next in &colors[1..] {
            if (best.0 - pb).abs() >= (next.0 - pb).abs() {
                best = *next;
            }
        }
        *c = C::new(best.1.r, best.1.g, best.1.b, c.a);
    }
}

/// `ImageHelpers.ReplaceColor`.
pub fn replace_color(bmp: &mut Bitmap, source: C, target: C, auto: bool, threshold: i32) {
    let source = if auto && !bmp.px.is_empty() {
        bmp.px[0]
    } else {
        source
    };
    for c in bmp.px.iter_mut() {
        let hit = if threshold == 0 {
            *c == source
        } else {
            (c.r as i32 - source.r as i32).abs()
                + (c.g as i32 - source.g as i32).abs()
                + (c.b as i32 - source.b as i32).abs()
                <= threshold
        };
        if hit {
            *c = target;
        }
    }
}

// ---------- outlines ----------

fn distance_to_threshold(src: &Bitmap, x: i32, y: i32, radius: i32) -> f32 {
    let minx = (x - radius).max(0);
    let maxx = (x + radius).min(src.w - 1);
    let miny = (y - radius).max(0);
    let maxy = (y + radius).min(src.h - 1);
    let mut dist2 = radius * radius + 1;
    for tx in minx..=maxx {
        for ty in miny..=maxy {
            if src.get(tx, ty).a == 255 {
                let (dx, dy) = (tx - x, ty - y);
                dist2 = dist2.min(dx * dx + dy * dy);
            }
        }
    }
    (dist2 as f64).sqrt() as f32
}

/// `ImageHelpers.MakeOutline`.
pub fn make_outline(src: &Bitmap, min_r: i32, max_r: i32, color: C) -> Bitmap {
    let mut out = Bitmap::new(src.w, src.h);
    for x in 0..src.w {
        for y in 0..src.h {
            let d = distance_to_threshold(src, x, y, max_r);
            if d > min_r as f32 && d < max_r as f32 {
                let mut a = 255u8;
                if d - (min_r as f32) < 1.0 {
                    a = (255.0 * (d - min_r as f32)) as u8;
                } else if (max_r as f32) - d < 1.0 {
                    a = (255.0 * (max_r as f32 - d)) as u8;
                }
                out.px[(y * src.w + x) as usize] = C::new(color.r, color.g, color.b, a);
            }
        }
    }
    out
}

/// `ImageHelpers.Outline`.
pub fn outline(src: Bitmap, size: i32, color: C, padding: i32, only: bool) -> Bitmap {
    let line = make_outline(&src, padding, padding + size + 1, color);
    if only {
        return line;
    }
    let mut canvas = Canvas::from_bitmap(&src);
    canvas.draw_bitmap(&line, 0.0, 0.0, &Paint::default());
    canvas.into_bitmap()
}

// ---------- geometry ----------

/// `ImageHelpers.AddCanvas`: `None` when nothing changes or the size would vanish.
pub fn add_canvas(src: &Bitmap, m: Padding, color: C) -> Option<Bitmap> {
    if m.all() == 0 || src.w + m.horizontal() < 1 || src.h + m.vertical() < 1 {
        return None;
    }
    let (w, h) = (src.w + m.horizontal(), src.h + m.vertical());
    let mut canvas = Canvas::new(w, h);
    canvas.draw_bitmap(src, m.left as f32, m.top as f32, &Paint::default());
    let mut bmp = canvas.into_bitmap();
    if color.a > 0 {
        // `CompositingMode.SourceCopy` fills of the four margins.
        let mut fill = |x0: i32, y0: i32, rw: i32, rh: i32| {
            for y in y0.max(0)..(y0 + rh).min(h) {
                for x in x0.max(0)..(x0 + rw).min(w) {
                    bmp.px[(y * w + x) as usize] = color;
                }
            }
        };
        if m.left > 0 {
            fill(0, 0, m.left, h);
        }
        if m.top > 0 {
            fill(0, 0, w, m.top);
        }
        if m.right > 0 {
            fill(w - m.right, 0, m.right, h);
        }
        if m.bottom > 0 {
            fill(0, h - m.bottom, w, m.bottom);
        }
    }
    Some(bmp)
}

/// `ImageHelpers.CropBitmap`: `None` unless the rectangle is inside the picture.
pub fn crop(src: &Bitmap, x: i32, y: i32, w: i32, h: i32) -> Option<Bitmap> {
    if x < 0 || y < 0 || w <= 0 || h <= 0 || x + w > src.w || y + h > src.h {
        return None;
    }
    let mut out = Bitmap::new(w, h);
    for yy in 0..h {
        for xx in 0..w {
            out.px[(yy * w + xx) as usize] = src.get(x + xx, y + yy);
        }
    }
    Some(out)
}

fn bgra(c: C) -> u32 {
    (c.a as u32) << 24 | (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32
}

/// `ImageHelpers.FindAutoCropRectangle`.
pub fn auto_crop_rect(src: &Bitmap, same_color: bool, sides: u32) -> (i32, i32, i32, i32) {
    let (w, h) = (src.w, src.h);
    let mut crop = (0, 0, w, h);
    if sides == 0 || w == 0 || h == 0 {
        return crop;
    }
    let first = src.get(0, 0);
    let mut mask = if first.a == 0 {
        0xFF00_0000
    } else {
        0xFFFF_FFFF
    };
    let mut check = bgra(first) & mask;
    let differs = |x: i32, y: i32, mask: u32, check: u32| bgra(src.get(x, y)) & mask != check;
    if sides & LEFT != 0 {
        let mut found = false;
        'l: for x in 0..w {
            for y in 0..h {
                if differs(x, y, mask, check) {
                    crop.0 = x;
                    crop.2 -= x;
                    found = true;
                    break 'l;
                }
            }
        }
        if !found {
            return crop;
        }
    }
    if sides & TOP != 0 {
        'top: for y in 0..h {
            for x in 0..w {
                if differs(x, y, mask, check) {
                    crop.1 = y;
                    crop.3 -= y;
                    break 'top;
                }
            }
        }
    }
    if !same_color {
        let last = src.get(w - 1, h - 1);
        mask = if last.a == 0 {
            0xFF00_0000
        } else {
            0xFFFF_FFFF
        };
        check = bgra(last) & mask;
    }
    if sides & RIGHT != 0 {
        'r: for x in (0..w).rev() {
            for y in 0..h {
                if differs(x, y, mask, check) {
                    crop.2 = x - crop.0 + 1;
                    break 'r;
                }
            }
        }
    }
    if sides & BOTTOM != 0 {
        'b: for y in (0..h).rev() {
            for x in 0..w {
                if differs(x, y, mask, check) {
                    crop.3 = y - crop.1 + 1;
                    break 'b;
                }
            }
        }
    }
    crop
}

/// `ImageHelpers.AutoCropImage`.
pub fn auto_crop(src: Bitmap, same_color: bool, sides: u32, padding: i32) -> Bitmap {
    let r = auto_crop_rect(&src, same_color, sides);
    if r == (0, 0, src.w, src.h) {
        return src;
    }
    let Some(cropped) = crop(&src, r.0, r.1, r.2, r.3) else {
        return src;
    };
    if padding > 0 {
        let color = src.get(0, 0);
        return add_canvas(
            &cropped,
            Padding::new(padding, padding, padding, padding),
            color,
        )
        .unwrap_or(cropped);
    }
    cropped
}

/// One axis of [`resample`]: `data` is `sw` x `sh`, the result `len` long on the axis.
fn resample_axis(data: &[[f32; 4]], sw: i32, sh: i32, len: i32, horizontal: bool) -> Vec<[f32; 4]> {
    let (n, other) = if horizontal { (sw, sh) } else { (sh, sw) };
    let scale = len as f32 / n as f32;
    let widen = if scale < 1.0 { 1.0 / scale } else { 1.0 };
    let support = 2.0 * widen;
    let mirror = |i: i32| -> i32 {
        if n == 1 {
            return 0;
        }
        let period = 2 * n;
        let i = i.rem_euclid(period);
        if i >= n { period - 1 - i } else { i }
    };
    let kernel = |x: f32| -> f32 {
        let x = x.abs();
        if x < 1.0 {
            1.5 * x * x * x - 2.5 * x * x + 1.0
        } else if x < 2.0 {
            -0.5 * x * x * x + 2.5 * x * x - 4.0 * x + 2.0
        } else {
            0.0
        }
    };
    let (ow, oh) = if horizontal { (len, sh) } else { (sw, len) };
    let mut out = vec![[0f32; 4]; (ow * oh) as usize];
    let mut taps: Vec<(i32, f32)> = Vec::new();
    for d in 0..len {
        let center = (d as f32 + 0.5) / scale - 0.5;
        let lo = (center - support).floor() as i32;
        let hi = (center + support).ceil() as i32;
        taps.clear();
        let mut sum = 0.0;
        for s in lo..=hi {
            let wgt = kernel((s as f32 - center) / widen);
            if wgt != 0.0 {
                taps.push((mirror(s), wgt));
                sum += wgt;
            }
        }
        if sum != 0.0 {
            for t in &mut taps {
                t.1 /= sum;
            }
        }
        for o in 0..other {
            let mut acc = [0f32; 4];
            for &(s, wgt) in &taps {
                let i = if horizontal { o * sw + s } else { s * sw + o };
                let p = data[i as usize];
                for k in 0..4 {
                    acc[k] += p[k] * wgt;
                }
            }
            let i = if horizontal { o * ow + d } else { d * ow + o };
            out[i as usize] = acc;
        }
    }
    out
}

/// A separable resampler with a Catmull-Rom kernel widened for reductions and
/// mirrored edges: GDI+'s high-quality bicubic with `WrapMode.TileFlipXY`.
pub fn resample(src: &Bitmap, w: i32, h: i32) -> Bitmap {
    if w < 1 || h < 1 || src.w < 1 || src.h < 1 {
        return Bitmap::new(w.max(0), h.max(0));
    }
    if w == src.w && h == src.h {
        return src.clone();
    }
    let premul: Vec<[f32; 4]> = src
        .px
        .iter()
        .map(|c| {
            let a = c.a as f32 / 255.0;
            [c.r as f32 * a, c.g as f32 * a, c.b as f32 * a, c.a as f32]
        })
        .collect();
    let horiz = resample_axis(&premul, src.w, src.h, w, true);
    let both = resample_axis(&horiz, w, src.h, h, false);
    let px = both
        .iter()
        .map(|p| {
            let a = p[3].clamp(0.0, 255.0);
            if a < 0.5 {
                return C::ZERO;
            }
            let k = 255.0 / a;
            let ch = |v: f32| (v * k).clamp(0.0, 255.0).round() as u8;
            C::new(ch(p[0]), ch(p[1]), ch(p[2]), a.round() as u8)
        })
        .collect();
    Bitmap::from_pixels(w, h, px)
}

/// `ImageHelpers.ApplyAspectRatio`.
pub fn aspect_ratio(w: i32, h: i32, b: &Bitmap) -> (i32, i32) {
    if w == 0 {
        (
            ((h as f32 / b.h as f32 * b.w as f32) as f64).round_ties_even() as i32,
            h,
        )
    } else if h == 0 {
        (
            w,
            ((w as f32 / b.w as f32 * b.h as f32) as f64).round_ties_even() as i32,
        )
    } else {
        (w, h)
    }
}

/// `ImageHelpers.ResizeImage(bmp, width, height)`.
pub fn resize(src: Bitmap, w: i32, h: i32) -> Bitmap {
    if w < 1 || h < 1 || (src.w == w && src.h == h) {
        return src;
    }
    resample(&src, w, h)
}

/// `ResizeImage(bmp, w, h, allowEnlarge, centerImage, backColor)`.
pub fn resize_fit(
    src: &Bitmap,
    w: i32,
    h: i32,
    allow_enlarge: bool,
    center: bool,
    back: C,
) -> Bitmap {
    let (ratio, nw, nh) = if !allow_enlarge && src.w <= w && src.h <= h {
        (1.0f64, src.w, src.h)
    } else {
        let rx = w as f64 / src.w as f64;
        let ry = h as f64 / src.h as f64;
        let r = if rx < ry { rx } else { ry };
        (r, (src.w as f64 * r) as i32, (src.h as f64 * r) as i32)
    };
    let (mut x, mut y) = (0, 0);
    if center {
        x += ((w as f64 - src.w as f64 * ratio) / 2.0) as i32;
        y += ((h as f64 - src.h as f64 * ratio) / 2.0) as i32;
    }
    let mut canvas = Canvas::new(w, h);
    if back.a > 0 {
        canvas.clear(back);
    }
    let scaled = resample(src, nw, nh);
    canvas.draw_bitmap(&scaled, x as f32, y as f32, &Paint::default());
    canvas.into_bitmap()
}

/// Draws `src` mapped by `m` with bicubic sampling (GDI+ `DrawImage` under a transform).
fn draw_transformed(canvas: &mut Canvas, src: &Bitmap, m: Transform) {
    canvas.draw_image_matrix(src, m, Sampling::CATMULL_ROM, &Paint::default().aa(true));
}

/// `ImageHelpers.RotateImage`.
pub fn rotate(src: &Bitmap, angle: f32, upsize: bool, clip: bool) -> Bitmap {
    if angle == 0.0 {
        return src.clone();
    }
    let (ow, oh) = (src.w, src.h);
    let (mut nw, mut nh) = (ow, oh);
    let mut scale = 1f32;
    if upsize || !clip {
        let rad = angle as f64 * std::f64::consts::PI / 180.0;
        let (c, s) = (rad.cos().abs(), rad.sin().abs());
        nw = (ow as f64 * c + oh as f64 * s).round_ties_even() as i32;
        nh = (ow as f64 * s + oh as f64 * c).round_ties_even() as i32;
    }
    if !upsize && !clip {
        scale = (ow as f32 / nw as f32).min(oh as f32 / nh as f32);
        nw = ow;
        nh = oh;
    }
    // `bmp.CreateEmptyBitmap()`: the result keeps the source's size.
    let mut canvas = Canvas::new(ow, oh);
    let m = Transform::from_translate(nw as f32 / 2.0, nh as f32 / 2.0)
        .pre_scale(scale, scale)
        .pre_concat(crate::sk::canvas::rotation(angle))
        .pre_translate(-ow as f32 / 2.0, -oh as f32 / 2.0);
    draw_transformed(&mut canvas, src, m);
    canvas.into_bitmap()
}

/// `ImageHelpers.AddSkew`: the picture drawn onto a parallelogram.
pub fn add_skew(src: &Bitmap, x: i32, y: i32) -> Bitmap {
    let mut canvas = Canvas::new(src.w + x.abs(), src.h + y.abs());
    let sx = -(0.min(x)) as f32;
    let sy = -(0.min(y)) as f32;
    let ex = 0.max(x) as f32;
    let ey = 0.max(y) as f32;
    // Upper-left, upper-right and lower-left corners go to these points.
    let p0 = (sx, sy);
    let p1 = (sx + src.w as f32 - 1.0, ey);
    let p2 = (ex, sy + src.h as f32 - 1.0);
    let (w, h) = (src.w as f32, src.h as f32);
    let m = Transform::from_row(
        (p1.0 - p0.0) / w,
        (p1.1 - p0.1) / w,
        (p2.0 - p0.0) / h,
        (p2.1 - p0.1) / h,
        p0.0,
        p0.1,
    );
    draw_transformed(&mut canvas, src, m);
    canvas.into_bitmap()
}

/// `ImageHelpers.AddReflection`.
pub fn add_reflection(src: &Bitmap, percentage: i32, max_a: i32, min_a: i32) -> Bitmap {
    let percentage = percentage.clamp(1, 100);
    let max_a = max_a.clamp(0, 255);
    let min_a = min_a.clamp(0, 255);
    let rh = (src.h as f32 * (percentage as f32 / 100.0)) as i32;
    let mut out = Bitmap::new(src.w, rh.max(0));
    let add = max_a - min_a;
    let height = (rh - 1) as f32;
    for y in 0..rh {
        let alpha = (max_a as f32 - add as f32 * (y as f32 / height)) as u8;
        for x in 0..src.w {
            let mut c = src.get(x, src.h - 1 - y);
            if c.a > alpha {
                c.a = alpha;
            }
            out.px[(y * src.w + x) as usize] = c;
        }
    }
    out
}

/// `ImageHelpers.DrawReflection`.
pub fn draw_reflection(
    src: &Bitmap,
    percentage: i32,
    max_a: i32,
    min_a: i32,
    offset: i32,
    skew: bool,
    skew_size: i32,
) -> Bitmap {
    let mut reflection = add_reflection(src, percentage, max_a, min_a);
    if skew {
        reflection = add_skew(&reflection, skew_size, 0);
    }
    let mut canvas = Canvas::new(reflection.w, src.h + reflection.h + offset);
    canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    canvas.draw_bitmap(&reflection, 0.0, (src.h + offset) as f32, &Paint::default());
    canvas.into_bitmap()
}

fn texture(src: &Bitmap) -> Shader {
    Shader::Image {
        bitmap: Arc::new(src.clone()),
        tile_x: Tile::Repeat,
        tile_y: Tile::Repeat,
        matrix: Transform::identity(),
        sampling: Sampling::Nearest,
    }
}

/// `ImageHelpers.RoundedCorners`.
pub fn rounded_corners(src: &Bitmap, radius: i32) -> Bitmap {
    let mut canvas = Canvas::new(src.w, src.h);
    let r = radius as f32;
    let (w, h) = (src.w as f32, src.h as f32);
    let paint = Paint::fill_aa(C::BLACK).with_shader(texture(src));
    if let Some(path) = rounded_rect_path(0.0, 0.0, w, h, r) {
        canvas.draw_path(&path, &paint);
    }
    canvas.into_bitmap()
}

/// `GraphicsPathExtensions.AddRoundedRectangle`: four arcs, a capsule when the radius
/// reaches half the shorter side, a rectangle at zero.
pub fn rounded_rect_path(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<tiny_skia::Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let mut pb = PathBuilder::new();
    if radius <= 0.0 {
        pb.push_rect(tiny_skia::Rect::from_xywh(x, y, w, h)?);
        return pb.finish();
    }
    let r = radius.min(w.min(h) / 2.0);
    let d = r * 2.0;
    crate::sk::canvas::add_arc(&mut pb, (x, y, x + d, y + d), 180.0, 90.0);
    crate::sk::canvas::add_arc(&mut pb, (x + w - d, y, x + w, y + d), 270.0, 90.0);
    crate::sk::canvas::add_arc(&mut pb, (x + w - d, y + h - d, x + w, y + h), 0.0, 90.0);
    crate::sk::canvas::add_arc(&mut pb, (x, y + h - d, x + d, y + h), 90.0, 90.0);
    pb.close();
    pb.finish()
}

/// A `LinearGradientBrush` for `GradientInfo` over a rectangle.
pub fn gradient_shader(g: &Gradient, x: f32, y: f32, w: f32, h: f32) -> Shader {
    let (colors, pos) = g.color_blend();
    let (p0, p1) = match g.mode.as_str() {
        "Horizontal" => ((x, y), (x + w, y)),
        "ForwardDiagonal" => ((x, y), (x + w, y + h)),
        "BackwardDiagonal" => ((x + w, y), (x, y + h)),
        _ => ((x, y), (x, y + h)),
    };
    Shader::Linear {
        p0,
        p1,
        colors,
        pos: Some(pos),
        tile: Tile::Clamp,
    }
}

/// What fills an area: a colour or a gradient.
pub enum Fill<'a> {
    Color(C),
    Gradient(&'a Gradient),
}

impl Fill<'_> {
    fn paint(&self, x: f32, y: f32, w: f32, h: f32) -> Paint {
        match self {
            Fill::Color(c) => Paint::fill(*c),
            Fill::Gradient(g) => Paint::default().with_shader(gradient_shader(g, x, y, w, h)),
        }
    }
}

/// `ImageHelpers.FillBackground`.
pub fn fill_background(src: &Bitmap, fill: Fill) -> Bitmap {
    let (w, h) = (src.w as f32, src.h as f32);
    let mut canvas = Canvas::new(src.w, src.h);
    canvas.draw_rect(rect(0.0, 0.0, w, h), &fill.paint(0.0, 0.0, w, h));
    canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    canvas.into_bitmap()
}

/// GDI+ dash patterns (in pen widths).
fn dash_intervals(style: &str, width: f32) -> Option<Vec<f32>> {
    let base: &[f32] = match style {
        "Dash" => &[3.0, 1.0],
        "Dot" => &[1.0, 1.0],
        "DashDot" => &[3.0, 1.0, 1.0, 1.0],
        "DashDotDot" => &[3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        _ => return None,
    };
    Some(base.iter().map(|v| v * width).collect())
}

/// `ImageHelpers.DrawBorder` with an inset pen (`DrawRectangleProper`).
pub fn draw_border(src: Bitmap, fill: Fill, size: i32, outside: bool, dash: &str) -> Bitmap {
    let (w, h) = if outside {
        (src.w + size * 2, src.h + size * 2)
    } else {
        (src.w, src.h)
    };
    let mut canvas = if outside {
        Canvas::new(w, h)
    } else {
        Canvas::from_bitmap(&src)
    };
    let s = size as f32;
    // `DrawRectangleProper`: one pixel narrower for a one-pixel pen; an inset pen keeps
    // the stroke inside the rectangle.
    let (rw, rh) = if size == 1 {
        (w as f32 - 1.0, h as f32 - 1.0)
    } else {
        (w as f32, h as f32)
    };
    let mut paint = fill.paint(0.0, 0.0, w as f32, h as f32);
    paint.style = Style::Stroke;
    paint.stroke_width = s;
    paint.cap = Cap::Butt;
    if let Some(d) = dash_intervals(dash, s) {
        paint.dash = Some((d, 0.0));
    }
    let r = if size == 1 {
        rect(0.5, 0.5, rw + 0.5, rh + 0.5)
    } else {
        rect(s / 2.0, s / 2.0, rw - s / 2.0, rh - s / 2.0)
    };
    if rw > 0.0 && rh > 0.0 {
        canvas.draw_rect(r, &paint);
    }
    if outside {
        canvas.draw_bitmap(&src, s, s, &Paint::default());
    }
    canvas.into_bitmap()
}

/// `ImageHelpers.DrawCheckers`.
pub fn draw_checkers(src: &Bitmap, size: i32, c1: C, c2: C) -> Bitmap {
    let mut pattern = Bitmap::new(size * 2, size * 2);
    for y in 0..size * 2 {
        for x in 0..size * 2 {
            let first = (x < size) == (y < size);
            pattern.px[(y * size * 2 + x) as usize] = if first { c1 } else { c2 };
        }
    }
    let mut canvas = Canvas::new(src.w, src.h);
    let paint = Paint::default().with_shader(texture(&pattern));
    canvas.draw_rect(rect(0.0, 0.0, src.w as f32, src.h as f32), &paint);
    canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    canvas.into_bitmap()
}

/// `ImageHelpers.DrawBackgroundImage`.
pub fn draw_background_image(src: &Bitmap, bg: &Bitmap, center: bool, tile: bool) -> Bitmap {
    let (w, h) = (src.w, src.h);
    let mut canvas = Canvas::new(w, h);
    if tile {
        let (mut tx, mut ty) = (0, 0);
        if center {
            tx = (w - bg.w) / 2 % bg.w;
            ty = (h - bg.h) / 2 % bg.h;
        }
        let paint = Paint::default().with_shader(Shader::Image {
            bitmap: Arc::new(bg.clone()),
            tile_x: Tile::Repeat,
            tile_y: Tile::Repeat,
            matrix: Transform::from_translate(tx as f32, ty as f32),
            sampling: Sampling::Nearest,
        });
        canvas.draw_rect(rect(0.0, 0.0, w as f32, h as f32), &paint);
    } else {
        let ratio = bg.w as f32 / bg.h as f32;
        let mut bw = w;
        let mut bh = (bw as f32 / ratio) as i32;
        if bh < h {
            bh = h;
            bw = (bh as f32 * ratio) as i32;
        }
        let (x, y) = if center {
            ((w - bw) / 2, (h - bh) / 2)
        } else {
            (0, 0)
        };
        let scaled = resample(bg, bw, bh);
        canvas.draw_bitmap(&scaled, x as f32, y as f32, &Paint::default());
    }
    canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    canvas.into_bitmap()
}

/// `ImageHelpers.AddShadow`.
pub fn add_shadow(
    src: &Bitmap,
    opacity: f32,
    size: i32,
    darkness: f32,
    color: C,
    offset: (i32, i32),
    auto_resize: bool,
) -> Bitmap {
    let mut shadow = Bitmap::new(src.w + size * 2, src.h + size * 2);
    let masked = color_matrix(src, &m_mask(opacity, color));
    for y in 0..src.h {
        for x in 0..src.w {
            shadow.px[((y + size) * shadow.w + x + size) as usize] = masked.get(x, y);
        }
    }
    if size > 0 {
        box_blur(&mut shadow, size);
    }
    if darkness > 1.0 {
        shadow = color_matrix(&shadow, &m_alpha(darkness, 0.0));
    }
    if auto_resize {
        let mut canvas = Canvas::new(shadow.w + offset.0.abs(), shadow.h + offset.1.abs());
        canvas.draw_bitmap(
            &shadow,
            0.max(offset.0) as f32,
            0.max(offset.1) as f32,
            &Paint::default(),
        );
        canvas.draw_bitmap(
            src,
            size.max(-offset.0 + size) as f32,
            size.max(-offset.1 + size) as f32,
            &Paint::default(),
        );
        canvas.into_bitmap()
    } else {
        let mut canvas = Canvas::new(src.w, src.h);
        canvas.draw_bitmap(
            &shadow,
            (-size + offset.0) as f32,
            (-size + offset.1) as f32,
            &Paint::default(),
        );
        canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
        canvas.into_bitmap()
    }
}

/// `ImageHelpers.CreateGradientMask`.
fn gradient_mask(src: &Bitmap, g: &Gradient, opacity: f32) -> Bitmap {
    let (w, h) = (src.w as f32, src.h as f32);
    let mut canvas = Canvas::new(src.w, src.h);
    if opacity <= 0.0 {
        return canvas.into_bitmap();
    }
    if g.is_valid() {
        canvas.draw_rect(
            rect(0.0, 0.0, w, h),
            &Paint::default().with_shader(gradient_shader(g, 0.0, 0.0, w, h)),
        );
    }
    let mut mask = canvas.into_bitmap();
    for (m, s) in mask.px.iter_mut().zip(&src.px) {
        m.a = (255f32).min(s.a as f32 * (m.a as f32 / 255.0) * opacity) as u8;
    }
    mask
}

/// `ImageHelpers.AddGlow`.
pub fn add_glow(
    src: &Bitmap,
    size: i32,
    strength: f32,
    color: C,
    offset: (i32, i32),
    gradient: Option<&Gradient>,
) -> Bitmap {
    if size < 0 || strength < 0.1 {
        return src.clone();
    }
    let blur = if size > 0 {
        let mut b = add_canvas(src, Padding::new(size, size, size, size), C::ZERO)
            .unwrap_or_else(|| src.clone());
        box_blur(&mut b, size);
        b
    } else {
        src.clone()
    };
    let mask = match gradient {
        Some(g) if g.is_valid() => gradient_mask(&blur, g, strength),
        _ => color_matrix(&blur, &m_mask(strength, color)),
    };
    let mut canvas = Canvas::new(mask.w + offset.0.abs(), mask.h + offset.1.abs());
    canvas.draw_bitmap(
        &mask,
        0.max(offset.0) as f32,
        0.max(offset.1) as f32,
        &Paint::default(),
    );
    canvas.draw_bitmap(
        src,
        size.max(-offset.0 + size) as f32,
        size.max(-offset.1 + size) as f32,
        &Paint::default(),
    );
    canvas.into_bitmap()
}

/// Fills a polygon (or GDI+'s closed cardinal curve, tension 0.5) with the picture as
/// a texture, on a transparent canvas of the picture's size.
fn fill_with_picture(src: &Bitmap, points: &[(f32, f32)], curve: bool) -> Bitmap {
    let mut canvas = Canvas::new(src.w, src.h);
    if points.len() < 3 {
        return canvas.into_bitmap();
    }
    let mut pb = PathBuilder::new();
    pb.move_to(points[0].0, points[0].1);
    if curve {
        let n = points.len();
        let t = 0.5 / 3.0;
        for i in 0..n {
            let p0 = points[(i + n - 1) % n];
            let p1 = points[i];
            let p2 = points[(i + 1) % n];
            let p3 = points[(i + 2) % n];
            let c1 = (p1.0 + (p2.0 - p0.0) * t, p1.1 + (p2.1 - p0.1) * t);
            let c2 = (p2.0 - (p3.0 - p1.0) * t, p2.1 - (p3.1 - p1.1) * t);
            pb.cubic_to(c1.0, c1.1, c2.0, c2.1, p2.0, p2.1);
        }
    } else {
        for p in &points[1..] {
            pb.line_to(p.0, p.1);
        }
    }
    pb.close();
    if let Some(path) = pb.finish() {
        // `PixelOffsetMode.Half`: pixel centres at integer coordinates.
        canvas.save();
        canvas.translate(0.5, 0.5);
        let paint = Paint::fill_aa(C::BLACK).with_shader(Shader::Image {
            bitmap: Arc::new(src.clone()),
            tile_x: Tile::Repeat,
            tile_y: Tile::Repeat,
            matrix: Transform::from_translate(-0.5, -0.5),
            sampling: Sampling::Nearest,
        });
        canvas.draw_path(&path, &paint);
        canvas.restore();
    }
    canvas.into_bitmap()
}

/// `ImageHelpers.WavyEdges`.
pub fn wavy_edges(src: &Bitmap, depth: i32, range: i32, sides: u32) -> Bitmap {
    if depth < 1 || range < 1 || sides == 0 {
        return src.clone();
    }
    let (w, h) = (src.w, src.h);
    let hwc = 2.max((w / range + 1) / 2 * 2) - 1;
    let vwc = 2.max((h / range + 1) / 2 * 2) - 1;
    let hwr = w / hwc;
    let vwr = h / vwc;
    let step = (range / depth).max(1).min(10);
    let wave = |t: i32, max: i32| -> i32 {
        ((1.0 - (t as f64 * std::f64::consts::PI / max as f64).cos()) * depth as f64 / 2.0) as i32
    };
    let mut pts: Vec<(i32, i32)> = Vec::new();
    if sides & TOP != 0 {
        let sx = if sides & LEFT != 0 { depth } else { 0 };
        let ex = if sides & RIGHT != 0 { w - depth } else { w };
        let mut x = sx;
        while x < ex {
            pts.push((x, wave(x, hwr)));
            x += step;
        }
        pts.push((ex, wave(ex, hwr)));
    } else {
        pts.push((0, 0));
    }
    if sides & RIGHT != 0 {
        let sy = if sides & TOP != 0 { depth } else { 0 };
        let ey = if sides & BOTTOM != 0 { h - depth } else { h };
        let mut y = sy;
        while y < ey {
            pts.push((w - depth + wave(y, vwr), y));
            y += step;
        }
        pts.push((w - depth + wave(ey, vwr), ey));
    } else {
        let last = pts[pts.len() - 1];
        pts.push((w, last.1));
    }
    if sides & BOTTOM != 0 {
        let sx = if sides & RIGHT != 0 { w - depth } else { w };
        let ex = if sides & LEFT != 0 { depth } else { 0 };
        let mut x = sx;
        while x >= ex {
            pts.push((x, h - depth + wave(x, hwr)));
            x -= step;
        }
        pts.push((ex, h - depth + wave(ex, hwr)));
    } else {
        let last = pts[pts.len() - 1];
        pts.push((last.0, h));
    }
    if sides & LEFT != 0 {
        let sy = if sides & BOTTOM != 0 { h - depth } else { h };
        let ey = if sides & TOP != 0 { depth } else { 0 };
        let mut y = sy;
        while y >= ey {
            pts.push((wave(y, vwr), y));
            y -= step;
        }
        pts.push((wave(ey, vwr), ey));
    } else {
        let last = pts[pts.len() - 1];
        pts.push((0, last.1));
    }
    if sides & TOP == 0 {
        let last = pts[pts.len() - 1];
        pts[0] = (last.0, 0);
    }
    let fp: Vec<(f32, f32)> = pts.iter().map(|p| (p.0 as f32, p.1 as f32)).collect();
    fill_with_picture(src, &fp, false)
}

/// `ImageHelpers.TornEdges` (random depths).
pub fn torn_edges(src: &Bitmap, depth: i32, range: i32, sides: u32, curved: bool) -> Bitmap {
    if depth < 1 || range < 1 || sides == 0 {
        return src.clone();
    }
    let (w, h) = (src.w, src.h);
    let hc = w / range;
    let vc = h / range;
    if hc < 2 && vc < 2 {
        return src.clone();
    }
    let mut rng = crate::dotnet::shared_random();
    let mut r = || rand_incl(&mut rng, 0, depth);
    let mut pts: Vec<(i32, i32)> = Vec::new();
    if sides & TOP != 0 && hc > 1 {
        let sx = if sides & LEFT != 0 && vc > 1 {
            depth
        } else {
            0
        };
        let ex = if sides & RIGHT != 0 && vc > 1 {
            w - depth
        } else {
            w
        };
        let mut x = sx;
        while x < ex {
            pts.push((x, r()));
            x += range;
        }
    } else {
        pts.push((0, 0));
        pts.push((w, 0));
    }
    if sides & RIGHT != 0 && vc > 1 {
        let sy = if sides & TOP != 0 && hc > 1 { depth } else { 0 };
        let ey = if sides & BOTTOM != 0 && hc > 1 {
            h - depth
        } else {
            h
        };
        let mut y = sy;
        while y < ey {
            pts.push((w - depth + r(), y));
            y += range;
        }
    } else {
        pts.push((w, 0));
        pts.push((w, h));
    }
    if sides & BOTTOM != 0 && hc > 1 {
        let sx = if sides & RIGHT != 0 && vc > 1 {
            w - depth
        } else {
            w
        };
        let ex = if sides & LEFT != 0 && vc > 1 {
            depth
        } else {
            0
        };
        let mut x = sx;
        while x >= ex {
            pts.push((x, h - depth + r()));
            x = (x / range - 1) * range;
        }
    } else {
        pts.push((w, h));
        pts.push((0, h));
    }
    if sides & LEFT != 0 && vc > 1 {
        let sy = if sides & BOTTOM != 0 && hc > 1 {
            h - depth
        } else {
            h
        };
        let ey = if sides & TOP != 0 && hc > 1 { depth } else { 0 };
        let mut y = sy;
        while y >= ey {
            pts.push((r(), y));
            y = (y / range - 1) * range;
        }
    } else {
        pts.push((0, h));
        pts.push((0, 0));
    }
    let mut distinct: Vec<(i32, i32)> = Vec::new();
    for p in pts {
        if !distinct.contains(&p) {
            distinct.push(p);
        }
    }
    let fp: Vec<(f32, f32)> = distinct.iter().map(|p| (p.0 as f32, p.1 as f32)).collect();
    fill_with_picture(src, &fp, curved)
}

/// `ImageHelpers.Slice` (`RandomFast` bounds are inclusive).
pub fn slice(src: &Bitmap, min_h: i32, max_h: i32, min_s: i32, max_s: i32) -> Bitmap {
    let mut rng = crate::dotnet::shared_random();
    let mut canvas = Canvas::new(src.w, src.h);
    let mut y = 0;
    while y < src.h {
        let sh = rand_incl(&mut rng, min_h, max_h).max(1);
        let shift = if rand_incl(&mut rng, 0, 1) == 0 {
            rand_incl(&mut rng, -max_s, -min_s)
        } else {
            rand_incl(&mut rng, min_s, max_s)
        };
        let part_h = sh.min(src.h - y);
        canvas.draw_bitmap_rect(
            src,
            rect(0.0, y as f32, src.w as f32, (y + part_h) as f32),
            rect(
                shift as f32,
                y as f32,
                (shift + src.w) as f32,
                (y + part_h) as f32,
            ),
            Sampling::Nearest,
            &Paint::default(),
        );
        y += sh;
    }
    canvas.into_bitmap()
}

/// `RGBSplit.Apply`.
pub fn rgb_split(src: &Bitmap, red: (i32, i32), green: (i32, i32), blue: (i32, i32)) -> Bitmap {
    let (w, h) = (src.w, src.h);
    let at = |x: i32, y: i32, o: (i32, i32)| {
        src.get((x - o.0).clamp(0, w - 1), (y - o.1).clamp(0, h - 1))
    };
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let (cr, cg, cb) = (at(x, y, red), at(x, y, green), at(x, y, blue));
            px.push(C::new(
                (cr.r as u32 * cr.a as u32 / 255) as u8,
                (cg.g as u32 * cg.a as u32 / 255) as u8,
                (cb.b as u32 * cb.a as u32 / 255) as u8,
                ((cr.a as u32 + cg.a as u32 + cb.a as u32) / 3) as u8,
            ));
        }
    }
    Bitmap::from_pixels(w, h, px)
}

/// `Bitmap.RotateFlip` for flips.
pub fn flip(src: &Bitmap, horizontal: bool, vertical: bool) -> Bitmap {
    let (w, h) = (src.w, src.h);
    let mut out = Bitmap::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let sx = if horizontal { w - 1 - x } else { x };
            let sy = if vertical { h - 1 - y } else { y };
            out.px[(y * w + x) as usize] = src.get(sx, sy);
        }
    }
    out
}

/// `Bitmap.RotateFlip((RotateFlipType)ImageRotateFlipType)`.
pub fn rotate_flip(src: &Bitmap, mode: &str) -> Bitmap {
    let rot90 = |b: &Bitmap| {
        let mut out = Bitmap::new(b.h, b.w);
        for y in 0..b.h {
            for x in 0..b.w {
                out.px[(x * b.h + (b.h - 1 - y)) as usize] = b.get(x, y);
            }
        }
        out
    };
    match mode {
        "Rotate90" => rot90(src),
        "Rotate180" => flip(src, true, true),
        "Rotate270" => rot90(&flip(src, true, true)),
        "FlipX" => flip(src, true, false),
        "Rotate90FlipX" => flip(&rot90(src), true, false),
        "FlipY" => flip(src, false, true),
        "Rotate90FlipY" => flip(&rot90(src), false, true),
        _ => src.clone(),
    }
}

/// Sides from `AnchorStyles` (all by default).
pub fn sides_or_all(sides: u32) -> u32 {
    if sides > ANCHOR_ALL {
        ANCHOR_ALL
    } else {
        sides
    }
}

/// Draws `img` into `dst` with the paint's blend (`Graphics.DrawImage(img, rect)` with a
/// compositing mode and an interpolation mode).
pub fn draw_image(
    canvas: &mut Canvas,
    img: &Bitmap,
    dst: (i32, i32, i32, i32),
    sampling: Sampling,
    blend: Blend,
    alpha: f32,
) {
    let scaled;
    let pic = if (dst.2, dst.3) != (img.w, img.h) && sampling != Sampling::Nearest {
        scaled = resample(img, dst.2, dst.3);
        &scaled
    } else {
        img
    };
    let mut paint = Paint::alpha((alpha.clamp(0.0, 1.0) * 255.0).round() as u8).with_blend(blend);
    paint.aa = true;
    canvas.draw_bitmap_to(
        pic,
        rect(
            dst.0 as f32,
            dst.1 as f32,
            (dst.0 + dst.2) as f32,
            (dst.1 + dst.3) as f32,
        ),
        sampling,
        &paint,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: i32, h: i32, c: C) -> Bitmap {
        Bitmap::filled(w, h, c)
    }

    #[test]
    fn colour_matrices_do_what_their_names_say() {
        let b = solid(2, 2, C::rgb(10, 100, 200));
        let inv = color_matrix(&b, &m_inverse());
        assert_eq!(inv.get(0, 0), C::rgb(245, 155, 55));
        let gray = color_matrix(&b, &m_grayscale(1.0));
        let g = gray.get(1, 1);
        assert!(g.r == g.g && g.g == g.b);
        let half = color_matrix(&b, &m_alpha(0.5, 0.0));
        assert_eq!(half.get(0, 0).a, 128);
    }

    #[test]
    fn box_blur_averages_and_keeps_size() {
        let mut b = Bitmap::new(9, 1);
        b.px[4] = C::rgb(255, 255, 255);
        box_blur(&mut b, 3);
        assert_eq!((b.w, b.h), (9, 1));
        assert!(b.px[4].r < 255 && b.px[3].r > 0);
    }

    #[test]
    fn canvas_crop_and_auto_crop() {
        let src = solid(4, 3, C::RED);
        let big = add_canvas(&src, Padding::new(1, 2, 3, 4), C::BLUE).unwrap();
        assert_eq!((big.w, big.h), (8, 9));
        assert_eq!(big.get(0, 0), C::BLUE);
        assert_eq!(big.get(1, 2), C::RED);
        let cropped = auto_crop(big, false, ANCHOR_ALL, 0);
        assert_eq!((cropped.w, cropped.h), (4, 3));
        assert!(crop(&src, 1, 1, 4, 1).is_none());
    }

    #[test]
    fn rotate_flip_and_resample() {
        let mut src = Bitmap::new(3, 2);
        src.px[0] = C::RED;
        let r = rotate_flip(&src, "Rotate90");
        assert_eq!((r.w, r.h), (2, 3));
        assert_eq!(r.get(1, 0), C::RED);
        let big = resample(&solid(4, 4, C::GREEN), 9, 7);
        assert_eq!((big.w, big.h), (9, 7));
        assert_eq!(big.get(4, 3), C::GREEN);
    }

    #[test]
    fn pixelate_averages_blocks() {
        let mut b = Bitmap::new(2, 1);
        b.px[0] = C::rgb(0, 0, 0);
        b.px[1] = C::rgb(200, 100, 50);
        pixelate(&mut b, 2, 0, C::BLACK);
        assert_eq!(b.px[0], b.px[1]);
        assert_eq!(b.px[0], C::rgb(100, 50, 25));
    }
}
