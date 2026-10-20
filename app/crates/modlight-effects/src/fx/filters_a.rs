//! Filters, part A: ASCII art .. Blood splash.

use std::f32::consts::{PI, TAU};

use modlight_core::icons as l;
use modlight_editor::effects::Category;
use tiny_skia::PathBuilder;

use crate::def::*;
use crate::dotnet::{DotNetRandom, shared_next};
use crate::helpers::{bilinear_sample, clamp_to_byte, clamp01, hash01, lerp, smooth_step};
use crate::sk::canvas::add_arc;
use crate::sk::{Bitmap, C, Canvas, Font, Paint, Style, round};

const F: Category = Category::Filters;

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "ascii_art",
            "ASCII art",
            F,
            l::SQUARE_TERMINAL,
            "Converts the image into ASCII character art.",
            ascii_art,
        )
        .params(|| {
            vec![
                int_slider("cell_size", "Cell size", 4, 24, 8),
                float_slider("contrast", "Contrast", 50.0, 200.0, 110.0),
                text("character_set", "Character set", "@%#*+=-:. "),
                checkbox("invert", "Invert", false),
                checkbox("dark_background", "Dark background", true),
                checkbox("use_source_color", "Use source color", true),
            ]
        }),
        Def::new(
            "add_noise",
            "Add noise",
            F,
            l::SPRAY_CAN,
            "Adds random noise to the image.",
            add_noise,
        )
        .params(|| vec![float_slider("amount", "Amount", 0.0, 100.0, 8.0)]),
        Def::new(
            "anaglyph_3d",
            "Anaglyph 3D",
            F,
            l::GLASSES,
            "Simulates a retro anaglyph 3D stereoscopic effect.",
            anaglyph,
        )
        .params(|| {
            vec![
                choice(
                    "mode",
                    "Mode",
                    &[
                        ("Red / Cyan", "RedCyan"),
                        ("Amber / Blue", "AmberBlue"),
                        ("Green / Magenta", "GreenMagenta"),
                    ],
                    "RedCyan",
                ),
                int_slider("separation_x", "Separation X", -100, 100, 10),
                int_slider("separation_y", "Separation Y", -100, 100, 0),
                float_slider("ghost_reduction", "Ghost reduction", 0.0, 100.0, 45.0),
            ]
        }),
        Def::new(
            "anamorphic_lens_flare",
            "Anamorphic lens flare",
            F,
            l::SPARKLE,
            "Adds cinematic horizontal lens flare streaks to bright areas.",
            anamorphic_flare,
        )
        .params(|| {
            vec![
                float_slider("intensity", "Intensity", 0.0, 100.0, 60.0),
                float_slider("threshold", "Threshold", 0.0, 100.0, 72.0),
                float_slider("streak_length", "Streak length", 0.0, 100.0, 55.0),
                float_slider("warmth", "Warmth", 0.0, 100.0, 45.0),
                float_slider("ghosting", "Ghosting", 0.0, 100.0, 35.0),
            ]
        }),
        Def::new(
            "anime_speed_lines",
            "Anime speed lines",
            F,
            l::SCAN_LINE,
            "Overlays radial speed lines in a manga/anime style.",
            speed_lines,
        )
        .params(|| {
            vec![
                float_slider("density", "Density", 10.0, 100.0, 70.0),
                float_slider("strength", "Strength", 0.0, 100.0, 65.0),
                float_slider("focus_radius", "Focus radius", 0.0, 80.0, 18.0),
                float_slider("center_x", "Center X", 0.0, 100.0, 50.0),
                float_slider("center_y", "Center Y", 0.0, 100.0, 50.0),
                float_slider("contrast", "Contrast", 0.0, 100.0, 35.0),
            ]
        }),
        Def::new(
            "art_deco_border",
            "Art deco border",
            F,
            l::TROPHY,
            "Adds a bold Art Deco border with geometric fan motifs, stepped lines, and metallic gold accents.",
            art_deco,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 20, 300, 64),
                color("bg_color", "Background color", C::rgb(15, 15, 25)),
                color("accent_color", "Accent color", C::rgb(212, 175, 55)),
                float_slider("fan_spacing", "Fan spacing", 30.0, 200.0, 80.0),
                int_slider("fan_rays", "Fan rays", 3, 12, 7),
                checkbox("inner_lines", "Inner stepped lines", true),
            ]
        }),
        Def::new(
            "bevel",
            "Bevel",
            F,
            l::LAYERS_2,
            "Adds a beveled edge effect with highlights and shadows.",
            bevel,
        )
        .params(|| {
            vec![
                int_slider("size", "Size", 1, 40, 10),
                float_slider("strength", "Strength", 0.0, 100.0, 70.0),
                float_slider("light_angle", "Light angle", 0.0, 360.0, 225.0),
                color("highlight_color", "Highlight color", C::new(255, 255, 255, 217)),
                color("shadow_color", "Shadow color", C::new(0, 0, 0, 176)),
            ]
        }),
        Def::new(
            "block_glitch",
            "Block glitch / Databending",
            F,
            l::CPU,
            "Simulates digital glitch artifacts with displaced color blocks.",
            block_glitch,
        )
        .params(|| {
            vec![
                int_slider("block_count", "Block count", 1, 240, 36),
                int_slider("min_block_width", "Min block width", 4, 400, 24),
                int_slider("max_block_width", "Max block width", 4, 900, 200),
                int_slider("min_block_height", "Min block height", 2, 200, 6),
                int_slider("max_block_height", "Max block height", 2, 500, 50),
                int_slider("max_displacement", "Max displacement", 0, 500, 50),
                int_slider("channel_shift", "Channel shift", 0, 64, 4),
                float_slider("noise_amount", "Noise amount", 0.0, 100.0, 10.0),
            ]
        }),
        Def::new(
            "blood_splash",
            "Blood splash",
            F,
            l::DROPLET,
            "Overlays procedural blood splash and drip effects.",
            blood_splash,
        )
        .params(|| {
            vec![
                float_slider("splash_amount", "Splash amount", 0.0, 100.0, 52.0),
                float_slider("drip_length", "Drip length", 0.0, 100.0, 48.0),
                float_slider("spread", "Spread", 0.0, 100.0, 42.0),
                float_slider("darkness", "Darkness", 0.0, 100.0, 58.0),
                float_slider("wet_shine", "Wet shine", 0.0, 100.0, 30.0),
            ]
        }),
    ]
}

