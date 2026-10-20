//! Filters, part D: Convolution matrix .. Dot matrix.

use std::f32::consts::PI;

use modlight_core::icons as l;
use modlight_editor::effects::Category;
use tiny_skia::PathBuilder;

use crate::def::*;
use crate::helpers::{
    apply_contrast, bilinear_sample, blurred_clamp, clamp_int, clamp_to_byte, clamp01,
    fractal_noise, hash01, lerp, lerp_color, luminance01, screen, smooth_step,
};
use crate::sk::filters::{Tile, matrix_convolution};
use crate::sk::{Bitmap, C, Canvas, Paint, Shader, Style, round};

const F: Category = Category::Filters;

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "convolution_matrix",
            "Convolution matrix",
            F,
            l::GRID_3X3,
            "Applies a custom 3x3 convolution kernel to the image.",
            convolution,
        )
        .params(|| {
            let k = |key: &str, label: &str, d: i32| int_numeric(key, label, -99, 99, d);
            vec![
                k("x0y0", "X0 Y0", 0),
                k("x1y0", "X1 Y0", 0),
                k("x2y0", "X2 Y0", 0),
                k("x0y1", "X0 Y1", 0),
                k("x1y1", "X1 Y1", 1),
                k("x2y1", "X2 Y1", 0),
                k("x0y2", "X0 Y2", 0),
                k("x1y2", "X1 Y2", 0),
                k("x2y2", "X2 Y2", 0),
                float_slider("factor", "Factor", 0.01f32 as f64, 10.0, 1.0),
                int_numeric("offset", "Offset", -255, 255, 0),
            ]
        }),
        Def::new(
            "crimson_border",
            "Crimson border",
            F,
            l::SWORD,
            "Adds a rich crimson/dark red border with an inner glow effect.",
            crimson,
        )
        .params(|| {
            vec![
                int_slider("size", "Size", 2, 60, 12),
                color("color", "Color", C::rgb(153, 0, 18)),
                float_slider("glow_strength", "Glow strength", 0.0, 100.0, 50.0),
                checkbox("inner_line", "Inner line", true),
                checkbox("outer_line", "Outer line", true),
            ]
        }),
        Def::new(
            "crosshatch",
            "Crosshatch",
            F,
            l::HASH,
            "Renders the image as a crosshatch pen drawing.",
            crosshatch,
        )
        .params(|| {
            vec![
                int_slider("lineSpacing", "Line spacing", 4, 24, 8),
                float_slider("lineThickness", "Line thickness", 0.5, 4.0, 1.2f32 as f64),
                float_slider("contrast", "Contrast", 50.0, 200.0, 120.0),
                int_slider("layerCount", "Layer count", 1, 6, 4),
            ]
        }),
        Def::new(
            "crystal_prism",
            "Crystal prism",
            F,
            l::GEM,
            "Refracts and disperses light through crystal-like facets.",
            crystal_prism,
        )
        .params(|| {
            vec![
                int_slider("facetSize", "Facet size", 6, 96, 24),
                float_slider("refraction", "Refraction", 0.0, 30.0, 8.0),
                float_slider("dispersion", "Dispersion", 0.0, 20.0, 4.0),
                float_slider("sparkle", "Sparkle", 0.0, 100.0, 25.0),
            ]
        }),
        Def::new(
            "crystalize_shards",
            "Crystalize shards",
            F,
            l::DIAMOND,
            "Breaks the image into crystalline Voronoi shards with edge highlights.",
            crystalize,
        )
        .params(|| {
            vec![
                int_slider("shardSize", "Shard size", 6, 80, 22),
                float_slider("jitter", "Jitter", 0.0, 100.0, 65.0),
                float_slider("edgeStrength", "Edge strength", 0.0, 100.0, 75.0),
                float_slider("shine", "Shine", 0.0, 100.0, 30.0),
            ]
        }),
        Def::new(
            "cyanotype",
            "Cyanotype",
            F,
            l::SWATCH_BOOK,
            "Simulates a cyanotype photographic printing process with blue-toned output.",
            cyanotype,
        )
        .params(|| {
            vec![
                float_slider("contrast", "Contrast", 50.0, 200.0, 118.0),
                float_slider("paper_texture", "Paper texture", 0.0, 100.0, 34.0),
                float_slider("stain", "Stain", 0.0, 100.0, 28.0),
                float_slider("grain", "Grain", 0.0, 100.0, 12.0),
                float_slider("vignette", "Vignette", 0.0, 100.0, 20.0),
            ]
        }),
        Def::new(
            "datamosh_smear",
            "Datamosh smear",
            F,
            l::CPU,
            "Simulates digital datamoshing with smeared pixel blocks and channel splitting.",
            datamosh,
        )
        .params(|| {
            vec![
                choice(
                    "direction",
                    "Direction",
                    &[("Horizontal", "Horizontal"), ("Vertical", "Vertical")],
                    "Horizontal",
                ),
                float_slider("smear_amount", "Smear amount", 0.0, 100.0, 58.0),
                float_slider("corruption", "Corruption", 0.0, 100.0, 36.0),
                int_slider("block_size", "Block size", 4, 64, 12),
                float_slider("drift", "Drift", -100.0, 100.0, 24.0),
                float_slider("channel_split", "Channel split", 0.0, 100.0, 25.0),
            ]
        }),
        Def::new(
            "diamond_pixelate",
            "Diamond pixelate",
            F,
            l::DIAMOND,
            "Pixelates the image using diamond-shaped blocks.",
            diamond_pixelate,
        )
        .params(|| vec![int_slider("size", "Size", 4, 100, 16)]),
        Def::new(
            "disposable_camera",
            "Disposable camera",
            F,
            l::CAMERA,
            "Simulates the look of a disposable film camera with flash, grain, and warmth.",
            disposable,
        )
        .params(|| {
            vec![
                float_slider("flash", "Flash", 0.0, 100.0, 55.0),
                float_slider("softness", "Softness", 0.0, 24.0, 8.0),
                float_slider("grain", "Grain", 0.0, 100.0, 24.0),
                float_slider("vignette", "Vignette", 0.0, 100.0, 34.0),
                float_slider("warmth", "Warmth", 0.0, 100.0, 42.0),
            ]
        }),
        Def::new(
            "dithering",
            "Dithering",
            F,
            l::GRID_2X2_CHECK,
            "Applies dithering to reduce color depth with various methods and palettes.",
            dithering,
        )
        .params(|| {
            vec![
                choice(
                    "method",
                    "Method",
                    &[
                        ("Floyd-Steinberg", "FloydSteinberg"),
                        ("Bayer 4x4", "Bayer4x4"),
                    ],
                    "FloydSteinberg",
                ),
                choice(
                    "palette",
                    "Palette",
                    &[
                        ("1-bit B&W", "OneBitBW"),
                        ("Web safe 216", "WebSafe216"),
                        ("RGB332", "RGB332"),
                        ("Grayscale 4", "Grayscale4"),
                    ],
                    "OneBitBW",
                ),
                checkbox("serpentine", "Serpentine", true),
                float_slider("strength", "Strength", 0.0, 100.0, 100.0),
            ]
        }),
        Def::new(
            "dot_matrix",
            "Dot Matrix",
            F,
            l::PRINTER,
            "Simulates a vintage dot matrix printer output.",
            dot_matrix,
        )
        .params(|| {
            vec![
                int_slider("cell_size", "Cell Size", 3, 30, 8),
                fine(
                    float_slider("dot_scale", "Dot Scale", 0.1, 1.0, 0.85),
                    0.05,
                    "{}{0:0.00}",
                ),
                checkbox("color_mode", "Color", false),
                color("dot_color", "Dot Color", C::rgb(20, 20, 20)),
                color("paper_color", "Paper Color", C::rgb(250, 245, 235)),
            ]
        }),
    ]
}

