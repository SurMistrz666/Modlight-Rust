//! Filters, part K: Pointillism .. Rope border.

use std::f32::consts::PI;

use modlight_core::icons as l;
use modlight_editor::effects::Category;

use crate::def::*;
use crate::helpers::{bilinear_sample, blend, clamp_to_byte, clamp01, hash01, smooth_step};
use crate::sk::filters::color_matrix;
use crate::sk::{Bitmap, Blend, C, Canvas, Font, Paint, Sampling, Shader, Tile, rect, round};

const F: Category = Category::Filters;

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "pointillism",
            "Pointillism",
            F,
            l::CIRCLE_DOT,
            "Renders the image as a pointillist painting with colored dots.",
            pointillism,
        )
        .params(|| {
            vec![
                int_slider("dot_size", "Dot size", 2, 24, 7),
                float_slider("density", "Density", 10.0, 100.0, 72.0),
                float_slider("jitter", "Jitter", 0.0, 100.0, 65.0),
                float_slider("color_boost", "Color boost", 0.0, 100.0, 20.0),
                float_slider("background_mix", "Background mix", 0.0, 100.0, 20.0),
            ]
        }),
        Def::new(
            "polaroid_frame",
            "Polaroid frame",
            F,
            l::CAMERA,
            "Wraps the image in a Polaroid-style instant photo frame.",
            polaroid,
        )
        .params(|| {
            vec![
                int_slider("border_size", "Border size", 5, 100, 30),
                int_slider("bottom_size", "Bottom size", 20, 200, 80),
                float_slider("rotation", "Rotation", -30.0, 30.0, 0.0),
                color("frame_color", "Frame color", C::rgb(250, 249, 245)),
                text("caption", "Caption", ""),
                int_slider("shadow_size", "Shadow size", 0, 60, 15),
            ]
        }),
        Def::new(
            "pop_art_grid",
            "Pop art grid",
            F,
            l::GRID_2X2,
            "Creates an Andy Warhol-style 2\u{d7}2 pop art grid with color variations.",
            pop_art,
        )
        .params(|| {
            vec![
                color("color1", "Color 1", C::rgb(255, 50, 50)),
                color("color2", "Color 2", C::rgb(50, 200, 50)),
                color("color3", "Color 3", C::rgb(50, 50, 255)),
                color("color4", "Color 4", C::rgb(255, 200, 50)),
                float_slider("strength", "Color strength", 0.0, 100.0, 60.0),
            ]
        }),
        Def::new(
            "rgb_split",
            "RGB split",
            F,
            l::SPLIT,
            "Splits and offsets the red, green, and blue color channels.",
            rgb_split,
        )
        .params(|| {
            vec![
                int_slider("offset_red_x", "Offset red X", -100, 100, -5),
                int_slider("offset_red_y", "Offset red Y", -100, 100, 0),
                int_slider("offset_green_x", "Offset green X", -100, 100, 0),
                int_slider("offset_green_y", "Offset green Y", -100, 100, 0),
                int_slider("offset_blue_x", "Offset blue X", -100, 100, 5),
                int_slider("offset_blue_y", "Offset blue Y", -100, 100, 0),
            ]
        }),
        Def::new(
            "radial_rainbow",
            "Radial rainbow",
            F,
            l::RAINBOW,
            "Applies a rainbow hue shift based on angle from center.",
            radial_rainbow,
        )
        .params(|| {
            vec![
                float_slider("strength", "Strength", 0.0, 100.0, 50.0),
                int_slider("rotations", "Rotations", 1, 5, 1),
            ]
        }),
        Def::new(
            "rainbow_border",
            "Rainbow border",
            F,
            l::RAINBOW,
            "Adds a vivid rainbow gradient border that cycles through the full color spectrum.",
            rainbow_border,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 2, 300, 24),
                float_slider("saturation", "Saturation", 0.0, 100.0, 100.0),
                float_slider("brightness", "Brightness", 0.0, 100.0, 95.0),
                float_slider("hue_offset", "Hue offset", 0.0, 360.0, 0.0),
                float_slider("glow_strength", "Glow", 0.0, 100.0, 40.0),
                checkbox("inner_line", "Inner line", true),
            ]
        }),
        Def::new(
            "rainy_window",
            "Rainy window",
            F,
            l::CLOUD_RAIN,
            "Simulates looking through a rain-streaked window with distortion and mist.",
            rainy_window,
        )
        .params(|| {
            vec![
                float_slider("distortion", "Distortion", 0.0, 30.0, 8.0),
                float_slider("streak_density", "Streak density", 0.0, 100.0, 45.0),
                float_slider("mist_amount", "Mist amount", 0.0, 100.0, 25.0),
                float_slider("droplet_amount", "Droplet amount", 0.0, 100.0, 35.0),
            ]
        }),
        Def::new(
            "reflection",
            "Reflection",
            F,
            l::FLIP_VERTICAL_2,
            "Adds a reflection to the bottom of the image.",
            reflection,
        )
        .params(|| {
            vec![
                int_slider("percentage", "Percentage", 1, 100, 20)
                    .snap(false)
                    .fmt("{}{0:0}%"),
                int_slider("max_alpha", "Max Alpha", 0, 255, 255).snap(false),
                int_slider("min_alpha", "Min Alpha", 0, 255, 0).snap(false),
                int_slider("offset", "Offset", 0, 100, 0).snap(false),
                checkbox("skew", "Skew", false),
                int_slider("skew_size", "Skew Size", 1, 100, 25).snap(false),
            ]
        }),
        Def::new(
            "retro_sunset",
            "Retro sunset",
            F,
            l::SUNSET,
            "Overlays retro 80s-style sunset gradient bands.",
            retro_sunset,
        )
        .params(|| {
            vec![
                float_slider("strength", "Strength", 0.0, 100.0, 50.0),
                checkbox("horizontal_lines", "Horizontal lines", true),
            ]
        }),
        Def::new(
            "riso_print",
            "Riso print",
            F,
            l::PRINTER,
            "Simulates a risograph print with halftone dots and ink colors.",
            riso_print,
        )
        .params(|| {
            vec![
                float_slider("ink_strength", "Ink strength", 0.0, 100.0, 70.0),
                float_slider("paper_fade", "Paper fade", 0.0, 100.0, 25.0),
                float_slider("offset", "Offset", -200.0, 200.0, 3.0),
                float_slider("dot_scale", "Dot scale", 0.0, 100.0, 18.0),
                float_slider("ink_noise", "Ink noise", 0.0, 100.0, 35.0),
                color("ink_color_a", "Ink color A", C::new(220, 70, 70, 255)),
                color("ink_color_b", "Ink color B", C::new(70, 200, 210, 255)),
            ]
        }),
        Def::new(
            "rope_border",
            "Rope border",
            F,
            l::LINK,
            "Adds a twisted rope/cord border with braided fiber detail and shading.",
            rope_border,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 12, 200, 40),
                color("rope_color", "Rope color", C::rgb(180, 145, 90)),
                color("bg_color", "Background color", C::rgb(60, 45, 30)),
                float_slider("twist_density", "Twist density", 10.0, 100.0, 50.0),
                float_slider("fiber_detail", "Fiber detail", 0.0, 100.0, 55.0),
            ]
        }),
    ]
}

