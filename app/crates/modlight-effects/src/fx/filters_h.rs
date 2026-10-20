//! Filters, part H: Light leak .. Miniature.

use modlight_core::icons as l;
use modlight_editor::effects::Category;

use crate::def::*;
use crate::fx::filters_d::convolve3x3;
use crate::helpers::{
    bilinear_sample, blur_bitmap, clamp_to_byte, clamp01, fractal_noise, hash01, lerp, smooth_step,
};
use crate::sk::text::Font;
use crate::sk::{Bitmap, Blend, C, Canvas, Paint, Shader, Tile, rect};

const F: Category = Category::Filters;

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "light_leak",
            "Light leak",
            F,
            l::SUN,
            "Adds a warm film light leak effect.",
            light_leak,
        )
        .params(|| {
            vec![
                float_slider("intensity", "Intensity", 0.0, 100.0, 50.0),
                color("color", "Color", C::new(255, 160, 50, 255)),
                choice(
                    "position",
                    "Position",
                    &[
                        ("Top left", "TopLeft"),
                        ("Top right", "TopRight"),
                        ("Bottom left", "BottomLeft"),
                        ("Bottom right", "BottomRight"),
                        ("Left", "Left"),
                        ("Right", "Right"),
                    ],
                    "TopRight",
                ),
            ]
        }),
        Def::new(
            "liquid_glass",
            "Liquid glass",
            F,
            l::GLASS_WATER,
            "Distorts the image as if viewed through liquid glass.",
            liquid_glass,
        )
        .params(|| {
            vec![
                float_slider("distortion", "Distortion", 0.0, 35.0, 9.0),
                float_slider("refraction", "Refraction", 0.0, 100.0, 45.0),
                int_slider("chroma_shift", "Chroma shift", 0, 8, 1),
                float_slider("gloss", "Gloss", 0.0, 100.0, 40.0),
                float_slider("flow_scale", "Flow scale", 40.0, 220.0, 100.0),
            ]
        }),
        Def::new(
            "liquid_mercury",
            "Liquid mercury",
            F,
            l::DROPLET,
            "Transforms the image into a reflective liquid mercury surface.",
            liquid_mercury,
        )
        .params(|| {
            vec![
                float_slider("reflection", "Reflection", 0.0, 100.0, 78.0),
                float_slider("ripple", "Ripple", 0.0, 100.0, 42.0),
                float_slider("shine", "Shine", 0.0, 100.0, 82.0),
                float_slider("fluidity", "Fluidity", 0.0, 100.0, 55.0),
                float_slider("depth", "Depth", 0.0, 100.0, 65.0),
            ]
        }),
        Def::new(
            "luminance_contour_lines",
            "Luminance contour lines",
            F,
            l::WAYPOINTS,
            "Draws contour lines based on luminance quantization levels.",
            contour_lines,
        )
        .params(|| {
            vec![
                int_slider("levels", "Levels", 2, 64, 12),
                float_slider("line_width", "Line width", 0.0, 200.0, 6.0),
                float_slider("line_strength", "Line strength", 0.0, 100.0, 65.0),
                float_slider("background_strength", "Background strength", 0.0, 100.0, 20.0),
                float_slider("threshold", "Threshold", 0.0, 255.0, 0.0),
                checkbox("invert", "Invert", false),
                color("line_color", "Line color", C::new(0, 0, 0, 255)),
            ]
        }),
        Def::new(
            "macos_window",
            "macOS",
            F,
            l::MONITOR,
            "Wraps the image in a macOS-style window frame with a title bar and traffic light buttons.",
            macos_window,
        )
        .params(|| {
            vec![
                text("title", "Title", ""),
                int_slider("corner_radius", "Corner radius", 0, 30, 10),
                checkbox("dark_mode", "Dark mode", false),
                int_slider("shadow_size", "Shadow size", 0, 60, 20),
                int_slider("padding", "Padding", 0, 100, 20),
            ]
        }),
        Def::new(
            "marble_border",
            "Marble border",
            F,
            l::GEM,
            "Adds a procedural marble-veined texture border around the image.",
            marble_border,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 2, 300, 50),
                color("base_color", "Base color", C::rgb(235, 230, 220)),
                color("vein_color", "Vein color", C::rgb(90, 80, 70)),
                float_slider("vein_strength", "Vein strength", 0.0, 100.0, 60.0),
                float_slider("vein_scale", "Vein scale", 5.0, 100.0, 30.0),
                checkbox("inner_bevel", "Inner bevel", true),
            ]
        }),
        Def::new(
            "matrix_digital_rain",
            "Matrix digital rain",
            F,
            l::TERMINAL,
            "Overlays a Matrix-style digital rain effect using source luminance.",
            matrix_rain,
        )
        .params(|| {
            vec![
                int_slider("cell_size", "Cell size", 6, 24, 12),
                float_slider("density", "Density", 0.0, 100.0, 85.0),
                int_slider("trail_length", "Trail length", 3, 50, 12),
                float_slider("glow_amount", "Glow amount", 0.0, 100.0, 40.0),
                float_slider("source_blend", "Source blend", 0.0, 100.0, 22.0),
                float_slider("rain_offset", "Rain offset", 0.0, 100.0, 0.0),
                float_slider("luminance_influence", "Luminance influence", 0.0, 100.0, 65.0),
                text("character_set", "Character set", MATRIX_CHARS),
            ]
        }),
        Def::new(
            "mean_removal",
            "Mean removal",
            F,
            l::SIGMA,
            "Removes the mean value from colors.",
            |s, _| {
                Ok(convolve3x3(
                    s,
                    &[-1.0, -1.0, -1.0, -1.0, 9.0, -1.0, -1.0, -1.0, -1.0],
                    1.0,
                    0.0,
                ))
            },
        )
        .immediate(),
        Def::new(
            "median_filter",
            "Median filter",
            F,
            l::FILTER,
            "Applies a median filter to reduce noise while preserving edges.",
            median,
        )
        .params(|| vec![int_slider("radius", "Radius", 1, 5, 1)]),
        Def::new(
            "minecraft_grass",
            "Minecraft grass",
            F,
            l::FLOWER,
            "Adds a Minecraft-style pixelated grass border to the top or bottom of the image.",
            minecraft_grass,
        )
        .params(|| {
            vec![
                int_slider("block_size", "Block size", 4, 64, 16),
                int_slider("grass_rows", "Grass rows", 1, 8, 2),
                int_slider("dirt_rows", "Dirt rows", 1, 16, 4),
                checkbox("top_edge", "Top edge", false),
                int_slider("overlap", "Overlap", 0, 8, 1),
                color("grass_color", "Grass color", C::rgb(89, 176, 48)),
                color("dirt_color", "Dirt color", C::rgb(134, 96, 67)),
            ]
        }),
        Def::new(
            "miniature",
            "Miniature",
            F,
            l::BOX,
            "Creates a toy miniature look by blurring top and bottom with boosted saturation.",
            miniature,
        )
        .params(|| {
            vec![
                int_slider("blur_radius", "Blur radius", 2, 40, 15),
                float_slider("focus_position", "Focus position %", 10.0, 90.0, 50.0),
                float_slider("focus_band", "Focus band %", 5.0, 50.0, 20.0),
                float_slider("saturation", "Saturation boost", 0.0, 50.0, 20.0),
            ]
        }),
    ]
}