/// `ConvolutionHelper.Apply3x3` (clamped edges, alpha kept).
pub fn convolve3x3(src: &Bitmap, kernel: &[f32; 9], gain: f32, bias: f32) -> Bitmap {
    matrix_convolution(src, 3, 3, kernel, gain, bias, (1, 1), Tile::Clamp, false)
}

fn convolution(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let factor = (p.f("factor") as f64).max(0.01) as f32;
    let gain = 1.0 / factor;
    let bias = p.i("offset") as f32;
    let keys = [
        "x0y0", "x1y0", "x2y0", "x0y1", "x1y1", "x2y1", "x0y2", "x1y2", "x2y2",
    ];
    let mut k = [0f32; 9];
    for (i, key) in keys.iter().enumerate() {
        k[i] = p.i(key) as f32;
    }
    Ok(convolve3x3(src, &k, gain, bias))
}

fn crimson(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let size = p.i("size").clamp(2, 60);
    let glow = p.f("glow_strength").clamp(0.0, 100.0) / 100.0;
    let (nw, nh) = (src.w + size * 2, src.h + size * 2);
    let base = p.c("color");
    let dark = C::new(
        (base.r as f32 * 0.5) as u8,
        (base.g as f32 * 0.5) as u8,
        (base.b as f32 * 0.5) as u8,
        base.a,
    );
    let light = C::new(
        (base.r as f32 + (255 - base.r) as f32 * 0.3).min(255.0) as u8,
        (base.g as f32 + (255 - base.g) as f32 * 0.3).min(255.0) as u8,
        (base.b as f32 + (255 - base.b) as f32 * 0.3).min(255.0) as u8,
        base.a,
    );
    let (fw, fh) = (nw as f32, nh as f32);
    let s = size as f32;
    let mut canvas = Canvas::new(nw, nh);
    canvas.draw_rect_xywh(0.0, 0.0, fw, fh, &Paint::fill_aa(base));
    let lin = |p0: (f32, f32), p1: (f32, f32), c0: C, c1: C| {
        Paint::fill_aa(C::BLACK).with_shader(Shader::Linear {
            p0,
            p1,
            colors: vec![c0, c1],
            pos: None,
            tile: Tile::Clamp,
        })
    };
    if glow > 0.0 {
        let gw = s * 0.7;
        let a200 = (200.0 * glow) as u8;
        let a180 = (180.0 * glow) as u8;
        canvas.draw_rect_xywh(
            0.0,
            0.0,
            fw,
            gw,
            &lin(
                (0.0, 0.0),
                (0.0, gw),
                dark.with_alpha(a200),
                base.with_alpha(0),
            ),
        );
        canvas.draw_rect_xywh(
            0.0,
            fh - gw,
            fw,
            gw,
            &lin(
                (0.0, fh),
                (0.0, fh - gw),
                dark.with_alpha(a200),
                base.with_alpha(0),
            ),
        );
        canvas.draw_rect_xywh(
            0.0,
            0.0,
            gw,
            fh,
            &lin(
                (0.0, 0.0),
                (gw, 0.0),
                dark.with_alpha(a180),
                base.with_alpha(0),
            ),
        );
        canvas.draw_rect_xywh(
            fw - gw,
            0.0,
            gw,
            fh,
            &lin(
                (fw, 0.0),
                (fw - gw, 0.0),
                dark.with_alpha(a180),
                base.with_alpha(0),
            ),
        );
        let ig = s * 0.4;
        let a120 = (120.0 * glow) as u8;
        canvas.draw_rect_xywh(
            s,
            s - ig,
            src.w as f32,
            ig,
            &lin(
                (0.0, s),
                (0.0, s - ig),
                light.with_alpha(a120),
                base.with_alpha(0),
            ),
        );
        canvas.draw_rect_xywh(
            s,
            fh - s,
            src.w as f32,
            ig,
            &lin(
                (0.0, fh - s),
                (0.0, fh - s + ig),
                light.with_alpha(a120),
                base.with_alpha(0),
            ),
        );
    }
    canvas.draw_bitmap(src, s, s, &Paint::default());
    if p.b("inner_line") {
        canvas.draw_rect_xywh(
            s - 0.5,
            s - 0.5,
            (src.w + 1) as f32,
            (src.h + 1) as f32,
            &Paint::stroke_aa(light.with_alpha(180), 1.0),
        );
    }
    if p.b("outer_line") {
        canvas.draw_rect_xywh(0.5, 0.5, fw - 1.0, fh - 1.0, &Paint::stroke_aa(dark, 1.5));
    }
    Ok(canvas.into_bitmap())
}

