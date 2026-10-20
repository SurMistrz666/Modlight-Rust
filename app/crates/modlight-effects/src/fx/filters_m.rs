//! Filters, part M: Stained glass .. Unsharp mask.

use std::f32::consts::PI;
use std::sync::Arc;

use modlight_core::icons as l;
use modlight_editor::effects::Category;
use tiny_skia::{PathBuilder, Transform};

use crate::def::*;
use crate::dotnet::{DotNetRandom, shared_random};
use crate::helpers::{
    bilinear_sample, blur_bitmap, clamp_to_byte, clamp01, fractal_noise, hash01, lerp, luminance01,
    screen, smooth_step,
};
use crate::sk::{Bitmap, C, Canvas, Paint, Sampling, Shader, Tile, round};

const F: Category = Category::Filters;

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "stained_glass",
            "Stained glass",
            F,
            l::HEXAGON,
            "Creates a stained glass mosaic effect with lead borders.",
            stained_glass,
        )
        .params(|| {
            vec![
                int_slider("tile_size", "Tile size", 6, 120, 22),
                float_slider("irregularity", "Irregularity", 0.0, 100.0, 55.0),
                float_slider("lead_width", "Lead width", 0.0, 12.0, 1.8f32 as f64),
                float_slider("lead_opacity", "Lead opacity", 0.0, 100.0, 85.0),
                float_slider("color_boost", "Color boost", 0.0, 100.0, 20.0),
            ]
        }),
        Def::new(
            "star_filter",
            "Star filter",
            F,
            l::STAR,
            "Adds star-shaped light streaks to bright areas.",
            star_filter,
        )
        .params(|| {
            vec![
                float_slider("threshold", "Threshold", 0.0, 100.0, 74.0),
                float_slider("length", "Length", 4.0, 80.0, 40.0),
                float_slider("strength", "Strength", 0.0, 100.0, 58.0),
                float_slider("rotation", "Rotation", 0.0, 360.0, 0.0),
                float_slider("warmth", "Warmth", 0.0, 100.0, 48.0),
            ]
        }),
        Def::new(
            "starfield_border",
            "Starfield border",
            F,
            l::ORBIT,
            "Adds a cosmic starfield border with scattered stars, nebula wisps, and subtle color tints.",
            starfield_border,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 12, 300, 60),
                color("bg_color", "Background color", C::rgb(5, 5, 18)),
                color("nebula_color", "Nebula tint", C::rgb(40, 10, 80)),
                float_slider("star_density", "Star density", 10.0, 100.0, 65.0),
                float_slider("nebula_intensity", "Nebula intensity", 0.0, 100.0, 45.0),
                checkbox("bright_stars", "Bright star glow", true),
            ]
        }),
        Def::new(
            "stripe_overlay",
            "Stripe overlay",
            F,
            l::BLINDS,
            "Overlays diagonal stripes on the image.",
            stripes,
        )
        .params(|| {
            vec![
                int_slider("width", "Stripe width", 2, 50, 10),
                int_slider("gap", "Gap width", 2, 50, 10),
                int_slider("angle", "Angle", 0, 180, 45),
                float_slider("opacity", "Opacity", 0.0, 100.0, 40.0),
                color("color", "Color", C::BLACK),
            ]
        }),
        Def::new(
            "surface_blur",
            "Surface blur",
            F,
            l::WAVES,
            "Blurs while preserving edges based on color similarity.",
            surface_blur,
        )
        .params(|| {
            vec![
                int_slider("radius", "Radius", 1, 8, 3),
                int_slider("threshold", "Threshold", 1, 100, 24),
            ]
        }),
        Def::new(
            "tv_static",
            "TV static",
            F,
            l::TV,
            "Blends black-and-white TV static noise over the image.",
            tv_static,
        )
        .params(|| vec![float_slider("amount", "Amount", 0.0, 100.0, 40.0)]),
        Def::new(
            "thermal_vision",
            "Thermal vision",
            F,
            l::THERMOMETER_SUN,
            "Maps image luminance to a thermal color gradient.",
            thermal,
        )
        .params(|| {
            vec![
                int_slider("levels", "Levels", 3, 16, 8),
                float_slider("contrast", "Contrast", 50.0, 200.0, 135.0),
                float_slider("glow", "Glow", 0.0, 100.0, 28.0),
                float_slider("blend", "Blend", 0.0, 100.0, 100.0),
                checkbox("invert", "Invert", false),
            ]
        }),
        Def::new(
            "tilt_shift",
            "Tilt-shift",
            F,
            l::FOCUS,
            "Simulates a tilt-shift lens with selective focus.",
            tilt_shift,
        )
        .params(|| {
            vec![
                choice(
                    "mode",
                    "Mode",
                    &[("Linear", "Linear"), ("Radial", "Radial")],
                    "Linear",
                ),
                float_slider("blur_radius", "Blur radius", 0.0, 30.0, 12.0),
                float_slider("focus_size", "Focus size", 5.0, 90.0, 30.0),
                float_slider("focus_position_x", "Focus position X", 0.0, 100.0, 50.0),
                float_slider("focus_position_y", "Focus position Y", 0.0, 100.0, 50.0),
                float_slider("falloff", "Falloff", 1.0, 60.0, 24.0),
                float_slider("saturation_boost", "Saturation boost", 0.0, 100.0, 35.0),
            ]
        }),
        Def::new(
            "torn_edge",
            "Torn edge",
            F,
            l::SCISSORS_LINE_DASHED,
            "Adds a torn edge border effect.",
            torn_edge,
        )
        .params(|| {
            vec![
                int_slider("depth", "Depth", 1, 100, 20).snap(false),
                int_slider("range", "Range", 1, 100, 20).snap(false),
                checkbox("top", "Top", true),
                checkbox("right", "Right", true),
                checkbox("bottom", "Bottom", true),
                checkbox("left", "Left", true),
                checkbox("curved", "Curved edges", false),
            ]
        }),
        Def::new(
            "triangle_pixelate",
            "Triangle pixelate",
            F,
            l::TRIANGLE,
            "Pixelates the image using triangular tiles.",
            triangle_pixelate,
        )
        .params(|| vec![int_slider("size", "Size", 8, 100, 24)]),
        Def::new(
            "unsharp_mask",
            "Unsharp mask",
            F,
            l::WAND_SPARKLES,
            "Sharpens the image using an unsharp mask technique.",
            unsharp,
        )
        .params(|| {
            vec![
                float_slider("radius", "Radius", 1.0, 100.0, 5.0),
                float_slider("amount", "Amount", 0.0, 500.0, 150.0),
                int_slider("threshold", "Threshold", 0, 255, 0),
            ]
        }),
    ]
}