fn ascii_art(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let cell = p.i("cell_size").clamp(4, 24);
    let contrast = p.f("contrast").clamp(50.0, 200.0) / 100.0;
    let charset: Vec<char> = {
        let input = p.s("character_set");
        let mut seen = Vec::new();
        for c in input.chars() {
            if !seen.contains(&c) {
                seen.push(c);
            }
        }
        if seen.is_empty() {
            "@%#*+=-:. ".chars().collect()
        } else {
            if seen.len() == 1 {
                seen.push(' ');
            }
            seen
        }
    };
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let dark = p.b("dark_background");
    let invert = p.b("invert");
    let use_source = p.b("use_source_color");
    let mut canvas = Canvas::new(w, h);
    canvas.clear(if dark { C::rgb(12, 12, 12) } else { C::WHITE });
    let font = Font::new("Consolas", cell as f32 * 1.02);
    let columns = (w as f32 / cell as f32).ceil() as i32;
    let rows = (h as f32 / cell as f32).ceil() as i32;
    let baseline = cell as f32 * 0.90;
    for row in 0..rows {
        let y0 = row * cell;
        let y1 = h.min(y0 + cell);
        for col in 0..columns {
            let x0 = col * cell;
            let x1 = w.min(x0 + cell);
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            let (mut sr, mut sg, mut sb, mut sa, mut n) = (0f32, 0f32, 0f32, 0f32, 0);
            for y in y0..y1 {
                for x in x0..x1 {
                    let c = src.get(x, y);
                    sr += c.r as f32;
                    sg += c.g as f32;
                    sb += c.b as f32;
                    sa += c.a as f32;
                    n += 1;
                }
            }
            let inv = 1.0 / n as f32;
            let (ar, ag, ab) = (sr * inv, sg * inv, sb * inv);
            let aa = clamp_to_byte(sa * inv);
            let mut lum = (0.2126 * ar + 0.7152 * ag + 0.0722 * ab) / 255.0;
            lum = ((lum - 0.5) * contrast + 0.5).clamp(0.0, 1.0);
            if invert {
                lum = 1.0 - lum;
            }
            let idx = round(lum * (charset.len() - 1) as f32) as i32;
            let c = charset[idx.clamp(0, charset.len() as i32 - 1) as usize];
            if c.is_whitespace() {
                continue;
            }
            let color = if use_source {
                C::new(clamp_to_byte(ar), clamp_to_byte(ag), clamp_to_byte(ab), aa)
            } else if dark {
                let v = clamp_to_byte((0.80 + 0.20 * lum) * 255.0);
                C::new(v, v, v, aa)
            } else {
                let v = clamp_to_byte((0.10 + 0.55 * (1.0 - lum)) * 255.0);
                C::new(v, v, v, aa)
            };
            canvas.draw_text(
                &c.to_string(),
                x0 as f32,
                y0 as f32 + baseline,
                &font,
                &Paint::fill_aa(color),
            );
        }
    }
    Ok(canvas.into_bitmap())
}

