//! Filters, part C: Chain link border .. Color splash.

use std::f32::consts::PI;

use modlight_core::icons as l;
use modlight_editor::effects::Category;
use tiny_skia::PathBuilder;

use crate::def::*;
use crate::dotnet::shared_next;
use crate::helpers::{
    bilinear_sample, clamp_to_byte, clamp01, fractal_noise, hash01, lerp, smooth_step,
};
use crate::sk::{Bitmap, Blend, C, Canvas, Cap, Join, Paint, Style, rect, round, roundd};

const F: Category = Category::Filters;

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "chain_link_border",
            "Chain link border",
            F,
            l::FENCE,
            "Adds a metallic chain-link fence border with interlocking wire loops and highlights.",
            chain_link,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 16, 250, 52),
                color("wire_color", "Wire color", C::rgb(170, 175, 180)),
                color("bg_color", "Background color", C::rgb(35, 40, 48)),
                float_slider("cell_size", "Cell size", 8.0, 60.0, 20.0),
                float_slider("wire_width", "Wire width", 1.0, 8.0, 2.5),
                checkbox("highlight", "Wire highlight", true),
            ]
        }),
        Def::new(
            "chalk_drawing",
            "Chalk Drawing",
            F,
            l::BRUSH,
            "Simulates a chalk drawing on a dark surface.",
            chalk,
        )
        .params(|| {
            vec![
                float_slider("detail", "Detail", 0.0, 100.0, 50.0),
                float_slider("roughness", "Roughness", 0.0, 100.0, 40.0),
                color("chalk_color", "Chalk Color", C::rgb(255, 255, 255)),
                color("board_color", "Board Color", C::rgb(30, 50, 40)),
            ]
        }),
        Def::new(
            "chromatic_aberration",
            "Chromatic aberration",
            F,
            l::APERTURE,
            "Simulates lens chromatic aberration with RGB channel splitting.",
            chromatic,
        )
        .params(|| {
            vec![
                float_slider("amount", "Amount", 0.0, 40.0, 8.0),
                float_slider("edgeStart", "Edge start", 0.0, 0.95f32 as f64, 0.20f32 as f64),
                float_slider("strength", "Strength", 0.0, 100.0, 75.0),
            ]
        }),
        Def::new(
            "chrome_metallic_border",
            "Chrome metallic border",
            F,
            l::SPARKLES,
            "Adds a shiny chrome metallic border with realistic specular highlights and reflections.",
            chrome,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 4, 300, 36),
                color("tint_color", "Tint color", C::rgb(200, 205, 210)),
                float_slider("reflectivity", "Reflectivity", 0.0, 100.0, 80.0),
                float_slider("bevel_strength", "Bevel strength", 0.0, 100.0, 70.0),
                checkbox("inner_line", "Inner line", true),
                checkbox("outer_line", "Outer line", true),
            ]
        }),
        Def::new(
            "circuit_board_border",
            "Circuit board border",
            F,
            l::CIRCUIT_BOARD,
            "Adds a PCB circuit board border with copper traces, vias, and pads on a dark substrate.",
            circuit,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 20, 300, 64),
                color("board_color", "Board color", C::rgb(12, 58, 32)),
                color("trace_color", "Trace color", C::rgb(184, 115, 51)),
                float_slider("trace_density", "Trace density", 10.0, 100.0, 55.0),
                checkbox("glow", "Trace glow", true),
                checkbox("inner_line", "Inner line", true),
            ]
        }),
        Def::new(
            "claymation_texture",
            "Claymation texture",
            F,
            l::PAINT_ROLLER,
            "Gives the image a sculpted claymation look with chunky quantization and relief texture.",
            claymation,
        )
        .params(|| {
            vec![
                int_slider("chunkSize", "Chunk size", 3, 18, 8),
                float_slider("smoothness", "Smoothness", 0.0, 100.0, 55.0),
                float_slider("relief", "Relief", 0.0, 100.0, 35.0),
                float_slider("saturation", "Saturation", -100.0, 100.0, 20.0),
                float_slider("textureGrain", "Texture grain", 0.0, 100.0, 30.0),
            ]
        }),
        Def::new(
            "clouds",
            "Clouds",
            F,
            l::CLOUD,
            "Overlays procedurally generated clouds onto the image.",
            clouds,
        )
        .params(|| {
            vec![
                float_slider("coverage", "Coverage", 0.0, 100.0, 55.0),
                float_slider("scale", "Scale", 0.0, 100.0, 72.0),
                float_slider("heightBias", "Height bias", 0.0, 100.0, 62.0),
                float_slider("softness", "Softness", 0.0, 100.0, 64.0),
                float_slider("sunlight", "Sunlight", 0.0, 100.0, 38.0),
            ]
        }),
        Def::new(
            "color_depth",
            "Color depth",
            F,
            l::LAYERS_2,
            "Reduces the number of bits per color channel.",
            color_depth,
        )
        .params(|| vec![int_slider("bitsPerChannel", "Bits per channel", 1, 8, 4)]),
        Def::new(
            "color_halftone",
            "Color halftone",
            F,
            l::CIRCLE_DOT,
            "Simulates CMYK color halftone printing with rotated dot grids.",
            color_halftone,
        )
        .params(|| vec![int_slider("size", "Dot size", 4, 30, 8)]),
        Def::new(
            "color_splash",
            "Color splash",
            F,
            l::DROPLETS,
            "Keeps one color and desaturates the rest of the image.",
            color_splash,
        )
        .params(|| {
            vec![
                int_slider("hue", "Target hue", 0, 360, 0),
                int_slider("tolerance", "Tolerance", 1, 180, 30),
            ]
        }),
    ]
}

