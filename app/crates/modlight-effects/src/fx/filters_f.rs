//! Filters, part F: Frosted glass + ice edges .. Heat haze refraction.

use std::f32::consts::PI;

use modlight_core::icons as l;
use modlight_editor::effects::Category;

use crate::def::*;
use crate::dotnet::DotNetRandom;
use crate::fx::filters_b::padded_blur;
use crate::helpers::{
    bilinear_sample, blur_bitmap, blurred_clamp, clamp_to_byte, clamp01, hash01, lerp, smooth_step,
};
use crate::sk::{Bitmap, Blend, C, Canvas, Cap, ColorFilter, Paint, Shader, Style, Tile};

const F: Category = Category::Filters;

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "frosted_glass_ice_edges",
            "Frosted glass + ice edges",
            F,
            l::SNOWFLAKE,
            "Combines frosted glass distortion with icy edge highlights and cool tinting.",
            frosted_ice,
        )
        .params(|| {
            vec![
                float_slider("distortion", "Distortion", 0.0, 30.0, 10.0),
                float_slider("blur", "Blur", 0.0, 100.0, 35.0),
                int_slider("edge_threshold", "Edge threshold", 1, 120, 28),
                float_slider("ice_strength", "Ice strength", 0.0, 100.0, 75.0),
                float_slider("tint", "Tint", 0.0, 100.0, 35.0),
            ]
        }),
        Def::new(
            "frosted_glass",
            "Frosted glass",
            F,
            l::SNOWFLAKE,
            "Simulates looking through frosted glass by randomizing nearby pixels.",
            frosted_glass,
        )
        .params(|| vec![int_slider("radius", "Radius", 1, 20, 5)]),
        Def::new(
            "gaussian_blur",
            "Gaussian blur",
            F,
            l::CIRCLE_GAUGE,
            "Applies a Gaussian blur effect.",
            |s, p| {
                let r = 1.max(p.i("radius"));
                Ok(padded_blur(s, r * 2, r as f32 / 3.0))
            },
        )
        .params(|| vec![int_slider("radius", "Radius", 1, 200, 15)]),
        Def::new(
            "ghost_trail",
            "Ghost trail",
            F,
            l::GHOST,
            "Creates fading ghost copies offset from the original image.",
            ghost_trail,
        )
        .params(|| {
            vec![
                int_slider("copies", "Copies", 1, 8, 3),
                int_slider("offset_x", "Offset X", -50, 50, 15),
                int_slider("offset_y", "Offset Y", -50, 50, 5),
                float_slider("fade", "Fade", 10.0, 90.0, 50.0),
            ]
        }),
        Def::new(
            "glare",
            "Glare",
            F,
            l::SPARKLE,
            "Adds a bright glare/light flare at a specified position.",
            glare,
        )
        .params(|| {
            vec![
                float_slider("position_x", "Position X (%)", 0.0, 100.0, 50.0),
                float_slider("position_y", "Position Y (%)", 0.0, 100.0, 30.0),
                float_slider("radius", "Radius", 5.0, 100.0, 30.0),
                float_slider("intensity", "Intensity", 0.0, 100.0, 80.0),
                int_slider("ray_count", "Ray count", 0, 12, 6),
                float_slider("ray_length", "Ray length", 0.0, 100.0, 50.0),
                color("color", "Color", C::WHITE),
            ]
        }),
        Def::new(
            "glitch_shift",
            "Glitch shift",
            F,
            l::ZAP,
            "Randomly shifts horizontal bands of the image for a digital glitch look.",
            glitch_shift,
        )
        .params(|| {
            vec![
                int_slider("intensity", "Intensity", 1, 100, 30),
                int_slider("bands", "Band count", 2, 50, 12),
            ]
        }),
        Def::new(
            "glow",
            "Glow",
            F,
            l::LIGHTBULB,
            "Applies a glowing effect.",
            glow,
        )
        .params(|| {
            vec![
                int_slider("size", "Size", 1, 100, 20).snap(false),
                float_slider("strength", "Strength", 1.0, 100.0, 80.0)
                    .snap(false)
                    .fmt("{}{0:0}%"),
                int_slider("offset_x", "Offset X", -100, 100, 0).snap(false),
                int_slider("offset_y", "Offset Y", -100, 100, 0).snap(false),
                color("color", "Color", C::WHITE),
                checkbox("auto_resize", "Auto resize", true),
            ]
        }),
        Def::new(
            "golden_border",
            "Golden border",
            F,
            l::CROWN,
            "Adds a luxurious picture frame border to the image.",
            golden_border,
        )
        .params(|| {
            vec![
                int_slider("size", "Size", 5, 80, 25),
                float_slider("bevel_strength", "Bevel strength", 0.0, 100.0, 60.0),
                color("color", "Color", C::rgb(212, 175, 55)),
                checkbox("inner_border", "Inner border", true),
                checkbox("outer_border", "Outer border", true),
            ]
        }),
        Def::new(
            "gradient_fade_border",
            "Gradient fade border",
            F,
            l::BLEND,
            "Adds a multi-stop gradient border that smoothly fades between configurable colors.",
            gradient_fade,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 10, 300, 50),
                color("outer_color", "Outer color", C::rgb(10, 10, 35)),
                color("mid_color", "Mid color", C::rgb(60, 20, 120)),
                color("inner_color", "Inner color", C::rgb(180, 80, 200)),
                float_slider("mid_position", "Mid-color position %", 10.0, 90.0, 50.0),
                checkbox("glow_edge", "Inner glow edge", true),
            ]
        }),
        Def::new(
            "halation",
            "Halation",
            F,
            l::SUNRISE,
            "Simulates film halation where bright areas bleed warm light into surrounding regions.",
            halation,
        )
        .params(|| {
            vec![
                float_slider("threshold", "Threshold", 0.0, 100.0, 72.0),
                float_slider("radius", "Radius", 1.0, 100.0, 18.0),
                float_slider("strength", "Strength", 0.0, 150.0, 60.0),
                float_slider("warmth", "Warmth", 0.0, 100.0, 70.0),
            ]
        }),
        Def::new(
            "halftone",
            "Halftone",
            F,
            l::CIRCLE_DOT,
            "Converts the image into a CMYK halftone dot pattern.",
            halftone,
        )
        .params(|| {
            vec![
                int_slider("cell_size", "Cell size", 3, 24, 8),
                float_slider("dot_softness", "Dot softness", 0.0, 100.0, 18.0),
                float_slider("ink_strength", "Ink strength", 0.0, 100.0, 90.0),
                float_slider("source_blend", "Source blend", 0.0, 100.0, 20.0),
                float_slider("angle_offset", "Angle offset", -45.0, 45.0, 0.0),
            ]
        }),
        Def::new(
            "heat_haze_refraction",
            "Heat haze refraction",
            F,
            l::THERMOMETER_SUN,
            "Simulates heat haze refraction with wavy distortion.",
            heat_haze,
        )
        .params(|| {
            vec![
                float_slider("strength", "Strength", 0.0, 100.0, 45.0),
                float_slider("frequency", "Frequency", 0.0, 100.0, 40.0),
                float_slider("blur_radius", "Blur radius", 0.0, 200.0, 10.0),
                float_slider("offset", "Offset", -200.0, 200.0, 6.0),
                float_slider(
                    "luminance_influence",
                    "Luminance influence",
                    0.0,
                    100.0,
                    55.0,
                ),
            ]
        }),
    ]
}

