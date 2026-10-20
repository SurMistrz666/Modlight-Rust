//! Filters, part G: Heat map .. Lens blur.

use std::f32::consts::PI;

use modlight_core::icons as l;
use modlight_editor::effects::Category;
use tiny_skia::PathBuilder;

use crate::def::*;
use crate::dotnet::shared_next;
use crate::fx::filters_e::transparent_blur;
use crate::helpers::{
    apply_contrast, bilinear_sample, blurred_clamp, clamp_to_byte, clamp01, fractal_noise,
    from_hsl, hash01, lerp, luminance, luminance01, screen, smooth_step,
};
use crate::sk::{Bitmap, C, Canvas, Paint, Style, round};

const F: Category = Category::Filters;

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "heat_map",
            "Heat map",
            F,
            l::FLAME,
            "Maps image luminance to a fire-like color gradient.",
            heat_map,
        )
        .immediate(),
        Def::new(
            "hexagonal_pixelate",
            "Hexagonal pixelate",
            F,
            l::HEXAGON,
            "Pixelates the image using hexagonal tiles.",
            hex_pixelate,
        )
        .params(|| vec![int_slider("size", "Size", 4, 100, 16)]),
        Def::new(
            "hologram_scan",
            "Hologram scan",
            F,
            l::SCAN_FACE,
            "Applies a holographic scanline effect with glitch and glow.",
            hologram,
        )
        .params(|| {
            vec![
                float_slider("scanline_strength", "Scanline strength", 0.0, 100.0, 45.0),
                float_slider("glitch_amount", "Glitch amount", 0.0, 100.0, 25.0),
                int_slider("chroma_shift", "Chroma shift", 0, 12, 2),
                float_slider("glow_amount", "Glow amount", 0.0, 100.0, 30.0),
            ]
        }),
        Def::new(
            "holographic_foil_shimmer",
            "Holographic foil shimmer",
            F,
            l::RAINBOW,
            "Overlays a rainbow holographic foil shimmer effect.",
            foil,
        )
        .params(|| {
            vec![
                float_slider("intensity", "Intensity", 0.0, 100.0, 65.0),
                float_slider("scale", "Scale", 40.0, 300.0, 120.0),
                float_slider("shift", "Shift", 0.0, 360.0, 0.0),
                float_slider("specular", "Specular", 0.0, 100.0, 45.0),
                float_slider("grain", "Grain", 0.0, 100.0, 20.0),
            ]
        }),
        Def::new(
            "infrared_false_color",
            "Infrared false color",
            F,
            l::FLAME,
            "Simulates infrared false color photography.",
            infrared,
        )
        .params(|| {
            vec![
                float_slider("intensity", "Intensity", 0.0, 100.0, 68.0),
                float_slider("foliage_shift", "Foliage shift", 0.0, 100.0, 78.0),
                float_slider("sky_darkening", "Sky darkening", 0.0, 100.0, 44.0),
                float_slider("glow", "Glow", 0.0, 100.0, 20.0),
                float_slider("contrast", "Contrast", 50.0, 200.0, 118.0),
            ]
        }),
        Def::new(
            "ink_splatter_drips",
            "Ink splatter + drips",
            F,
            l::PAINTBRUSH,
            "Adds ink splatter blobs and vertical drip marks.",
            ink_splatter,
        )
        .params(|| {
            vec![
                float_slider("ink_amount", "Ink amount", 0.0, 100.0, 55.0),
                float_slider("drip_length", "Drip length", 0.0, 100.0, 45.0),
                float_slider("spread", "Spread", 0.0, 100.0, 35.0),
                float_slider("paper_fade", "Paper fade", 0.0, 100.0, 20.0),
            ]
        }),
        Def::new(
            "inner_shadow",
            "Inner shadow",
            F,
            l::SQUARE_DASHED,
            "Adds an inner shadow along the edges of opaque regions.",
            inner_shadow,
        )
        .params(|| {
            vec![
                float_slider("opacity", "Opacity", 0.0, 100.0, 70.0),
                int_slider("size", "Size", 0, 300, 20),
                color("color", "Color", C::BLACK),
                int_numeric("offset_x", "Offset X", -300, 300, 0),
                int_numeric("offset_y", "Offset Y", -300, 300, 0),
            ]
        }),
        Def::new(
            "jpeg_artifact",
            "JPEG artifact",
            F,
            l::FILE_IMAGE,
            "Simulates JPEG compression artifacts with blocking and ringing.",
            jpeg_artifact,
        )
        .params(|| {
            vec![
                float_slider("compression", "Compression", 0.0, 100.0, 62.0),
                int_slider("block_size", "Block size", 4, 32, 8),
                float_slider("chroma_bleed", "Chroma bleed", 0.0, 100.0, 45.0),
                float_slider("ringing", "Ringing", 0.0, 100.0, 28.0),
            ]
        }),
        Def::new(
            "lava_molten_border",
            "Lava molten border",
            F,
            l::FLAME,
            "Adds a volcanic border with dark cracked rock and glowing molten lava seams.",
            lava,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 12, 300, 56),
                color("rock_color", "Rock color", C::rgb(30, 25, 22)),
                color("lava_color", "Lava color", C::rgb(255, 80, 10)),
                float_slider("crack_density", "Crack density", 10.0, 100.0, 60.0),
                float_slider("glow_intensity", "Glow intensity", 10.0, 100.0, 70.0),
                checkbox("ember_sparks", "Ember sparks", true),
            ]
        }),
        Def::new(
            "lens_blur",
            "Lens blur (bokeh)",
            F,
            l::APERTURE,
            "Applies a disc-shaped lens blur with highlight boost.",
            lens_blur,
        )
        .params(|| {
            vec![
                int_slider("radius", "Radius", 1, 15, 8),
                float_slider(
                    "highlight_threshold",
                    "Highlight threshold",
                    0.0,
                    100.0,
                    70.0,
                ),
                float_slider("highlight_boost", "Highlight boost", 0.0, 200.0, 85.0),
            ]
        }),
    ]
}

