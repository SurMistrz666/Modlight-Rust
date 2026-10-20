//! Filters, part I: Mosaic polygon .. Oil slick interference.

use std::f32::consts::PI;

use modlight_core::icons as l;
use modlight_editor::effects::Category;
use tiny_skia::PathBuilder;

use crate::def::*;
use crate::helpers::{
    bilinear_sample, blurred_clamp, clamp_to_byte, clamp01, hash01, hsv_to_rgb, lerp, screen,
    smooth_step,
};
use crate::sk::{Bitmap, C, Canvas, Join, Paint, round};

const F: Category = Category::Filters;

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "mosaic_polygon",
            "Mosaic polygon",
            F,
            l::HEXAGON,
            "Creates a mosaic using hexagonal or triangular polygon tiles.",
            mosaic_polygon,
        )
        .params(|| {
            vec![
                int_slider("cell_size", "Cell size", 8, 80, 24),
                choice(
                    "shape",
                    "Shape",
                    &[("Hexagon", "Hexagon"), ("Triangle", "Triangle")],
                    "Hexagon",
                ),
                float_slider("border_width", "Border width", 0.0, 6.0, 1.0),
                float_slider("border_opacity", "Border opacity", 0.0, 100.0, 45.0),
                float_slider("randomness", "Randomness", 0.0, 100.0, 18.0),
            ]
        }),
        Def::new(
            "mosaic_tile_border",
            "Mosaic tile border",
            F,
            l::GRID_3X3,
            "Adds a colorful mosaic tile border with individual square tiles, grout lines, and subtle per-tile variation.",
            mosaic_tile_border,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 12, 300, 48),
                float_slider("tile_size", "Tile size", 4.0, 60.0, 14.0),
                color("grout_color", "Grout color", C::rgb(210, 205, 195)),
                float_slider("grout_width", "Grout width", 0.5, 8.0, 2.0),
                float_slider("color_variety", "Color variety", 0.0, 100.0, 70.0),
                float_slider("saturation", "Saturation", 0.0, 100.0, 85.0),
            ]
        }),
        Def::new(
            "motion_blur",
            "Motion blur",
            F,
            l::MOVE_HORIZONTAL,
            "Applies a directional motion blur at a specified angle.",
            motion_blur,
        )
        .params(|| {
            vec![
                int_slider("distance", "Distance", 1, 200, 12),
                float_slider("angle", "Angle", 0.0, 360.0, 0.0),
            ]
        }),
        Def::new(
            "nebula_starfield",
            "Nebula starfield",
            F,
            l::SPARKLES,
            "Generates a procedural nebula and starfield overlay.",
            nebula,
        )
        .params(|| {
            vec![
                float_slider("intensity", "Intensity", 0.0, 100.0, 70.0),
                float_slider("scale", "Scale", 0.0, 100.0, 80.0),
                float_slider("hue_shift", "Hue shift", -180.0, 180.0, -15.0),
                float_slider("star_density", "Star density", 0.0, 100.0, 55.0),
                float_slider("star_size", "Star size", 0.0, 200.0, 10.0),
                float_slider("twinkle", "Twinkle", 0.0, 100.0, 40.0),
                float_slider("vignette_strength", "Vignette strength", 0.0, 100.0, 18.0),
            ]
        }),
        Def::new(
            "neon_edge_glow",
            "Neon edge glow",
            F,
            l::ZAP,
            "Detects edges and applies a neon glow effect.",
            neon_edge_glow,
        )
        .params(|| {
            vec![
                float_slider("edge_strength", "Edge strength", 0.1, 6.0, 2.2),
                int_slider("threshold", "Threshold", 0, 255, 36),
                float_slider("glow_radius", "Glow radius", 0.0, 40.0, 8.0),
                float_slider("glow_intensity", "Glow intensity", 0.0, 250.0, 120.0),
                float_slider("base_dim", "Base dim", 0.0, 100.0, 30.0),
                color("neon_color", "Neon color", C::new(0, 240, 255, 255)),
            ]
        }),
        Def::new(
            "neon_glow_border",
            "Neon glow border",
            F,
            l::ZAP,
            "Adds a vibrant neon light tube border with a diffused outer glow on a dark background.",
            neon_glow_border,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 10, 200, 50),
                color("neon_color", "Neon color", C::rgb(0, 255, 200)),
                float_slider("glow_intensity", "Glow intensity", 10.0, 100.0, 75.0),
                float_slider("tube_width", "Tube width", 1.0, 20.0, 4.0),
                color("bg_color", "Background color", C::rgb(10, 10, 18)),
                checkbox("double_tube", "Double tube", false),
            ]
        }),
        Def::new(
            "night_vision",
            "Night vision",
            F,
            l::SCAN_EYE,
            "Simulates a night vision green phosphor display effect.",
            night_vision,
        )
        .params(|| {
            vec![
                float_slider("intensity", "Intensity", 0.0, 100.0, 78.0),
                float_slider("glow", "Glow", 0.0, 100.0, 42.0),
                float_slider("noise", "Noise", 0.0, 100.0, 18.0),
                float_slider("vignette", "Vignette", 0.0, 100.0, 45.0),
                float_slider("scanlines", "Scanlines", 0.0, 100.0, 35.0),
            ]
        }),
        Def::new(
            "oil_paint",
            "Oil paint",
            F,
            l::PAINTBRUSH_VERTICAL,
            "Simulates an oil painting by quantizing color intensity levels.",
            oil_paint,
        )
        .params(|| {
            vec![
                int_slider("radius", "Radius", 1, 6, 3),
                int_slider("levels", "Levels", 8, 64, 24),
            ]
        }),
        Def::new(
            "oil_slick_interference",
            "Oil slick interference",
            F,
            l::RAINBOW,
            "Simulates iridescent oil slick interference patterns.",
            oil_slick,
        )
        .params(|| {
            vec![
                float_slider("intensity", "Intensity", 0.0, 100.0, 58.0),
                float_slider("scale", "Scale", 0.0, 100.0, 70.0),
                float_slider("darkness", "Darkness", 0.0, 100.0, 45.0),
                float_slider("gloss", "Gloss", 0.0, 100.0, 60.0),
                float_slider("shift", "Shift", 0.0, 360.0, 0.0),
            ]
        }),
    ]
}