fn glass_hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32)
        .wrapping_mul(374_761_393)
        .wrapping_add(y.wrapping_mul(668_265_263) as u32);
    h ^= seed;
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    (h & 0xFF_FFFF) as f32 / 16_777_215.0
}

fn stained_glass(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let tile = p.i("tile_size").clamp(6, 120);
    let irr = p.f("irregularity").clamp(0.0, 100.0) / 100.0;
    let lead = p.f("lead_width").clamp(0.0, 12.0);
    let lead_op = p.f("lead_opacity").clamp(0.0, 100.0) / 100.0;
    let boost = p.f("color_boost").clamp(0.0, 100.0) / 100.0;
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let gw = (w as f32 / tile as f32).ceil() as i32 + 2;
    let gh = (h as f32 / tile as f32).ceil() as i32 + 2;
    let jitter = tile as f32 * 0.5 * irr;
    let base_seed = 1337u32.wrapping_mul(2_246_822_519);
    let ci = |v: i32| v.clamp(0, 255) as u8;
    let mut grid = Vec::with_capacity((gw * gh) as usize);
    for gy in 0..gh {
        for gx in 0..gw {
            let bx = ((gx - 1) as f32 + 0.5) * tile as f32;
            let by = ((gy - 1) as f32 + 0.5) * tile as f32;
            let ox = (glass_hash(gx, gy, base_seed ^ 0x9E37_79B9) * 2.0 - 1.0) * jitter;
            let oy = (glass_hash(gx, gy, base_seed ^ 0x85EB_CA6B) * 2.0 - 1.0) * jitter;
            let cx = (bx + ox).clamp(0.0, (w - 1) as f32);
            let cy = (by + oy).clamp(0.0, (h - 1) as f32);
            let sx = (round(cx) as i32).clamp(0, w - 1);
            let sy = (round(cy) as i32).clamp(0, h - 1);
            let mut c = src.get(sx, sy);
            if boost > 0.0 {
                let (r, g, b) = (c.r as f32, c.g as f32, c.b as f32);
                let gray = (r + g + b) / 3.0;
                let f = 1.0 + boost;
                c = C::new(
                    ci(round(gray + (r - gray) * f) as i32),
                    ci(round(gray + (g - gray) * f) as i32),
                    ci(round(gray + (b - gray) * f) as i32),
                    c.a,
                );
            }
            grid.push((cx, cy, c));
        }
    }
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let gy = y / tile + 1;
        for x in 0..w {
            let gx = x / tile + 1;
            let (mut nearest, mut second) = (f32::MAX, f32::MAX);
            let mut color = src.get(x, y);
            for ny in gy - 1..=gy + 1 {
                for nx in gx - 1..=gx + 1 {
                    let (cx, cy, c) = grid[(ny * gw + nx) as usize];
                    let dx = x as f32 - cx;
                    let dy = y as f32 - cy;
                    let d2 = dx * dx + dy * dy;
                    if d2 < nearest {
                        second = nearest;
                        nearest = d2;
                        color = c;
                    } else if d2 < second {
                        second = d2;
                    }
                }
            }
            let mut out = color;
            if lead > 0.0 && second < f32::MAX {
                let ed = (second.sqrt() - nearest.sqrt()) * 0.5;
                if ed < lead {
                    let a = (lead_op * (1.0 - ed / lead)).clamp(0.0, 1.0);
                    let keep = 1.0 - a;
                    out = C::new(
                        ci(round(out.r as f32 * keep) as i32),
                        ci(round(out.g as f32 * keep) as i32),
                        ci(round(out.b as f32 * keep) as i32),
                        out.a,
                    );
                }
            }
            px.push(out);
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn star_filter(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let threshold = p.f("threshold").clamp(0.0, 100.0) / 100.0;
    let length = p.f("length").clamp(4.0, 80.0);
    let strength = p.f("strength").clamp(0.0, 100.0) / 100.0;
    let warmth = p.f("warmth").clamp(0.0, 100.0) / 100.0;
    if strength <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let n = (round(length / 4.0) as i32).clamp(3, 18);
    let step = (length / n as f32).max(1.0);
    let base = p.f("rotation") * (PI / 180.0);
    let angles = [base, base + PI * 0.5, base + PI * 0.25, base + PI * 0.75];
    let weights = [1.0, 1.0, 0.68, 0.68];
    let dirs: Vec<(f32, f32)> = angles.iter().map(|a| (a.cos(), a.sin())).collect();
    let denom = 0.0001f32.max(1.0 - threshold);
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let mut streak = 0f32;
            for (d, &(dx, dy)) in dirs.iter().enumerate() {
                for i in 1..=n {
                    let falloff = 1.0 - i as f32 / (n + 1) as f32;
                    let off = i as f32 * step;
                    let pos =
                        bilinear_sample(&src.px, w, h, x as f32 + dx * off, y as f32 + dy * off);
                    let neg =
                        bilinear_sample(&src.px, w, h, x as f32 - dx * off, y as f32 - dy * off);
                    let la = (luminance01(pos) - threshold).max(0.0) / denom;
                    let lb = (luminance01(neg) - threshold).max(0.0) / denom;
                    streak += (la * la + lb * lb) * falloff * weights[d];
                }
            }
            let star = strength * (streak / (n as f32 * 3.6));
            let s = src.get(x, y);
            let r = screen(s.r as f32 / 255.0, star * (0.90 + warmth * 0.18));
            let g = screen(s.g as f32 / 255.0, star * (0.74 + warmth * 0.08));
            let b = screen(s.b as f32 / 255.0, star * (0.52 - warmth * 0.16));
            px.push(C::new(
                clamp_to_byte(r * 255.0),
                clamp_to_byte(g * 255.0),
                clamp_to_byte(b * 255.0),
                s.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn starfield_border(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(12, 300);
    let dens = p.f("star_density").clamp(10.0, 100.0) / 100.0;
    let neb = p.f("nebula_intensity").clamp(0.0, 100.0) / 100.0;
    let (bg, nc) = (p.c("bg_color"), p.c("nebula_color"));
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let seed = 4207;
    let scale = 0.012f32;
    let mut px = vec![C::ZERO; (nw * nh) as usize];
    for y in 0..nh {
        let top = y < border;
        let bottom = y >= border + src.h;
        for x in 0..nw {
            let left = x < border;
            let right = x >= border + src.w;
            if !top && !bottom && !left && !right {
                continue;
            }
            let (mut r, mut g, mut b) = (
                bg.r as f32 / 255.0,
                bg.g as f32 / 255.0,
                bg.b as f32 / 255.0,
            );
            let (xf, yf) = (x as f32, y as f32);
            let n1 = fractal_noise(xf * scale, yf * scale, 4, 2.0, 0.5, seed);
            let n2 = fractal_noise(
                xf * scale * 0.7 + 300.0,
                yf * scale * 0.7 + 500.0,
                3,
                2.0,
                0.5,
                seed ^ 0x33,
            );
            let m1 = clamp01((n1 + 0.3) * 0.7) * neb;
            let m2 = clamp01((n2 + 0.2) * 0.5) * neb * 0.5;
            r += (nc.r as f32 / 255.0) * m1 + 0.1 * m2;
            g += (nc.g as f32 / 255.0) * m1 + 0.15 * m2;
            b += (nc.b as f32 / 255.0) * m1 + 0.25 * m2;
            let sh = hash01(x, y, seed ^ 0xAA);
            let st = 1.0 - dens * 0.015;
            if sh > st {
                let mut sb = (sh - st) / (1.0 - st);
                sb *= sb;
                let hs = hash01(x ^ 0x55, y ^ 0x33, seed ^ 0xBB);
                r += sb * (0.7 + hs * 0.3);
                g += sb * (0.7 + (1.0 - hs) * 0.3);
                b += sb * 0.9;
            }
            px[(y * nw + x) as usize] = C::new(
                clamp_to_byte(r * 255.0),
                clamp_to_byte(g * 255.0),
                clamp_to_byte(b * 255.0),
                255,
            );
        }
    }
    let mut canvas = Canvas::from_bitmap(&Bitmap::from_pixels(nw, nh, px));
    if p.b("bright_stars") {
        let count = ((nw + nh) as f32 * 0.15 * dens) as i32;
        for i in 0..count {
            let sx = hash01(i, 0, seed ^ 0xDE) * nw as f32;
            let sy = hash01(0, i, seed ^ 0xEF) * nh as f32;
            let in_border = sx < border as f32
                || sx >= (border + src.w) as f32
                || sy < border as f32
                || sy >= (border + src.h) as f32;
            if !in_border {
                continue;
            }
            let br = hash01(i, i, seed ^ 0xF1);
            let size = 0.8 + br * 2.5;
            let temp = hash01(i ^ 0x77, i ^ 0x99, seed ^ 0xF2);
            let r = clamp_to_byte(200.0 + 55.0 * br);
            let g = clamp_to_byte(190.0 + 55.0 * br * (0.9 + temp * 0.1));
            let b = clamp_to_byte(180.0 + 75.0 * br * (0.5 + temp * 0.5));
            let a = clamp_to_byte(160.0 + 95.0 * br);
            let glow = Paint::fill_aa(C::new(r, g, b, a / 2)).with_mask_blur(3.0);
            canvas.draw_circle(sx, sy, size * 2.5, &glow);
            canvas.draw_circle(sx, sy, size, &Paint::fill_aa(C::new(r, g, b, a)));
        }
    }
    canvas.draw_bitmap(src, border as f32, border as f32, &Paint::default());
    Ok(canvas.into_bitmap())
}

fn stripes(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let alpha = (p.f("opacity") / 100.0).clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return Ok(src.clone());
    }
    let sw = p.i("width");
    let period = 4.max(sw + p.i("gap"));
    let (w, h) = (src.w, src.h);
    let c = p.c("color");
    let paint = Paint::fill_aa(c.with_alpha((c.a as f32 * alpha) as u8));
    let diag = ((w * w + h * h) as f32).sqrt();
    let count = (diag / period as f32) as i32 + 2;
    let mut canvas = Canvas::from_bitmap(src);
    canvas.save();
    canvas.rotate_degrees_about(p.i("angle") as f32, w as f32 / 2.0, h as f32 / 2.0);
    let sx = -diag / 2.0 + w as f32 / 2.0;
    let sy = -diag / 2.0 + h as f32 / 2.0;
    for i in 0..count {
        let x = sx + (i * period) as f32;
        canvas.draw_rect_xywh(x, sy, sw as f32, diag * 2.0, &paint);
    }
    canvas.restore();
    Ok(canvas.into_bitmap())
}

fn surface_blur(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let radius = p.i("radius").clamp(1, 8);
    let threshold = p.i("threshold").clamp(1, 100);
    let tsum = threshold * 3;
    let scale = 0.65 / 1.max(tsum) as f32;
    let (w, h) = (src.w, src.h);
    let mut offsets = Vec::new();
    for ky in -radius..=radius {
        for kx in -radius..=radius {
            if kx != 0 || ky != 0 {
                offsets.push((kx, ky));
            }
        }
    }
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let c0 = src.get(x, y);
            let (mut sr, mut sg, mut sb, mut sa, mut sw) =
                (c0.r as f32, c0.g as f32, c0.b as f32, c0.a as f32, 1f32);
            for &(kx, ky) in &offsets {
                let c = src.get((x + kx).clamp(0, w - 1), (y + ky).clamp(0, h - 1));
                let delta = (c.r as i32 - c0.r as i32).abs()
                    + (c.g as i32 - c0.g as i32).abs()
                    + (c.b as i32 - c0.b as i32).abs();
                if delta > tsum {
                    continue;
                }
                let wt = 1.0 - delta as f32 * scale;
                sr += c.r as f32 * wt;
                sg += c.g as f32 * wt;
                sb += c.b as f32 * wt;
                sa += c.a as f32 * wt;
                sw += wt;
            }
            let inv = 1.0 / 0.0001f32.max(sw);
            px.push(C::new(
                clamp_to_byte(sr * inv),
                clamp_to_byte(sg * inv),
                clamp_to_byte(sb * inv),
                clamp_to_byte(sa * inv),
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn tv_static(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let blend = (p.f("amount") / 100.0).clamp(0.0, 1.0);
    if blend <= 0.0 {
        return Ok(src.clone());
    }
    let mut rng = DotNetRandom::new(42);
    let px = src
        .px
        .iter()
        .map(|c| {
            let n = rng.next_max(256) as f32;
            let mix = |v: u8| (v as f32 + (n - v as f32) * blend) as u8;
            C::new(mix(c.r), mix(c.g), mix(c.b), c.a)
        })
        .collect();
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}

fn thermal_gradient(t: f32) -> C {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: C, b: C, t: f32| {
        C::new(
            clamp_to_byte(lerp(a.r as f32, b.r as f32, t)),
            clamp_to_byte(lerp(a.g as f32, b.g as f32, t)),
            clamp_to_byte(lerp(a.b as f32, b.b as f32, t)),
            255,
        )
    };
    if t <= 0.20 {
        mix(C::rgb(0, 0, 52), C::rgb(0, 34, 170), t / 0.20)
    } else if t <= 0.40 {
        mix(C::rgb(0, 34, 170), C::rgb(0, 160, 255), (t - 0.20) / 0.20)
    } else if t <= 0.60 {
        mix(C::rgb(0, 160, 255), C::rgb(0, 255, 210), (t - 0.40) / 0.20)
    } else if t <= 0.76 {
        mix(C::rgb(0, 255, 210), C::rgb(255, 255, 0), (t - 0.60) / 0.16)
    } else if t <= 0.90 {
        mix(C::rgb(255, 255, 0), C::rgb(255, 80, 0), (t - 0.76) / 0.14)
    } else {
        mix(C::rgb(255, 80, 0), C::rgb(255, 255, 255), (t - 0.90) / 0.10)
    }
}

fn thermal(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let levels = p.i("levels").clamp(3, 16);
    let contrast = p.f("contrast").clamp(50.0, 200.0) / 100.0;
    let glow = p.f("glow").clamp(0.0, 100.0) / 100.0;
    let blend = p.f("blend").clamp(0.0, 100.0) / 100.0;
    let invert = p.b("invert");
    if src.w <= 0 || src.h <= 0 {
        return Ok(src.clone());
    }
    let px = src
        .px
        .iter()
        .map(|s| {
            let mut lum = (0.2126 * s.r as f32 + 0.7152 * s.g as f32 + 0.0722 * s.b as f32) / 255.0;
            lum = ((lum - 0.5) * contrast + 0.5).clamp(0.0, 1.0);
            if invert {
                lum = 1.0 - lum;
            }
            let q = round(lum * (levels - 1) as f32) / 1f32.max(levels as f32 - 1.0);
            let t = thermal_gradient(q);
            let gb = glow * q.powf(2.2);
            let tr = t.r as f32 + t.r as f32 * gb * 0.20;
            let tg = t.g as f32 + t.g as f32 * gb * 0.25;
            let tb = t.b as f32 + t.b as f32 * gb * 0.35;
            C::new(
                clamp_to_byte(lerp(s.r as f32, tr, blend)),
                clamp_to_byte(lerp(s.g as f32, tg, blend)),
                clamp_to_byte(lerp(s.b as f32, tb, blend)),
                s.a,
            )
        })
        .collect();
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}

fn tilt_shift(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let blur = p.f("blur_radius").clamp(0.0, 30.0);
    let focus = p.f("focus_size").clamp(5.0, 90.0) / 100.0;
    let fx = p.f("focus_position_x").clamp(0.0, 100.0) / 100.0;
    let fy = p.f("focus_position_y").clamp(0.0, 100.0) / 100.0;
    let falloff = p.f("falloff").clamp(1.0, 60.0) / 100.0;
    let sat = p.f("saturation_boost").clamp(0.0, 100.0) / 100.0;
    let radial = p.e("mode") == "Radial";
    let blurred = if blur > 0.01 {
        blur_bitmap(src, blur, blur)
    } else {
        src.clone()
    };
    let cx = fx * (w - 1) as f32;
    let cy = fy * (h - 1) as f32;
    let min_dim = w.min(h) as f32;
    let half_lin = (h as f32 * focus) * 0.5;
    let rad_r = (min_dim * focus) * 0.5;
    let fall_lin = (h as f32 * falloff).max(1.0);
    let fall_rad = (min_dim * falloff).max(1.0);
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let mask = if radial {
                let (dx, dy) = (x as f32 - cx, y as f32 - cy);
                smooth_step(rad_r, rad_r + fall_rad, (dx * dx + dy * dy).sqrt())
            } else {
                smooth_step(half_lin, half_lin + fall_lin, (y as f32 - cy).abs())
            };
            let s = src.get(x, y);
            let b = blurred.get(x, y);
            let mut r = lerp(s.r as f32, b.r as f32, mask);
            let mut g = lerp(s.g as f32, b.g as f32, mask);
            let mut bl = lerp(s.b as f32, b.b as f32, mask);
            let amount = sat * (1.0 - mask * 0.7);
            if amount > 0.0 {
                let gray = (r + g + bl) / 3.0;
                let f = 1.0 + amount;
                r = gray + (r - gray) * f;
                g = gray + (g - gray) * f;
                bl = gray + (bl - gray) * f;
            }
            px.push(C::new(
                clamp_to_byte(r),
                clamp_to_byte(g),
                clamp_to_byte(bl),
                s.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn torn_edge(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (depth, range) = (p.i("depth"), p.i("range"));
    let (top, right, bottom, left) = (p.b("top"), p.b("right"), p.b("bottom"), p.b("left"));
    if depth < 1 || range < 1 || (!top && !right && !bottom && !left) {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let hc = w / range;
    let vc = h / range;
    if hc < 2 && vc < 2 {
        return Ok(src.clone());
    }
    let mut rand = shared_random();
    let mut pts: Vec<(f32, f32)> = Vec::new();
    if top && hc > 1 {
        let sx = if left && vc > 1 { depth } else { 0 };
        let ex = if right && vc > 1 { w - depth } else { w };
        let mut x = sx;
        while x < ex {
            pts.push((x as f32, rand.next_range(0, depth + 1) as f32));
            x += range;
        }
    } else {
        pts.push((0.0, 0.0));
        pts.push((w as f32, 0.0));
    }
    if right && vc > 1 {
        let sy = if top && hc > 1 { depth } else { 0 };
        let ey = if bottom && hc > 1 { h - depth } else { h };
        let mut y = sy;
        while y < ey {
            pts.push(((w - depth + rand.next_range(0, depth + 1)) as f32, y as f32));
            y += range;
        }
    } else {
        pts.push((w as f32, 0.0));
        pts.push((w as f32, h as f32));
    }
    if bottom && hc > 1 {
        let sx = if right && vc > 1 { w - depth } else { w };
        let ex = if left && vc > 1 { depth } else { 0 };
        let mut x = sx;
        while x >= ex {
            pts.push((x as f32, (h - depth + rand.next_range(0, depth + 1)) as f32));
            x -= range;
        }
    } else {
        pts.push((w as f32, h as f32));
        pts.push((0.0, h as f32));
    }
    if left && vc > 1 {
        let sy = if bottom && hc > 1 { h - depth } else { h };
        let ey = if top && hc > 1 { depth } else { 0 };
        let mut y = sy;
        while y >= ey {
            pts.push((rand.next_range(0, depth + 1) as f32, y as f32));
            y -= range;
        }
    } else {
        pts.push((0.0, h as f32));
        pts.push((0.0, 0.0));
    }
    let mut distinct: Vec<(f32, f32)> = Vec::new();
    for (i, pt) in pts.iter().enumerate() {
        if i == 0 || *pt != pts[i - 1] {
            distinct.push(*pt);
        }
    }
    if distinct.len() > 1 && distinct.last() == distinct.first() {
        distinct.pop();
    }
    let mut canvas = Canvas::new(w, h);
    if distinct.len() > 2 {
        let mut pb = PathBuilder::new();
        if p.b("curved") {
            let (lp, fp) = (distinct[distinct.len() - 1], distinct[0]);
            pb.move_to((lp.0 + fp.0) / 2.0, (lp.1 + fp.1) / 2.0);
            for i in 0..distinct.len() {
                let cur = distinct[i];
                let next = distinct[(i + 1) % distinct.len()];
                pb.quad_to(cur.0, cur.1, (cur.0 + next.0) / 2.0, (cur.1 + next.1) / 2.0);
            }
        } else {
            pb.move_to(distinct[0].0, distinct[0].1);
            for pt in &distinct[1..] {
                pb.line_to(pt.0, pt.1);
            }
        }
        pb.close();
        if let Some(path) = pb.finish() {
            let paint = Paint::fill_aa(C::BLACK).with_shader(Shader::Image {
                bitmap: Arc::new(src.clone()),
                tile_x: Tile::Clamp,
                tile_y: Tile::Clamp,
                matrix: Transform::identity(),
                sampling: Sampling::Nearest,
            });
            canvas.draw_path(&path, &paint);
        }
    }
    Ok(canvas.into_bitmap())
}

fn triangle_pixelate(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let size = 8.max(p.i("size"));
    let (w, h) = (src.w, src.h);
    let mut canvas = Canvas::new(w, h);
    let half = size as f32 / 2.0;
    let s = size as f32;
    let mut row = 0;
    while (row as f32) * half < (h + size) as f32 {
        let mut col = 0;
        while col * size < w + size {
            let x = (col * size) as f32;
            let y = row as f32 * half;
            let pts = if (row + col) % 2 == 0 {
                [(x, y), (x + s, y), (x + s / 2.0, y + half)]
            } else {
                [(x, y + half), (x + s, y + half), (x + s / 2.0, y)]
            };
            let cx = (pts[0].0 + pts[1].0 + pts[2].0) / 3.0;
            let cy = (pts[0].1 + pts[1].1 + pts[2].1) / 3.0;
            let c = src.get((cx as i32).clamp(0, w - 1), (cy as i32).clamp(0, h - 1));
            let mut pb = PathBuilder::new();
            pb.move_to(pts[0].0, pts[0].1);
            pb.line_to(pts[1].0, pts[1].1);
            pb.line_to(pts[2].0, pts[2].1);
            pb.close();
            if let Some(path) = pb.finish() {
                canvas.draw_path(&path, &Paint::fill(c));
            }
            col += 1;
        }
        row += 1;
    }
    Ok(canvas.into_bitmap())
}

fn unsharp(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let radius = p.f("radius").clamp(1.0, 100.0);
    let amount = p.f("amount").clamp(0.0, 500.0) / 100.0;
    let threshold = p.i("threshold").clamp(0, 255);
    if amount <= 0.0 {
        return Ok(src.clone());
    }
    let blurred = blur_bitmap(src, radius / 3.0, radius / 3.0);
    let ch = |o: u8, b: u8| -> u8 {
        let diff = o as i32 - b as i32;
        if diff.abs() < threshold {
            return o;
        }
        clamp_to_byte(o as f32 + diff as f32 * amount)
    };
    let px = src
        .px
        .iter()
        .zip(&blurred.px)
        .map(|(o, b)| C::new(ch(o.r, b.r), ch(o.g, b.g), ch(o.b, b.b), o.a))
        .collect();
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}