fn heat_map(src: &Bitmap, _: &P) -> Result<Bitmap, String> {
    let px = src
        .px
        .iter()
        .map(|c| {
            let t = (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0;
            let (r, g, b) = if t < 0.2 {
                (0, 0, ((t / 0.2) * 255.0) as u8)
            } else if t < 0.4 {
                (0, (((t - 0.2) / 0.2) * 255.0) as u8, 255)
            } else if t < 0.6 {
                (0, 255, ((1.0 - (t - 0.4) / 0.2) * 255.0) as u8)
            } else if t < 0.8 {
                ((((t - 0.6) / 0.2) * 255.0) as u8, 255, 0)
            } else {
                let s = (t - 0.8) / 0.2;
                (255, ((1.0 - s) * 255.0) as u8, (s * 255.0) as u8)
            };
            C::new(r, g, b, c.a)
        })
        .collect();
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}

fn hex_pixelate(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let size = 4.max(p.i("size"));
    let (w, h) = (src.w, src.h);
    let mut canvas = Canvas::new(w, h);
    let s = size as f32;
    let hex_w = s * 2.0;
    let hex_h = s * 3f32.sqrt();
    let mut row = -1f32;
    while row * hex_h / 2.0 < h as f32 + hex_h {
        let yc = row * hex_h / 2.0;
        let xo = if row % 2.0 == 0.0 { 0.0 } else { s * 1.5 };
        let mut col = -1f32;
        while col * hex_w * 0.75 < w as f32 + hex_w {
            let xc = col * s * 3.0 + xo;
            let sx = (xc as i32).clamp(0, w - 1);
            let sy = (yc as i32).clamp(0, h - 1);
            let mut pb = PathBuilder::new();
            for i in 0..6 {
                let a = PI / 3.0 * i as f32 - PI / 6.0;
                let (x, y) = (xc + s * a.cos(), yc + s * a.sin());
                if i == 0 {
                    pb.move_to(x, y);
                } else {
                    pb.line_to(x, y);
                }
            }
            pb.close();
            if let Some(path) = pb.finish() {
                canvas.draw_path(&path, &Paint::fill(src.get(sx, sy)));
            }
            col += 1.0;
        }
        row += 1.0;
    }
    Ok(canvas.into_bitmap())
}

fn hologram(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let scan = p.f("scanline_strength").clamp(0.0, 100.0) / 100.0;
    let glitch = p.f("glitch_amount").clamp(0.0, 100.0) / 100.0;
    let chroma = p.i("chroma_shift").clamp(0, 12) as f32;
    let glow = p.f("glow_amount").clamp(0.0, 100.0) / 100.0;
    if scan <= 0.0 && glitch <= 0.0 && chroma <= 0.0 && glow <= 0.0 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let seed = 6006;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let band = y / 6;
        let bn = hash01(band, 13, seed);
        let bs = hash01(band, 31, seed) * 2.0 - 1.0;
        let max_shift = glitch * 18.0;
        let burst = if bn > 0.60 { (bn - 0.60) / 0.40 } else { 0.0 };
        let row_shift =
            burst * max_shift * bs + (y as f32 * 0.11 + seed as f32 * 0.001).sin() * glitch * 2.5;
        let line_sine = 0.5 + 0.5 * (y as f32 * PI).sin();
        let sf = 1.0 - scan * (0.30 + 0.70 * line_sine);
        for x in 0..w {
            let fx = x as f32 + row_shift;
            let sr = bilinear_sample(&src.px, w, h, fx + chroma, y as f32);
            let sg = bilinear_sample(&src.px, w, h, fx, y as f32);
            let sb = bilinear_sample(&src.px, w, h, fx - chroma, y as f32);
            let (r, g, b) = (
                sr.r as f32 / 255.0,
                sg.g as f32 / 255.0,
                sb.b as f32 / 255.0,
            );
            let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            let mut hr = lum * 0.18 + r * 0.20;
            let mut hg = lum * 0.85 + g * 0.35;
            let mut hb = lum * 1.25 + b * 0.45;
            let pulse =
                0.85 + 0.15 * (y as f32 * 0.45 + x as f32 * 0.03 + seed as f32 * 0.0003).sin();
            hr *= sf * pulse;
            hg *= sf * pulse;
            hb *= sf * pulse;
            let gb = glow * lum.powf(2.2);
            hr += gb * 0.08;
            hg += gb * 0.25;
            hb += gb * 0.45;
            if bn > 0.92 && hash01(x, y, seed ^ 99) > 0.97 {
                hr += 0.35;
                hg += 0.35;
                hb += 0.35;
            }
            px.push(C::new(
                clamp_to_byte(hr * 255.0),
                clamp_to_byte(hg * 255.0),
                clamp_to_byte(hb * 255.0),
                sg.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn foil(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let intensity = p.f("intensity").clamp(0.0, 100.0) / 100.0;
    let scale = p.f("scale").clamp(40.0, 300.0);
    let inv = 1.0 / scale;
    let phase = p.f("shift").clamp(0.0, 360.0) * (PI / 180.0);
    let spec = p.f("specular").clamp(0.0, 100.0) / 100.0;
    let grain = p.f("grain").clamp(0.0, 100.0) / 100.0;
    if intensity <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let seed = 2026;
    let hsv = |hh: f32, s: f32, v: f32| -> (f32, f32, f32) {
        if s <= 0.0001 {
            return (v, v, v);
        }
        let scaled = hh * 6.0;
        let sector = scaled.floor() as i32;
        let f = scaled - sector as f32;
        let pp = v * (1.0 - s);
        let q = v * (1.0 - s * f);
        let t = v * (1.0 - s * (1.0 - f));
        match sector {
            0 => (v, t, pp),
            1 => (q, v, pp),
            2 => (pp, v, t),
            3 => (pp, q, v),
            4 => (t, pp, v),
            _ => (v, pp, q),
        }
    };
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let (sr, sg, sb) = (s.r as f32 / 255.0, s.g as f32 / 255.0, s.b as f32 / 255.0);
            let lum = 0.2126 * sr + 0.7152 * sg + 0.0722 * sb;
            let u = x as f32 * inv;
            let v = y as f32 * inv;
            let wa = (u * 2.7 + v * 1.9 + phase).sin();
            let wb = (u * 3.6 - v * 2.2 - phase * 0.7).cos();
            let wc = ((u + v) * 4.1 + phase * 1.3).sin();
            let wave = wa * 0.45 + wb * 0.35 + wc * 0.2;
            let mut hue = wave * 0.5 + 0.5 + u * 0.08 + v * 0.04;
            hue -= hue.floor();
            let (hr, hg, hb) = hsv(hue, 0.65 + 0.25 * lum, 0.80 + 0.20 * lum);
            let scan = 0.5 + 0.5 * (((u + v * 0.35) * 35.0) + phase * 2.0).sin();
            let shimmer = intensity * (scan * 0.55 + 0.45);
            let mut band = (u * 8.0 - v * 5.0 + phase * 1.5).sin();
            band = band.max(0.0).powf(8.0) * spec;
            let mut or = hr * shimmer + band;
            let mut og = hg * shimmer + band * 0.92;
            let mut ob = hb * shimmer + band * 1.08;
            if grain > 0.001 {
                let sparkle = hash01(x, y, seed).powf(22.0) * grain * 1.8;
                or += sparkle;
                og += sparkle * 0.95;
                ob += sparkle * 1.1;
            }
            let blend = intensity * (0.42 + 0.58 * lum);
            let scr = |s: f32, o: f32| 1.0 - (1.0 - s) * (1.0 - o.min(1.0));
            px.push(C::new(
                clamp_to_byte(lerp(sr, scr(sr, or), blend) * 255.0),
                clamp_to_byte(lerp(sg, scr(sg, og), blend) * 255.0),
                clamp_to_byte(lerp(sb, scr(sb, ob), blend) * 255.0),
                clamp_to_byte(s.a as f32),
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn infrared(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let intensity = p.f("intensity").clamp(0.0, 100.0) / 100.0;
    let fs = p.f("foliage_shift").clamp(0.0, 100.0) / 100.0;
    let sky = p.f("sky_darkening").clamp(0.0, 100.0) / 100.0;
    let glow = p.f("glow").clamp(0.0, 100.0) / 100.0;
    let contrast = p.f("contrast").clamp(50.0, 200.0) / 100.0;
    if intensity <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let gb = if glow > 0.01 {
        Some(blurred_clamp(src, 6.0 + glow * 12.0))
    } else {
        None
    };
    let gp = gb.as_ref().map_or(&src.px, |b| &b.px);
    let px = src
        .px
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let (hh, ss, ll) = s.to_hsl();
            let foliage = (45.0..=170.0).contains(&hh) && ss >= 18.0;
            let is_sky = (170.0..=260.0).contains(&hh) && ss >= 12.0;
            let (nh, ns, nl) = if foliage {
                (
                    lerp(hh, 8.0 + ll * 0.16, fs * (0.55 + intensity * 0.35)),
                    (ss * (1.0 + 0.18 * intensity + 0.32 * fs)).min(100.0),
                    (ll + intensity * 14.0 + fs * 10.0).min(100.0),
                )
            } else if is_sky {
                (
                    lerp(hh, 196.0, 0.22 + intensity * 0.28),
                    (ss * (1.03 + 0.10 * intensity)).min(100.0),
                    (ll * (1.0 - sky * 0.46)).max(0.0),
                )
            } else {
                (
                    lerp(hh, hh + 6.0, intensity * 0.10),
                    (ss * (1.0 + intensity * 0.06)).min(100.0),
                    (ll + intensity * 3.0).clamp(0.0, 100.0),
                )
            };
            let m = from_hsl(nh, ns, nl, s.a);
            let mut r = apply_contrast(m.r as f32 / 255.0, contrast);
            let mut g = apply_contrast(m.g as f32 / 255.0, contrast);
            let mut b = apply_contrast(m.b as f32 / 255.0, contrast);
            if glow > 0.0001 {
                let gm = ((luminance01(gp[i]) - 0.52) / 0.48).max(0.0).powf(1.55) * glow;
                r = screen(r, gm * 0.80);
                g = screen(g, gm * 0.18);
                b = screen(b, gm * 0.42);
            }
            C::new(
                clamp_to_byte(r * 255.0),
                clamp_to_byte(g * 255.0),
                clamp_to_byte(b * 255.0),
                s.a,
            )
        })
        .collect();
    Ok(Bitmap::from_pixels(w, h, px))
}

fn ink_splatter(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let ink = p.f("ink_amount").clamp(0.0, 100.0) / 100.0;
    let drip_len = p.f("drip_length").clamp(0.0, 100.0) / 100.0;
    let spread = p.f("spread").clamp(0.0, 100.0) / 100.0;
    let fade = p.f("paper_fade").clamp(0.0, 100.0) / 100.0;
    if ink <= 0.0001 && fade <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let seed = shared_next(1, i32::MAX);
    let sc = 14.0 + (1.0 - spread) * 16.0;
    let sr = 3.6 + spread * 11.5;
    let sd = 0.04 + ink * 0.25;
    let dc = 12.0 + (1.0 - spread) * 24.0;
    let dp = 0.12 + ink * 0.42;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let (mut r, mut g, mut b, a) = (s.r as f32, s.g as f32, s.b as f32, s.a as f32);
            if fade > 0.0001 {
                let gray = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                let pn = (hash01(x, y, seed ^ 131) * 2.0 - 1.0) * fade * 8.0;
                let target = gray + 9.0 * fade + pn;
                let mix = fade * 0.48;
                r = lerp(r, target, mix);
                g = lerp(g, target, mix);
                b = lerp(b, target + 4.0, mix);
            }
            // Splat mask.
            let (cxi, cyi) = (
                (x as f32 / sc).floor() as i32,
                (y as f32 / sc).floor() as i32,
            );
            let mut splat = 0f32;
            for oy in -1..=1 {
                for ox in -1..=1 {
                    let (nx, ny) = (cxi + ox, cyi + oy);
                    if hash01(nx, ny, seed ^ 991) > sd {
                        continue;
                    }
                    let cx = (nx as f32 + hash01(nx, ny, seed ^ 991 ^ 177)) * sc;
                    let cy = (ny as f32 + hash01(nx, ny, seed ^ 991 ^ 571)) * sc;
                    let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                    let lr = sr * (0.42 + hash01(nx, ny, seed ^ 991 ^ 887) * 0.95);
                    let core = 1.0 - smooth_step(lr * 0.28, lr, d);
                    if core > splat {
                        splat = core;
                    }
                }
            }
            // Drip mask.
            let dseed = seed ^ 3701;
            let column = (x as f32 / dc).floor() as i32;
            let mut drips = 0f32;
            for c in column - 1..=column + 1 {
                if hash01(c, 0, dseed) > dp {
                    continue;
                }
                let cx = (c as f32 + hash01(c, 1, dseed ^ 211)) * dc;
                let start = hash01(c, 2, dseed ^ 419) * h as f32 * 0.75;
                let len =
                    (0.08 + drip_len * 0.85) * (0.4 + hash01(c, 3, dseed ^ 677) * 0.9) * h as f32;
                let th = (0.9 + spread * 3.2) * (0.65 + hash01(c, 4, dseed ^ 991) * 0.95);
                let dy = y as f32 - start;
                if dy < 0.0 || dy > len {
                    continue;
                }
                let dx = (x as f32 - cx).abs();
                let line = 1.0 - smooth_step(th * 0.45, th, dx);
                if line <= 0.0 {
                    continue;
                }
                let tail = 1.0 - smooth_step(0.0, len, dy);
                let mut drip = line * (0.55 + 0.45 * tail);
                let tip_y = start + len;
                let td = ((x as f32 - cx).powi(2) + (y as f32 - tip_y).powi(2) * 0.55).sqrt();
                let tip = 1.0 - smooth_step(th * 0.2, th * 1.6, td);
                drip = drip.max(tip * 0.9);
                if drip > drips {
                    drips = drip;
                }
            }
            let speckle = hash01(x, y, seed ^ 5239).powf(26.0) * ink * 0.85;
            let mask = (splat * 0.9 + drips * 0.85 + speckle).clamp(0.0, 1.0) * ink;
            if mask > 0.0001 {
                let bleed = (splat - 0.25).max(0.0) * 0.3;
                r = lerp(r, 11.0 + bleed * 10.0, mask);
                g = lerp(g, 11.0 + bleed * 8.0, mask);
                b = lerp(b, 15.0 + bleed * 12.0, mask);
            }
            px.push(C::new(
                clamp_to_byte(r),
                clamp_to_byte(g),
                clamp_to_byte(b),
                clamp_to_byte(a),
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn inner_shadow(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let opacity = p.f("opacity").clamp(0.0, 100.0) / 100.0;
    let size = p.i("size").clamp(0, 300);
    let color = p.c("color");
    let (ox, oy) = (p.i("offset_x"), p.i("offset_y"));
    if opacity <= 0.0 || (size <= 0 && ox == 0 && oy == 0) || color.a == 0 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let mask: Vec<C> = src.px.iter().map(|c| C::new(c.a, c.a, c.a, c.a)).collect();
    let mask = Bitmap::from_pixels(w, h, mask);
    let mut shifted = Canvas::new(w, h);
    shifted.draw_bitmap(&mask, -ox as f32, -oy as f32, &Paint::default());
    let shifted = shifted.into_bitmap();
    let blurred = if size > 0 {
        transparent_blur(&shifted, size as f32 / 2.0)
    } else {
        shifted
    };
    let ca = color.a as f32 / 255.0;
    let px = src
        .px
        .iter()
        .zip(&blurred.px)
        .map(|(s, b)| {
            let sa = s.a as f32 / 255.0;
            if sa <= 0.0 {
                return *s;
            }
            let inner = clamp01(sa - b.a as f32 / 255.0);
            let mix = inner * opacity * ca;
            C::new(
                clamp_to_byte(lerp(s.r as f32, color.r as f32, mix)),
                clamp_to_byte(lerp(s.g as f32, color.g as f32, mix)),
                clamp_to_byte(lerp(s.b as f32, color.b as f32, mix)),
                s.a,
            )
        })
        .collect();
    Ok(Bitmap::from_pixels(w, h, px))
}

fn ycbcr(c: C) -> (f32, f32, f32) {
    let (r, g, b) = (c.r as f32, c.g as f32, c.b as f32);
    (
        0.299 * r + 0.587 * g + 0.114 * b,
        128.0 - 0.168736 * r - 0.331264 * g + 0.5 * b,
        128.0 + 0.5 * r - 0.418688 * g - 0.081312 * b,
    )
}

fn jpeg_artifact(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let amount = p.f("compression").clamp(0.0, 100.0) / 100.0;
    let bs = p.i("block_size").clamp(4, 32);
    let bleed = p.f("chroma_bleed").clamp(0.0, 100.0) / 100.0;
    let ringing = p.f("ringing").clamp(0.0, 100.0) / 100.0;
    if amount <= 0.0001 && ringing <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let mut comp = vec![C::ZERO; src.px.len()];
    let ls = 1.0 + amount * 22.0;
    let cs = 2.0 + amount * 38.0;
    let q = |v: f32, s: f32| round(v / s) * s;
    let mut by = 0;
    while by < h {
        let y1 = h.min(by + bs);
        let mut bx = 0;
        while bx < w {
            let x1 = w.min(bx + bs);
            let (mut ay, mut acb, mut acr, mut n) = (0f32, 0f32, 0f32, 0);
            for y in by..y1 {
                for x in bx..x1 {
                    let (yy, cb, cr) = ycbcr(src.get(x, y));
                    ay += yy;
                    acb += cb;
                    acr += cr;
                    n += 1;
                }
            }
            if n > 0 {
                let inv = 1.0 / n as f32;
                ay *= inv;
                acb *= inv;
                acr *= inv;
            }
            for y in by..y1 {
                for x in bx..x1 {
                    let s = src.get(x, y);
                    let (yy, cb, cr) = ycbcr(s);
                    let qy = q(lerp(yy, ay, amount * 0.16), ls);
                    let k = 0.15 + bleed * 0.70 + amount * 0.08;
                    let qcb = q(lerp(cb, acb, k), cs);
                    let qcr = q(lerp(cr, acr, k), cs);
                    let r = qy + 1.402 * (qcr - 128.0);
                    let g = qy - 0.344136 * (qcb - 128.0) - 0.714136 * (qcr - 128.0);
                    let b = qy + 1.772 * (qcb - 128.0);
                    comp[(y * w + x) as usize] =
                        C::new(clamp_to_byte(r), clamp_to_byte(g), clamp_to_byte(b), s.a);
                }
            }
            bx += bs;
        }
        by += bs;
    }
    if ringing <= 0.0001 {
        return Ok(Bitmap::from_pixels(w, h, comp));
    }
    let mut px = Vec::with_capacity(comp.len());
    for y in 0..h {
        for x in 0..w {
            let cur = comp[(y * w + x) as usize];
            let (lx, ly) = (x % bs, y % bs);
            let fx = lx <= 1 || lx >= bs - 2;
            let fy = ly <= 1 || ly >= bs - 2;
            if !fx && !fy {
                px.push(cur);
                continue;
            }
            let get = |xx: i32, yy: i32| luminance(comp[(yy * w + xx) as usize]);
            let edge = ((get((x + 1).min(w - 1), y) - get((x - 1).max(0), y))
                + (get(x, (y + 1).min(h - 1)) - get(x, (y - 1).max(0))))
                * 0.25;
            let osc = ((x + y) as f32 * 0.82 + ((x / bs) + (y / bs)) as f32 * 1.7).sin();
            let sharpen = edge * osc * ringing * 1.0 * 0.32;
            px.push(C::new(
                clamp_to_byte(cur.r as f32 + sharpen * 1.05),
                clamp_to_byte(cur.g as f32 + sharpen * 0.86),
                clamp_to_byte(cur.b as f32 - sharpen * 0.30),
                cur.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn lava(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(12, 300);
    let density = p.f("crack_density").clamp(10.0, 100.0) / 100.0;
    let glow = p.f("glow_intensity").clamp(10.0, 100.0) / 100.0;
    let (rock, lavac) = (p.c("rock_color"), p.c("lava_color"));
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let seed = 8371;
    let ns = 0.04 + density * 0.06;
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
            let (xf, yf) = (x as f32, y as f32);
            let n1 = fractal_noise(xf * ns, yf * ns, 4, 2.0, 0.5, seed);
            let n2 = fractal_noise(
                xf * ns * 1.7 + 100.0,
                yf * ns * 1.7 + 200.0,
                3,
                2.0,
                0.5,
                seed ^ 0x55,
            );
            let cw = 0.08 + density * 0.12;
            let c1 = 1.0 - smooth_step(0.0, cw, n1.abs());
            let c2 = 1.0 - smooth_step(0.0, cw, n2.abs());
            let cm = c1.max(c2);
            let lb = cm * glow;
            let rn = hash01(x, y, seed ^ 0x77) * 0.15 - 0.075;
            let mut rr = (rock.r as f32 / 255.0 + rn) * (1.0 - lb) + (lavac.r as f32 / 255.0) * lb;
            let mut rg = (rock.g as f32 / 255.0 + rn) * (1.0 - lb) + (lavac.g as f32 / 255.0) * lb;
            let mut rb = (rock.b as f32 / 255.0 + rn) * (1.0 - lb) + (lavac.b as f32 / 255.0) * lb;
            if cm > 0.7 {
                let boost = (cm - 0.7) / 0.3 * glow;
                rr += boost * 0.6;
                rg += boost * 0.3;
                rb += boost * 0.05;
            }
            px[(y * nw + x) as usize] = C::new(
                clamp_to_byte(rr * 255.0),
                clamp_to_byte(rg * 255.0),
                clamp_to_byte(rb * 255.0),
                255,
            );
        }
    }
    let base = Bitmap::from_pixels(nw, nh, px);
    let mut canvas = Canvas::from_bitmap(&base);
    if p.b("ember_sparks") {
        let count = (nw + nh) / 8;
        for i in 0..count {
            let sx = hash01(i, 0, seed ^ 0xAB) * nw as f32;
            let sy = hash01(0, i, seed ^ 0xCD) * nh as f32;
            let in_border = sx < border as f32
                || sx >= (border + src.w) as f32
                || sy < border as f32
                || sy >= (border + src.h) as f32;
            if !in_border {
                continue;
            }
            let ln = fractal_noise(sx * ns, sy * ns, 4, 2.0, 0.5, seed).abs();
            if ln > 0.15 {
                continue;
            }
            let br = hash01(i, i, seed ^ 0xEF);
            let color = C::new(
                clamp_to_byte(255.0 * (0.8 + br * 0.2)),
                clamp_to_byte(180.0 * br),
                clamp_to_byte(40.0 * br),
                clamp_to_byte(150.0 + br * 100.0),
            );
            let paint = Paint {
                mask_blur: Some(2.0),
                ..Paint::fill_aa(color)
            };
            canvas.draw_circle(sx, sy, 1.0 + br * 2.0, &paint);
        }
    }
    let gp = Paint {
        style: Style::Stroke,
        stroke_width: 4.0,
        mask_blur: Some(6.0),
        ..Paint::fill_aa(lavac.with_alpha((60.0 * glow) as u8))
    };
    let b = border as f32;
    canvas.draw_rect_xywh(b, b, src.w as f32, src.h as f32, &gp);
    canvas.draw_bitmap(src, b, b, &Paint::default());
    Ok(canvas.into_bitmap())
}

fn lens_blur(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let radius = p.i("radius").clamp(1, 15);
    let threshold = p.f("highlight_threshold").clamp(0.0, 100.0) / 100.0;
    let boost = p.f("highlight_boost").clamp(0.0, 200.0) / 100.0;
    let use_boost = boost > 0.001;
    let tinv = 1.0 / 0.0001f32.max(1.0 - threshold);
    if radius <= 1 && boost <= 0.01 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let mut kernel = Vec::new();
    let r2 = radius * radius;
    let ir = 1.0 / radius as f32;
    for y in -radius..=radius {
        for x in -radius..=radius {
            let d2 = x * x + y * y;
            if d2 > r2 {
                continue;
            }
            let d = (d2 as f32).sqrt() * ir;
            kernel.push((x, y, 1.0 - d * 0.35));
        }
    }
    let lum: Vec<f32> = src
        .px
        .iter()
        .map(|c| (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0)
        .collect();
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let (mut sr, mut sg, mut sb, mut sa, mut sw) = (0f32, 0f32, 0f32, 0f32, 0f32);
            for &(kx, ky, kw) in &kernel {
                let sx = (x + kx).clamp(0, w - 1);
                let sy = (y + ky).clamp(0, h - 1);
                let i = (sy * w + sx) as usize;
                let c = src.px[i];
                let mut hw = 1.0;
                if use_boost && lum[i] > threshold {
                    hw += (lum[i] - threshold) * tinv * boost;
                }
                let wt = kw * hw;
                sr += c.r as f32 * wt;
                sg += c.g as f32 * wt;
                sb += c.b as f32 * wt;
                sa += c.a as f32 * wt;
                sw += wt;
            }
            if sw <= 0.0001 {
                px.push(src.get(x, y));
                continue;
            }
            let inv = 1.0 / sw;
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