fn mosaic_polygon(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let cell = p.i("cell_size").clamp(8, 80);
    let bw = p.f("border_width").clamp(0.0, 6.0);
    let bo = p.f("border_opacity").clamp(0.0, 100.0) / 100.0;
    let randomness = p.f("randomness").clamp(0.0, 100.0) / 100.0;
    let seed = 9031;
    let sp = &src.px;
    let mut canvas = Canvas::new(w, h);
    let stroke =
        Paint::stroke_aa(C::new(0, 0, 0, clamp_to_byte(255.0 * bo)), bw).with_join(Join::Round);
    let draw_stroke = bw > 0.0 && stroke.color.a > 0;
    if p.e("shape") != "Triangle" {
        let radius = cell as f32 * 0.5;
        let vertical = 1.732_050_8 * radius;
        let horizontal = 1.5 * radius;
        let jitter = randomness * radius * 0.45;
        let mut col = -1;
        loop {
            let center_x = radius + col as f32 * horizontal;
            if center_x > w as f32 + radius {
                break;
            }
            let start_y = radius + if col & 1 != 0 { vertical * 0.5 } else { 0.0 };
            let mut center_y = start_y - vertical;
            while center_y < h as f32 + radius {
                let gx = col + 4096;
                let gy = ((center_y + vertical) / vertical.max(0.0001)).floor() as i32;
                let ox = (hash01(gx, gy, seed ^ 17) * 2.0 - 1.0) * jitter;
                let oy = (hash01(gx, gy, seed ^ 73) * 2.0 - 1.0) * jitter;
                let (cx, cy) = (center_x + ox, center_y + oy);
                let color = bilinear_sample(sp, w, h, cx, cy);
                let mut pb = PathBuilder::new();
                for i in 0..6 {
                    let a = (PI / 3.0) * i as f32 - PI / 6.0;
                    let (x, y) = (cx + a.cos() * radius, cy + a.sin() * radius);
                    if i == 0 {
                        pb.move_to(x, y);
                    } else {
                        pb.line_to(x, y);
                    }
                }
                pb.close();
                if let Some(path) = pb.finish() {
                    canvas.draw_path(&path, &Paint::fill_aa(color));
                    if draw_stroke {
                        canvas.draw_path(&path, &stroke);
                    }
                }
                center_y += vertical;
            }
            col += 1;
        }
    } else {
        let side = cell as f32;
        let th = side * 0.866_025_4;
        let half = side * 0.5;
        let jitter = randomness * side * 0.30;
        let rows = (h as f32 / th).ceil() as i32 + 2;
        let cols = (w as f32 / half).ceil() as i32 + 2;
        for row in -1..rows {
            let by = row as f32 * th;
            for col in -1..cols {
                let bx = col as f32 * half;
                let up = (row + col) & 1 == 0;
                let pts = if up {
                    [(bx, by + th), (bx + half, by), (bx + side, by + th)]
                } else {
                    [(bx, by), (bx + side, by), (bx + half, by + th)]
                };
                let mut cx = (pts[0].0 + pts[1].0 + pts[2].0) / 3.0;
                let mut cy = (pts[0].1 + pts[1].1 + pts[2].1) / 3.0;
                cx += (hash01(col, row, seed ^ 173) * 2.0 - 1.0) * jitter;
                cy += (hash01(col, row, seed ^ 719) * 2.0 - 1.0) * jitter;
                let color = bilinear_sample(sp, w, h, cx, cy);
                let mut pb = PathBuilder::new();
                pb.move_to(pts[0].0, pts[0].1);
                pb.line_to(pts[1].0, pts[1].1);
                pb.line_to(pts[2].0, pts[2].1);
                pb.close();
                if let Some(path) = pb.finish() {
                    canvas.draw_path(&path, &Paint::fill_aa(color));
                    if draw_stroke {
                        canvas.draw_path(&path, &stroke);
                    }
                }
            }
        }
    }
    Ok(canvas.into_bitmap())
}