fn light_leak(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let alpha = (p.f("intensity") / 100.0).clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w as f32, src.h as f32);
    let center = match p.e("position").as_str() {
        "TopLeft" => (0.0, 0.0),
        "BottomLeft" => (0.0, h),
        "BottomRight" => (w, h),
        "Left" => (0.0, h / 2.0),
        "Right" => (w, h / 2.0),
        _ => (w, 0.0),
    };
    let radius = (w * w + h * h).sqrt() * 0.8;
    let leak = p.c("color").with_alpha((255.0 * alpha) as u8);
    let mut canvas = Canvas::from_bitmap(src);
    let paint = Paint::default()
        .with_shader(Shader::Radial {
            center,
            radius,
            colors: vec![leak, C::TRANSPARENT],
            pos: Some(vec![0.0, 1.0]),
            tile: Tile::Clamp,
        })
        .with_blend(Blend::Screen);
    canvas.draw_rect_xywh(0.0, 0.0, w, h, &paint);
    Ok(canvas.into_bitmap())
}

fn lum01(c: C) -> f32 {
    (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0
}

fn liquid_glass(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let dist = p.f("distortion").clamp(0.0, 35.0);
    let refraction = p.f("refraction").clamp(0.0, 100.0) / 100.0;
    let chroma = p.i("chroma_shift").clamp(0, 8);
    let gloss = p.f("gloss").clamp(0.0, 100.0) / 100.0;
    let flow = p.f("flow_scale").clamp(40.0, 220.0) / 100.0;
    if dist <= 0.0 && refraction <= 0.0 && chroma <= 0 && gloss <= 0.0 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let sp = &src.px;
    let seed = 9421;
    let fx = 0.072 / flow;
    let fy = 0.061 / flow;
    let mut px = Vec::with_capacity(sp.len());
    for y in 0..h {
        let row = y * w;
        let yt = if y > 0 { y - 1 } else { 0 };
        let yb = if y < h - 1 { y + 1 } else { h - 1 };
        for x in 0..w {
            let xl = if x > 0 { x - 1 } else { 0 };
            let xr = if x < w - 1 { x + 1 } else { w - 1 };
            let cell = hash01(x / 18, y / 18, seed) * 2.0 - 1.0;
            let (xf, yf) = (x as f32, y as f32);
            let w1 = (xf * fx + yf * (fy * 0.45) + cell * 2.4 + seed as f32 * 0.001).sin();
            let w2 = (xf * (fx * 0.58) - yf * (fy * 1.12) + cell * 1.7).cos();
            let w3 = ((x + y) as f32 * (fx * 0.62) + cell * 3.1).sin();
            let mut ox = dist * (0.52 * w1 + 0.26 * w3);
            let mut oy = dist * (0.46 * w2 - 0.22 * w3);
            if refraction > 0.0 {
                let gx = lum01(sp[(row + xr) as usize]) - lum01(sp[(row + xl) as usize]);
                let gy = lum01(sp[(yb * w + x) as usize]) - lum01(sp[(yt * w + x) as usize]);
                let edge = (gx * gx + gy * gy).sqrt();
                let boost = 1.0 + edge * 3.2 * refraction;
                ox = ox * boost + gx * refraction * dist * 3.0;
                oy = oy * boost + gy * refraction * dist * 3.0;
            }
            let sx = xf + ox;
            let sy = yf + oy;
            let (mut r, mut g, mut b, a);
            if chroma > 0 {
                let shift = chroma as f32 * 0.75;
                let cr = bilinear_sample(sp, w, h, sx + shift, sy - shift * 0.15);
                let cg = bilinear_sample(sp, w, h, sx, sy);
                let cb = bilinear_sample(sp, w, h, sx - shift, sy + shift * 0.15);
                r = cr.r as f32;
                g = cg.g as f32;
                b = cb.b as f32;
                a = cg.a;
            } else {
                let c = bilinear_sample(sp, w, h, sx, sy);
                r = c.r as f32;
                g = c.g as f32;
                b = c.b as f32;
                a = c.a;
            }
            if refraction > 0.0 {
                let n1 = bilinear_sample(sp, w, h, sx + 1.0, sy);
                let n2 = bilinear_sample(sp, w, h, sx - 1.0, sy);
                let n3 = bilinear_sample(sp, w, h, sx, sy + 1.0);
                let n4 = bilinear_sample(sp, w, h, sx, sy - 1.0);
                let br = (n1.r as f32 + n2.r as f32 + n3.r as f32 + n4.r as f32) * 0.25;
                let bg = (n1.g as f32 + n2.g as f32 + n3.g as f32 + n4.g as f32) * 0.25;
                let bb = (n1.b as f32 + n2.b as f32 + n3.b as f32 + n4.b as f32) * 0.25;
                let mix = 0.06 + 0.18 * refraction;
                r = lerp(r, br, mix);
                g = lerp(g, bg, mix);
                b = lerp(b, bb, mix);
            }
            let ridge = (w1 - w2).abs();
            let micro = w3.abs();
            let spec = (1.0 - ridge * 1.35).max(0.0).powf(6.0);
            let streak = (1.0 - micro * 1.55).max(0.0).powf(4.0);
            let lum = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0;
            let hl = gloss * (spec * 0.62 + streak * 0.24 + lum.powf(2.8) * 0.14);
            r += hl * 88.0;
            g += hl * 102.0;
            b += hl * 126.0;
            let tint = (0.04 + 0.10 * refraction) * (0.4 + 0.6 * gloss);
            r = lerp(r, r + 6.0, tint);
            g = lerp(g, g + 12.0, tint);
            b = lerp(b, b + 22.0, tint);
            px.push(C::new(
                clamp_to_byte(r),
                clamp_to_byte(g),
                clamp_to_byte(b),
                a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn mercury_env(u: f32, v: f32) -> f32 {
    let a = (u * 11.5 + v * 2.1 + 0.7).sin();
    let b = (u * 18.5 - v * 8.2 - 1.1).cos();
    let c = ((u * 0.82 + v * 1.28) * 12.0 + 2.6).sin();
    (a * 0.42 + b * 0.36 + c * 0.22) * 0.5 + 0.5
}

fn liquid_mercury(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let reflection = p.f("reflection").clamp(0.0, 100.0) / 100.0;
    let ripple = p.f("ripple").clamp(0.0, 100.0) / 100.0;
    let shine = p.f("shine").clamp(0.0, 100.0) / 100.0;
    let fluidity = p.f("fluidity").clamp(0.0, 100.0) / 100.0;
    let depth = p.f("depth").clamp(0.0, 100.0) / 100.0;
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let iw = 1.0 / 1.max(w - 1) as f32;
    let ih = 1.0 / 1.max(h - 1) as f32;
    let sp = &src.px;
    let lum = |c: C| 0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32;
    let mut px = Vec::with_capacity(sp.len());
    for y in 0..h {
        let row = y * w;
        let v = y as f32 * ih;
        for x in 0..w {
            let s = sp[(row + x) as usize];
            let l = lum(s) / 255.0;
            let ll = lum(sp[(row + 0.max(x - 1)) as usize]) / 255.0;
            let lr = lum(sp[(row + (w - 1).min(x + 1)) as usize]) / 255.0;
            let lt = lum(sp[(0.max(y - 1) * w + x) as usize]) / 255.0;
            let lb = lum(sp[((h - 1).min(y + 1) * w + x) as usize]) / 255.0;
            let gx = lr - ll;
            let gy = lb - lt;
            let u = x as f32 * iw;
            let wa = (u * 12.0 + v * 6.5 + 0.8).sin();
            let wb = (u * 7.5 - v * 14.0 - 1.4).cos();
            let field = (wa * 0.6 + wb * 0.4) * ripple;
            let mut nx = -gx * (1.35 + depth * 3.8) + field * 0.45;
            let mut ny = -gy * (1.35 + depth * 3.8) + field * 0.22;
            let mut nz = 1f32;
            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            if len <= 0.0001 {
                nx = 0.0;
                ny = 0.0;
                nz = 1.0;
            } else {
                let inv = 1.0 / len;
                nx *= inv;
                ny *= inv;
                nz *= inv;
            }
            let ru = u + nx * (0.55 + fluidity * 0.55) + wb * fluidity * 0.05;
            let rv = v - ny * (0.85 + fluidity * 0.45) + wa * fluidity * 0.03;
            let env = mercury_env(ru, rv);
            let diffuse = clamp01(nz * 0.65 + 0.15);
            let fresnel = (1.0 - clamp01(nz)).powf(2.7);
            let specular = clamp01(nx * -0.22 + ny * -0.32 + nz * 0.92).powf(12.0 + shine * 42.0);
            let mut metal = clamp01(l * 0.20 + diffuse * 0.26 + env * (0.32 + reflection * 0.42));
            metal = clamp01((metal - 0.5) * (1.15 + depth * 1.55) + 0.5);
            let mut silver = lerp(0.06, 0.94, metal);
            silver = lerp(silver, 1.0, specular * (0.34 + shine * 0.46));
            silver = lerp(silver, 1.0, fresnel * (0.20 + reflection * 0.22));
            let cool = 0.01 + env * 0.03;
            px.push(C::new(
                clamp_to_byte(clamp01(silver - cool) * 255.0),
                clamp_to_byte(clamp01(silver) * 255.0),
                clamp_to_byte(clamp01(silver + cool * 1.25) * 255.0),
                s.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn contour_lines(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let levels = p.i("levels").clamp(2, 64);
    let bg = p.f("background_strength").clamp(0.0, 100.0) / 100.0;
    let ls = p.f("line_strength").clamp(0.0, 100.0) / 100.0;
    if ls <= 0.0 && bg <= 0.0 {
        return Ok(src.clone());
    }
    let bias = p.f("threshold").clamp(0.0, 255.0) / 255.0;
    let lw = p.f("line_width").clamp(0.0, 200.0) / 200.0;
    let feather = 0.04 + 0.22 * lw;
    let lc = p.c("line_color");
    let la = lc.a as f32 / 255.0;
    let (lr, lg, lb) = (
        lc.r as f32 / 255.0,
        lc.g as f32 / 255.0,
        lc.b as f32 / 255.0,
    );
    let invert = p.b("invert");
    if src.w <= 0 || src.h <= 0 {
        return Ok(src.clone());
    }
    let px = src
        .px
        .iter()
        .map(|s| {
            let lum = clamp01(lum01(*s) + bias);
            let scaled = lum * levels as f32;
            let qi = scaled.floor();
            let frac = scaled - qi;
            let ql = clamp01((qi + 0.5) / levels as f32);
            let d = frac.min(1.0 - frac);
            let t = smooth_step(0.0, feather, d);
            let mut mask = 1.0 - t;
            mask *= mask;
            if invert {
                mask = 1.0 - mask;
            }
            let bl = clamp01(lerp(lum, ql, bg));
            let mix = mask * ls * la;
            C::new(
                clamp_to_byte(lerp(bl, lr, mix) * 255.0),
                clamp_to_byte(lerp(bl, lg, mix) * 255.0),
                clamp_to_byte(lerp(bl, lb, mix) * 255.0),
                s.a,
            )
        })
        .collect();
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}

const TITLE_BAR: i32 = 38;
const BUTTON_RADIUS: f32 = 6.0;

fn macos_window(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let radius = p.i("corner_radius").clamp(0, 30) as f32;
    let shadow = p.i("shadow_size").clamp(0, 60);
    let padding = p.i("padding").clamp(0, 100);
    let dark = p.b("dark_mode");
    let title = p.s("title");
    let ww = src.w;
    let wh = TITLE_BAR + src.h;
    let tw = ww + padding * 2 + shadow * 2;
    let th = wh + padding * 2 + shadow * 2;
    let mut canvas = Canvas::new(tw, th);
    let wx = (padding + shadow) as f32;
    let wy = (padding + shadow) as f32;
    let window = rect(wx, wy, wx + ww as f32, wy + wh as f32);
    if shadow > 0 {
        let paint = Paint::fill_aa(C::new(0, 0, 0, 60)).with_mask_blur(shadow as f32 / 2.0);
        canvas.draw_round_rect(window, radius, radius, &paint);
    }
    canvas.save();
    canvas.clip_round_rect(window, radius, radius, true);
    let (bar, bar_border) = if dark {
        (C::rgb(56, 55, 57), C::rgb(45, 44, 46))
    } else {
        (C::rgb(232, 232, 232), C::rgb(208, 208, 208))
    };
    canvas.draw_rect_xywh(wx, wy, ww as f32, TITLE_BAR as f32, &Paint::fill_aa(bar));
    let by = wy + TITLE_BAR as f32;
    canvas.draw_line(
        wx,
        by,
        wx + ww as f32,
        by,
        &Paint::stroke_aa(bar_border, 1.0),
    );
    let button_y = wy + TITLE_BAR as f32 / 2.0;
    let button_x = wx + 14.0 + BUTTON_RADIUS;
    for (i, c) in [
        C::rgb(255, 95, 87),
        C::rgb(255, 189, 46),
        C::rgb(39, 201, 63),
    ]
    .into_iter()
    .enumerate()
    {
        let x = button_x + 20.0 * i as f32;
        canvas.draw_circle(x, button_y, BUTTON_RADIUS, &Paint::fill_aa(c));
        let edge = C::new(
            (c.r as f32 * 0.8) as u8,
            (c.g as f32 * 0.8) as u8,
            (c.b as f32 * 0.8) as u8,
            200,
        );
        canvas.draw_circle(x, button_y, BUTTON_RADIUS, &Paint::stroke_aa(edge, 0.5));
    }
    if !title.trim().is_empty() {
        let color = if dark {
            C::rgb(220, 220, 220)
        } else {
            C::rgb(74, 74, 74)
        };
        let font = Font::new("San Francisco", 13.0);
        let ty = wy + TITLE_BAR as f32 / 2.0 + 13.0 / 3.0;
        let tx = wx + ww as f32 / 2.0 - font.measure(&title) / 2.0;
        canvas.draw_text(&title, tx, ty, &font, &Paint::fill_aa(color));
    }
    canvas.draw_bitmap(src, wx, wy + TITLE_BAR as f32, &Paint::default());
    canvas.restore();
    let border = if dark {
        C::new(70, 70, 70, 180)
    } else {
        C::new(180, 180, 180, 180)
    };
    canvas.draw_round_rect(window, radius, radius, &Paint::stroke_aa(border, 1.0));
    Ok(canvas.into_bitmap())
}

fn shade(c: C, amount: f32) -> C {
    C::new(
        clamp_to_byte((c.r as f32 / 255.0 + amount) * 255.0),
        clamp_to_byte((c.g as f32 / 255.0 + amount) * 255.0),
        clamp_to_byte((c.b as f32 / 255.0 + amount) * 255.0),
        c.a,
    )
}

fn marble_border(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(2, 300);
    let vs = p.f("vein_strength").clamp(0.0, 100.0) / 100.0;
    let scale = p.f("vein_scale").clamp(5.0, 100.0) / 1000.0;
    let (base, vein) = (p.c("base_color"), p.c("vein_color"));
    let bevel = p.b("inner_bevel");
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let mut px = vec![C::ZERO; (nw * nh) as usize];
    let zone = border as f32 * 0.4;
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
            let (xf, yf) = (x as f32, y as f32);
            let turb = fractal_noise(xf * scale, yf * scale, 6, 2.0, 0.5, 42);
            let diag = (x + y) as f32 * scale * 2.0;
            let mut m = (diag + vs * turb * 20.0).sin();
            m = (m + 1.0) * 0.5;
            m = m.powf(1.5);
            let mut c = C::new(
                clamp_to_byte(base.r as f32 + (vein.r as f32 - base.r as f32) * m),
                clamp_to_byte(base.g as f32 + (vein.g as f32 - base.g as f32) * m),
                clamp_to_byte(base.b as f32 + (vein.b as f32 - base.b as f32) * m),
                255,
            );
            if bevel {
                let mut d = f32::MAX;
                if xf < il {
                    d = d.min(il - xf);
                }
                if xf >= ir {
                    d = d.min(xf - (ir - 1.0));
                }
                if yf < it {
                    d = d.min(it - yf);
                }
                if yf >= ib {
                    d = d.min(yf - (ib - 1.0));
                }
                let t = 1.0 - (d / zone).clamp(0.0, 1.0);
                let depth = smooth_step(0.0, 1.0, t);
                if depth > 0.0 {
                    c = shade(c, -depth * 0.25);
                }
            }
            px[(y * nw + x) as usize] = c;
        }
    }
    let mut canvas = Canvas::from_bitmap(&Bitmap::from_pixels(nw, nh, px));
    let b = border as f32;
    canvas.draw_bitmap(src, b, b, &Paint::default());
    canvas.draw_rect_xywh(
        b - 0.75,
        b - 0.75,
        src.w as f32 + 1.5,
        src.h as f32 + 1.5,
        &Paint::stroke_aa(shade(vein, -0.3), 1.5),
    );
    Ok(canvas.into_bitmap())
}

const MATRIX_CHARS: &str = "01<>[]{}*+-/\\=#$%&";

fn matrix_rain(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let cell = p.i("cell_size").clamp(6, 24);
    let density = p.f("density").clamp(0.0, 100.0) / 100.0;
    let trail = p.i("trail_length").clamp(3, 50);
    let glow = p.f("glow_amount").clamp(0.0, 100.0) / 100.0;
    let blend = p.f("source_blend").clamp(0.0, 100.0) / 100.0;
    let offset = p.f("rain_offset").clamp(0.0, 100.0) / 100.0;
    let li = p.f("luminance_influence").clamp(0.0, 100.0) / 100.0;
    let set = p.s("character_set");
    let chars: Vec<u16> = if set.trim().is_empty() {
        MATRIX_CHARS.encode_utf16().collect()
    } else {
        let t = set.trim();
        if t.encode_utf16().count() == 1 {
            format!("{t}0").encode_utf16().collect()
        } else {
            t.encode_utf16().collect()
        }
    };
    let n = chars.len();
    let seed = 1337;
    let columns = (w as f32 / cell as f32).ceil() as i32;
    let rows = (h as f32 / cell as f32).ceil() as i32;
    let mut canvas = Canvas::new(w, h);
    canvas.draw_rect(
        rect(0.0, 0.0, w as f32, h as f32),
        &Paint::fill(C::rgb(0, 12, 0)),
    );
    if blend > 0.0 {
        canvas.draw_bitmap(src, 0.0, 0.0, &Paint::alpha(clamp_to_byte(blend * 255.0)));
    }
    let font = Font::new("Consolas", cell as f32 * 1.02);
    let glow_sigma = 2.2 + glow * 3.4;
    let baseline = cell as f32 * 0.88;
    let trail_factor = |row: i32, head: f32, len: f32| -> f32 {
        let mut d = row as f32 - head;
        if d < 0.0 {
            d += rows as f32;
        }
        if d < 0.0 || d > len {
            return 0.0;
        }
        1.0 - d / len.max(1.0)
    };
    for col in 0..columns {
        let head = ((hash01(col, seed, seed ^ 9) + offset) % 1.0) * rows as f32;
        let head2 = ((hash01(col, seed ^ 37, seed ^ 101) + offset * 0.63) % 1.0) * rows as f32;
        for row in 0..rows {
            let (x0, y0) = (col * cell, row * cell);
            if x0 >= w || y0 >= h {
                continue;
            }
            let (x1, y1) = (w.min(x0 + cell), h.min(y0 + cell));
            let (mut sl, mut sa, mut count) = (0f32, 0f32, 0);
            for y in y0..y1 {
                for x in x0..x1 {
                    let c = src.get(x, y);
                    sl += lum01(c);
                    sa += c.a as f32;
                    count += 1;
                }
            }
            let (lum, alpha) = if count == 0 {
                (0.0, 255)
            } else {
                let inv = 1.0 / count as f32;
                (sl * inv, clamp_to_byte(sa * inv))
            };
            let spawn = hash01(col, row, seed ^ 503);
            let local = density * (0.35 + 0.65 * (li * lum + (1.0 - li)));
            if spawn > local {
                continue;
            }
            let t1 = trail_factor(row, head, trail as f32);
            let t2 = trail_factor(row, head2, trail as f32 * 0.75);
            let tf = t1.max(t2);
            let base = lum * li + 0.35 * (1.0 - li);
            let intensity = (base * 0.55 + tf * 0.9).clamp(0.0, 1.2);
            if intensity <= 0.03 {
                continue;
            }
            let mut ci = ((lum * n as f32 + row as f32 * 0.61 + col as f32 * 0.17) % n as f32)
                .floor()
                .abs() as usize;
            if ci >= n {
                ci = n - 1;
            }
            let glyph = char::from_u32(chars[ci] as u32)
                .unwrap_or('\u{FFFD}')
                .to_string();
            let gx = x0 as f32;
            let gy = y0 as f32 + baseline;
            let gs = glow * (intensity - 0.35).max(0.0);
            if gs > 0.0 {
                let mut paint = Paint::fill_aa(C::new(
                    clamp_to_byte(56.0 + intensity * 72.0),
                    clamp_to_byte(180.0 + intensity * 60.0),
                    clamp_to_byte(56.0 + intensity * 70.0),
                    clamp_to_byte(gs * 180.0),
                ));
                if glow > 0.0 {
                    paint = paint.with_mask_blur(glow_sigma);
                }
                canvas.draw_text(&glyph, gx, gy, &font, &paint);
            }
            let paint = Paint::fill_aa(C::new(
                clamp_to_byte(28.0 + intensity * 118.0),
                clamp_to_byte(120.0 + intensity * 135.0),
                clamp_to_byte(28.0 + intensity * 118.0),
                alpha,
            ));
            canvas.draw_text(&glyph, gx, gy, &font, &paint);
        }
    }
    Ok(canvas.into_bitmap())
}

fn median(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let radius = p.i("radius").clamp(1, 5);
    let (w, h) = (src.w, src.h);
    let n = ((radius * 2 + 1) * (radius * 2 + 1)) as usize;
    let (mut rv, mut gv, mut bv, mut av) = (
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    );
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            rv.clear();
            gv.clear();
            bv.clear();
            av.clear();
            for ky in -radius..=radius {
                let sy = (y + ky).clamp(0, h - 1);
                for kx in -radius..=radius {
                    let c = src.get((x + kx).clamp(0, w - 1), sy);
                    rv.push(c.r);
                    gv.push(c.g);
                    bv.push(c.b);
                    av.push(c.a);
                }
            }
            rv.sort_unstable();
            gv.sort_unstable();
            bv.sort_unstable();
            av.sort_unstable();
            let mid = rv.len() >> 1;
            px.push(C::new(rv[mid], gv[mid], bv[mid], av[mid]));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn minecraft_grass(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let bs = p.i("block_size").clamp(4, 64);
    let grass_rows = p.i("grass_rows").clamp(1, 8);
    let dirt_rows = p.i("dirt_rows").clamp(1, 16);
    let top = p.b("top_edge");
    let (grass, dirt) = (p.c("grass_color"), p.c("dirt_color"));
    let total = grass_rows + dirt_rows;
    let border_h = total * bs;
    let overlap = p.i("overlap").clamp(0, 8) * bs;
    let nh = src.h + border_h - overlap;
    let mut canvas = Canvas::new(src.w, nh);
    let image_y = if top { border_h - overlap } else { 0 };
    canvas.draw_bitmap(src, 0.0, image_y as f32, &Paint::default());
    let border_y = if top { 0 } else { src.h - overlap };
    let cols = (src.w + bs - 1) / bs;
    let ch = |v: i32| v.clamp(0, 255) as u8;
    for col in 0..cols {
        let jagged = hash01(col, 0, 42) > 0.5;
        for row in 0..total {
            let x = col * bs;
            let y = if top {
                border_h - (row + 1) * bs
            } else {
                border_y + row * bs
            };
            let from_top = if top { total - 1 - row } else { row };
            if from_top == 0 && jagged {
                continue;
            }
            let variation = hash01(col, row, 137);
            let base = if from_top < grass_rows {
                let rv = (variation * 20.0 - 10.0) as i32;
                let gv = (variation * 30.0 - 15.0) as i32;
                C::rgb(
                    ch(grass.r as i32 + rv),
                    ch(grass.g as i32 + gv),
                    ch(grass.b as i32 + rv),
                )
            } else {
                let dv = (variation * 24.0 - 12.0) as i32;
                C::rgb(
                    ch(dirt.r as i32 + dv),
                    ch(dirt.g as i32 + dv),
                    ch(dirt.b as i32 + dv + 4),
                )
            };
            let dw = bs.min(src.w - x);
            if dw <= 0 {
                continue;
            }
            canvas.draw_rect_xywh(x as f32, y as f32, dw as f32, bs as f32, &Paint::fill(base));
            if hash01(col, row + 100, 271) > 0.6 {
                let ss = 1.max(bs / 4);
                let sx = x as f32 + hash01(col, row, 311) * (bs - ss) as f32;
                let sy = y as f32 + hash01(col, row, 419) * (bs - ss) as f32;
                let dark = C::rgb(
                    ch(base.r as i32 - 25),
                    ch(base.g as i32 - 20),
                    ch(base.b as i32 - 20),
                );
                canvas.draw_rect_xywh(sx, sy, ss as f32, ss as f32, &Paint::fill(dark));
            }
        }
    }
    Ok(canvas.into_bitmap())
}

fn miniature(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let radius = 1.max(p.i("blur_radius")) as f32;
    let (w, h) = (src.w, src.h);
    let blurred = blur_bitmap(src, radius, radius);
    let focus = h as f32 * p.f("focus_position") / 100.0;
    let half = h as f32 * p.f("focus_band") / 200.0;
    let (top, bottom) = (focus - half, focus + half);
    let boost = 1.0 + p.f("saturation") / 100.0;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let yf = y as f32;
        let t = if yf < top {
            1.0 - ((top - yf) / (top + 1.0)).clamp(0.0, 1.0)
        } else if yf > bottom {
            1.0 - ((yf - bottom) / (h as f32 - bottom + 1.0)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let sw = t * t * (3.0 - 2.0 * t);
        for x in 0..w {
            let s = src.get(x, y);
            let b = blurred.get(x, y);
            let mix = |a: u8, b: u8| (a as f32 * sw + b as f32 * (1.0 - sw)) as u8;
            let m = C::new(mix(s.r, b.r), mix(s.g, b.g), mix(s.b, b.b), s.a);
            let (hue, sat, lum) = m.to_hsl();
            px.push(C::from_hsl(hue, (sat * boost).clamp(0.0, 100.0), lum, s.a));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}