fn crosshatch(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let spacing = p.i("lineSpacing").clamp(4, 24) as f32;
    let thick = p.f("lineThickness").clamp(0.5, 4.0);
    let contrast = p.f("contrast").clamp(50.0, 200.0) / 100.0;
    let layers = p.i("layerCount").clamp(1, 6);
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let th = [0.14f32, 0.28, 0.42, 0.58, 0.74, 0.88];
    let is_line = |v: f32| {
        let mut m = v % spacing;
        if m < 0.0 {
            m += spacing;
        }
        m.min(spacing - m) <= thick * 0.5
    };
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let mut lum = (0.2126 * s.r as f32 + 0.7152 * s.g as f32 + 0.0722 * s.b as f32) / 255.0;
            lum = ((lum - 0.5) * contrast + 0.5).clamp(0.0, 1.0);
            let dark = 1.0 - lum;
            let (xf, yf) = (x as f32, y as f32);
            let mut active = 0;
            if layers >= 1 && dark > th[0] && is_line(((x + y) as f32 * 0.7071).abs()) {
                active += 1;
            }
            if layers >= 2 && dark > th[1] && is_line(((x - y) as f32 * 0.7071).abs()) {
                active += 1;
            }
            if layers >= 3 && dark > th[2] && is_line(yf) {
                active += 1;
            }
            if layers >= 4 && dark > th[3] && is_line(xf) {
                active += 1;
            }
            if layers >= 5 && dark > th[4] && is_line(xf * 0.5 + yf) {
                active += 1;
            }
            if layers >= 6 && dark > th[5] && is_line(xf * 0.5 - yf) {
                active += 1;
            }
            let ink = active as f32 / layers as f32;
            let mut v = lum * (1.0 - ink * 0.94);
            v = v.min(lum + 0.06).clamp(0.0, 1.0);
            let b = clamp_to_byte(v * 255.0);
            px.push(C::new(b, b, b, s.a));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn crystal_prism(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let facet = clamp_int(p.i("facetSize"), 6, 96);
    let refraction = p.f("refraction").clamp(0.0, 30.0);
    let dispersion = p.f("dispersion").clamp(0.0, 20.0);
    let sparkle = p.f("sparkle").clamp(0.0, 100.0) / 100.0;
    if refraction <= 0.0 && dispersion <= 0.0 && sparkle <= 0.0 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let seed = 4771;
    let blend = 0.58 + 0.25 * (refraction / 30.0);
    let fs = facet as f32;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let (cx, cy) = (x / facet, y / facet);
            let h0 = hash01(cx, cy, seed);
            let h1 = hash01(cx, cy, seed ^ 149);
            let h2 = hash01(cx, cy, seed ^ 313);
            let h3 = hash01(cx, cy, seed ^ 341);
            let ccx = (cx as f32 + 0.20 + 0.60 * h0) * fs;
            let ccy = (cy as f32 + 0.20 + 0.60 * h1) * fs;
            let dx = x as f32 - ccx;
            let dy = y as f32 - ccy;
            let dist = (dx * dx + dy * dy).sqrt();
            let radial = 1.0 - clamp01(dist / (fs * 0.85));
            let angle = h2 * PI * 2.0;
            let (dir_x, dir_y) = (angle.cos(), angle.sin());
            let om = refraction * (0.30 + 0.70 * radial);
            let sx = x as f32 + dir_x * om;
            let sy = y as f32 + dir_y * om;
            let disp = dispersion * (0.25 + 0.75 * radial);
            let cr = bilinear_sample(&src.px, w, h, sx + dir_y * disp, sy - dir_x * disp);
            let cg = bilinear_sample(&src.px, w, h, sx, sy);
            let cb = bilinear_sample(&src.px, w, h, sx - dir_y * disp, sy + dir_x * disp);
            let shade = 0.88 + (h3 - 0.5) * 0.30;
            let mut pr = (cr.r as f32 / 255.0) * shade;
            let mut pg = (cg.g as f32 / 255.0) * shade;
            let mut pb = (cb.b as f32 / 255.0) * shade;
            let fx = (x % facet) as f32 / fs;
            let fy = (y % facet) as f32 / fs;
            let border = fx.min(1.0 - fx).min(fy.min(1.0 - fy));
            let edge = 1.0 - smooth_step(0.04, 0.28, border);
            let sm = edge * edge * sparkle;
            pr += sm * 0.32;
            pg += sm * 0.34;
            pb += sm * 0.38;
            let o = src.get(x, y);
            let r = lerp(o.r as f32 / 255.0, pr, blend);
            let g = lerp(o.g as f32 / 255.0, pg, blend);
            let b = lerp(o.b as f32 / 255.0, pb, blend);
            px.push(C::new(
                clamp_to_byte(r * 255.0),
                clamp_to_byte(g * 255.0),
                clamp_to_byte(b * 255.0),
                o.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn crystalize(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let cell = p.i("shardSize").clamp(6, 80) as f32;
    let inv = 1.0 / cell;
    let jitter = p.f("jitter").clamp(0.0, 100.0) / 100.0;
    let es = p.f("edgeStrength").clamp(0.0, 100.0) / 100.0;
    let shine = p.f("shine").clamp(0.0, 100.0) / 100.0;
    let (w, h) = (src.w, src.h);
    let seed = 4242;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let fx = x as f32 * inv;
            let fy = y as f32 * inv;
            let cx = fx.floor() as i32;
            let cy = fy.floor() as i32;
            let (mut nd, mut sd) = (f32::MAX, f32::MAX);
            let (mut nsx, mut nsy) = (x as f32, y as f32);
            let (mut ncx, mut ncy) = (cx, cy);
            for oy in -1..=1 {
                for ox in -1..=1 {
                    let (gx, gy) = (cx + ox, cy + oy);
                    let jx = (hash01(gx, gy, seed) * 2.0 - 1.0) * 0.45 * jitter;
                    let jy = (hash01(gx, gy, seed ^ 911) * 2.0 - 1.0) * 0.45 * jitter;
                    let sx = (gx as f32 + 0.5 + jx) * cell;
                    let sy = (gy as f32 + 0.5 + jy) * cell;
                    let dx = sx - x as f32;
                    let dy = sy - y as f32;
                    let d = dx * dx + dy * dy;
                    if d < nd {
                        sd = nd;
                        nd = d;
                        nsx = sx;
                        nsy = sy;
                        ncx = gx;
                        ncy = gy;
                    } else if d < sd {
                        sd = d;
                    }
                }
            }
            let s = bilinear_sample(&src.px, w, h, nsx, nsy);
            let (mut r, mut g, mut b, a) = (s.r as f32, s.g as f32, s.b as f32, s.a as f32);
            let nearest = nd.max(0.0).sqrt();
            let second = sd.max(0.0).sqrt();
            let mut edge = 1.0 - smooth_step(0.0, cell * 0.12, second - nearest);
            edge *= es;
            let noise = hash01(ncx, ncy, seed ^ 1777);
            let center = 1.0 - (nearest / 1f32.max(cell)).min(1.0);
            let shade = 1.0 + (noise - 0.5) * 0.25 + center * shine * 0.45;
            r *= shade;
            g *= shade;
            b *= shade;
            if edge > 0.001 {
                let ed = edge * 0.78;
                r = lerp(r, r * 0.22, ed);
                g = lerp(g, g * 0.26, ed);
                b = lerp(b, b * 0.34, ed);
                let glint = edge * (0.20 + noise * 0.25);
                r += 38.0 * glint;
                g += 52.0 * glint;
                b += 80.0 * glint;
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

fn cyanotype(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let contrast = p.f("contrast").clamp(50.0, 200.0) / 100.0;
    let paper_t = p.f("paper_texture").clamp(0.0, 100.0) / 100.0;
    let stain = p.f("stain").clamp(0.0, 100.0) / 100.0;
    let grain = p.f("grain").clamp(0.0, 100.0) / 100.0;
    let vig = p.f("vignette").clamp(0.0, 100.0) / 100.0;
    let (w, h) = (src.w, src.h);
    let cx = (w - 1) as f32 * 0.5;
    let cy = (h - 1) as f32 * 0.5;
    let max_d = 1f32.max((cx * cx + cy * cy).sqrt());
    let paper = C::rgb(241, 236, 220);
    let mid = C::rgb(87, 136, 170);
    let deep = C::rgb(11, 45, 92);
    let seed = 1842;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let lum = apply_contrast(luminance01(s), contrast);
            let ink = 1.0 - lum;
            let tone = if ink < 0.36 {
                lerp_color(paper, mid, ink / 0.36)
            } else {
                lerp_color(mid, deep, (ink - 0.36) / 0.64)
            };
            let (xf, yf) = (x as f32, y as f32);
            let pn = (fractal_noise(xf * 0.022, yf * 0.022, 4, 2.1, 0.55, seed) - 0.5) * paper_t;
            let sn = fractal_noise(xf * 0.006, yf * 0.006, 5, 2.0, 0.5, seed ^ 901);
            let sm = smooth_step(0.56, 0.92, sn) * stain;
            let gn = (hash01(x, y, seed ^ 2111) * 2.0 - 1.0) * grain * 0.08;
            let radial = ((xf - cx).powf(2.0) + (yf - cy).powf(2.0)).sqrt() / max_d;
            let vm = clamp01(1.0 - vig * radial.powf(1.7) * 0.24);
            let r = clamp01((tone.r as f32 / 255.0 + pn * 0.08 + sm * 0.06 + gn) * vm);
            let g = clamp01((tone.g as f32 / 255.0 + pn * 0.10 + sm * 0.08 + gn) * vm);
            let b = clamp01((tone.b as f32 / 255.0 + pn * 0.14 + sm * 0.10 + gn) * vm);
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

fn datamosh(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let smear = p.f("smear_amount").clamp(0.0, 100.0) / 100.0;
    let corruption = p.f("corruption").clamp(0.0, 100.0) / 100.0;
    let bs = p.i("block_size").clamp(4, 64);
    let drift = p.f("drift").clamp(-100.0, 100.0);
    let split = p.f("channel_split").clamp(0.0, 100.0) / 100.0 * 7.0;
    if smear <= 0.0001 && corruption <= 0.0001 && drift.abs() <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let seed = 9011;
    let dist = smear * bs as f32 * 5.5;
    let mut px = vec![C::ZERO; src.px.len()];
    let sample = |x: f32, y: f32| bilinear_sample(&src.px, w, h, x, y);
    if p.e("direction") == "Horizontal" {
        for y in 0..h {
            let rn = hash01(y / bs, 7, seed) * 2.0 - 1.0;
            let base = ((y as f32 * 0.09 + 0.4).sin() + rn * 1.15) * drift * 0.34;
            let mut carry = 0f32;
            let mut bx = 0;
            while bx < w {
                let bi = bx / bs;
                let trig = hash01(bi, y / bs, seed ^ 313);
                if trig > 1.0 - corruption * 0.78 {
                    let sign = hash01(bi, y, seed ^ 743) * 2.0 - 1.0;
                    carry += sign * dist * (0.35 + trig);
                } else {
                    carry *= 0.82;
                }
                for x in bx..w.min(bx + bs) {
                    let fx = x as f32 - carry - base;
                    let sr = sample(fx + split, y as f32);
                    let sg = sample(fx, y as f32);
                    let sb = sample(fx - split, y as f32);
                    px[(y * w + x) as usize] = C::new(sr.r, sg.g, sb.b, sg.a);
                }
                bx += bs;
            }
        }
    } else {
        for x in 0..w {
            let cn = hash01(x / bs, 11, seed) * 2.0 - 1.0;
            let base = ((x as f32 * 0.07 + 1.1).cos() + cn * 1.20) * drift * 0.34;
            let mut carry = 0f32;
            let mut by = 0;
            while by < h {
                let bi = by / bs;
                let trig = hash01(x / bs, bi, seed ^ 919);
                if trig > 1.0 - corruption * 0.78 {
                    let sign = hash01(x, bi, seed ^ 1597) * 2.0 - 1.0;
                    carry += sign * dist * (0.35 + trig);
                } else {
                    carry *= 0.82;
                }
                for y in by..h.min(by + bs) {
                    let fy = y as f32 - carry - base;
                    let sr = sample(x as f32, fy + split);
                    let sg = sample(x as f32, fy);
                    let sb = sample(x as f32, fy - split);
                    px[(y * w + x) as usize] = C::new(sr.r, sg.g, sb.b, sg.a);
                }
                by += bs;
            }
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn diamond_pixelate(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let size = 4.max(p.i("size"));
    let (w, h) = (src.w, src.h);
    let mut canvas = Canvas::new(w, h);
    let half = size / 2;
    let mut cy = 0;
    while cy < h + size {
        let mut cx = 0;
        while cx < w + size {
            let off = if (cy / half) % 2 == 0 { 0 } else { half };
            let dx = cx + off;
            let color = src.get(dx.clamp(0, w - 1), cy.clamp(0, h - 1));
            let mut pb = PathBuilder::new();
            pb.move_to(dx as f32, (cy - half) as f32);
            pb.line_to((dx + half) as f32, cy as f32);
            pb.line_to(dx as f32, (cy + half) as f32);
            pb.line_to((dx - half) as f32, cy as f32);
            pb.close();
            if let Some(path) = pb.finish() {
                canvas.draw_path(&path, &Paint::fill(color));
            }
            cx += size;
        }
        cy += half;
    }
    Ok(canvas.into_bitmap())
}

fn disposable(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let flash = p.f("flash").clamp(0.0, 100.0) / 100.0;
    let soft = p.f("softness").clamp(0.0, 24.0);
    let grain = p.f("grain").clamp(0.0, 100.0) / 100.0;
    let vig = p.f("vignette").clamp(0.0, 100.0) / 100.0;
    let warmth = p.f("warmth").clamp(0.0, 100.0) / 100.0;
    if flash <= 0.0001 && soft <= 0.0001 && grain <= 0.0001 && vig <= 0.0001 && warmth <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let cx = (w - 1) as f32 * 0.56;
    let cy = (h - 1) as f32 * 0.44;
    let max_d = 1f32.max((cx * cx + cy * cy).sqrt());
    let blur = if soft > 0.01 {
        Some(blurred_clamp(src, soft))
    } else {
        None
    };
    let bpx = blur.as_ref().map_or(&src.px, |b| &b.px);
    let sigma_inv = 1.0 / 0.0001f32.max(2.0 * (0.22 + flash * 0.20f32).powf(2.0));
    let smix = (soft / 24.0) * 0.34;
    let contrast = 1.08 + flash * 0.16 + warmth * 0.08;
    let seed = 1996;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let s = src.px[i];
            let so = bpx[i];
            let mut r = lerp(s.r as f32 / 255.0, so.r as f32 / 255.0, smix);
            let mut g = lerp(s.g as f32 / 255.0, so.g as f32 / 255.0, smix);
            let mut b = lerp(s.b as f32 / 255.0, so.b as f32 / 255.0, smix);
            r = apply_contrast(r, contrast);
            g = apply_contrast(g, contrast);
            b = apply_contrast(b, contrast);
            let dx = (x as f32 - cx) / 1.max(w - 1) as f32;
            let dy = (y as f32 - cy) / 1.max(h - 1) as f32;
            let d = (dx * dx + dy * dy).sqrt();
            let fm = flash * (-(d * d) * sigma_inv).exp();
            r = screen(r, fm * (0.22 + warmth * 0.10));
            g = screen(g, fm * 0.18);
            b = screen(b, fm * (0.11 - warmth * 0.02));
            if warmth > 0.0001 {
                let wm = warmth * (0.14 + fm * 0.30);
                r = lerp(r, (r + 0.22).min(1.0), wm);
                g = lerp(g, (g + 0.10).min(1.0), wm * 0.75);
                b = lerp(b, (b - 0.05).max(0.0), wm * 0.70);
            }
            let radial = ((x as f32 - (w - 1) as f32 * 0.5).powf(2.0)
                + (y as f32 - (h - 1) as f32 * 0.5).powf(2.0))
            .sqrt()
                / max_d;
            let vm = clamp01(1.0 - vig * radial.powf(1.85) * 0.62);
            let n = (hash01(x, y, seed) * 2.0 - 1.0) * grain * 0.11;
            r = clamp01(r * vm + n * 1.02);
            g = clamp01(g * vm + n);
            b = clamp01(b * vm + n * 0.92);
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

fn quantize_channel(v: f32, levels: i32) -> u8 {
    let c = v.clamp(0.0, 255.0) / 255.0;
    let steps = levels - 1;
    let q = round(c * steps as f32) as i32;
    clamp_to_byte(q as f32 / steps as f32 * 255.0)
}

fn quantize(r: f32, g: f32, b: f32, palette: &str) -> (u8, u8, u8) {
    match palette {
        "OneBitBW" => {
            let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            let v = if lum >= 128.0 { 255 } else { 0 };
            (v, v, v)
        }
        "WebSafe216" => (
            quantize_channel(r, 6),
            quantize_channel(g, 6),
            quantize_channel(b, 6),
        ),
        "RGB332" => (
            quantize_channel(r, 8),
            quantize_channel(g, 8),
            quantize_channel(b, 4),
        ),
        _ => {
            let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            let v = quantize_channel(lum, 4);
            (v, v, v)
        }
    }
}

fn dithering(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let strength = p.f("strength").clamp(0.0, 100.0) / 100.0;
    if strength <= 0.0 {
        return Ok(src.clone());
    }
    let palette = p.e("palette");
    let n = src.px.len();
    let mut out = vec![C::ZERO; n];
    if p.e("method") == "Bayer4x4" {
        let bayer = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                let s = src.px[i];
                let t = bayer[(y & 3) as usize][(x & 3) as usize] as f32 / 16.0 - 0.5;
                let bias = t * 36.0;
                let (qr, qg, qb) = quantize(
                    s.r as f32 + bias,
                    s.g as f32 + bias,
                    s.b as f32 + bias,
                    &palette,
                );
                out[i] = C::new(qr, qg, qb, s.a);
            }
        }
    } else {
        let serp = p.b("serpentine");
        let mut r: Vec<f32> = src.px.iter().map(|c| c.r as f32).collect();
        let mut g: Vec<f32> = src.px.iter().map(|c| c.g as f32).collect();
        let mut b: Vec<f32> = src.px.iter().map(|c| c.b as f32).collect();
        for y in 0..h {
            let rev = serp && (y & 1) == 1;
            let dir = if rev { -1 } else { 1 };
            let mut x = if rev { w - 1 } else { 0 };
            let end = if rev { -1 } else { w };
            while x != end {
                let i = (y * w + x) as usize;
                let (cr, cg, cb) = (r[i], g[i], b[i]);
                let (qr, qg, qb) = quantize(cr, cg, cb, &palette);
                out[i] = C::new(qr, qg, qb, src.px[i].a);
                let (er, eg, eb) = (cr - qr as f32, cg - qg as f32, cb - qb as f32);
                let mut dist = |xx: i32, yy: i32, f: f32| {
                    if xx < 0 || xx >= w || yy < 0 || yy >= h {
                        return;
                    }
                    let j = (yy * w + xx) as usize;
                    r[j] = (r[j] + er * f).clamp(0.0, 255.0);
                    g[j] = (g[j] + eg * f).clamp(0.0, 255.0);
                    b[j] = (b[j] + eb * f).clamp(0.0, 255.0);
                };
                dist(x + dir, y, 7.0 / 16.0);
                dist(x - dir, y + 1, 3.0 / 16.0);
                dist(x, y + 1, 5.0 / 16.0);
                dist(x + dir, y + 1, 1.0 / 16.0);
                x += dir;
            }
        }
    }
    let px = src
        .px
        .iter()
        .zip(&out)
        .map(|(s, d)| {
            C::new(
                clamp_to_byte(lerp(s.r as f32, d.r as f32, strength)),
                clamp_to_byte(lerp(s.g as f32, d.g as f32, strength)),
                clamp_to_byte(lerp(s.b as f32, d.b as f32, strength)),
                s.a,
            )
        })
        .collect();
    Ok(Bitmap::from_pixels(w, h, px))
}

fn dot_matrix(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    let size = p.i("cell_size").clamp(2, 100);
    let scale = p.f("dot_scale").clamp(0.05, 1.0);
    let max_r = size as f32 * 0.5 * scale;
    let cols = (w + size - 1) / size;
    let rows = (h + size - 1) / size;
    let mut canvas = Canvas::new(w, h);
    canvas.clear(p.c("paper_color"));
    let color_mode = p.b("color_mode");
    let dot = p.c("dot_color");
    for gy in 0..rows {
        for gx in 0..cols {
            let (x0, y0) = (gx * size, gy * size);
            let (x1, y1) = ((x0 + size).min(w), (y0 + size).min(h));
            let (mut sr, mut sg, mut sb, mut n) = (0i64, 0i64, 0i64, 0i64);
            for yy in y0..y1 {
                for xx in x0..x1 {
                    let c = src.get(xx, yy);
                    sr += c.r as i64;
                    sg += c.g as i64;
                    sb += c.b as i64;
                    n += 1;
                }
            }
            if n == 0 {
                continue;
            }
            let (ar, ag, ab) = ((sr / n) as u8, (sg / n) as u8, (sb / n) as u8);
            let lum = (0.299 * ar as f32 + 0.587 * ag as f32 + 0.114 * ab as f32) / 255.0;
            let r = (1.0 - lum) * max_r;
            if r < 0.3 {
                continue;
            }
            let color = if color_mode { C::rgb(ar, ag, ab) } else { dot };
            let paint = Paint {
                style: Style::Fill,
                ..Paint::fill_aa(color)
            };
            canvas.draw_circle(
                x0 as f32 + size as f32 * 0.5,
                y0 as f32 + size as f32 * 0.5,
                r,
                &paint,
            );
        }
    }
    Ok(canvas.into_bitmap())
}
