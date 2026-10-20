//! Filters, part L: Rust / corrosion .. Stained glass border.

use std::f32::consts::PI;

use modlight_core::icons as l;
use modlight_editor::effects::Category;

use crate::def::*;
use crate::dotnet::{shared_next, shared_random};
use crate::fx::filters_d::convolve3x3;
use crate::helpers::{
    apply_contrast, bilinear_sample, blurred_clamp, clamp_to_byte, clamp01, fractal_noise, hash01,
    lerp, luminance01, screen, smooth_step,
};
use crate::sk::filters::matrix_convolution;
use crate::sk::{Bitmap, Blend, C, Canvas, ColorFilter, Paint, Sampling, Shader, Tile, rect};

const F: Category = Category::Filters;

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "rust_corrosion",
            "Rust / corrosion",
            F,
            l::BRUSH_CLEANING,
            "Simulates rust and corrosion effects with pitting and streaks.",
            rust,
        )
        .params(|| {
            vec![
                float_slider("corrosion", "Corrosion", 0.0, 100.0, 58.0),
                float_slider("pitting", "Pitting", 0.0, 100.0, 44.0),
                float_slider("streaks", "Streaks", 0.0, 100.0, 36.0),
                float_slider("dirt", "Dirt", 0.0, 100.0, 26.0),
                float_slider("edge_wear", "Edge wear", 0.0, 100.0, 52.0),
            ]
        }),
        Def::new(
            "scanline",
            "Scanlines",
            F,
            l::SCAN_LINE,
            "Adds retro CRT-style horizontal scanlines.",
            scanline,
        )
        .params(|| {
            vec![
                int_slider("spacing", "Line spacing", 2, 20, 4),
                float_slider("opacity", "Opacity", 0.0, 100.0, 50.0),
            ]
        }),
        Def::new(
            "shadow",
            "Shadow",
            F,
            l::CLOUD_MOON,
            "Adds a drop shadow to the image.",
            shadow,
        )
        .params(|| {
            vec![
                float_slider("opacity", "Opacity", 0.0, 100.0, 80.0)
                    .snap(false)
                    .fmt("{}{0:0}%"),
                int_slider("size", "Size", 0, 100, 20).snap(false),
                int_numeric("offset_x", "Offset X", -1000, 1000, 5),
                int_numeric("offset_y", "Offset Y", -1000, 1000, 5),
                color("color", "Color", C::BLACK),
                checkbox("auto_resize", "Auto resize", true),
            ]
        }),
        Def::new(
            "sharpen",
            "Sharpen",
            F,
            l::CROSSHAIR,
            "Sharpens the image.",
            sharpen,
        )
        .params(|| vec![int_slider("strength", "Strength", 0, 100, 50)]),
        Def::new("slice", "Slice", F, l::SLICE, "Slices the image.", slice).params(|| {
            vec![
                int_slider("min_height", "Min Height", 1, 100, 10).snap(false),
                int_slider("max_height", "Max Height", 1, 200, 100).snap(false),
                int_slider("min_shift", "Min Shift", 0, 100, 0).snap(false),
                int_slider("max_shift", "Max Shift", 0, 100, 10).snap(false),
            ]
        }),
        Def::new(
            "smoke_overlay",
            "Smoke overlay",
            F,
            l::HAZE,
            "Adds a soft procedural smoke overlay to the image.",
            smoke,
        )
        .params(|| {
            vec![
                float_slider("density", "Density", 0.0, 100.0, 42.0),
                float_slider("scale", "Scale", 0.0, 100.0, 68.0),
                float_slider("drift", "Drift", -100.0, 100.0, 18.0),
                float_slider("softness", "Softness", 0.0, 100.0, 55.0),
                float_slider("contrast", "Contrast", 0.0, 100.0, 48.0),
            ]
        }),
        Def::new(
            "smooth",
            "Smooth",
            F,
            l::WAVES,
            "Applies a smoothing effect.",
            |s, _| {
                let k = 1.0 / 9.0;
                Ok(convolve3x3(s, &[k; 9], 1.0, 0.0))
            },
        )
        .immediate(),
        Def::new(
            "snowfall_depth_fog",
            "Snowfall + depth fog",
            F,
            l::CLOUD_SNOW,
            "Simulates snowfall with depth-based fog.",
            snowfall,
        )
        .params(|| {
            vec![
                float_slider("snow_amount", "Snow amount", 0.0, 100.0, 55.0),
                float_slider("flake_size", "Flake size", 1.0, 14.0, 5.0),
                float_slider("wind", "Wind", -100.0, 100.0, 20.0),
                float_slider("fog_amount", "Fog amount", 0.0, 100.0, 40.0),
                float_slider("fog_height", "Fog height", 10.0, 100.0, 65.0),
            ]
        }),
        Def::new(
            "sobel_edge",
            "Sobel edge",
            F,
            l::SCAN,
            "Detects edges using the Sobel operator.",
            sobel,
        )
        .params(|| {
            vec![
                float_slider("strength", "Strength", 0.1f32 as f64, 5.0, 1.2f32 as f64),
                int_slider("threshold", "Threshold", 0, 255, 20),
            ]
        }),
        Def::new(
            "soft_diffusion",
            "Soft diffusion",
            F,
            l::CIRCLE_FADING_PLUS,
            "Applies a soft diffusion glow with bloom and warmth.",
            soft_diffusion,
        )
        .params(|| {
            vec![
                float_slider("amount", "Amount", 0.0, 100.0, 52.0),
                float_slider("radius", "Radius", 0.0, 30.0, 12.0),
                float_slider("highlight_bloom", "Highlight bloom", 0.0, 100.0, 40.0),
                float_slider("contrast_softening", "Contrast softening", 0.0, 100.0, 28.0),
                float_slider("warmth", "Warmth", 0.0, 100.0, 22.0),
            ]
        }),
        Def::new(
            "spin_blur",
            "Spin blur",
            F,
            l::ROTATE_CW,
            "Applies a rotational motion blur around a center point.",
            spin_blur,
        )
        .params(|| {
            vec![
                float_slider("angle", "Angle", 0.0, 180.0, 20.0),
                int_slider("samples", "Samples", 4, 64, 24),
                float_slider("center_x", "Center X", 0.0, 100.0, 50.0),
                float_slider("center_y", "Center Y", 0.0, 100.0, 50.0),
            ]
        }),
        Def::new(
            "stained_glass_border",
            "Stained glass border",
            F,
            l::PALETTE,
            "Adds a stained glass border with jewel-toned glass panels divided by dark lead came lines.",
            stained_border,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 20, 300, 60),
                float_slider("panel_width", "Panel width", 10.0, 120.0, 40.0),
                float_slider("lead_thickness", "Lead thickness", 1.0, 12.0, 3.5),
                float_slider("glass_brightness", "Glass brightness", 30.0, 100.0, 75.0),
                checkbox("glow", "Glass glow", true),
                checkbox("inner_line", "Inner line", true),
            ]
        }),
    ]
}