fn mosaic_tile_border(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(12, 300);
    let ts = p.f("tile_size").clamp(4.0, 60.0);
    let gw = p.f("grout_width").clamp(0.5, 8.0);
    let variety = p.f("color_variety").clamp(0.0, 100.0) / 100.0;
    let sat = p.f("saturation").clamp(0.0, 100.0);
    let grout = p.c("grout_color");
    let seed = 5923;
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let mut px = vec![C::ZERO; (nw * nh) as usize];
    let gh = gw * 0.5;
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
            let tc = (xf / ts).floor() as i32;
            let tr = (yf / ts).floor() as i32;
            let mut wx = xf % ts;
            let mut wy = yf % ts;
            if wx < 0.0 {
                wx += ts;
            }
            if wy < 0.0 {
                wy += ts;
            }
            let i = (y * nw + x) as usize;
            if wx < gh || wx > ts - gh || wy < gh || wy > ts - gh {
                px[i] = grout;
                continue;
            }
            let hue = hash01(tc, tr, seed) * 360.0 * variety;
            let bright = 55.0 + hash01(tc ^ 0x55, tr ^ 0xAA, seed ^ 0x17) * 40.0;
            let tile = C::from_hsv(hue, sat, bright, 255);
            let d = (wx - gh).min(ts - gh - wx).min((wy - gh).min(ts - gh - wy));
            let bevel = smooth_step(ts * 0.15, 0.0, d) * 0.2;
            let noise = (hash01(x, y, seed ^ 0x99) - 0.5) * 0.06;
            px[i] = C::new(
                clamp_to_byte((tile.r as f32 / 255.0 - bevel + noise) * 255.0),
                clamp_to_byte((tile.g as f32 / 255.0 - bevel + noise) * 255.0),
                clamp_to_byte((tile.b as f32 / 255.0 - bevel + noise) * 255.0),
                255,
            );
        }
    }
    let mut canvas = Canvas::from_bitmap(&Bitmap::from_pixels(nw, nh, px));
    let b = border as f32;
    canvas.draw_bitmap(src, b, b, &Paint::default());
    canvas.draw_rect_xywh(
        b - gw * 0.5,
        b - gw * 0.5,
        src.w as f32 + gw,
        src.h as f32 + gw,
        &Paint::stroke(grout, gw),
    );
    Ok(canvas.into_bitmap())
}