fn pointillism(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let dot = p.i("dot_size").clamp(2, 24);
    let density = p.f("density").clamp(10.0, 100.0) / 100.0;
    let jitter = p.f("jitter").clamp(0.0, 100.0) / 100.0;
    let boost = p.f("color_boost").clamp(0.0, 100.0) / 100.0;
    let bg = p.f("background_mix").clamp(0.0, 100.0) / 100.0;
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let seed = 2026;
    let mut canvas = Canvas::new(w, h);
    if bg > 0.0 {
        canvas.draw_bitmap(src, 0.0, 0.0, &Paint::alpha(clamp_to_byte(bg * 255.0)));
    }
    let step = 2.max(dot);
    let ja = jitter * step as f32 * 0.6;
    let mut gy = 0;
    while gy < h {
        let mut gx = 0;
        while gx < w {
            let (cxi, cyi) = (gx / step, gy / step);
            if hash01(cxi, cyi, seed) <= density {
                let jx = (hash01(cxi, cyi, seed ^ 111) * 2.0 - 1.0) * ja;
                let jy = (hash01(cxi, cyi, seed ^ 257) * 2.0 - 1.0) * ja;
                let cx = (gx as f32 + step as f32 * 0.5 + jx).clamp(0.0, w as f32 - 1.0);
                let cy = (gy as f32 + step as f32 * 0.5 + jy).clamp(0.0, h as f32 - 1.0);
                let mut c = bilinear_sample(&src.px, w, h, cx, cy);
                if boost > 0.0 {
                    let (r, g, b) = (c.r as f32, c.g as f32, c.b as f32);
                    let gray = (r + g + b) / 3.0;
                    let f = 1.0 + boost;
                    c = C::new(
                        clamp_to_byte(gray + (r - gray) * f),
                        clamp_to_byte(gray + (g - gray) * f),
                        clamp_to_byte(gray + (b - gray) * f),
                        c.a,
                    );
                }
                let rj = 0.6 + hash01(cxi, cyi, seed ^ 509) * 0.9;
                let radius = (dot as f32 * 0.36 * rj).max(1.0);
                canvas.draw_circle(cx, cy, radius, &Paint::fill_aa(c));
            }
            gx += step;
        }
        gy += step;
    }
    Ok(canvas.into_bitmap())
}