fn frosted_ice(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let distortion = p.f("distortion").clamp(0.0, 30.0);
    let blur_mix = p.f("blur").clamp(0.0, 100.0) / 100.0;
    let thr = p.i("edge_threshold").clamp(1, 120) as f32;
    let ice = p.f("ice_strength").clamp(0.0, 100.0) / 100.0;
    let tint = p.f("tint").clamp(0.0, 100.0) / 100.0;
    let (w, h) = (src.w, src.h);
    let (right, bottom) = (w - 1, h - 1);
    let seed = 1337;
    let lum: Vec<f32> = src
        .px
        .iter()
        .map(|c| (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0)
        .collect();
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let row = y * w;
        let yu = if y > 0 { y - 1 } else { 0 };
        let yd = if y < bottom { y + 1 } else { bottom };
        for x in 0..w {
            let cx = hash01(x / 9, y / 9, seed) * 2.0 - 1.0;
            let cy = hash01(x / 9, y / 9, seed ^ 977) * 2.0 - 1.0;
            let fine = hash01(x, y, seed ^ 4099) * 2.0 - 1.0;
            let sx = x as f32 + (cx * 0.75 + fine * 0.25) * distortion;
            let sy = y as f32 + (cy * 0.75 - fine * 0.25) * distortion;
            let s = bilinear_sample(&src.px, w, h, sx, sy);
            let (mut r, mut g, mut b, mut a) = (s.r as f32, s.g as f32, s.b as f32, s.a as f32);
            if blur_mix > 0.001 {
                let s1 = bilinear_sample(&src.px, w, h, sx - 1.0, sy);
                let s2 = bilinear_sample(&src.px, w, h, sx + 1.0, sy);
                let s3 = bilinear_sample(&src.px, w, h, sx, sy - 1.0);
                let s4 = bilinear_sample(&src.px, w, h, sx, sy + 1.0);
                let avg = |f: fn(&C) -> u8| {
                    (f(&s1) as f32 + f(&s2) as f32 + f(&s3) as f32 + f(&s4) as f32) * 0.25
                };
                r = lerp(r, avg(|c| c.r), blur_mix * 0.85);
                g = lerp(g, avg(|c| c.g), blur_mix * 0.85);
                b = lerp(b, avg(|c| c.b), blur_mix * 0.85);
                a = lerp(a, avg(|c| c.a), blur_mix * 0.6);
            }
            let xl = if x > 0 { x - 1 } else { 0 };
            let xr = if x < right { x + 1 } else { right };
            let gx = (lum[(row + xr) as usize] - lum[(row + xl) as usize]).abs();
            let gy = (lum[(yd * w + x) as usize] - lum[(yu * w + x) as usize]).abs();
            let metric = (gx + gy) * 255.0;
            let edge = clamp01((metric - thr) / 1f32.max(255.0 - thr)) * ice;
            let cool = tint * 0.45 + edge * 0.55;
            r = lerp(r, r * 0.82 + 16.0, cool);
            g = lerp(g, g * 0.95 + 24.0, cool);
            b = lerp(b, b * 1.08 + 56.0, cool);
            if edge > 0.001 {
                let spec = hash01(x, y, seed ^ 12345).powf(18.0) * edge;
                let glow = edge * (0.55 + spec * 0.8);
                r += 120.0 * glow;
                g += 142.0 * glow;
                b += 175.0 * glow;
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

fn frosted_glass(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let r = 1.max(p.i("radius"));
    let (w, h) = (src.w, src.h);
    let mut rng = DotNetRandom::new(42);
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let ox = (x + rng.next_range(-r, r + 1)).clamp(0, w - 1);
            let oy = (y + rng.next_range(-r, r + 1)).clamp(0, h - 1);
            px.push(src.get(ox, oy));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn ghost_trail(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let copies = 1.max(p.i("copies"));
    let (ox, oy) = (p.i("offset_x"), p.i("offset_y"));
    let fade = (p.f("fade") / 100.0).clamp(0.1, 0.9);
    let mut canvas = Canvas::new(src.w, src.h);
    for i in (1..=copies).rev() {
        let alpha = (1.0 - fade).powf(i as f32);
        canvas.draw_bitmap(
            src,
            (ox * i) as f32,
            (oy * i) as f32,
            &Paint::alpha((255.0 * alpha) as u8),
        );
    }
    canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    Ok(canvas.into_bitmap())
}

fn glare(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let intensity = p.f("intensity").clamp(0.0, 100.0) / 100.0;
    if intensity <= 0.0 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let cx = w as f32 * p.f("position_x").clamp(0.0, 100.0) / 100.0;
    let cy = h as f32 * p.f("position_y").clamp(0.0, 100.0) / 100.0;
    let diag = ((w * w + h * h) as f32).sqrt();
    let radius = diag * p.f("radius").clamp(5.0, 100.0) / 200.0;
    let ray_len = diag * p.f("ray_length").clamp(0.0, 100.0) / 200.0;
    let color = p.c("color");
    let ga = (255.0 * intensity) as u8;
    let center = color.with_alpha(ga);
    let edge = color.with_alpha(0);
    let mut canvas = Canvas::from_bitmap(src);
    let paint = Paint::fill_aa(C::BLACK)
        .with_shader(Shader::Radial {
            center: (cx, cy),
            radius,
            colors: vec![center, color.with_alpha((ga as f32 * 0.4) as u8), edge],
            pos: Some(vec![0.0, 0.4, 1.0]),
            tile: Tile::Clamp,
        })
        .with_blend(Blend::Screen);
    canvas.draw_rect_xywh(0.0, 0.0, w as f32, h as f32, &paint);
    let rays = p.i("ray_count").clamp(0, 12);
    if rays > 0 && ray_len > 1.0 {
        let step = 360.0 / rays as f32;
        let rw = radius * 0.15;
        for i in 0..rays {
            let a = i as f32 * step * PI / 180.0;
            let (ex, ey) = (cx + a.cos() * ray_len, cy + a.sin() * ray_len);
            let rp = Paint {
                style: Style::Stroke,
                stroke_width: rw,
                cap: Cap::Round,
                ..Paint::fill_aa(C::BLACK)
            }
            .with_shader(Shader::Linear {
                p0: (cx, cy),
                p1: (ex, ey),
                colors: vec![center, edge],
                pos: Some(vec![0.0, 1.0]),
                tile: Tile::Clamp,
            })
            .with_blend(Blend::Screen);
            canvas.draw_line(cx, cy, ex, ey, &rp);
        }
    }
    Ok(canvas.into_bitmap())
}

fn glitch_shift(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let intensity = p.i("intensity");
    if intensity <= 0 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let mut dst = src.px.clone();
    let mut rng = DotNetRandom::new(42);
    let max_shift = w * intensity / 100;
    let bands = 2.max(p.i("bands"));
    for _ in 0..bands {
        let start = rng.next_max(h);
        let bh = rng.next_range(2, 3.max(h / bands));
        let shift = rng.next_range(-max_shift, max_shift + 1);
        for y in start..(start + bh).min(h) {
            for x in 0..w {
                let sx = x - shift;
                dst[(y * w + x) as usize] = if sx >= 0 && sx < w {
                    src.px[(y * w + sx) as usize]
                } else {
                    C::BLACK
                };
            }
        }
    }
    Ok(Bitmap::from_pixels(w, h, dst))
}

/// `GlowImageEffect(size, strength, color, offsetX, offsetY, autoResize)`.
pub fn glow_with(
    src: &Bitmap,
    size: i32,
    strength: f32,
    color: C,
    ox: i32,
    oy: i32,
    auto: bool,
) -> Bitmap {
    let pad = if auto { size } else { 0 };
    let el = if auto { 0.max(-ox) + pad } else { 0 };
    let er = if auto { 0.max(ox) + pad } else { 0 };
    let et = if auto { 0.max(-oy) + pad } else { 0 };
    let eb = if auto { 0.max(oy) + pad } else { 0 };
    let (nw, nh) = (src.w + el + er, src.h + et + eb);
    let mut canvas = Canvas::new(nw, nh);
    let glow = color.with_alpha((255.0 * strength / 100.0) as u8);
    let paint = Paint {
        color_filter: Some(ColorFilter::Blend(glow, Blend::SrcIn)),
        image_blur: Some((size as f32, size as f32)),
        ..Paint::default()
    };
    canvas.draw_bitmap(src, (el + ox) as f32, (et + oy) as f32, &paint);
    canvas.draw_bitmap(src, el as f32, et as f32, &Paint::default());
    canvas.into_bitmap()
}

fn glow(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    Ok(glow_with(
        src,
        p.i("size"),
        p.f("strength"),
        p.c("color"),
        p.i("offset_x"),
        p.i("offset_y"),
        p.b("auto_resize"),
    ))
}

fn darken(c: C, f: f32) -> C {
    C::new(
        (c.r as f32 * (1.0 - f)) as u8,
        (c.g as f32 * (1.0 - f)) as u8,
        (c.b as f32 * (1.0 - f)) as u8,
        c.a,
    )
}

fn lighten(c: C, f: f32) -> C {
    C::new(
        (c.r as f32 + (255 - c.r) as f32 * f).min(255.0) as u8,
        (c.g as f32 + (255 - c.g) as f32 * f).min(255.0) as u8,
        (c.b as f32 + (255 - c.b) as f32 * f).min(255.0) as u8,
        c.a,
    )
}

fn linear_paint(p0: (f32, f32), p1: (f32, f32), c0: C, c1: C) -> Paint {
    Paint::fill_aa(C::BLACK).with_shader(Shader::Linear {
        p0,
        p1,
        colors: vec![c0, c1],
        pos: None,
        tile: Tile::Clamp,
    })
}

fn golden_border(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let size = p.i("size").clamp(5, 80);
    let bevel = p.f("bevel_strength").clamp(0.0, 100.0) / 100.0;
    let base = p.c("color");
    let dark = darken(base, 0.55);
    let light = lighten(base, 0.35);
    let hl = lighten(base, 0.60);
    let (nw, nh) = (src.w + size * 2, src.h + size * 2);
    let (w, h, s) = (nw as f32, nh as f32, size as f32);
    let mut canvas = Canvas::new(nw, nh);
    let bp = Paint::fill_aa(base);
    canvas.draw_rect_xywh(0.0, 0.0, w, s, &bp);
    canvas.draw_rect_xywh(0.0, h - s, w, s, &bp);
    canvas.draw_rect_xywh(0.0, 0.0, s, h, &bp);
    canvas.draw_rect_xywh(w - s, 0.0, s, h, &bp);
    if bevel > 0.0 {
        let bw = s * 0.35 * bevel;
        let a = |v: f32| (v * bevel) as u8;
        canvas.draw_rect_xywh(
            0.0,
            0.0,
            w,
            bw,
            &linear_paint(
                (0.0, 0.0),
                (0.0, bw),
                hl.with_alpha(a(200.0)),
                base.with_alpha(0),
            ),
        );
        canvas.draw_rect_xywh(
            0.0,
            0.0,
            bw,
            h,
            &linear_paint(
                (0.0, 0.0),
                (bw, 0.0),
                hl.with_alpha(a(180.0)),
                base.with_alpha(0),
            ),
        );
        canvas.draw_rect_xywh(
            0.0,
            h - bw,
            w,
            bw,
            &linear_paint(
                (0.0, h),
                (0.0, h - bw),
                dark.with_alpha(a(200.0)),
                base.with_alpha(0),
            ),
        );
        canvas.draw_rect_xywh(
            w - bw,
            0.0,
            bw,
            h,
            &linear_paint(
                (w, 0.0),
                (w - bw, 0.0),
                dark.with_alpha(a(200.0)),
                base.with_alpha(0),
            ),
        );
        let ib = s * 0.25 * bevel;
        canvas.draw_rect_xywh(
            s,
            s - ib,
            w - s * 2.0,
            ib,
            &linear_paint(
                (0.0, s),
                (0.0, s - ib),
                dark.with_alpha(a(160.0)),
                base.with_alpha(0),
            ),
        );
        canvas.draw_rect_xywh(
            s - ib,
            s,
            ib,
            h - s * 2.0,
            &linear_paint(
                (s, 0.0),
                (s - ib, 0.0),
                dark.with_alpha(a(140.0)),
                base.with_alpha(0),
            ),
        );
        canvas.draw_rect_xywh(
            s,
            h - s,
            w - s * 2.0,
            ib,
            &linear_paint(
                (0.0, h - s),
                (0.0, h - s + ib),
                light.with_alpha(a(140.0)),
                base.with_alpha(0),
            ),
        );
        canvas.draw_rect_xywh(
            w - s,
            s,
            ib,
            h - s * 2.0,
            &linear_paint(
                (w - s, 0.0),
                (w - s + ib, 0.0),
                light.with_alpha(a(140.0)),
                base.with_alpha(0),
            ),
        );
        let gy = s * 0.5;
        let gd = Paint::stroke_aa(dark.with_alpha(a(120.0)), 1.0);
        canvas.draw_line(0.0, gy, w, gy, &gd);
        canvas.draw_line(0.0, h - gy, w, h - gy, &gd);
        canvas.draw_line(gy, 0.0, gy, h, &gd);
        canvas.draw_line(w - gy, 0.0, w - gy, h, &gd);
        let gl = Paint::stroke_aa(hl.with_alpha(a(80.0)), 1.0);
        canvas.draw_line(0.0, gy + 1.0, w, gy + 1.0, &gl);
        canvas.draw_line(0.0, h - gy + 1.0, w, h - gy + 1.0, &gl);
        canvas.draw_line(gy + 1.0, 0.0, gy + 1.0, h, &gl);
        canvas.draw_line(w - gy + 1.0, 0.0, w - gy + 1.0, h, &gl);
    }
    canvas.draw_bitmap(src, s, s, &Paint::default());
    if p.b("inner_border") {
        canvas.draw_rect_xywh(
            s - 1.0,
            s - 1.0,
            (src.w + 2) as f32,
            (src.h + 2) as f32,
            &Paint::stroke_aa(dark, 1.5),
        );
    }
    if p.b("outer_border") {
        canvas.draw_rect_xywh(0.5, 0.5, w - 1.0, h - 1.0, &Paint::stroke_aa(dark, 1.5));
    }
    Ok(canvas.into_bitmap())
}

fn gradient_fade(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(10, 300);
    let mid_t = p.f("mid_position").clamp(10.0, 90.0) / 100.0;
    let (oc, mc, ic) = (p.c("outer_color"), p.c("mid_color"), p.c("inner_color"));
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let lerp_c = |a: C, b: C, t: f32| {
        C::new(
            clamp_to_byte(a.r as f32 + (b.r as f32 - a.r as f32) * t),
            clamp_to_byte(a.g as f32 + (b.g as f32 - a.g as f32) * t),
            clamp_to_byte(a.b as f32 + (b.b as f32 - a.b as f32) * t),
            clamp_to_byte(a.a as f32 + (b.a as f32 - a.a as f32) * t),
        )
    };
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
            let dl = if left { border - x } else { 0 };
            let dr = if right { x - border - src.w + 1 } else { 0 };
            let dt = if top { border - y } else { 0 };
            let db = if bottom { y - border - src.h + 1 } else { 0 };
            let d = dl.max(dr).max(dt.max(db));
            let t = (d as f32 / border as f32).clamp(0.0, 1.0);
            px[(y * nw + x) as usize] = if t < mid_t {
                lerp_c(ic, mc, smooth_step(0.0, 1.0, t / mid_t))
            } else {
                lerp_c(mc, oc, smooth_step(0.0, 1.0, (t - mid_t) / (1.0 - mid_t)))
            };
        }
    }
    let base = Bitmap::from_pixels(nw, nh, px);
    let mut canvas = Canvas::from_bitmap(&base);
    let b = border as f32;
    canvas.draw_bitmap(src, b, b, &Paint::default());
    if p.b("glow_edge") {
        let gp = Paint {
            mask_blur: Some(4.0),
            ..Paint::stroke_aa(ic.with_alpha(120), 3.0)
        };
        canvas.draw_rect_xywh(b, b, src.w as f32, src.h as f32, &gp);
    }
    Ok(canvas.into_bitmap())
}

fn halation(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let threshold = p.f("threshold").clamp(0.0, 100.0) / 100.0;
    let radius = p.f("radius").clamp(1.0, 100.0);
    let strength = p.f("strength").clamp(0.0, 150.0) / 100.0;
    let warmth = p.f("warmth").clamp(0.0, 100.0) / 100.0;
    if strength <= 0.0 {
        return Ok(src.clone());
    }
    let low = (threshold - 0.18).clamp(0.0, 1.0);
    let high = (threshold + 0.12).clamp(0.0, 1.0);
    let rb = lerp(1.20, 1.85, warmth);
    let gbst = lerp(0.55, 0.85, warmth);
    let bb = lerp(0.35, 0.15, warmth);
    let halo: Vec<C> = src
        .px
        .iter()
        .map(|c| {
            let lum = (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0;
            let m = smooth_step(low, high, lum);
            if m <= 0.0 {
                return C::ZERO;
            }
            let gain = m * (0.40 + 0.60 * lum.powf(1.4));
            C::new(
                clamp_to_byte(c.r as f32 * gain * rb),
                clamp_to_byte(c.g as f32 * gain * gbst),
                clamp_to_byte(c.b as f32 * gain * bb),
                clamp_to_byte(c.a as f32 * m),
            )
        })
        .collect();
    let halo = Bitmap::from_pixels(src.w, src.h, halo);
    let blurred = blurred_clamp(&halo, radius);
    let ch = |s: u8, hn: f32| {
        let s = s as f32 / 255.0;
        let mut sc = 1.0 - (1.0 - s) * (1.0 - hn.clamp(0.0, 1.0));
        if hn > 1.0 {
            sc += (hn - 1.0) * 0.35;
        }
        clamp_to_byte(sc * 255.0)
    };
    let px = src
        .px
        .iter()
        .zip(&blurred.px)
        .map(|(s, hc)| {
            let hr = (hc.r as f32 / 255.0) * strength * (1.0 + warmth * 0.5);
            let hg = (hc.g as f32 / 255.0) * strength * (0.85 + warmth * 0.15);
            let hb = (hc.b as f32 / 255.0) * strength * (0.65 - warmth * 0.25);
            C::new(ch(s.r, hr), ch(s.g, hg), ch(s.b, hb), s.a)
        })
        .collect();
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}

fn halftone(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let cell = p.i("cell_size").clamp(3, 24);
    let soft = p.f("dot_softness").clamp(0.0, 100.0) / 100.0;
    let ink = p.f("ink_strength").clamp(0.0, 100.0) / 100.0;
    let blend = p.f("source_blend").clamp(0.0, 100.0) / 100.0;
    let off = p.f("angle_offset").clamp(-45.0, 45.0) * (PI / 180.0);
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let rot = |a: f32| (a.cos(), a.sin());
    let cr = rot(15.0 * (PI / 180.0) + off);
    let mr = rot(75.0 * (PI / 180.0) + off);
    let yr = rot(0.0 + off);
    let kr = rot(45.0 * (PI / 180.0) + off);
    let dot = |x: i32, y: i32, (ca, sa): (f32, f32), intensity: f32| -> f32 {
        if intensity <= 0.0001 {
            return 0.0;
        }
        let u = (x as f32 * ca + y as f32 * sa) / cell as f32;
        let v = (-(x as f32) * sa + y as f32 * ca) / cell as f32;
        let fu = (u - u.floor()) - 0.5;
        let fv = (v - v.floor()) - 0.5;
        let d = (fu * fu + fv * fv).sqrt();
        let radius = 0.5 * intensity.clamp(0.0, 1.0).sqrt();
        let s = 0.01 + soft * 0.14;
        1.0 - smooth_step(0f32.max(radius - s), radius + s, d)
    };
    let blend_ink = |r: &mut f32, g: &mut f32, b: &mut f32, ir: f32, ig: f32, ib: f32, a: f32| {
        let t = a.clamp(0.0, 1.0);
        *r = lerp(*r, ir, t);
        *g = lerp(*g, ig, t);
        *b = lerp(*b, ib, t);
    };
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let (r, g, b) = (s.r as f32 / 255.0, s.g as f32 / 255.0, s.b as f32 / 255.0);
            let mut k = 1.0 - r.max(g.max(b));
            let den = 1.0 - k;
            let (c, m, yv) = if den <= 0.0001 {
                (0.0, 0.0, 0.0)
            } else {
                (
                    ((1.0 - r - k) / den).clamp(0.0, 1.0),
                    ((1.0 - g - k) / den).clamp(0.0, 1.0),
                    ((1.0 - b - k) / den).clamp(0.0, 1.0),
                )
            };
            if den > 0.0001 {
                k = k.clamp(0.0, 1.0);
            }
            let (mut or, mut og, mut ob) = (1f32, 1f32, 1f32);
            blend_ink(
                &mut or,
                &mut og,
                &mut ob,
                0.0,
                1.0,
                1.0,
                dot(x, y, cr, c) * ink,
            );
            blend_ink(
                &mut or,
                &mut og,
                &mut ob,
                1.0,
                0.0,
                1.0,
                dot(x, y, mr, m) * ink,
            );
            blend_ink(
                &mut or,
                &mut og,
                &mut ob,
                1.0,
                1.0,
                0.0,
                dot(x, y, yr, yv) * ink,
            );
            blend_ink(
                &mut or,
                &mut og,
                &mut ob,
                0.0,
                0.0,
                0.0,
                dot(x, y, kr, k) * ink * 1.1,
            );
            or = lerp(or, r, blend);
            og = lerp(og, g, blend);
            ob = lerp(ob, b, blend);
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

fn heat_haze(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let s01 = p.f("strength").clamp(0.0, 100.0) / 100.0;
    if s01 <= 0.0 {
        return Ok(src.clone());
    }
    let f01 = p.f("frequency").clamp(0.0, 100.0) / 100.0;
    let blur_r = p.f("blur_radius").clamp(0.0, 30.0);
    let max_off = p.f("offset").clamp(-200.0, 200.0) * (0.5 + 1.2 * f01) * s01;
    let li = p.f("luminance_influence").clamp(0.0, 100.0) / 100.0;
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let blurred = if blur_r > 0.01 && s01 > 0.01 {
        Some(blur_bitmap(src, blur_r, blur_r))
    } else {
        None
    };
    let seed = 2026;
    let iw = 1.0 / 1f32.max((w - 1) as f32);
    let ih = 1.0 / 1f32.max((h - 1) as f32);
    let phase = (seed % 10_000) as f32 * 0.017;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let v = y as f32 * ih;
        for x in 0..w {
            let i = (y * w + x) as usize;
            let s = src.px[i];
            let l01 =
                (0.2126 * s.r as f32 + 0.7152 * s.g as f32 + 0.0722 * s.b as f32) / (255.0 * 1.0);
            let lw = lerp(1.0, l01, li);
            let u = x as f32 * iw;
            let w1 = ((u * 10.0 + v * 6.0) * (0.6 + f01 * 1.8) + phase).sin();
            let w2 = ((u * 7.0 - v * 12.0) * (0.4 + f01 * 1.7) - phase * 1.23).cos();
            let _w3 = hash01(x, y, seed) * 2.0 - 1.0;
            let dx = (w1 * 0.60 + w2 * 0.40) * max_off * lw;
            let dy = (w2 * 0.55 - w1 * 0.45) * max_off * lw;
            let wp = bilinear_sample(&src.px, w, h, x as f32 + dx, y as f32 + dy);
            let (mut wr, mut wg, mut wb) = (
                wp.r as f32 / 255.0,
                wp.g as f32 / 255.0,
                wp.b as f32 / 255.0,
            );
            if let Some(b) = &blurred {
                let bl = b.px[i];
                let mix = s01 * (0.25 + 0.75 * (blur_r / 30.0));
                wr = lerp(wr, bl.r as f32 / 255.0, mix);
                wg = lerp(wg, bl.g as f32 / 255.0, mix);
                wb = lerp(wb, bl.b as f32 / 255.0, mix);
            }
            px.push(C::new(
                clamp_to_byte(lerp(s.r as f32 / 255.0, wr, s01) * 255.0),
                clamp_to_byte(lerp(s.g as f32 / 255.0, wg, s01) * 255.0),
                clamp_to_byte(lerp(s.b as f32 / 255.0, wb, s01) * 255.0),
                s.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}