fn chain_link(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(16, 250);
    let cell = p.f("cell_size").clamp(8.0, 60.0);
    let ww = p.f("wire_width").clamp(1.0, 8.0);
    let wire = p.c("wire_color");
    let highlight = p.b("highlight");
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let mut canvas = Canvas::new(nw, nh);
    canvas.clear(p.c("bg_color"));
    let half = cell * 0.5;
    let wire_paint = Paint {
        style: Style::Stroke,
        stroke_width: ww,
        cap: Cap::Round,
        ..Paint::fill_aa(wire)
    };
    let light = C::new(
        clamp_to_byte(wire.r as f32 + (255 - wire.r) as f32 * 0.4),
        clamp_to_byte(wire.g as f32 + (255 - wire.g) as f32 * 0.4),
        clamp_to_byte(wire.b as f32 + (255 - wire.b) as f32 * 0.4),
        wire.a,
    );
    let hl_paint = Paint {
        stroke_width: ww * 0.5,
        color: light,
        ..wire_paint.clone()
    };
    let mesh = |canvas: &mut Canvas, bx: f32, by: f32, bw: f32, bh: f32| {
        canvas.save();
        canvas.clip_rect(rect(bx, by, bx + bw, by + bh), false);
        let (sx, sy) = (bx - cell * 2.0, by - cell * 2.0);
        let (ex, ey) = (bx + bw + cell * 2.0, by + bh + cell * 2.0);
        let mut y = sy;
        while y <= ey {
            let mut x = sx;
            while x <= ex {
                let mut pb = PathBuilder::new();
                pb.move_to(x, y - half);
                pb.line_to(x + half, y);
                pb.line_to(x, y + half);
                pb.line_to(x - half, y);
                pb.line_to(x, y - half);
                if let Some(path) = pb.finish() {
                    canvas.draw_path(&path, &wire_paint);
                }
                if highlight {
                    let mut pb = PathBuilder::new();
                    pb.move_to(x - half, y);
                    pb.line_to(x, y - half);
                    pb.line_to(x + half, y);
                    if let Some(path) = pb.finish() {
                        canvas.draw_path(&path, &hl_paint);
                    }
                }
                x += cell;
            }
            y += cell;
        }
        canvas.restore();
    };
    let b = border as f32;
    mesh(&mut canvas, 0.0, 0.0, nw as f32, b);
    mesh(&mut canvas, 0.0, (border + src.h) as f32, nw as f32, b);
    mesh(&mut canvas, 0.0, b, b, src.h as f32);
    mesh(&mut canvas, (border + src.w) as f32, b, b, src.h as f32);
    let dark = C::new(
        clamp_to_byte(wire.r as f32 * 0.7),
        clamp_to_byte(wire.g as f32 * 0.7),
        clamp_to_byte(wire.b as f32 * 0.7),
        wire.a,
    );
    canvas.draw_rect_xywh(
        b - 1.0,
        b - 1.0,
        (src.w + 2) as f32,
        (src.h + 2) as f32,
        &Paint::stroke_aa(dark, ww * 1.5),
    );
    canvas.draw_bitmap(src, b, b, &Paint::default());
    Ok(canvas.into_bitmap())
}