fn polaroid(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(5, 100);
    let bottom = p.i("bottom_size").clamp(20, 200);
    let shadow = p.i("shadow_size").clamp(0, 60);
    let rotation = p.f("rotation").clamp(-30.0, 30.0);
    let pw = src.w + border * 2;
    let ph = src.h + border + bottom;
    let rad = rotation.abs() * PI / 180.0;
    let (cos, sin) = (rad.cos(), rad.sin());
    let rw = (pw as f32 * cos + ph as f32 * sin).ceil() as i32;
    let rh = (pw as f32 * sin + ph as f32 * cos).ceil() as i32;
    let cw = rw.max(pw) + shadow * 2 + 20;
    let ch = rh.max(ph) + shadow * 2 + 20;
    let mut canvas = Canvas::new(cw, ch);
    canvas.save();
    canvas.translate(cw as f32 / 2.0, ch as f32 / 2.0);
    canvas.rotate_degrees(rotation);
    canvas.translate(-pw as f32 / 2.0, -ph as f32 / 2.0);
    let (pwf, phf) = (pw as f32, ph as f32);
    if shadow > 0 {
        let paint = Paint::fill_aa(C::new(0, 0, 0, 50)).with_mask_blur(shadow as f32 / 2.0);
        canvas.draw_rect_xywh(0.0, 0.0, pwf, phf, &paint);
    }
    canvas.draw_rect_xywh(0.0, 0.0, pwf, phf, &Paint::fill_aa(p.c("frame_color")));
    canvas.draw_rect_xywh(
        0.0,
        0.0,
        pwf,
        phf,
        &Paint::stroke_aa(C::rgb(200, 198, 190), 1.0),
    );
    let b = border as f32;
    canvas.draw_bitmap(src, b, b, &Paint::default());
    canvas.draw_rect_xywh(
        b,
        b,
        src.w as f32,
        src.h as f32,
        &Paint::stroke_aa(C::new(180, 175, 165, 100), 1.0),
    );
    let caption = p.s("caption");
    if !caption.trim().is_empty() {
        let size = 12f32.max(bottom as f32 * 0.3);
        let font = Font::new("Segoe Script", size);
        let tx = pwf / 2.0 - font.measure(&caption) / 2.0;
        let ty = (src.h + border) as f32 + bottom as f32 / 2.0 + size / 3.0;
        canvas.draw_text(&caption, tx, ty, &font, &Paint::fill_aa(C::rgb(60, 55, 50)));
    }
    canvas.restore();
    Ok(canvas.into_bitmap())
}

fn pop_art(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    let (hw, hh) = (w / 2, h / 2);
    let s = (p.f("strength") / 100.0).clamp(0.0, 1.0);
    let g = 0.33;
    let gray = color_matrix(
        src,
        &[
            g, g, g, 0.0, 0.0, g, g, g, 0.0, 0.0, g, g, g, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
        ],
    );
    let tints = [p.c("color1"), p.c("color2"), p.c("color3"), p.c("color4")];
    let (wf, hf, hwf, hhf) = (w as f32, h as f32, hw as f32, hh as f32);
    let cells = [
        rect(0.0, 0.0, hwf, hhf),
        rect(hwf, 0.0, wf, hhf),
        rect(0.0, hhf, hwf, hf),
        rect(hwf, hhf, wf, hf),
    ];
    let mut canvas = Canvas::new(w, h);
    let full = rect(0.0, 0.0, wf, hf);
    for (i, cell) in cells.iter().enumerate() {
        canvas.save();
        canvas.clip_rect(*cell, false);
        canvas.draw_bitmap_rect(&gray, full, *cell, Sampling::Nearest, &Paint::default());
        let tint = Paint::fill(tints[i].with_alpha((255.0 * s) as u8)).with_blend(Blend::Modulate);
        canvas.draw_rect(*cell, &tint);
        canvas.restore();
    }
    Ok(canvas.into_bitmap())
}