fn lum01(c: C) -> f32 {
    (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0
}

fn rust(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let corrosion = p.f("corrosion").clamp(0.0, 100.0) / 100.0;
    let pitting = p.f("pitting").clamp(0.0, 100.0) / 100.0;
    let streaks = p.f("streaks").clamp(0.0, 100.0) / 100.0;
    let dirt = p.f("dirt").clamp(0.0, 100.0) / 100.0;
    let wear = p.f("edge_wear").clamp(0.0, 100.0) / 100.0;
    if corrosion <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let fnz = fractal_noise;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let v = y as f32 / 1.max(h - 1) as f32;
        for x in 0..w {
            let s = src.get(x, y);
            let (mut r, mut g, mut b) = (s.r as f32, s.g as f32, s.b as f32);
            let edge = clamp01(
                ((lum01(src.get((w - 1).min(x + 1), y)) - lum01(src.get(0.max(x - 1), y))).abs()
                    + (lum01(src.get(x, (h - 1).min(y + 1))) - lum01(src.get(x, 0.max(y - 1))))
                        .abs())
                    * 1.1,
            );
            let (xf, yf) = (x as f32, y as f32);
            let ws = 3.5 + corrosion * 12.0;
            let wx = (fnz(xf * 0.010, yf * 0.010, 3, 2.05, 0.55, 541) * 2.0 - 1.0) * ws;
            let wy = (fnz((xf + 91.7) * 0.010, (yf - 63.4) * 0.010, 3, 2.00, 0.55, 887) * 2.0
                - 1.0)
                * ws
                * 0.75;
            let (sx, sy) = (xf + wx, yf + wy);
            let coarse = fnz(sx * 0.016, sy * 0.016, 3, 2.00, 0.56, 701);
            let medium = fnz(sx * 0.040, sy * 0.040, 3, 2.15, 0.52, 1301);
            let fine = fnz(sx * 0.105, sy * 0.105, 2, 2.30, 0.60, 1907);
            let ss = fnz(xf * 0.012 + 17.2, yf * 0.0035 + 4.1, 3, 2.00, 0.58, 2203);
            let mut streak = smooth_step(0.70 - streaks * 0.08, 0.98, ss) * streaks * v;
            streak *= 0.42
                + 0.58
                    * fnz(
                        (xf - 41.3) * 0.022,
                        (yf + 18.7) * 0.060,
                        2,
                        2.20,
                        0.62,
                        3251,
                    );
            let mut mask = coarse * 0.52 + medium * 0.32 + fine * 0.16;
            mask = smooth_step(0.48 - corrosion * 0.18, 0.94, mask);
            mask = mask.max(edge * wear * 0.72);
            mask = clamp01(mask * corrosion + streak);
            if mask > 0.0001 {
                let tone = coarse * 0.56 + medium * 0.32 + fine * 0.12;
                let warmth = fnz(
                    (sx + 37.1) * 0.030,
                    (sy - 22.4) * 0.030,
                    2,
                    2.10,
                    0.60,
                    4783,
                );
                let rr = lerp(126.0, 214.0, tone * 0.72 + warmth * 0.28);
                let rg = lerp(44.0, 108.0, tone);
                let rb = lerp(12.0, 40.0, tone * 0.88);
                r = lerp(r, rr, mask * 0.86);
                g = lerp(g, rg, mask * 0.82);
                b = lerp(b, rb, mask * 0.78);
                let pn = fnz(sx * 0.150, sy * 0.150, 2, 2.35, 0.58, 6121);
                let pit = smooth_step(0.72, 0.96, pn) * pitting * mask;
                r *= 1.0 - pit * 0.26;
                g *= 1.0 - pit * 0.34;
                b *= 1.0 - pit * 0.38;
            }
            if dirt > 0.0001 {
                let grime =
                    (fnz(xf * 0.028, yf * 0.028, 3, 2.20, 0.55, 4021) * 2.0 - 1.0) * dirt * 14.0;
                r += grime * 0.80;
                g += grime * 0.70;
                b += grime * 0.55;
            }
            px.push(C::new(
                clamp_to_byte(r),
                clamp_to_byte(g),
                clamp_to_byte(b),
                s.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn scanline(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let alpha = (p.f("opacity") / 100.0).clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return Ok(src.clone());
    }
    let mut canvas = Canvas::from_bitmap(src);
    let paint = Paint::stroke(C::new(0, 0, 0, (255.0 * alpha) as u8), 1.0);
    let step = 2.max(p.i("spacing"));
    let mut y = 0;
    while y < src.h {
        canvas.draw_line(0.0, y as f32, src.w as f32, y as f32, &paint);
        y += step;
    }
    Ok(canvas.into_bitmap())
}

fn shadow(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let size = p.i("size");
    let (ox, oy) = (p.i("offset_x"), p.i("offset_y"));
    let auto = p.b("auto_resize");
    let pad = size;
    let el = if auto { 0.max(-ox) + pad } else { 0 };
    let er = if auto { 0.max(ox) + pad } else { 0 };
    let et = if auto { 0.max(-oy) + pad } else { 0 };
    let eb = if auto { 0.max(oy) + pad } else { 0 };
    let (nw, nh) = (src.w + el + er, src.h + et + eb);
    let mut canvas = Canvas::new(nw, nh);
    let c = p.c("color");
    let shadow = C::new(c.r, c.g, c.b, (255.0 * p.f("opacity") / 100.0) as u8);
    let sigma = size as f32 / 2.0;
    let paint = Paint {
        color_filter: Some(ColorFilter::Blend(shadow, Blend::SrcIn)),
        image_blur: (sigma > 0.0).then_some((sigma, sigma)),
        ..Paint::default()
    };
    canvas.draw_bitmap(src, (el + ox) as f32, (et + oy) as f32, &paint);
    canvas.draw_bitmap(src, el as f32, et as f32, &Paint::default());
    Ok(canvas.into_bitmap())
}

fn sharpen(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let strength = p.i("strength");
    if strength <= 0 {
        return Ok(src.clone());
    }
    let s = (strength as f32 / 100.0).clamp(0.0, 1.0);
    let (c, e) = (1.0 + 4.0 * s, -s);
    let k = [0.0, e, 0.0, e, c, e, 0.0, e, 0.0];
    Ok(matrix_convolution(
        src,
        3,
        3,
        &k,
        1.0,
        0.0,
        (1, 1),
        Tile::Decal,
        true,
    ))
}

fn slice(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (min_h, max_h) = (p.i("min_height"), p.i("max_height"));
    let (min_s, max_s) = (p.i("min_shift"), p.i("max_shift"));
    if min_h <= 0 && max_h <= 0 {
        return Ok(src.clone());
    }
    let lo_h = 1.max(min_h.min(max_h));
    let hi_h = lo_h.max(min_h.max(max_h));
    let lo_s = min_s.abs().min(max_s.abs());
    let hi_s = min_s.abs().max(max_s.abs());
    let mut rand = shared_random();
    let nw = src.w + hi_s * 2;
    let mut canvas = Canvas::new(nw, src.h);
    let mut y = 0;
    while y < src.h {
        let sh = rand.next_range(lo_h, hi_h + 1).min(src.h - y);
        let shift = if rand.next_max(2) == 0 {
            rand.next_range(-hi_s, -lo_s + 1)
        } else {
            rand.next_range(lo_s, hi_s + 1)
        };
        let (yf, hf) = (y as f32, sh as f32);
        let left = (hi_s + shift) as f32;
        canvas.draw_bitmap_rect(
            src,
            rect(0.0, yf, src.w as f32, yf + hf),
            rect(left, yf, left + src.w as f32, yf + hf),
            Sampling::Nearest,
            &Paint::default(),
        );
        y += sh;
    }
    Ok(canvas.into_bitmap())
}

fn smoke(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let density = p.f("density").clamp(0.0, 100.0) / 100.0;
    let scale = p.f("scale").clamp(0.0, 100.0) / 100.0;
    let drift = p.f("drift").clamp(-100.0, 100.0) / 100.0;
    let softness = p.f("softness").clamp(0.0, 100.0) / 100.0;
    let contrast = p.f("contrast").clamp(0.0, 100.0) / 100.0;
    if density <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let cf = 0.012 + (1.0 - scale) * 0.035;
    let mf = cf * 2.2;
    let ff = cf * 4.4;
    let fnz = fractal_noise;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let drift_off = drift * y as f32 * 0.014;
        for x in 0..w {
            let s = src.get(x, y);
            let (sr, sg, sb) = (s.r as f32 / 255.0, s.g as f32 / 255.0, s.b as f32 / 255.0);
            let pxf = x as f32 + drift_off;
            let nx = pxf * cf;
            let ny = y as f32 * cf;
            let wx = (fnz(nx * 0.85 + 7.2, ny * 0.85 - 3.1, 3, 2.05, 0.56, 911) * 2.0 - 1.0)
                * (0.75 + scale * 1.45);
            let wy = (fnz(nx * 0.90 - 5.4, ny * 0.90 + 11.8, 3, 2.00, 0.56, 1201) * 2.0 - 1.0)
                * (0.60 + scale * 1.15);
            let (sx, sy) = (nx + wx, ny + wy);
            let coarse = fnz(sx, sy, 4, 2.00, 0.55, 1559);
            let medium = fnz(
                sx * (mf / cf) + 13.1,
                sy * (mf / cf) - 7.6,
                3,
                2.10,
                0.52,
                2081,
            );
            let fine = fnz(
                sx * (ff / cf) - 9.4,
                sy * (ff / cf) + 4.7,
                2,
                2.20,
                0.58,
                2593,
            );
            let (xf, yf) = (x as f32, y as f32);
            let fa = (xf * cf * 16.0 + yf * cf * 6.0 + 0.7).sin();
            let fb = (xf * cf * 9.0 - yf * cf * 13.0 - 1.4).cos();
            let wisps = 1.0 - (medium * 2.0 - 1.0).abs();
            let mut sm = coarse * 0.58 + wisps * 0.24 + fine * 0.18;
            sm = clamp01(sm + fa * 0.08 + fb * 0.06);
            sm = clamp01((sm - 0.5) * (0.92 + contrast * 1.5) + 0.5);
            let th = 0.44 - density * 0.16;
            let mut a = smooth_step(th, 0.96, sm);
            a = a.powf(1.55 - softness * 1.10) * density * 0.68;
            px.push(C::new(
                clamp_to_byte(lerp(sr, 0.82 + sm * 0.10, a) * 255.0),
                clamp_to_byte(lerp(sg, 0.84 + sm * 0.10, a) * 255.0),
                clamp_to_byte(lerp(sb, 0.86 + sm * 0.12, a) * 255.0),
                s.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

#[allow(clippy::too_many_arguments)]
fn flake_layer(x: i32, y: i32, cell: f32, radius: f32, density: f32, wind: f32, seed: i32) -> f32 {
    let px = x as f32 + wind;
    let py = y as f32;
    let cx = (px / cell).floor() as i32;
    let cy = (py / cell).floor() as i32;
    let mut strongest = 0f32;
    for oy in -1..=1 {
        for ox in -1..=1 {
            let (nx, ny) = (cx + ox, cy + oy);
            if hash01(nx, ny, seed) > density {
                continue;
            }
            let ccx = (nx as f32 + hash01(nx, ny, seed ^ 1777)) * cell;
            let ccy = (ny as f32 + hash01(nx, ny, seed ^ 3907)) * cell;
            let (dx, dy) = (px - ccx, py - ccy);
            let d2 = dx * dx + dy * dy;
            let fr = radius * (0.6 + hash01(nx, ny, seed ^ 727) * 0.9);
            if d2 >= fr * fr {
                continue;
            }
            let a = 1.0 - smooth_step(fr * 0.3, fr, d2.sqrt());
            if a > strongest {
                strongest = a;
            }
        }
    }
    strongest
}

fn snowfall(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let snow = p.f("snow_amount").clamp(0.0, 100.0) / 100.0;
    let flake = p.f("flake_size").clamp(1.0, 14.0);
    let wind = p.f("wind").clamp(-100.0, 100.0) / 100.0;
    let fog = p.f("fog_amount").clamp(0.0, 100.0) / 100.0;
    let fog_h = p.f("fog_height").clamp(10.0, 100.0) / 100.0;
    if snow <= 0.0001 && fog <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let seed = shared_next(1, i32::MAX);
    let inv_h = 1.0 / 1.max(h - 1) as f32;
    let c1 = (22.0 - flake * 0.9).clamp(8.0, 24.0);
    let c2 = 6f32.max(c1 * 0.58);
    let r1 = 0.9 + flake * 0.22;
    let r2 = 0.55 + flake * 0.14;
    let d1 = 0.015 + snow * 0.22;
    let d2 = 0.04 + snow * 0.30;
    let fog_start = 1.0 - fog_h;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let ny = y as f32 * inv_h;
        let depth = 1.0 - ny;
        let persp = 0.55 + ny * 0.45;
        let wind_off = wind * y as f32 * 0.38;
        for x in 0..w {
            let s = src.get(x, y);
            let (mut r, mut g, mut b) = (s.r as f32, s.g as f32, s.b as f32);
            if fog > 0.0001 {
                let fm = smooth_step(fog_start, 1.0, depth);
                if fm > 0.0001 {
                    let coarse = hash01(
                        (x as f32 * 0.06) as i32,
                        (y as f32 * 0.06) as i32,
                        seed ^ 191,
                    );
                    let broad = hash01(
                        (x as f32 * 0.018) as i32,
                        (y as f32 * 0.018) as i32,
                        seed ^ 379,
                    );
                    let fnoise = coarse * 0.65 + broad * 0.35;
                    let f = fog * fm * (0.75 + 0.25 * fnoise) * (0.62 + 0.38 * depth);
                    r = lerp(r, 226.0, f);
                    g = lerp(g, 236.0, f);
                    b = lerp(b, 248.0, f);
                }
            }
            if snow > 0.0001 {
                let l1 = flake_layer(x, y, c1, r1, d1, wind_off, seed ^ 911);
                let l2 = flake_layer(x, y, c2, r2, d2, wind_off * 1.3, seed ^ 3001);
                let sparkle = hash01(x, y, seed ^ 4451).powf(20.0) * snow * 0.9;
                let sn = ((l1 * 0.72 + l2 * 0.48 + sparkle) * persp).clamp(0.0, 1.0);
                if sn > 0.0001 {
                    r = lerp(r, 255.0, sn);
                    g = lerp(g, 255.0, sn);
                    b = lerp(b, 255.0, sn);
                }
            }
            px.push(C::new(
                clamp_to_byte(r),
                clamp_to_byte(g),
                clamp_to_byte(b),
                clamp_to_byte(s.a as f32),
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn sobel(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let strength = p.f("strength").clamp(0.1, 5.0);
    let threshold = p.i("threshold").clamp(0, 255);
    let (w, h) = (src.w, src.h);
    let luma: Vec<i32> = src
        .px
        .iter()
        .map(|c| ((c.r as i32 * 77 + c.g as i32 * 150 + c.b as i32 * 29) >> 8) & 0xFF)
        .collect();
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let r0 = (y - 1).clamp(0, h - 1) * w;
        let r1 = y * w;
        let r2 = (y + 1).clamp(0, h - 1) * w;
        for x in 0..w {
            let x0 = (x - 1).clamp(0, w - 1);
            let x2 = (x + 1).clamp(0, w - 1);
            let l = |row: i32, col: i32| luma[(row + col) as usize];
            let gx = (-l(r0, x0) + l(r0, x2))
                + (-2 * l(r1, x0) + 2 * l(r1, x2))
                + (-l(r2, x0) + l(r2, x2));
            let gy =
                (-l(r0, x0) - 2 * l(r0, x) - l(r0, x2)) + (l(r2, x0) + 2 * l(r2, x) + l(r2, x2));
            let mag = ((gx * gx + gy * gy) as f32).sqrt() * strength;
            let e = if mag >= threshold as f32 {
                clamp_to_byte(mag)
            } else {
                0
            };
            px.push(C::new(e, e, e, src.get(x, y).a));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn soft_diffusion(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let amount = p.f("amount").clamp(0.0, 100.0) / 100.0;
    let radius = p.f("radius").clamp(0.0, 30.0);
    let bloom = p.f("highlight_bloom").clamp(0.0, 100.0) / 100.0;
    let softness = p.f("contrast_softening").clamp(0.0, 100.0) / 100.0;
    let warmth = p.f("warmth").clamp(0.0, 100.0) / 100.0;
    if amount <= 0.0001 {
        return Ok(src.clone());
    }
    let blur = blurred_clamp(src, radius.max(0.1));
    let post = 1.0 - softness * 0.28;
    let px = src
        .px
        .iter()
        .zip(&blur.px)
        .map(|(s, bl)| {
            let (sr, sg, sb) = (s.r as f32 / 255.0, s.g as f32 / 255.0, s.b as f32 / 255.0);
            let (br, bg, bb) = (
                bl.r as f32 / 255.0,
                bl.g as f32 / 255.0,
                bl.b as f32 / 255.0,
            );
            let bl_lum = luminance01(*bl);
            let mask = 0.45 + bloom * bl_lum.powf(1.7) * 0.85;
            let dr = screen(sr, br * mask * (1.02 + warmth * 0.08));
            let dg = screen(sg, bg * mask * 0.96);
            let db = screen(sb, bb * mask * (0.92 - warmth * 0.06));
            let mut r = apply_contrast(lerp(sr, dr, amount), post);
            let mut g = apply_contrast(lerp(sg, dg, amount), post);
            let mut b = apply_contrast(lerp(sb, db, amount), post);
            if warmth > 0.0001 {
                let wm = warmth * bl_lum.powf(1.35) * 0.22;
                r = screen(r, wm);
                g = screen(g, wm * 0.55);
                b = lerp(b, (b - wm * 0.12).max(0.0), wm);
            }
            C::new(
                clamp_to_byte(r * 255.0),
                clamp_to_byte(g * 255.0),
                clamp_to_byte(b * 255.0),
                s.a,
            )
        })
        .collect();
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}

fn spin_blur(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let angle = p.f("angle").clamp(0.0, 180.0);
    let n = p.i("samples").clamp(4, 64) as usize;
    if angle <= 0.001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let cx = (p.f("center_x").clamp(0.0, 100.0) / 100.0) * (w - 1) as f32;
    let cy = (p.f("center_y").clamp(0.0, 100.0) / 100.0) * (h - 1) as f32;
    let max_rad = angle * (PI / 180.0);
    let half = (n - 1) as f32 * 0.5;
    let inv_half = if half <= 0.0001 { 1.0 } else { 1.0 / half };
    let table: Vec<(f32, f32, f32)> = (0..n)
        .map(|i| {
            let t = (i as f32 - half) * inv_half;
            let th = t * max_rad;
            (th.sin(), th.cos(), 1.0 - t.abs() * 0.7)
        })
        .collect();
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            if dx * dx + dy * dy < 0.25 {
                px.push(src.get(x, y));
                continue;
            }
            let (mut sr, mut sg, mut sb, mut sa, mut sw) = (0f32, 0f32, 0f32, 0f32, 0f32);
            for &(sin, cos, wt) in &table {
                let sx = cx + (dx * cos - dy * sin);
                let sy = cy + (dx * sin + dy * cos);
                let c = bilinear_sample(&src.px, w, h, sx, sy);
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

const JEWEL_HUES: [f32; 9] = [0.0, 30.0, 60.0, 120.0, 180.0, 210.0, 270.0, 300.0, 340.0];

#[allow(clippy::too_many_arguments)]
fn glass_panels(
    canvas: &mut Canvas,
    (rx, ry, rw, rh): (f32, f32, f32, f32),
    horizontal: bool,
    panel_w: f32,
    lead: f32,
    brightness: f32,
    seed: i32,
    glow: bool,
) {
    let primary = if horizontal { rw } else { rh };
    let cross = if horizontal { rh } else { rw };
    let rows = 1f32.max(cross / panel_w);
    let ph = cross / rows;
    canvas.save();
    canvas.clip_rect(rect(rx, ry, rx + rw, ry + rh), false);
    let mut index = 0i32;
    let mut cr = 0f32;
    while cr < rows {
        let cs = cr * ph;
        let ce = cs + ph;
        let mut along = 0f32;
        while along < primary {
            let wv = hash01(index, seed, seed ^ 0x7F);
            let len = panel_w * (0.65 + wv * 0.75);
            let end = (along + len).min(primary);
            let hh = hash01(index ^ index.wrapping_mul(7), seed ^ 0x33, seed);
            let hue = JEWEL_HUES[((hh * 9.0) as i32 % 9) as usize];
            let sat = 70.0 + hash01(index, seed ^ 0x55, seed) * 30.0;
            let (x1, y1, x2, y2) = if horizontal {
                (
                    rx + along + lead * 0.5,
                    ry + cs + lead * 0.5,
                    rx + end - lead * 0.5,
                    ry + ce - lead * 0.5,
                )
            } else {
                (
                    rx + cs + lead * 0.5,
                    ry + along + lead * 0.5,
                    rx + ce - lead * 0.5,
                    ry + end - lead * 0.5,
                )
            };
            if x2 > x1 + 1.0 && y2 > y1 + 1.0 {
                glass_panel(
                    canvas,
                    x1,
                    y1,
                    x2,
                    y2,
                    C::from_hsv(hue, sat, brightness, 255),
                    lead,
                    glow,
                );
            }
            along = end;
            index += 1;
        }
        cr += 1.0;
    }
    canvas.restore();
}

#[allow(clippy::too_many_arguments)]
fn glass_panel(
    canvas: &mut Canvas,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    base: C,
    lead: f32,
    glow: bool,
) {
    let (w, h) = (x2 - x1, y2 - y1);
    if w < 1.0 || h < 1.0 {
        return;
    }
    if glow {
        let paint = Paint::fill_aa(base.with_alpha(60)).with_mask_blur(lead * 1.8);
        canvas.draw_rect_xywh(x1, y1, w, h, &paint);
    }
    let up = |v: u8| (v as i32 + 63).min(255) as u8;
    let down = |v: u8| (v as i32 - 38).max(0) as u8;
    let light = C::new(up(base.r), up(base.g), up(base.b), 245);
    let dark = C::new(down(base.r), down(base.g), down(base.b), 235);
    let glass = Paint::fill_aa(C::BLACK).with_shader(Shader::Linear {
        p0: (x1, y1),
        p1: (x2, y2),
        colors: vec![light, dark],
        pos: None,
        tile: Tile::Clamp,
    });
    canvas.draw_rect_xywh(x1, y1, w, h, &glass);
    let spec = Paint::fill_aa(C::BLACK).with_shader(Shader::Radial {
        center: (x1 + w * 0.2, y1 + h * 0.2),
        radius: w.min(h) * 0.4,
        colors: vec![C::new(255, 255, 255, 90), C::TRANSPARENT],
        pos: None,
        tile: Tile::Clamp,
    });
    canvas.draw_rect_xywh(x1, y1, w, h, &spec);
}

fn stained_border(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(20, 300);
    let panel_w = p.f("panel_width").clamp(10.0, 120.0);
    let lead = p.f("lead_thickness").clamp(1.0, 12.0);
    let brightness = p.f("glass_brightness").clamp(30.0, 100.0);
    let glow = p.b("glow");
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let mut canvas = Canvas::new(nw, nh);
    let lead_c = C::rgb(14, 10, 8);
    let fill = Paint::fill(lead_c);
    let (nwf, nhf, b, sh) = (nw as f32, nh as f32, border as f32, src.h as f32);
    canvas.draw_rect_xywh(0.0, 0.0, nwf, b, &fill);
    canvas.draw_rect_xywh(0.0, nhf - b, nwf, b, &fill);
    canvas.draw_rect_xywh(0.0, b, b, sh, &fill);
    canvas.draw_rect_xywh(nwf - b, b, b, sh, &fill);
    glass_panels(
        &mut canvas,
        (0.0, 0.0, nwf, b),
        true,
        panel_w,
        lead,
        brightness,
        0x1A,
        glow,
    );
    glass_panels(
        &mut canvas,
        (0.0, nhf - b, nwf, b),
        true,
        panel_w,
        lead,
        brightness,
        0x2B,
        glow,
    );
    glass_panels(
        &mut canvas,
        (0.0, b, b, sh),
        false,
        panel_w,
        lead,
        brightness,
        0x3C,
        glow,
    );
    glass_panels(
        &mut canvas,
        (nwf - b, b, b, sh),
        false,
        panel_w,
        lead,
        brightness,
        0x4D,
        glow,
    );
    canvas.draw_bitmap(src, b, b, &Paint::default());
    if p.b("inner_line") {
        canvas.draw_rect_xywh(b, b, src.w as f32, sh, &Paint::stroke(lead_c, lead * 1.5));
    }
    Ok(canvas.into_bitmap())
}