fn chalk(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    let detail = p.f("detail").clamp(0.0, 100.0) / 100.0;
    let rough = p.f("roughness").clamp(0.0, 100.0) / 100.0;
    let lum: Vec<f32> = src
        .px
        .iter()
        .map(|c| (0.299 * c.r as f32 + 0.587 * c.g as f32 + 0.114 * c.b as f32) / 255.0)
        .collect();
    let mut edge = vec![0f32; (w * h).max(0) as usize];
    let mut max_edge = 0f32;
    for y in 1..h - 1 {
        let row = y * w;
        for x in 1..w - 1 {
            let l = |xx: i32, yy: i32| lum[(yy * w + xx) as usize];
            let gx = -l(x - 1, y - 1) + l(x + 1, y - 1) - 2.0 * l(x - 1, y) + 2.0 * l(x + 1, y)
                - l(x - 1, y + 1)
                + l(x + 1, y + 1);
            let gy = -l(x - 1, y - 1) - 2.0 * l(x, y - 1) - l(x + 1, y - 1)
                + l(x - 1, y + 1)
                + 2.0 * l(x, y + 1)
                + l(x + 1, y + 1);
            let m = (gx * gx + gy * gy).sqrt();
            edge[(row + x) as usize] = m;
            if m > max_edge {
                max_edge = m;
            }
        }
    }
    if max_edge > 0.0 {
        let inv = 1.0 / max_edge;
        for e in edge.iter_mut() {
            *e *= inv;
        }
    }
    let mut rng: i32 = 12345;
    let noise: Vec<f32> = (0..edge.len())
        .map(|_| {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            (rng & 0x7FFF_FFFF) as f32 / i32::MAX as f32
        })
        .collect();
    let (bc, cc) = (p.c("board_color"), p.c("chalk_color"));
    let (br, bg, bb) = (
        bc.r as f32 / 255.0,
        bc.g as f32 / 255.0,
        bc.b as f32 / 255.0,
    );
    let (cr, cg, cb) = (
        cc.r as f32 / 255.0,
        cc.g as f32 / 255.0,
        cc.b as f32 / 255.0,
    );
    let ew = 0.3 + detail * 0.7;
    let lw = 1.0 - ew * 0.5;
    let px = (0..edge.len())
        .map(|i| {
            let mut ci = (edge[i] * ew + lum[i] * lw).clamp(0.0, 1.0);
            ci *= 1.0 - rough * (1.0 - noise[i]) * 0.6;
            ci *= lum[i].powf(0.5);
            ci = ci.clamp(0.0, 1.0);
            let o = |b: f32, c: f32| round((b + (c - b) * ci).clamp(0.0, 1.0) * 255.0) as u8;
            C::new(o(br, cr), o(bg, cg), o(bb, cb), src.px[i].a)
        })
        .collect();
    Ok(Bitmap::from_pixels(w, h, px))
}