fn motion_blur(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let distance = p.i("distance").clamp(1, 200);
    if distance <= 1 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let rad = p.f("angle") * (PI / 180.0);
    let (dx, dy) = (rad.cos(), rad.sin());
    let start = -((distance - 1) / 2);
    let end = distance / 2;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let (mut r, mut g, mut b, mut a, mut n) = (0i32, 0i32, 0i32, 0i32, 0i32);
            for i in start..=end {
                let sx = (round(x as f32 + i as f32 * dx) as i32).clamp(0, w - 1);
                let sy = (round(y as f32 + i as f32 * dy) as i32).clamp(0, h - 1);
                let c = src.get(sx, sy);
                r += c.r as i32;
                g += c.g as i32;
                b += c.b as i32;
                a += c.a as i32;
                n += 1;
            }
            px.push(C::new(
                (r / n) as u8,
                (g / n) as u8,
                (b / n) as u8,
                (a / n) as u8,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn nebula(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let intensity = p.f("intensity").clamp(0.0, 100.0) / 100.0;
    if intensity <= 0.0 {
        return Ok(src.clone());
    }
    let scale = p.f("scale").clamp(0.0, 100.0) / 100.0;
    let hue_shift = p.f("hue_shift").clamp(-180.0, 180.0);
    let density = p.f("star_density").clamp(0.0, 100.0) / 100.0;
    let size = p.f("star_size").clamp(0.0, 200.0) / 200.0;
    let twinkle = p.f("twinkle").clamp(0.0, 100.0) / 100.0;
    let vignette = p.f("vignette_strength").clamp(0.0, 100.0) / 100.0;
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let seed = 1337;
    let cx = hash01(seed, 1, seed ^ 31) * (w - 1) as f32;
    let cy = hash01(seed, 2, seed ^ 47) * (h - 1) as f32;
    let max_dist = (((w - 1) as f32 * (w - 1) as f32) + ((h - 1) as f32 * (h - 1) as f32))
        .sqrt()
        .max(1.0);
    let mut base_hue = (hue_shift + 180.0) * (1.0 / 360.0);
    base_hue -= base_hue.floor();
    let (nr, ng, nb) = hsv_to_rgb(base_hue, 0.85, 1.0);
    let cell = (round(10.0 - size * 6.0) as i32).clamp(3, 16);
    let threshold = 1.0 - density;
    let rbase = 0.18 + 0.55 * size;
    let rfeather = 0.05 + 0.10 * size;
    let freq = 0.25 + scale * 2.25;
    let phase = (seed % 1000) as f32 * 0.013;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let (xf, yf) = (x as f32, y as f32);
            let nx = (xf - cx) / w as f32;
            let ny = (yf - cy) / h as f32;
            let dist = (nx * nx + ny * ny).sqrt();
            let u = xf / 1f32.max((w - 1) as f32);
            let v = yf / 1f32.max((h - 1) as f32);
            let wa = (u * freq * 8.0 + v * freq * 5.0 + phase).sin();
            let wb = (u * freq * 13.0 - v * freq * 9.0 + phase * 1.37).sin();
            let wc = ((u + v) * freq * 7.0 + phase * 0.77).sin();
            let mut neb = wa * 0.45 + wb * 0.35 + wc * 0.20;
            neb = neb * 0.5 + 0.5;
            let falloff = (-(dist * dist) * (2.2 + scale * 3.2)).exp();
            let neb_strength = intensity * (0.10 + 0.90 * neb) * falloff;
            let (cxi, cyi) = (x / cell, y / cell);
            let mut star = 0f32;
            if hash01(cxi, cyi, seed ^ 0x3A17) > threshold {
                let ox = hash01(cxi, cyi, seed ^ 0xBEEF);
                let oy = hash01(cxi, cyi, seed ^ 0xCAFE);
                let scx = (cxi * cell) as f32 + ox * cell as f32;
                let scy = (cyi * cell) as f32 + oy * cell as f32;
                let dx = xf - scx;
                let dy = yf - scy;
                let mut d01 = (dx * dx + dy * dy).sqrt() / (cell as f32 * 0.5);
                let ts = hash01(cxi, cyi, seed ^ 0x51A7);
                let tw = 0.5
                    + 0.5 * (ts * PI * 10.0 + phase + cxi as f32 * 0.91 + cyi as f32 * 1.37).sin();
                let trad = lerp(1.0, 0.82 + tw * 0.70, twinkle);
                let tint = lerp(1.0, 0.35 + tw * 1.65, twinkle);
                d01 /= 0.35f32.max(trad);
                let r = rbase;
                let fw = rfeather * lerp(1.0, 1.35, twinkle);
                star = 1.0 - smooth_step(r, r + fw, d01);
                if twinkle > 0.001 {
                    let ss = 0.08 + size * 0.20;
                    let ax = dx.abs() / 0.0001f32.max(cell as f32 * ss);
                    let ay = dy.abs() / 0.0001f32.max(cell as f32 * ss);
                    let sx = 1.0 - smooth_step(0.2, 1.45, ax);
                    let sy = 1.0 - smooth_step(0.2, 1.45, ay);
                    let halo = 1.0 - smooth_step(r * 0.7, r + (1.25 + size), d01);
                    let glint = sx.max(sy) * halo;
                    star = star * tint + glint * twinkle * (tint - 1.0).max(0.0) * 0.45;
                }
            }
            let star_strength = intensity * star * (0.25 + 0.75 * size);
            let rc = ((xf - cx) * (xf - cx) + (yf - cy) * (yf - cy)).sqrt() / max_dist;
            let vign = clamp01(1.0 - vignette * rc * rc);
            let add = (neb_strength + star_strength).min(1.0);
            let rem = 1.0 - add;
            let (sr, sg, sb) = (s.r as f32 / 255.0, s.g as f32 / 255.0, s.b as f32 / 255.0);
            let or = (sr * rem + nr * neb_strength + nr * star_strength) * vign;
            let og = (sg * rem + ng * neb_strength + ng * star_strength) * vign;
            let ob = (sb * rem + nb * neb_strength + nb * star_strength) * vign;
            px.push(C::new(
                clamp_to_byte(or * 255.0),
                clamp_to_byte(og * 255.0),
                clamp_to_byte(ob * 255.0),
                s.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn neon_edge_glow(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let strength = p.f("edge_strength").clamp(0.1, 6.0);
    let threshold = p.i("threshold").clamp(0, 255);
    let radius = p.f("glow_radius").clamp(0.0, 40.0);
    let gi = p.f("glow_intensity").clamp(0.0, 250.0) / 100.0;
    let dim = p.f("base_dim").clamp(0.0, 100.0) / 100.0;
    let neon = p.c("neon_color");
    let (w, h) = (src.w, src.h);
    let naf = neon.a as f32 / 255.0;
    let luma: Vec<i32> = src
        .px
        .iter()
        .map(|c| ((c.r as i32 * 77 + c.g as i32 * 150 + c.b as i32 * 29) >> 8) & 0xFF)
        .collect();
    let mut core = Vec::with_capacity(src.px.len());
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
            let ramp = if mag <= threshold as f32 {
                0.0
            } else {
                let t = clamp01((mag - threshold as f32) / 1f32.max(255.0 - threshold as f32));
                t * t * (3.0 - 2.0 * t)
            };
            let mask = ramp * naf;
            let s = src.get(x, y);
            core.push(C::new(
                clamp_to_byte(neon.r as f32 * mask),
                clamp_to_byte(neon.g as f32 * mask),
                clamp_to_byte(neon.b as f32 * mask),
                clamp_to_byte(s.a as f32 * mask),
            ));
        }
    }
    // The glow blurs the stored (premultiplied) core; the composite reads the raw array.
    let glow = blurred_clamp(&Bitmap::from_pixels(w, h, core.clone()), radius);
    let base = 1.0 - dim;
    let cg = 0.85 + 0.35 * gi.min(1.6);
    let px = src
        .px
        .iter()
        .zip(core.iter().zip(&glow.px))
        .map(|(s, (c, g))| {
            C::new(
                clamp_to_byte(s.r as f32 * base + c.r as f32 * cg + g.r as f32 * gi),
                clamp_to_byte(s.g as f32 * base + c.g as f32 * cg + g.g as f32 * gi),
                clamp_to_byte(s.b as f32 * base + c.b as f32 * cg + g.b as f32 * gi),
                s.a,
            )
        })
        .collect();
    Ok(Bitmap::from_pixels(w, h, px))
}

fn neon_glow_border(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(10, 200);
    let glow = p.f("glow_intensity").clamp(10.0, 100.0) / 100.0;
    let tube = p.f("tube_width").clamp(1.0, 20.0);
    let neon = p.c("neon_color");
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let mut canvas = Canvas::new(nw, nh);
    canvas.clear(p.c("bg_color"));
    let b = border as f32;
    canvas.draw_bitmap(src, b, b, &Paint::default());
    let off = b * 0.5;
    let rect = crate::sk::rect(off, off, nw as f32 - off, nh as f32 - off);
    let sigma = b * 0.35 * glow;
    let a = |v: f32| (v * glow) as u8;
    canvas.draw_rect(
        rect,
        &Paint::stroke_aa(neon.with_alpha(a(120.0)), tube * 5.0).with_mask_blur(sigma),
    );
    canvas.draw_rect(
        rect,
        &Paint::stroke_aa(neon.with_alpha(a(180.0)), tube * 3.0).with_mask_blur(sigma * 0.35),
    );
    canvas.draw_rect(rect, &Paint::stroke_aa(C::WHITE.with_alpha(a(220.0)), tube));
    canvas.draw_rect(
        rect,
        &Paint::stroke_aa(neon.with_alpha(a(240.0)), tube * 1.4),
    );
    if p.b("double_tube") {
        let io = b * 0.78;
        let inner = crate::sk::rect(io, io, nw as f32 - io, nh as f32 - io);
        canvas.draw_rect(
            inner,
            &Paint::stroke_aa(neon.with_alpha(a(100.0)), tube * 2.5).with_mask_blur(sigma * 0.25),
        );
        canvas.draw_rect(
            inner,
            &Paint::stroke_aa(neon.with_alpha(a(200.0)), tube * 0.8),
        );
    }
    Ok(canvas.into_bitmap())
}

fn lum255(c: C) -> f32 {
    0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32
}

fn night_vision(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let intensity = p.f("intensity").clamp(0.0, 100.0) / 100.0;
    let glow = p.f("glow").clamp(0.0, 100.0) / 100.0;
    let noise = p.f("noise").clamp(0.0, 100.0) / 100.0;
    let vignette = p.f("vignette").clamp(0.0, 100.0) / 100.0;
    let scanlines = p.f("scanlines").clamp(0.0, 100.0) / 100.0;
    if intensity <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let cx = (w - 1) as f32 * 0.5;
    let cy = (h - 1) as f32 * 0.5;
    let max_dist = (cx * cx + cy * cy).sqrt().max(1.0);
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let lum = lum255(s) / 255.0;
            let mut avg = lum;
            avg += lum255(src.get(0.max(x - 1), y)) / 255.0;
            avg += lum255(src.get((w - 1).min(x + 1), y)) / 255.0;
            avg += lum255(src.get(x, 0.max(y - 1))) / 255.0;
            avg += lum255(src.get(x, (h - 1).min(y + 1))) / 255.0;
            avg /= 5.0;
            let mut ph = lerp(lum, avg, glow * 0.45) + lum.powf(0.82) * glow * 0.26;
            ph *= 0.72 + intensity * 0.68;
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt() / max_dist;
            let vm = clamp01(1.0 - vignette * dist * dist) * (1.02 - dist * 0.08);
            let scan = 1.0 - scanlines * 0.16 * (0.5 + 0.5 * (y as f32 * PI).sin());
            let grain = (hash01(x, y, 1229) * 2.0 - 1.0) * noise * 0.12;
            let r = clamp01((ph * 0.12 + grain * 0.35) * vm * scan);
            let g = clamp01((ph * 1.08 + grain) * vm * scan);
            let b = clamp01((ph * 0.18 + grain * 0.28) * vm * scan);
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

fn oil_paint(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let radius = p.i("radius").clamp(1, 6);
    let levels = p.i("levels").clamp(8, 64) as usize;
    let (w, h) = (src.w, src.h);
    let mut count = [0i32; 64];
    let mut sums = [[0i32; 4]; 64];
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            count[..levels].fill(0);
            sums[..levels].fill([0; 4]);
            for ky in -radius..=radius {
                let sy = (y + ky).clamp(0, h - 1);
                for kx in -radius..=radius {
                    let c = src.get((x + kx).clamp(0, w - 1), sy);
                    let mut i = (c.r as usize + c.g as usize + c.b as usize) * levels / 768;
                    if i >= levels {
                        i = levels - 1;
                    }
                    count[i] += 1;
                    sums[i][0] += c.r as i32;
                    sums[i][1] += c.g as i32;
                    sums[i][2] += c.b as i32;
                    sums[i][3] += c.a as i32;
                }
            }
            let mut best = 0;
            for i in 1..levels {
                if count[i] > count[best] {
                    best = i;
                }
            }
            let n = count[best].max(1);
            let s = sums[best];
            px.push(C::new(
                (s[0] / n) as u8,
                (s[1] / n) as u8,
                (s[2] / n) as u8,
                (s[3] / n) as u8,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn oil_slick(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let intensity = p.f("intensity").clamp(0.0, 100.0) / 100.0;
    let scale = 24.0 + p.f("scale").clamp(0.0, 100.0) / 100.0 * 180.0;
    let inv = 1.0 / scale;
    let darkness = p.f("darkness").clamp(0.0, 100.0) / 100.0;
    let gloss = p.f("gloss").clamp(0.0, 100.0) / 100.0;
    let shift = p.f("shift").clamp(0.0, 360.0) / 360.0;
    if intensity <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let l01 = |c: C| lum255(c) / 255.0;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let (sr, sg, sb) = (s.r as f32 / 255.0, s.g as f32 / 255.0, s.b as f32 / 255.0);
            let lum = 0.2126 * sr + 0.7152 * sg + 0.0722 * sb;
            let edge = clamp01(
                ((l01(src.get((w - 1).min(x + 1), y)) - l01(src.get(0.max(x - 1), y))).abs()
                    + (l01(src.get(x, (h - 1).min(y + 1))) - l01(src.get(x, 0.max(y - 1)))).abs())
                    * 1.2,
            );
            let u = x as f32 * inv;
            let v = y as f32 * inv;
            let wa = (u * 2.7 + v * 1.6 + 0.7).sin();
            let wb = (u * 4.4 - v * 2.8 - 1.1).cos();
            let wc = ((u + v) * 3.6 + 1.9).sin();
            let mut bands = wa * 0.40 + wb * 0.34 + wc * 0.26;
            bands = bands * 0.5 + 0.5;
            let mut hue = shift + bands * 0.32 + u * 0.06 + v * 0.03;
            hue -= hue.floor();
            let mask = clamp01(intensity * ((1.0 - lum) * 0.45 + edge * 0.30 + bands * 0.25));
            let (ir, ig, ib) = hsv_to_rgb(hue, 0.84, 0.92 + gloss * 0.08);
            let dr = sr * (1.0 - darkness * 0.58);
            let dg = sg * (1.0 - darkness * 0.60);
            let db = sb * (1.0 - darkness * 0.66);
            let gm = gloss * smooth_step(0.58, 0.96, bands);
            let or = screen(dr, ir * mask + gm * 0.24);
            let og = screen(dg, ig * mask + gm * 0.20);
            let ob = screen(db, ib * mask + gm * 0.28);
            px.push(C::new(
                clamp_to_byte(or * 255.0),
                clamp_to_byte(og * 255.0),
                clamp_to_byte(ob * 255.0),
                s.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}