fn rgb_split(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    let (rx, ry) = (p.i("offset_red_x"), p.i("offset_red_y"));
    let (gx, gy) = (p.i("offset_green_x"), p.i("offset_green_y"));
    let (bx, by) = (p.i("offset_blue_x"), p.i("offset_blue_y"));
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let cr = src.get((x - rx).clamp(0, w - 1), (y - ry).clamp(0, h - 1));
            let cg = src.get((x - gx).clamp(0, w - 1), (y - gy).clamp(0, h - 1));
            let cb = src.get((x - bx).clamp(0, w - 1), (y - by).clamp(0, h - 1));
            px.push(C::new(
                (cr.r as i32 * cr.a as i32 / 255) as u8,
                (cg.g as i32 * cg.a as i32 / 255) as u8,
                (cb.b as i32 * cb.a as i32 / 255) as u8,
                ((cr.a as i32 + cg.a as i32 + cb.a as i32) / 3) as u8,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn radial_rainbow(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let s = (p.f("strength") / 100.0).clamp(0.0, 1.0);
    if s <= 0.0 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let rots = 1.max(p.i("rotations")) as f32;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let angle = (y as f32 - cy).atan2(x as f32 - cx);
            let shift = ((angle + PI) / (2.0 * PI)) * 360.0 * rots;
            let c = src.get(x, y);
            let (hue, sat, lum) = c.to_hsl();
            let mut nh = (hue + shift * s) % 360.0;
            if nh < 0.0 {
                nh += 360.0;
            }
            px.push(C::from_hsl(nh, sat, lum, c.a));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn rainbow_border(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(2, 300);
    let sat = p.f("saturation").clamp(0.0, 100.0);
    let brightness = p.f("brightness").clamp(0.0, 100.0);
    let hue_off = p.f("hue_offset") % 360.0;
    let glow = p.f("glow_strength").clamp(0.0, 100.0) / 100.0;
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let pw = src.w as f32 + border as f32 * 2.0;
    let phh = src.h as f32 + border as f32 * 2.0;
    let total = 2.0 * (pw + phh - 4.0);
    let mut px = vec![C::ZERO; (nw * nh) as usize];
    let (il, it) = (border as f32, border as f32);
    let (ir, ib) = ((border + src.w) as f32, (border + src.h) as f32);
    for y in 0..nh {
        let top = y < border;
        let bottom = y >= border + src.h;
        for x in 0..nw {
            let left = x < border;
            let right = x >= border + src.w;
            if !top && !bottom && !left && !right {
                continue;
            }
            let cx = (x as f32).clamp(0.0, (nw - 1) as f32);
            let cy = (y as f32).clamp(0.0, (nh - 1) as f32);
            let dt = cy;
            let db = (nh - 1) as f32 - cy;
            let dl = cx;
            let dr = (nw - 1) as f32 - cx;
            let min = dt.min(db).min(dl.min(dr));
            let along = if min == dt {
                cx
            } else if min == dr {
                pw + cy
            } else if min == db {
                pw + phh + ((nw - 1) as f32 - cx)
            } else {
                pw + phh + (nw - 1) as f32 + ((nh - 1) as f32 - cy)
            };
            let hue = ((along / total) * 360.0 + hue_off) % 360.0;
            let kx = (x as f32).clamp(il, ir - 1.0);
            let ky = (y as f32).clamp(it, ib - 1.0);
            let d = ((x as f32 - kx) * (x as f32 - kx) + (y as f32 - ky) * (y as f32 - ky)).sqrt();
            let cross = (d / border as f32).clamp(0.0, 1.0);
            let scale = if glow > 0.0 {
                1.0 - smooth_step(0.6, 1.0, cross) * glow * 0.5
            } else {
                1.0
            };
            px[(y * nw + x) as usize] = C::from_hsv(hue, sat, brightness * scale, 255);
        }
    }
    let mut canvas = Canvas::from_bitmap(&Bitmap::from_pixels(nw, nh, px));
    let b = border as f32;
    canvas.draw_bitmap(src, b, b, &Paint::default());
    if p.b("inner_line") {
        canvas.draw_rect_xywh(
            b - 0.75,
            b - 0.75,
            src.w as f32 + 1.5,
            src.h as f32 + 1.5,
            &Paint::stroke_aa(C::new(255, 255, 255, 120), 1.5),
        );
    }
    Ok(canvas.into_bitmap())
}

fn rainy_window(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let dist = p.f("distortion").clamp(0.0, 30.0);
    let streaks = p.f("streak_density").clamp(0.0, 100.0) / 100.0;
    let mist = p.f("mist_amount").clamp(0.0, 100.0) / 100.0;
    let drops = p.f("droplet_amount").clamp(0.0, 100.0) / 100.0;
    if dist <= 0.0 && streaks <= 0.0 && mist <= 0.0 && drops <= 0.0 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let sp = &src.px;
    let seed = 8723;
    let cell = 8f32.max(30.0 - drops * 22.0);
    let two_pi = PI * 2.0;
    let mut px = Vec::with_capacity(sp.len());
    for y in 0..h {
        let y_wave = y as f32 * 0.09;
        for x in 0..w {
            let (xf, yf) = (x as f32, y as f32);
            let phase = hash01(x / 8, 3, seed) * two_pi;
            let wa = (yf * 0.075 + phase).sin();
            let wb = (xf * 0.022 + yf * 0.15 + phase * 0.5).sin();
            let ox = dist * (0.38 * wa + 0.18 * wb);
            let oy = dist * 0.08 * (xf * 0.08 + phase * 1.3).sin();
            let (sx, sy) = (xf + ox, yf + oy);
            let mut base = bilinear_sample(sp, w, h, sx, sy);
            if mist > 0.0 {
                let b0 = bilinear_sample(sp, w, h, sx - 1.25, sy);
                let b1 = bilinear_sample(sp, w, h, sx + 1.25, sy);
                let b2 = bilinear_sample(sp, w, h, sx, sy - 1.25);
                let b3 = bilinear_sample(sp, w, h, sx, sy + 1.25);
                let avg = |a: u8, b: u8, c: u8, d: u8| {
                    ((a as i32 + b as i32 + c as i32 + d as i32) / 4) as u8
                };
                let blur = C::new(
                    avg(b0.r, b1.r, b2.r, b3.r),
                    avg(b0.g, b1.g, b2.g, b3.g),
                    avg(b0.b, b1.b, b2.b, b3.b),
                    avg(b0.a, b1.a, b2.a, b3.a),
                );
                base = blend(base, blur, mist * 0.45);
            }
            let cn = hash01(x / 6, 11, seed);
            let mut streak = 0.0;
            if cn > 1.0 - 0.72 * streaks {
                let sig = (y_wave + cn * 18.0).sin();
                streak = sig.max(0.0).powf(3.0) * streaks;
            }
            let mut drop = 0.0;
            let cxi = (xf / cell) as i32;
            let cyi = (yf / cell) as i32;
            if hash01(cxi, cyi, seed ^ 0x2A31) > 0.93 - 0.55 * drops {
                let ccx = (cxi as f32 + hash01(cxi, cyi, seed ^ 193)) * cell;
                let ccy = (cyi as f32 + hash01(cxi, cyi, seed ^ 431)) * cell;
                let r = cell * (0.12 + 0.24 * hash01(cxi, cyi, seed ^ 997));
                let (dx, dy) = (xf - ccx, yf - ccy);
                let radial = (dx * dx + dy * dy).sqrt() / r.max(0.001);
                drop = 1.0 - smooth_step(0.2, 1.0, radial);
            }
            let lum =
                (0.2126 * base.r as f32 + 0.7152 * base.g as f32 + 0.0722 * base.b as f32) / 255.0;
            let fog = C::new(186, 196, 206, base.a);
            let mc = blend(base, fog, mist * (0.2 + (1.0 - lum) * 0.8));
            let wet = 1.0 - (0.08 * mist + 0.06 * streak);
            let hl = drop * 0.50 + streak * 0.20;
            px.push(C::new(
                clamp_to_byte(mc.r as f32 * wet + hl * 88.0),
                clamp_to_byte(mc.g as f32 * wet + hl * 96.0),
                clamp_to_byte(mc.b as f32 * wet + hl * 116.0),
                mc.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn reflection(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let pct = p.i("percentage");
    if pct <= 0 {
        return Ok(src.clone());
    }
    let offset = p.i("offset");
    let (skew, skew_size) = (p.b("skew"), p.i("skew_size"));
    let rh = (src.h as f32 * pct as f32 / 100.0) as i32;
    if rh <= 0 {
        // Upstream draws into `new SKBitmap(width, 0)`, which has no pixels, and the
        // effect throws (the editor keeps the picture).
        return Err("The reflection is less than one pixel tall.".into());
    }
    let nh = src.h + offset + rh;
    let mut nw = src.w;
    if skew && skew_size > 0 {
        nw += (rh as f32 * (skew_size as f32 / 100.0)) as i32;
    }
    let mut canvas = Canvas::new(nw, nh);
    canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    let mut flipped = Canvas::new(src.w, rh);
    flipped.translate(0.0, src.h as f32);
    flipped.scale(1.0, -1.0);
    flipped.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    let flipped = flipped.into_bitmap();
    let mut masked = Canvas::new(src.w, rh);
    masked.draw_bitmap(&flipped, 0.0, 0.0, &Paint::default());
    let gradient = Paint::default()
        .with_shader(Shader::Linear {
            p0: (0.0, 0.0),
            p1: (0.0, rh as f32),
            colors: vec![
                C::new(255, 255, 255, p.i("max_alpha") as u8),
                C::new(255, 255, 255, p.i("min_alpha") as u8),
            ],
            pos: None,
            tile: Tile::Clamp,
        })
        .with_blend(Blend::DstIn);
    masked.draw_rect(rect(0.0, 0.0, src.w as f32, rh as f32), &gradient);
    let masked = masked.into_bitmap();
    if skew && skew_size > 0 {
        canvas.save();
        canvas.translate(0.0, (src.h + offset) as f32);
        canvas.skew(skew_size as f32 / 100.0, 0.0);
        canvas.draw_bitmap(&masked, 0.0, 0.0, &Paint::default());
        canvas.restore();
    } else {
        canvas.draw_bitmap(&masked, 0.0, (src.h + offset) as f32, &Paint::default());
    }
    Ok(canvas.into_bitmap())
}

fn retro_sunset(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let alpha = (p.f("strength") / 100.0).clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let ga = (200.0 * alpha) as u8;
    let colors = [
        C::rgb(120, 0, 180),
        C::rgb(220, 20, 80),
        C::rgb(255, 100, 0),
        C::rgb(255, 200, 0),
    ]
    .iter()
    .map(|c| c.with_alpha(ga))
    .collect();
    let mut canvas = Canvas::from_bitmap(src);
    let paint = Paint::default()
        .with_shader(Shader::Linear {
            p0: (0.0, 0.0),
            p1: (0.0, h as f32),
            colors,
            pos: Some(vec![0.0, 0.33, 0.66, 1.0]),
            tile: Tile::Clamp,
        })
        .with_blend(Blend::Overlay);
    canvas.draw_rect_xywh(0.0, 0.0, w as f32, h as f32, &paint);
    if p.b("horizontal_lines") {
        let line = Paint::stroke(C::new(0, 0, 0, (60.0 * alpha) as u8), 1.0);
        let spacing = 3.max(h / 80);
        let mut y = 0;
        while y < h {
            canvas.draw_line(0.0, y as f32, w as f32, y as f32, &line);
            y += spacing;
        }
    }
    Ok(canvas.into_bitmap())
}

fn riso_dot(x: i32, y: i32, cell: i32, coverage: f32, noise: f32, seed: i32, ink: i32) -> f32 {
    let cx = (x / cell) * cell + cell / 2;
    let cy = (y / cell) * cell + cell / 2;
    let dx = (x - cx) as f32;
    let dy = (y - cy) as f32;
    let dist = (dx * dx + dy * dy).sqrt();
    let mut r = cell as f32 * 0.45 * coverage.max(0.0).sqrt();
    let jitter = (hash01(x, y, seed ^ ink) * 2.0 - 1.0) * noise;
    r *= 1.0 + 0.25 * jitter;
    let edge = 1.0 + 0.8 * noise;
    1.0 - smooth_step(r, r + edge, dist)
}

fn riso_print(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let ink = p.f("ink_strength").clamp(0.0, 100.0) / 100.0;
    if ink <= 0.0 {
        return Ok(src.clone());
    }
    let fade = p.f("paper_fade").clamp(0.0, 100.0) / 100.0;
    let dot_scale = p.f("dot_scale").clamp(0.0, 100.0) / 100.0;
    let noise = p.f("ink_noise").clamp(0.0, 100.0) / 100.0;
    let misreg = p.f("offset").clamp(-200.0, 200.0) * 0.30;
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let seed = 2026;
    let cell = (round(4.0 + dot_scale * 20.0) as i32).clamp(3, 28);
    let (ia, ib) = (p.c("ink_color_a"), p.c("ink_color_b"));
    let aw = (ia.a as f32 / 255.0) * ink;
    let bw = (ib.a as f32 / 255.0) * ink;
    let keep = clamp01(1.0 - fade * 0.90);
    let f = |c: C| (c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0);
    let (ar, ag, ab) = f(ia);
    let (br, bg, bb) = f(ib);
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let lum = (0.2126 * s.r as f32 + 0.7152 * s.g as f32 + 0.0722 * s.b as f32) / 255.0;
            let cov = clamp01(1.0 - lum);
            let da = riso_dot(x, y, cell, cov, noise, seed, 0xA11);
            let xb = round(x as f32 + misreg) as i32;
            let db = riso_dot(xb, y, cell, cov, noise, seed, 0xB22);
            let a = clamp01(da * aw * keep);
            let b = clamp01(db * bw * keep);
            let mut or = (1.0 - a) + ar * a;
            let mut og = (1.0 - a) + ag * a;
            let mut ob = (1.0 - a) + ab * a;
            or = or * (1.0 - b) + br * b;
            og = og * (1.0 - b) + bg * b;
            ob = ob * (1.0 - b) + bb * b;
            if noise > 0.001 {
                let n = hash01(x, y, seed ^ 0x99) * 2.0 - 1.0;
                let g = n * noise * 0.06;
                or = (or + g).max(0.0);
                og = (og + g * 0.92).max(0.0);
                ob = (ob + g * 1.06).max(0.0);
            }
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

fn rope_border(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(12, 200);
    let tf = p.f("twist_density").clamp(10.0, 100.0) / 1000.0;
    let fiber = p.f("fiber_detail").clamp(0.0, 100.0) / 100.0;
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let rope = p.c("rope_color");
    let bg = p.c("bg_color");
    let (rr, rg, rb) = (
        rope.r as f32 / 255.0,
        rope.g as f32 / 255.0,
        rope.b as f32 / 255.0,
    );
    let seed = 8271;
    let radius = border as f32 * 0.42;
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
            let ccross = border as f32 * 0.5;
            let (along, across, bseed) = if top {
                (x as f32, y as f32 - ccross, seed ^ 0x1A)
            } else if bottom {
                let a = ((y - (nh - border)) as f32) - (border as f32 - ccross);
                ((nw - x) as f32, -a, seed ^ 0x2B)
            } else if left {
                ((nh - y) as f32, x as f32 - ccross, seed ^ 0x3C)
            } else {
                let a = ((x - (nw - border)) as f32) - (border as f32 - ccross);
                (y as f32, -a, seed ^ 0x4D)
            };
            let mut best = f32::MAX;
            let mut best_shade = 0.0;
            let sr = radius * 0.38;
            for s in 0..3 {
                let phase = s as f32 * (PI * 2.0 / 3.0);
                let off = (along * tf * PI * 2.0 + phase).sin() * radius * 0.45;
                let d = (across - off).abs();
                if d < sr && d < best {
                    best = d;
                    let n = d / sr;
                    best_shade = 1.0 - n * n;
                }
            }
            px[(y * nw + x) as usize] = if best < radius * 0.38 {
                let fnoise = if fiber > 0.0 {
                    (hash01((along * 1.5) as i32, (across * 3.0) as i32, bseed) * 2.0 - 1.0)
                        * fiber
                        * 0.18
                } else {
                    0.0
                };
                let twist = (along * tf * PI * 2.0).cos() * 0.08;
                let shade = best_shade * 0.35 + twist + fnoise;
                C::new(
                    clamp_to_byte((rr + shade) * 255.0),
                    clamp_to_byte((rg + shade * 0.8) * 255.0),
                    clamp_to_byte((rb + shade * 0.5) * 255.0),
                    255,
                )
            } else {
                bg
            };
        }
    }
    let mut canvas = Canvas::from_bitmap(&Bitmap::from_pixels(nw, nh, px));
    canvas.draw_bitmap(src, border as f32, border as f32, &Paint::default());
    Ok(canvas.into_bitmap())
}