fn chromatic(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let amount = p.f("amount").clamp(0.0, 40.0);
    let edge_start = p.f("edgeStart").clamp(0.0, 0.95);
    let strength = p.f("strength").clamp(0.0, 100.0) / 100.0;
    if amount <= 0.0 || strength <= 0.0 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let (right, bottom) = (w - 1, h - 1);
    let cx = right as f32 * 0.5;
    let cy = bottom as f32 * 0.5;
    let icx = if cx > 0.0 { 1.0 / cx } else { 0.0 };
    let icy = if cy > 0.0 { 1.0 / cy } else { 0.0 };
    let lum = |c: C| (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let dyn_ = (y as f32 - cy) * icy;
        for x in 0..w {
            let o = src.get(x, y);
            let dxn = (x as f32 - cx) * icx;
            let d01 = (dxn * dxn + dyn_ * dyn_).sqrt() * 0.70710678;
            let radial = smooth_step(edge_start, 1.0, d01);
            if radial <= 0.0 {
                px.push(o);
                continue;
            }
            let dl = (dxn * dxn + dyn_ * dyn_).sqrt();
            let dx = if dl > 0.0001 { dxn / dl } else { 0.0 };
            let dy = if dl > 0.0001 { dyn_ / dl } else { 0.0 };
            let mix = strength * radial;
            if mix <= 0.0001 {
                px.push(o);
                continue;
            }
            let shift = amount * radial;
            let sr = bilinear_sample(&src.px, w, h, x as f32 + dx * shift, y as f32 + dy * shift);
            let sb = bilinear_sample(&src.px, w, h, x as f32 - dx * shift, y as f32 - dy * shift);
            let split = smooth_step(0.01, 0.30, (lum(sr) - lum(sb)).abs());
            let boost = mix * (0.85 + 1.35 * split);
            let red = lerp(o.r as f32, sr.r as f32, mix) + (sr.r as f32 - o.r as f32) * boost;
            let green = lerp(
                o.g as f32,
                (sr.g as f32 + o.g as f32 + sb.g as f32) / 3.0,
                mix * 0.06,
            );
            let blue = lerp(o.b as f32, sb.b as f32, mix) + (sb.b as f32 - o.b as f32) * boost;
            let alpha = lerp(
                o.a as f32,
                (sr.a as f32 + o.a as f32 + sb.a as f32) / 3.0,
                mix,
            );
            px.push(C::new(
                clamp_to_byte(red),
                clamp_to_byte(green),
                clamp_to_byte(blue),
                clamp_to_byte(alpha),
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

const CHROME_STOPS: [(f32, f32); 9] = [
    (0.00, 0.15),
    (0.10, 0.80),
    (0.22, 0.48),
    (0.38, 0.12),
    (0.52, 0.72),
    (0.68, 0.90),
    (0.80, 0.55),
    (0.90, 0.22),
    (1.00, 0.35),
];

fn chrome(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(4, 300);
    let refl = p.f("reflectivity").clamp(0.0, 100.0) / 100.0;
    let bevel = p.f("bevel_strength").clamp(0.0, 100.0) / 100.0;
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let tint = p.c("tint_color");
    let (tr, tg, tb) = (
        tint.r as f32 / 200.0,
        tint.g as f32 / 200.0,
        tint.b as f32 / 200.0,
    );
    let gradient = |t: f32| {
        for i in 1..CHROME_STOPS.len() {
            let (p0, v0) = CHROME_STOPS[i - 1];
            let (p1, v1) = CHROME_STOPS[i];
            if t <= p1 {
                let local = (t - p0) / (p1 - p0);
                return lerp(0.5, lerp(v0, v1, local), refl);
            }
        }
        0.5
    };
    let (sw, sh) = (src.w, src.h);
    let mut px = vec![C::ZERO; (nw * nh) as usize];
    for y in 0..nh {
        let top = y < border;
        let bottom = y >= border + sh;
        for x in 0..nw {
            let left = x < border;
            let right = x >= border + sw;
            if !top && !bottom && !left && !right {
                continue;
            }
            let (il, it) = (border as f32, border as f32);
            let (ir, ib) = ((border + sw) as f32, (border + sh) as f32);
            let cxp = (x as f32).clamp(il, ir - 1.0);
            let cyp = (y as f32).clamp(it, ib - 1.0);
            let d = ((x as f32 - cxp).powi(2) + (y as f32 - cyp).powi(2)).sqrt();
            let cross = (d / border as f32).clamp(0.0, 1.0);
            let mut v = gradient(cross);
            let in_x = x >= border && x < border + sw;
            let in_y = y >= border && y < border + sh;
            let side = if y < border && in_x {
                bevel * 0.5
            } else if y >= border + sh && in_x {
                bevel * -0.4
            } else if x < border && in_y {
                bevel * 0.2
            } else if x >= border + sw && in_y {
                bevel * -0.2
            } else {
                0.0
            };
            v = (v + side * 0.12).clamp(0.0, 1.0);
            let br = v * 255.0;
            px[(y * nw + x) as usize] = C::new(
                clamp_to_byte(br * tr),
                clamp_to_byte(br * tg),
                clamp_to_byte(br * tb),
                255,
            );
        }
    }
    let base = Bitmap::from_pixels(nw, nh, px);
    let mut canvas = Canvas::from_bitmap(&base);
    let b = border as f32;
    canvas.draw_bitmap(src, b, b, &Paint::default());
    if p.b("inner_line") {
        canvas.draw_rect_xywh(
            b - 0.5,
            b - 0.5,
            sw as f32 + 1.0,
            sh as f32 + 1.0,
            &Paint::stroke_aa(C::rgb(40, 40, 45), 1.0),
        );
    }
    if p.b("outer_line") {
        canvas.draw_rect_xywh(
            0.75,
            0.75,
            nw as f32 - 1.5,
            nh as f32 - 1.5,
            &Paint::stroke_aa(C::rgb(20, 20, 22), 1.5),
        );
    }
    Ok(canvas.into_bitmap())
}

fn circuit(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(20, 300);
    let density = p.f("trace_density").clamp(10.0, 100.0) / 100.0;
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let board = p.c("board_color");
    let trace = p.c("trace_color");
    let glow = p.b("glow");
    let b = border as f32;
    let mut canvas = Canvas::new(nw, nh);
    let bp = Paint::fill(board);
    canvas.draw_rect_xywh(0.0, 0.0, nw as f32, b, &bp);
    canvas.draw_rect_xywh(0.0, (nh - border) as f32, nw as f32, b, &bp);
    canvas.draw_rect_xywh(0.0, b, b, src.h as f32, &bp);
    canvas.draw_rect_xywh((nw - border) as f32, b, b, src.h as f32, &bp);
    let tw = 1.5f32.max(b * 0.045);
    let pad_r = tw * 1.8;
    let grid = 6f32.max(b * 0.22) as i32;
    let glow_paint = Paint {
        style: Style::Stroke,
        stroke_width: tw * 3.5,
        mask_blur: Some(tw * 1.5),
        ..Paint::fill_aa(trace.with_alpha(45))
    };
    let trace_paint = Paint {
        style: Style::Stroke,
        stroke_width: tw,
        cap: Cap::Round,
        join: Join::Round,
        ..Paint::fill_aa(trace)
    };
    let pad = Paint::fill_aa(trace);
    let hole = Paint::fill_aa(board);
    let traces =
        |canvas: &mut Canvas, rx: f32, ry: f32, rw: f32, rh: f32, horizontal: bool, seed: i32| {
            canvas.save();
            canvas.clip_rect(rect(rx, ry, rx + rw, ry + rh), false);
            let primary = if horizontal { rw } else { rh };
            let cross_len = if horizontal { rh } else { rw };
            let n = (primary / grid as f32 * density) as i32 + 1;
            for i in 0..n {
                let h1 = hash01(i, seed, seed ^ 0x7F);
                let h2 = hash01(i, seed ^ 0x13, seed);
                let h3 = hash01(i, seed ^ 0x31, seed ^ 0x55);
                let along = h1 * primary;
                let across = h2 * cross_len * 0.85 + cross_len * 0.05;
                let tlen = (0.15 + h3 * 0.65) * primary * 0.5;
                let tend = (along + tlen).min(primary - 1.0);
                let (x1, y1, x2, y2) = if horizontal {
                    (rx + along, ry + across, rx + tend, ry + across)
                } else {
                    (rx + across, ry + along, rx + across, ry + tend)
                };
                if glow {
                    canvas.draw_line(x1, y1, x2, y2, &glow_paint);
                }
                canvas.draw_line(x1, y1, x2, y2, &trace_paint);
                canvas.draw_circle(x1, y1, pad_r, &pad);
                let via = pad_r * 0.85;
                canvas.draw_circle(x2, y2, via * 1.5, &pad);
                canvas.draw_circle(x2, y2, via * 0.65, &hole);
                let h4 = hash01(i, seed ^ 0x9A, seed ^ 0xCF);
                if h4 < 0.4 && tlen > grid as f32 * 1.5 {
                    let at = along + tlen * 0.5;
                    let ec =
                        across + if h4 < 0.2 { -1.0 } else { 1.0 } * grid as f32 * (0.5 + h3 * 0.8);
                    let (ex, ey, ex2, ey2) = if horizontal {
                        (rx + at, ry + across, rx + at, ry + ec)
                    } else {
                        (rx + across, ry + at, rx + ec, ry + at)
                    };
                    canvas.draw_line(ex, ey, ex2, ey2, &trace_paint);
                    canvas.draw_circle(ex2, ey2, pad_r * 0.9, &pad);
                }
            }
            canvas.restore();
        };
    let seed = 8157;
    traces(&mut canvas, 0.0, 0.0, nw as f32, b, true, seed ^ 0x1A);
    traces(
        &mut canvas,
        0.0,
        (nh - border) as f32,
        nw as f32,
        b,
        true,
        seed ^ 0x2B,
    );
    traces(&mut canvas, 0.0, b, b, src.h as f32, false, seed ^ 0x3C);
    traces(
        &mut canvas,
        (nw - border) as f32,
        b,
        b,
        src.h as f32,
        false,
        seed ^ 0x4D,
    );
    canvas.draw_bitmap(src, b, b, &Paint::default());
    if p.b("inner_line") {
        canvas.draw_rect_xywh(
            b - tw * 0.4,
            b - tw * 0.4,
            src.w as f32 + tw * 0.8,
            src.h as f32 + tw * 0.8,
            &Paint::stroke_aa(trace.with_alpha(200), tw * 0.8),
        );
    }
    Ok(canvas.into_bitmap())
}

fn claymation(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let chunk = p.i("chunkSize").clamp(3, 18);
    let smooth = p.f("smoothness").clamp(0.0, 100.0) / 100.0;
    let relief = p.f("relief").clamp(0.0, 100.0) / 100.0;
    let sat = p.f("saturation").clamp(-100.0, 100.0) / 100.0;
    let grain = p.f("textureGrain").clamp(0.0, 100.0) / 100.0;
    let seed = shared_next(1, i32::MAX);
    let (w, h) = (src.w, src.h);
    let (right, bottom) = (w - 1, h - 1);
    let br = (chunk / 4).clamp(1, 4);
    let smix = smooth * 0.9;
    let sf = 1.0 + sat;
    let cell = chunk as f32 * 1.85;
    let levels = (20 - chunk).clamp(5, 18);
    let qs = 255.0 / (levels - 1) as f32;
    let q = |v: f32| round(v / qs) * qs;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let yu = (y - br).max(0);
        let yd = (y + br).min(bottom);
        for x in 0..w {
            let xl = (x - br).max(0);
            let xr = (x + br).min(right);
            let c = src.get(x, y);
            let n = src.get(x, yu);
            let s = src.get(x, yd);
            let wv = src.get(xl, y);
            let e = src.get(xr, y);
            let nwv = src.get(xl, yu);
            let se = src.get(xr, yd);
            let avg = |f: fn(C) -> u8| {
                (f(c) as f32 * 3.0
                    + f(n) as f32 * 1.5
                    + f(s) as f32 * 1.5
                    + f(wv) as f32 * 1.5
                    + f(e) as f32 * 1.5
                    + f(nwv) as f32
                    + f(se) as f32)
                    / 10.0
            };
            let mut r = q(lerp(c.r as f32, avg(|c| c.r), smix));
            let mut g = q(lerp(c.g as f32, avg(|c| c.g), smix));
            let mut b = q(lerp(c.b as f32, avg(|c| c.b), smix));
            let fx = x as f32 / cell;
            let fy = y as f32 / cell;
            let cxi = fx.floor() as i32;
            let cyi = fy.floor() as i32;
            let tx = fx - cxi as f32;
            let ty = fy - cyi as f32;
            let coarse = lerp(
                lerp(hash01(cxi, cyi, seed), hash01(cxi + 1, cyi, seed), tx),
                lerp(
                    hash01(cxi, cyi + 1, seed),
                    hash01(cxi + 1, cyi + 1, seed),
                    tx,
                ),
                ty,
            );
            let ra = (((x as f32 + coarse * 13.0) / cell) * PI * 2.0).sin();
            let rb = (((y as f32 - coarse * 11.0) / cell) * PI * 2.0).cos();
            let ridge = ra * rb;
            let fine = (hash01(x, y, seed ^ 919) * 2.0 - 1.0) * grain;
            let ro = ((coarse - 0.5) * 2.0 + ridge * 0.65) * relief * 23.0;
            let go = fine * (4.0 + relief * 7.0);
            let off = ro + go;
            r += off + relief * 2.0;
            g += off;
            b += off - relief * 2.5;
            let gray = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            r = gray + (r - gray) * sf;
            g = gray + (g - gray) * sf;
            b = gray + (b - gray) * sf;
            px.push(C::new(
                clamp_to_byte(r),
                clamp_to_byte(g),
                clamp_to_byte(b),
                c.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn puff(x: f32, y: f32, cx: f32, cy: f32, radius: f32, f: &mut f32, l: &mut f32, s: &mut f32) {
    let nx = (x - cx) / (radius * 1.18);
    let dy = y - cy;
    let ny = if dy < 0.0 {
        dy / (radius * 0.96)
    } else {
        dy / (radius * 0.70)
    };
    let d = (nx * nx + ny * ny).sqrt();
    if d >= 1.0 {
        return;
    }
    let pf = 1.0 - smooth_step(0.10, 1.0, d);
    let ll = clamp01(0.72 - nx * 0.34 - ny * 0.58);
    let under = clamp01(ny * 0.90 + 0.12);
    *f = 1.0 - (1.0 - *f) * (1.0 - pf);
    *l = l.max(pf * ll);
    *s = s.max(pf * (1.0 - ll) * under);
}

fn cloud_layer(x: f32, y: f32, cell: f32, spawn_t: f32, seed: i32) -> (f32, f32, f32) {
    let (mut f, mut l, mut s) = (0f32, 0f32, 0f32);
    let cxi = (x / cell).floor() as i32;
    let cyi = (y / cell).floor() as i32;
    for oy in -1..=1 {
        for ox in -1..=1 {
            let nx = cxi + ox;
            let ny = cyi + oy;
            if hash01(nx, ny, seed) < spawn_t {
                continue;
            }
            let cx = (nx as f32 + 0.18 + hash01(nx, ny, seed ^ 0x0713) * 0.64) * cell;
            let cy = (ny as f32 + 0.22 + hash01(nx, ny, seed ^ 0x1731) * 0.52) * cell;
            let r = cell * (0.28 + hash01(nx, ny, seed ^ 0x2A51) * 0.20);
            puff(x, y, cx, cy, r, &mut f, &mut l, &mut s);
            let chance = hash01(nx, ny, seed ^ 0x3C17);
            if chance > 0.32 {
                let a = hash01(nx, ny, seed ^ 0x49D3) * (PI * 2.0);
                let off = r * (0.35 + hash01(nx, ny, seed ^ 0x5123) * 0.40);
                let sr = r * (0.42 + hash01(nx, ny, seed ^ 0x6891) * 0.28);
                puff(
                    x,
                    y,
                    cx + a.cos() * off,
                    cy + a.sin() * off * 0.72,
                    sr,
                    &mut f,
                    &mut l,
                    &mut s,
                );
            }
        }
    }
    (f, l, s)
}

fn clouds(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let coverage = p.f("coverage").clamp(0.0, 100.0) / 100.0;
    let scale = p.f("scale").clamp(0.0, 100.0) / 100.0;
    let hb = p.f("heightBias").clamp(0.0, 100.0) / 100.0;
    let soft = p.f("softness").clamp(0.0, 100.0) / 100.0;
    let sun = p.f("sunlight").clamp(0.0, 100.0) / 100.0;
    if coverage <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let cell = 36.0 + scale * 140.0;
    let dcell = 18f32.max(cell * 0.52);
    let wf = 0.0045 + (1.0 - scale) * 0.012;
    let wa = 10.0 + scale * 40.0;
    let ef = 0.010 + (1.0 - scale) * 0.020;
    let spawn = 0.82 - coverage * 0.45;
    let iw = if w > 1 { 1.0 / (w as f32 - 1.0) } else { 0.0 };
    let ih = if h > 1 { 1.0 / (h as f32 - 1.0) } else { 0.0 };
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let v = y as f32 * ih;
        let band = 0.20 + hb * 0.55;
        let vm = 1.0 - smooth_step(band, 1.0, v);
        for x in 0..w {
            let s = src.get(x, y);
            let (sr, sg, sb) = (s.r as f32 / 255.0, s.g as f32 / 255.0, s.b as f32 / 255.0);
            let u = x as f32 * iw;
            let (xf, yf) = (x as f32, y as f32);
            let wx = (fractal_noise(xf * wf + 4.7, yf * wf * 0.92 - 2.1, 3, 2.02, 0.56, 1411)
                * 2.0
                - 1.0)
                * wa;
            let wy = (fractal_noise(xf * wf * 0.88 - 6.4, yf * wf + 3.6, 3, 2.10, 0.54, 2311)
                * 2.0
                - 1.0)
                * (wa * 0.65);
            let (sx, sy) = (xf + wx, yf + wy);
            let (mf, ml, ms) = cloud_layer(sx, sy, cell, spawn, 3571);
            let (df, dl, ds) = cloud_layer(
                sx + cell * 0.24,
                sy - cell * 0.15,
                dcell,
                spawn + 0.08,
                5171,
            );
            let field = 1.0 - (1.0 - mf) * (1.0 - df * 0.72);
            let noise = fractal_noise(sx * ef - 3.2, sy * ef + 5.9, 3, 2.00, 0.56, 6211);
            let breakup = 1.0 - (noise * 2.0 - 1.0).abs();
            let curls = fractal_noise(
                sx * ef * 2.2 + 11.3,
                sy * ef * 1.9 - 7.6,
                2,
                2.12,
                0.58,
                7351,
            );
            let bank = 0.5 + 0.5 * (u * 5.6 + v * 1.7 + 0.8).sin();
            let mut st = field * (0.72 + 0.22 * breakup + 0.10 * curls);
            st = clamp01(st + bank * 0.07);
            let thr = 0.30 - coverage * 0.18 - vm * 0.08;
            let mut mask = smooth_step(thr, 0.84, st * (0.70 + 0.30 * vm));
            mask = mask.powf(1.45 - soft * 0.95);
            let cl = ml.max(dl * 0.88);
            let cs = ms.max(ds * 0.92);
            let sb_ = clamp01(0.68 + ((0.78 - u) * 0.42 + (0.40 - v) * 0.74));
            let hl = sun * mask * (0.18 + 0.34 * cl + 0.18 * sb_);
            let shade = mask * ((0.08 + 0.22 * cs) * (0.90 - sun * 0.20));
            let body = 0.79 + 0.12 * st + 0.05 * breakup;
            let cr = clamp01(body - shade + hl * 1.06);
            let cg = clamp01((body + 0.03) - shade * 0.94 + hl);
            let cb = clamp01((body + 0.08) - shade * 0.84 + hl * 0.82);
            let haze = coverage * vm * (0.006 + 0.016 * st);
            let hr = lerp(sr, 0.84, haze);
            let hg = lerp(sg, 0.89, haze);
            let hbv = lerp(sb, 0.95, haze);
            let blend = mask * (0.52 + coverage * 0.34);
            px.push(C::new(
                clamp_to_byte(lerp(hr, cr, blend) * 255.0),
                clamp_to_byte(lerp(hg, cg, blend) * 255.0),
                clamp_to_byte(lerp(hbv, cb, blend) * 255.0),
                s.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn color_depth(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let bits = p.i("bitsPerChannel").clamp(1, 8);
    if bits == 8 {
        return Ok(src.clone());
    }
    let colors = 2f64.powi(bits);
    let interval = 255.0 / (colors - 1.0);
    let remap = |c: u8| roundd(roundd(c as f64 / interval) * interval) as u8;
    let px = src
        .px
        .iter()
        .map(|c| C::new(remap(c.r), remap(c.g), remap(c.b), c.a))
        .collect();
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}

fn color_halftone(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let size = 4.max(p.i("size"));
    let (w, h) = (src.w, src.h);
    let mut canvas = Canvas::new(w, h);
    canvas.clear(C::WHITE);
    let channels = [
        (C::rgb(0, 255, 255), 15f32),
        (C::rgb(255, 0, 255), 75.0),
        (C::rgb(255, 255, 0), 0.0),
        (C::rgb(0, 0, 0), 45.0),
    ];
    for (color, _angle) in channels {
        let paint = Paint::fill_aa(color).with_blend(Blend::Multiply);
        let mut gy = -size;
        while gy < h + size * 2 {
            let mut gx = -size;
            while gx < w + size * 2 {
                let sx = gx.clamp(0, w - 1);
                let sy = gy.clamp(0, h - 1);
                let sc = src.get(sx, sy);
                let v = if color.r == 0 && color.g == 0 && color.b == 0 {
                    1.0 - (0.2126 * sc.r as f32 + 0.7152 * sc.g as f32 + 0.0722 * sc.b as f32)
                        / 255.0
                } else if color.r == 0 {
                    1.0 - sc.r as f32 / 255.0
                } else if color.g == 0 {
                    1.0 - sc.g as f32 / 255.0
                } else {
                    1.0 - sc.b as f32 / 255.0
                };
                let r = size as f32 * 0.5 * v;
                if r > 0.5 {
                    canvas.draw_circle(gx as f32, gy as f32, r, &paint);
                }
                gx += size;
            }
            gy += size;
        }
    }
    Ok(canvas.into_bitmap())
}

fn color_splash(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let target = p.i("hue") as f32;
    let tol = p.i("tolerance") as f32;
    let px = src
        .px
        .iter()
        .map(|c| {
            let (hue, sat, _) = c.to_hsl();
            let mut diff = (hue - target).abs();
            if diff > 180.0 {
                diff = 360.0 - diff;
            }
            if diff <= tol && sat > 5.0 {
                *c
            } else {
                let g = (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) as u8;
                C::new(g, g, g, c.a)
            }
        })
        .collect();
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}