fn add_noise(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let amount = p.f("amount").clamp(0.0, 100.0);
    let amp = round(amount * 1.27) as i32;
    if amp <= 0 {
        return Ok(src.clone());
    }
    fn hash(mut x: u32) -> u32 {
        x ^= x >> 16;
        x = x.wrapping_mul(0x7FEB352D);
        x ^= x >> 15;
        x = x.wrapping_mul(0x846CA68B);
        x ^= x >> 16;
        x
    }
    let seed = 1337u32.wrapping_mul(747796405);
    let cb = |v: i32| v.clamp(0, 255) as u8;
    let px = src
        .px
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut hh = hash(i as u32 ^ seed);
            let nr = ((hh & 0xFF) as i32 - 128) * amp / 128;
            hh = hash(hh ^ 0x9E3779B9);
            let ng = ((hh & 0xFF) as i32 - 128) * amp / 128;
            hh = hash(hh ^ 0x85EBCA6B);
            let nb = ((hh & 0xFF) as i32 - 128) * amp / 128;
            C::new(
                cb(c.r as i32 + nr),
                cb(c.g as i32 + ng),
                cb(c.b as i32 + nb),
                c.a,
            )
        })
        .collect();
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}

fn anaglyph(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let ghost = p.f("ghost_reduction").clamp(0.0, 100.0) / 100.0;
    let (sx, sy) = (p.i("separation_x"), p.i("separation_y"));
    if sx == 0 && sy == 0 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let (hx, hy) = (sx as f32 * 0.5, sy as f32 * 0.5);
    let mode = p.e("mode");
    let lum = |c: C| (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let left = bilinear_sample(&src.px, w, h, x as f32 - hx, y as f32 - hy);
            let right = bilinear_sample(&src.px, w, h, x as f32 + hx, y as f32 + hy);
            let (lg, rg) = (lum(left), lum(right));
            let (r, g, b) = match mode.as_str() {
                "AmberBlue" => (
                    lerp(left.r as f32 / 255.0, lg, ghost * 0.85),
                    lerp(left.g as f32 / 255.0, lg, ghost * 0.50),
                    lerp(right.b as f32 / 255.0, rg, ghost),
                ),
                "GreenMagenta" => (
                    lerp(right.r as f32 / 255.0, rg, ghost * 0.85),
                    lerp(left.g as f32 / 255.0, lg, ghost),
                    lerp(right.b as f32 / 255.0, rg, ghost * 0.85),
                ),
                _ => (
                    lerp(left.r as f32 / 255.0, lg, ghost),
                    lerp(right.g as f32 / 255.0, rg, ghost * 0.90),
                    lerp(right.b as f32 / 255.0, rg, ghost),
                ),
            };
            px.push(C::new(
                clamp_to_byte(r * 255.0),
                clamp_to_byte(g * 255.0),
                clamp_to_byte(b * 255.0),
                ((left.a as i32 + right.a as i32) / 2) as u8,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn lum255(c: C) -> f32 {
    0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32
}

fn anamorphic_flare(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let intensity = p.f("intensity").clamp(0.0, 100.0) / 100.0;
    let threshold = p.f("threshold").clamp(0.0, 100.0) / 100.0;
    let streak = p.f("streak_length").clamp(0.0, 100.0) / 100.0;
    let warmth = p.f("warmth").clamp(0.0, 100.0) / 100.0;
    let ghosting = p.f("ghosting").clamp(0.0, 100.0) / 100.0;
    if intensity <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let cx = (w - 1) as f32 * 0.5;
    let cy = (h - 1) as f32 * 0.5;
    let sigma = 0.14 + streak * 0.22;
    let sigma_inv = 1.0 / 0.0001f32.max(2.0 * sigma * sigma);
    let max_offset = 10.0 + streak * 120.0;
    let steps = 5 + round(streak * 4.0) as i32;
    let screen = |s: f32, o: f32| {
        let o = 1f32.min(0f32.max(o));
        1.0 - (1.0 - s) * (1.0 - o)
    };
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let yn = (y as f32 - cy) / 1f32.max((h - 1) as f32);
        let profile = 0.62 + 0.38 * (-(yn * yn) * sigma_inv).exp();
        for x in 0..w {
            let s = src.get(x, y);
            let (sr, sg, sb) = (s.r as f32 / 255.0, s.g as f32 / 255.0, s.b as f32 / 255.0);
            let mut flare = 0.0f32;
            for i in -steps..=steps {
                let t = i as f32 / 1.max(steps) as f32;
                let off = t * max_offset;
                let sample = bilinear_sample(&src.px, w, h, x as f32 + off, y as f32);
                let lum = lum255(sample) / 255.0;
                let bright = clamp01((lum - threshold) / 0.0001f32.max(1.0 - threshold));
                let weight = 1.0 / (1.0 + (i.abs() as f32) * 0.7);
                flare += bright * weight;
            }
            let gx = ((w - 1) - x) as f32 + (x as f32 - cx) * (0.40 + ghosting * 0.85);
            let gs = bilinear_sample(&src.px, w, h, gx, y as f32);
            let gl = lum255(gs) / 255.0;
            let mut ghost =
                clamp01((gl - threshold * 0.82) / 0.0001f32.max(1.0 - threshold * 0.82));
            ghost *= intensity * ghosting * 0.55 * profile;
            let fs = flare * intensity * 0.12 * profile;
            let warm_g = lerp(1.0, 0.86, warmth);
            let warm_b = lerp(1.0, 0.58, warmth);
            let ghost_r = lerp(0.88, 1.0, warmth);
            let ghost_b = lerp(1.0, 0.74, warmth);
            let or = fs + ghost * ghost_r;
            let og = fs * warm_g + ghost * 0.82;
            let ob = fs * warm_b + ghost * ghost_b;
            px.push(C::new(
                clamp_to_byte(screen(sr, or) * 255.0),
                clamp_to_byte(screen(sg, og) * 255.0),
                clamp_to_byte(screen(sb, ob) * 255.0),
                s.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn speed_lines(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let density = p.f("density").clamp(10.0, 100.0) / 100.0;
    let strength = p.f("strength").clamp(0.0, 100.0) / 100.0;
    let focus = p.f("focus_radius").clamp(0.0, 80.0) / 100.0;
    let contrast = p.f("contrast").clamp(0.0, 100.0) / 100.0;
    if strength <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let cx = (p.f("center_x").clamp(0.0, 100.0) / 100.0) * (w - 1) as f32;
    let cy = (p.f("center_y").clamp(0.0, 100.0) / 100.0) * (h - 1) as f32;
    let min_dim = (w as f32).min(h as f32);
    let focus_px = min_dim * focus;
    let feather = 8f32.max(min_dim * 0.22);
    let freq = 40.0 + density * 180.0;
    let seed = 9911;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let (mut r, mut g, mut b, a) = (s.r as f32, s.g as f32, s.b as f32, s.a as f32);
            let gray = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            let desat = 0.25 + contrast * 0.45;
            r = lerp(r, gray, desat);
            g = lerp(g, gray, desat);
            b = lerp(b, gray, desat);
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let radial = smooth_step(focus_px, focus_px + feather, dist);
            let theta = dy.atan2(dx);
            let band =
                dist * 0.018 + hash01((dist * 0.12) as i32, (theta * 160.0) as i32, seed) * 0.85;
            let stripe = 0.5 + 0.5 * (theta * freq + band).sin();
            let bright = stripe.powf(36.0);
            let darkl = (1.0 - stripe).powf(30.0);
            let mask = radial * strength;
            let bm = bright * mask * 0.92;
            let dm = darkl * mask * 0.58;
            r = lerp(r, 255.0, bm);
            g = lerp(g, 255.0, bm);
            b = lerp(b, 255.0, bm);
            r = lerp(r, 0.0, dm);
            g = lerp(g, 0.0, dm);
            b = lerp(b, 0.0, dm);
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

fn art_deco(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(20, 300);
    let spacing = p.f("fan_spacing").clamp(30.0, 200.0);
    let rays = p.i("fan_rays").clamp(3, 12);
    let accent = p.c("accent_color");
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let mut canvas = Canvas::new(nw, nh);
    canvas.clear(p.c("bg_color"));
    let stroke = Paint {
        style: Style::Stroke,
        stroke_width: 1.5,
        ..Paint::fill_aa(accent)
    };
    let fill = Paint::fill_aa(accent.with_alpha(40));
    let radius = border as f32 * 0.65;
    let b = border as f32;
    let fan = |canvas: &mut Canvas, cx: f32, cy: f32, angle0: f32, arc_start: f32| {
        let mut pb = PathBuilder::new();
        for i in 0..=rays {
            let a = angle0 + PI * i as f32 / rays as f32;
            pb.move_to(cx, cy);
            pb.line_to(cx + a.cos() * radius, cy + a.sin() * radius);
        }
        add_arc(
            &mut pb,
            (cx - radius, cy - radius, cx + radius, cy + radius),
            arc_start,
            180.0,
        );
        if let Some(path) = pb.finish() {
            canvas.draw_path(&path, &fill);
            canvas.draw_path(&path, &stroke);
        }
    };
    // Rows along the top and bottom edges.
    for (cy, flip) in [(b * 0.5, false), (nh as f32 - b * 0.5, true)] {
        let mut cx = b;
        while cx <= (nw - border) as f32 {
            fan(
                &mut canvas,
                cx,
                cy,
                if flip { 0.0 } else { PI },
                if flip { 0.0 } else { 180.0 },
            );
            cx += spacing;
        }
    }
    // Columns along the left and right edges.
    for (cx, flip) in [(b * 0.5, false), (nw as f32 - b * 0.5, true)] {
        let mut cy = b;
        while cy <= (nh - border) as f32 {
            fan(
                &mut canvas,
                cx,
                cy,
                if flip { -PI / 2.0 } else { PI / 2.0 },
                if flip { 270.0 } else { 90.0 },
            );
            cy += spacing;
        }
    }
    let line = Paint {
        style: Style::Stroke,
        stroke_width: 2.0,
        ..Paint::fill_aa(accent)
    };
    canvas.draw_rect_xywh(2.0, 2.0, (nw - 4) as f32, (nh - 4) as f32, &line);
    canvas.draw_rect_xywh(6.0, 6.0, (nw - 12) as f32, (nh - 12) as f32, &line);
    canvas.draw_rect_xywh(
        b - 2.0,
        b - 2.0,
        (src.w + 4) as f32,
        (src.h + 4) as f32,
        &line,
    );
    if p.b("inner_lines") {
        let thin = Paint {
            style: Style::Stroke,
            stroke_width: 1.0,
            ..Paint::fill_aa(accent.with_alpha(100))
        };
        let i1 = b * 0.25;
        canvas.draw_rect_xywh(i1, i1, nw as f32 - i1 * 2.0, nh as f32 - i1 * 2.0, &thin);
        let i2 = b * 0.75;
        canvas.draw_rect_xywh(i2, i2, nw as f32 - i2 * 2.0, nh as f32 - i2 * 2.0, &thin);
    }
    let size = b * 0.3;
    for (cx, cy) in [
        (4.0, 4.0),
        (nw as f32 - 4.0, 4.0),
        (4.0, nh as f32 - 4.0),
        (nw as f32 - 4.0, nh as f32 - 4.0),
    ] {
        let mut pb = PathBuilder::new();
        pb.move_to(cx, cy - size);
        pb.line_to(cx + size, cy);
        pb.line_to(cx, cy + size);
        pb.line_to(cx - size, cy);
        pb.close();
        if let Some(path) = pb.finish() {
            canvas.draw_path(&path, &stroke);
        }
    }
    canvas.draw_bitmap(src, b, b, &Paint::default());
    Ok(canvas.into_bitmap())
}

fn bevel(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let size = p.i("size").clamp(1, 40);
    let strength = p.f("strength").clamp(0.0, 100.0) / 100.0;
    let hc = p.c("highlight_color");
    let sc = p.c("shadow_color");
    if strength <= 0.0 || size <= 0 || (hc.a == 0 && sc.a == 0) {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let has_transparent = src.px.iter().any(|c| c.a != 255);
    let angle = p.f("light_angle") * PI / 180.0;
    let (dir_x, dir_y) = (angle.cos(), angle.sin());
    let count = size.clamp(1, 12) as usize;
    let mut offsets = vec![0f32; count];
    let mut weights = vec![0f32; count];
    let mut weight_sum = 0f32;
    for i in 0..count {
        let t = (i as f32 + 1.0) / count as f32;
        offsets[i] = t * size as f32;
        weights[i] = 1.0 - t * 0.72;
        weight_sum += weights[i];
    }
    let ha = hc.a as f32 / 255.0;
    let sa = sc.a as f32 / 255.0;
    let edge = |len: i32, from_start: bool| -> Vec<f32> {
        (0..len)
            .map(|i| {
                let d = if from_start { i } else { (len - 1) - i };
                if d < 0 || d >= size {
                    0.0
                } else {
                    1.0 - smooth_step(0.0, size as f32, d as f32)
                }
            })
            .collect()
    };
    let (lw, rw, tw, bw) = (edge(w, true), edge(w, false), edge(h, true), edge(h, false));
    let mut ox = vec![0i32; count];
    let mut oy = vec![0i32; count];
    if has_transparent {
        for i in 0..count {
            ox[i] = round(dir_x * offsets[i]) as i32;
            oy[i] = round(dir_y * offsets[i]) as i32;
        }
    }
    let sample_alpha = |x: f32, y: f32| -> f32 {
        let (right, bottom) = (w - 1, h - 1);
        let x = x.clamp(0.0, right as f32);
        let y = y.clamp(0.0, bottom as f32);
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let x1 = if x0 < right { x0 + 1 } else { right };
        let y1 = if y0 < bottom { y0 + 1 } else { bottom };
        let (tx, ty) = (x - x0 as f32, y - y0 as f32);
        let a = |xx: i32, yy: i32| src.get(xx, yy).a as f32;
        let a0 = lerp(a(x0, y0), a(x1, y0), tx);
        let a1 = lerp(a(x0, y1), a(x1, y1), tx);
        lerp(a0, a1, ty) / 255.0
    };
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let (tws, bws) = (tw[y as usize], bw[y as usize]);
        for x in 0..w {
            let o = src.get(x, y);
            if o.a == 0 {
                px.push(o);
                continue;
            }
            let oa = o.a as f32 / 255.0;
            let (lws, rws) = (lw[x as usize], rw[x as usize]);
            let bound = lws * -dir_x + rws * dir_x + tws * -dir_y + bws * dir_y;
            let presence =
                clamp01(lws.max(rws).max(tws.max(bws)) + (lws + rws + tws + bws).min(1.0) * 0.35);
            let mut response = bound * 0.95;
            let mut edge_presence = presence;
            if has_transparent {
                let mut ar = 0f32;
                let mut ap = 0f32;
                for i in 0..count {
                    let (dx, dy) = (ox[i], oy[i]);
                    let toward = sample_alpha((x + dx) as f32, (y + dy) as f32);
                    let away = sample_alpha((x - dx) as f32, (y - dy) as f32);
                    let d = away - toward;
                    ar += d * weights[i];
                    ap += d.abs() * weights[i];
                }
                ar /= weight_sum;
                ap = clamp01((ap / weight_sum) * 1.45);
                response += ar * 1.25;
                edge_presence = clamp01(edge_presence.max(ap));
            }
            let hm = clamp01(response.max(0.0) * 1.55 * strength) * edge_presence * ha * oa;
            let sm = clamp01((-response).max(0.0) * 1.55 * strength) * edge_presence * sa * oa;
            let (mut r, mut g, mut b) = (o.r as f32, o.g as f32, o.b as f32);
            if sm > 0.0 {
                r = lerp(r, sc.r as f32, sm);
                g = lerp(g, sc.g as f32, sm);
                b = lerp(b, sc.b as f32, sm);
            }
            if hm > 0.0 {
                r = lerp(r, hc.r as f32, hm);
                g = lerp(g, hc.g as f32, hm);
                b = lerp(b, hc.b as f32, hm);
            }
            let lift = (hm - sm) * 26.0 * strength;
            px.push(C::new(
                clamp_to_byte(r + lift),
                clamp_to_byte(g + lift),
                clamp_to_byte(b + lift),
                o.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn block_glitch(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let count = p.i("block_count").clamp(1, 240);
    let min_w = p.i("min_block_width").clamp(4, 400);
    let max_w = p.i("max_block_width").clamp(min_w, 900);
    let min_h = p.i("min_block_height").clamp(2, 200);
    let max_h = p.i("max_block_height").clamp(min_h, 500);
    let max_shift = p.i("max_displacement").clamp(0, 500);
    let ch_shift = p.i("channel_shift").clamp(0, 64);
    let noise = p.f("noise_amount").clamp(0.0, 100.0) / 100.0;
    let mut dst = src.px.clone();
    let mut rng = DotNetRandom::new(1945);
    for _ in 0..count {
        let bw = rng.next_range(min_w, max_w + 1);
        let bh = rng.next_range(min_h, max_h + 1);
        let x = rng.next_range(0, 1.max(w - 1));
        let y = rng.next_range(0, 1.max(h - 1));
        let right = w.min(x + bw);
        let bottom = h.min(y + bh);
        if right <= x || bottom <= y {
            continue;
        }
        let dx = rng.next_range(-max_shift, max_shift + 1);
        let dy = rng.next_range(-(1.max(max_shift / 6)), 2.max(max_shift / 6 + 1));
        let rs = rng.next_range(-ch_shift, ch_shift + 1);
        let bs = rng.next_range(-ch_shift, ch_shift + 1);
        for py in y..bottom {
            for pxx in x..right {
                let sx = (pxx + dx).clamp(0, w - 1);
                let sy = (py + dy).clamp(0, h - 1);
                let srx = (sx + rs).clamp(0, w - 1);
                let sbx = (sx - bs).clamp(0, w - 1);
                let base = src.get(sx, sy);
                let rc = src.get(srx, sy);
                let bc = src.get(sbx, sy);
                let mut n = || {
                    if noise > 0.0 {
                        (rng.next_double() as f32 * 2.0 - 1.0) * noise * 48.0
                    } else {
                        0.0
                    }
                };
                let nr = n();
                let ng = n();
                let nb = n();
                dst[(py * w + pxx) as usize] = C::new(
                    clamp_to_byte(rc.r as f32 + nr),
                    clamp_to_byte(base.g as f32 + ng),
                    clamp_to_byte(bc.b as f32 + nb),
                    base.a,
                );
            }
        }
    }
    Ok(Bitmap::from_pixels(w, h, dst))
}

fn blood_splash(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let amount = p.f("splash_amount").clamp(0.0, 100.0) / 100.0;
    let drip_len = p.f("drip_length").clamp(0.0, 100.0) / 100.0;
    let spread = p.f("spread").clamp(0.0, 100.0) / 100.0;
    let darkness = p.f("darkness").clamp(0.0, 100.0) / 100.0;
    let wet = p.f("wet_shine").clamp(0.0, 100.0) / 100.0;
    if amount <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let seed = shared_next(1, i32::MAX);
    let splash_cell = 18.0 + (1.0 - spread) * 20.0;
    let splash_radius = 5.5 + spread * 15.5;
    let splash_density = 0.025 + amount * 0.16;
    let spray_cell = 10.0 + (1.0 - spread) * 10.0;
    let spray_radius = 0.85 + spread * 2.5;
    let spray_density = 0.035 + amount * 0.22;
    let drip_cell = 16.0 + (1.0 - spread) * 18.0;
    let drip_prob = 0.08 + amount * 0.26;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let (mut r, mut g, mut b, a) = (s.r as f32, s.g as f32, s.b as f32, s.a as f32);
            let splash = splash_mask(x, y, splash_cell, splash_radius, splash_density, seed ^ 991);
            let spray = droplet_field(x, y, spray_cell, spray_radius, spray_density, seed ^ 2081);
            let drips = drip_mask(x, y, h, drip_cell, drip_prob, drip_len, spread, seed ^ 4201);
            let mist = hash01(x, y, seed ^ 6311).powf(24.0) * amount * (0.18 + spread * 0.45);
            let mask = (splash * 0.98 + spray * 0.58 + drips * 0.88 + mist * 0.32).clamp(0.0, 1.0)
                * (0.52 + amount * 0.48);
            if mask > 0.0001 {
                let clot = (splash - 0.32).max(0.0) * (0.85 + darkness * 0.35);
                let pooled = clot.max(drips * 0.28);
                let dm = (darkness * 0.72 + pooled * 0.55).clamp(0.0, 1.0);
                let tr = lerp(198.0, 78.0, dm);
                let tg = lerp(24.0, 6.0, dm);
                let tb = lerp(26.0, 10.0, dm);
                let cov = (mask + splash * 0.12).clamp(0.0, 1.0);
                r = lerp(r, tr, cov);
                g = lerp(g, tg, cov);
                b = lerp(b, tb, cov);
                let absorb = (pooled * 0.24 + darkness * mask * 0.16).clamp(0.0, 0.42);
                r *= 1.0 - absorb * 0.18;
                g *= 1.0 - absorb * 0.34;
                b *= 1.0 - absorb * 0.38;
                if wet > 0.0001 {
                    let sn = hash01(
                        (x as f32 * 0.28) as i32,
                        (y as f32 * 0.28) as i32,
                        seed ^ 7607,
                    );
                    let sm =
                        smooth_step(0.76, 0.985, sn) * smooth_step(0.22, 0.82, splash.max(drips));
                    let shine = sm * wet * (0.25 + 0.75 * splash.max(drips));
                    if shine > 0.0001 {
                        r = lerp(r, 255.0, shine * 0.24);
                        g = lerp(g, 132.0, shine * 0.20);
                        b = lerp(b, 122.0, shine * 0.14);
                    }
                }
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

fn droplet(x: f32, y: f32, cx: f32, cy: f32, radius: f32, seed: i32) -> f32 {
    let dx = x - cx;
    let dy = y - cy;
    let dist = (dx * dx + dy * dy).sqrt();
    let edge = hash01((cx * 0.25) as i32, (cy * 0.25) as i32, seed);
    let ir = radius * (0.78 + edge * 0.34);
    1.0 - smooth_step(ir * 0.22, ir, dist)
}

fn splash_mask(x: i32, y: i32, cell: f32, radius: f32, density: f32, seed: i32) -> f32 {
    let cxi = (x as f32 / cell).floor() as i32;
    let cyi = (y as f32 / cell).floor() as i32;
    let mut strongest = 0f32;
    for oy in -1..=1 {
        for ox in -1..=1 {
            let nx = cxi + ox;
            let ny = cyi + oy;
            if hash01(nx, ny, seed) > density {
                continue;
            }
            let cx = (nx as f32 + hash01(nx, ny, seed ^ 157)) * cell;
            let cy = (ny as f32 + hash01(nx, ny, seed ^ 389)) * cell;
            let lr = radius * (0.48 + hash01(nx, ny, seed ^ 701) * 0.92);
            strongest = strongest.max(droplet(x as f32, y as f32, cx, cy, lr, seed ^ 911));
            for i in 0..3 {
                let chance = hash01(nx * 17 + i, ny * 19 - i, seed ^ 1237);
                if chance > 0.82 {
                    continue;
                }
                let angle = hash01(nx * 23 + i, ny * 29 + i, seed ^ 1601) * TAU;
                let orbit = lr * (0.32 + hash01(nx * 31 + i, ny * 37 + i, seed ^ 1907) * 1.12);
                let sx = cx + angle.cos() * orbit;
                let sy = cy + angle.sin() * orbit * (0.72 + chance * 0.55);
                let sr = lr * (0.16 + chance * 0.32);
                strongest =
                    strongest.max(droplet(x as f32, y as f32, sx, sy, sr, seed ^ (2401 + i)));
            }
        }
    }
    strongest
}

fn droplet_field(x: i32, y: i32, cell: f32, radius: f32, density: f32, seed: i32) -> f32 {
    let cxi = (x as f32 / cell).floor() as i32;
    let cyi = (y as f32 / cell).floor() as i32;
    let mut strongest = 0f32;
    for oy in -1..=1 {
        for ox in -1..=1 {
            let nx = cxi + ox;
            let ny = cyi + oy;
            if hash01(nx, ny, seed) > density {
                continue;
            }
            let cx = (nx as f32 + hash01(nx, ny, seed ^ 223)) * cell;
            let cy = (ny as f32 + hash01(nx, ny, seed ^ 521)) * cell;
            let lr = radius * (0.65 + hash01(nx, ny, seed ^ 883) * 0.9);
            strongest = strongest.max(droplet(x as f32, y as f32, cx, cy, lr, seed ^ 1291));
        }
    }
    strongest
}

#[allow(clippy::too_many_arguments)]
fn drip_mask(
    x: i32,
    y: i32,
    height: i32,
    cell: f32,
    prob: f32,
    drip_len: f32,
    spread: f32,
    seed: i32,
) -> f32 {
    let column = (x as f32 / cell).floor() as i32;
    let mut strongest = 0f32;
    for c in column - 2..=column + 2 {
        if hash01(c, 0, seed) > prob {
            continue;
        }
        let base_x = (c as f32 + hash01(c, 1, seed ^ 191)) * cell;
        let start_y = hash01(c, 2, seed ^ 487) * height as f32 * 0.58;
        let length =
            (0.12 + drip_len * 0.9) * (0.36 + hash01(c, 3, seed ^ 761) * 0.88) * height as f32;
        let thick = (1.15 + spread * 3.8) * (0.68 + hash01(c, 4, seed ^ 1033) * 0.85);
        let dy = y as f32 - start_y;
        if dy < 0.0 || dy > length {
            continue;
        }
        let sway = (hash01(c, 5, seed ^ 1327) * 2.0 - 1.0) * spread * 5.5;
        let freq = 0.012 + hash01(c, 6, seed ^ 1607) * 0.035;
        let phase = hash01(c, 7, seed ^ 1931) * TAU;
        let lane = base_x + (dy * freq + phase).sin() * sway + dy * sway * 0.018;
        let dx = (x as f32 - lane).abs();
        let line = 1.0 - smooth_step(thick * 0.45, thick, dx);
        if line <= 0.0 {
            continue;
        }
        let fade = 1.0 - smooth_step(0.0, length, dy);
        let drip = line * (0.48 + 0.52 * fade);
        let tip_y = start_y + length;
        let tdx = x as f32 - lane;
        let tdy = y as f32 - tip_y;
        let tip_d = (tdx * tdx + tdy * tdy * 0.52).sqrt();
        let tip = 1.0 - smooth_step(thick * 0.24, thick * 1.75, tip_d);
        let sdx = x as f32 - base_x;
        let sdy = y as f32 - start_y;
        let sd = (sdx * sdx + sdy * sdy * 0.7).sqrt();
        let shoulder = 1.0 - smooth_step(thick * 0.20, thick * 1.9, sd);
        strongest = strongest.max(drip.max((tip * 0.92).max(shoulder * 0.38)));
    }
    strongest
}
