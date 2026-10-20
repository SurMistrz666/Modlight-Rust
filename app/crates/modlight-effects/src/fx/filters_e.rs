//! Filters, part E: Double exposure .. Frost ice border (and Fake hair, listed under
//! Drawings).

use std::f32::consts::PI;

use modlight_core::icons as l;
use modlight_editor::effects::Category;
use tiny_skia::PathBuilder;

use crate::def::*;
use crate::dotnet::{DotNetRandom, shared_next};
use crate::fx::filters_d::convolve3x3;
use crate::helpers::{
    apply_contrast, bilinear_sample, blurred_clamp, clamp_to_byte, clamp01, fractal_noise, hash01,
    lerp, luminance01, screen, smooth_step,
};
use crate::sk::{Bitmap, Blend, C, Canvas, Cap, Join, Paint, Style, rect_xywh, round};

const F: Category = Category::Filters;

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "double_exposure",
            "Double exposure",
            F,
            l::IMAGES,
            "Blends a blurred ghost of the image over itself to simulate double exposure.",
            double_exposure,
        )
        .params(|| {
            vec![
                float_slider("blend_amount", "Blend amount", 0.0, 100.0, 62.0),
                int_numeric("offset_x", "Offset X", -500, 500, 22),
                int_numeric("offset_y", "Offset Y", -500, 500, -14),
                float_slider("ghost_blur", "Ghost blur", 0.0, 30.0, 8.0),
                float_slider("highlight_bias", "Highlight bias", 0.0, 100.0, 55.0),
                float_slider("contrast", "Contrast", 50.0, 200.0, 112.0),
            ]
        }),
        Def::new(
            "dream_glow",
            "Dream glow",
            F,
            l::CLOUD,
            "Creates a dreamy soft glow by blending a blurred copy over the original.",
            dream_glow,
        )
        .params(|| {
            vec![
                int_slider("radius", "Blur radius", 1, 50, 15),
                float_slider("opacity", "Blend opacity", 0.0, 100.0, 50.0),
            ]
        }),
        Def::new(
            "edge_detect",
            "Edge detect",
            F,
            l::SCAN_SEARCH,
            "Detects visible edges in the image.",
            |s, _| {
                Ok(convolve3x3(
                    s,
                    &[-1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                    1.0,
                    127.0,
                ))
            },
        )
        .immediate(),
        Def::new(
            "edge_feather",
            "Edge feather",
            F,
            l::CIRCLE_DASHED,
            "Softens the edges of non-transparent regions with a feathered alpha falloff.",
            edge_feather,
        )
        .params(|| vec![float_slider("radius", "Radius", 0.0, 100.0, 12.0)]),
        Def::new(
            "emboss",
            "Emboss",
            F,
            l::STAMP,
            "Applies an emboss effect.",
            |s, _| {
                Ok(convolve3x3(
                    s,
                    &[-1.0, 0.0, -1.0, 0.0, 4.0, 0.0, -1.0, 0.0, -1.0],
                    1.0,
                    127.0,
                ))
            },
        )
        .immediate(),
        Def::new(
            "embroidery",
            "Embroidery",
            F,
            l::STAMP,
            "Simulates a cross-stitch embroidery pattern.",
            embroidery,
        )
        .params(|| {
            vec![
                int_slider("stitch_size", "Stitch Size", 4, 40, 10),
                int_slider("color_count", "Color Count", 2, 64, 16),
                checkbox("show_grid", "Show Grid", true),
                color("fabric_color", "Fabric Color", C::rgb(240, 235, 220)),
            ]
        }),
        Def::new(
            "engraving",
            "Engraving",
            F,
            l::PEN_LINE,
            "Simulates a line engraving effect like banknote illustrations.",
            engraving,
        )
        .params(|| {
            vec![
                fine(
                    float_slider("line_spacing", "Line Spacing", 2.0, 20.0, 5.0),
                    0.5,
                    "{}{0:0.0}",
                ),
                float_slider("angle", "Angle", 0.0, 180.0, 45.0),
                fine(
                    float_slider("line_width_max", "Max Line Width", 0.5, 8.0, 2.5),
                    0.1,
                    "{}{0:0.0}",
                ),
                color("line_color", "Line Color", C::rgb(20, 20, 40)),
                color("paper_color", "Paper Color", C::rgb(245, 240, 230)),
            ]
        }),
        Def::new(
            "etched_glass",
            "Etched glass",
            F,
            l::GLASS_WATER,
            "Simulates an etched glass surface with frost, engraving, and refraction effects.",
            etched_glass,
        )
        .params(|| {
            vec![
                float_slider("frost", "Frost", 0.0, 100.0, 48.0),
                float_slider("engrave", "Engrave", 0.0, 100.0, 68.0),
                float_slider("refraction", "Refraction", 0.0, 100.0, 18.0),
                float_slider("highlight", "Highlight", 0.0, 100.0, 42.0),
                float_slider("background_fade", "Background fade", 0.0, 100.0, 38.0),
            ]
        }),
        Def::new(
            "fake_hair",
            "Fake Hair",
            Category::Drawings,
            l::SCISSORS,
            "Scatters realistic hair strands over the image to annoy your friends.",
            fake_hair,
        )
        .params(|| {
            vec![
                int_slider("count", "Hair Count", 1, 100, 1),
                float_slider("length", "Length", 20.0, 600.0, 180.0),
                fine(
                    float_slider("thickness", "Thickness", 0.5, 4.0, 1.0),
                    0.1,
                    "{}{0:0.0}",
                ),
                float_slider("curliness", "Curliness", 0.0, 100.0, 30.0),
                color("hair_color", "Hair Color", C::new(20, 15, 10, 220)),
            ]
        }),
        Def::new(
            "film_grain",
            "Film grain",
            F,
            l::FILM,
            "Adds organic film-like grain texture.",
            film_grain,
        )
        .params(|| {
            vec![
                int_slider("amount", "Amount", 1, 100, 30),
                checkbox("monochrome", "Monochrome", true),
            ]
        }),
        Def::new(
            "film_strip_frame",
            "Film strip frame",
            F,
            l::FILM,
            "Wraps the image in a cinema film strip frame with sprocket holes.",
            film_strip,
        )
        .params(|| {
            vec![
                int_slider("border_size", "Border size", 10, 120, 40),
                float_slider("hole_size", "Hole size", 20.0, 80.0, 55.0),
                color("frame_color", "Frame color", C::rgb(15, 15, 15)),
                checkbox("side_strips", "Side strips", false),
                checkbox("scratches", "Scratches", true),
            ]
        }),
        Def::new(
            "frost_ice_border",
            "Frost ice border",
            F,
            l::SNOWFLAKE,
            "Adds a frosted ice crystal border with rime buildup and icy blue highlights.",
            frost_ice,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 10, 300, 55),
                color("ice_color", "Ice color", C::rgb(200, 225, 255)),
                float_slider("crystallization", "Crystallization", 0.0, 100.0, 65.0),
                float_slider("transparency", "Transparency", 0.0, 80.0, 25.0),
                checkbox("sparkle", "Ice sparkle", true),
            ]
        }),
    ]
}

fn double_exposure(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let blend = p.f("blend_amount").clamp(0.0, 100.0) / 100.0;
    let blur = p.f("ghost_blur").clamp(0.0, 30.0);
    let hb = p.f("highlight_bias").clamp(0.0, 100.0) / 100.0;
    let contrast = p.f("contrast").clamp(50.0, 200.0) / 100.0;
    let (ox, oy) = (p.i("offset_x"), p.i("offset_y"));
    if blend <= 0.0001 && blur <= 0.0001 && ox == 0 && oy == 0 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let ghost_bmp = if blur > 0.01 {
        Some(blurred_clamp(src, blur))
    } else {
        None
    };
    let gp = ghost_bmp.as_ref().map_or(&src.px, |b| &b.px);
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let g = bilinear_sample(gp, w, h, (x - ox) as f32, (y - oy) as f32);
            let (sr, sg, sb) = (s.r as f32 / 255.0, s.g as f32 / 255.0, s.b as f32 / 255.0);
            let gl = luminance01(g);
            let gr = apply_contrast(
                lerp(g.r as f32 / 255.0, gl * (1.12 + hb * 0.48), 0.68),
                contrast,
            );
            let gg = apply_contrast(lerp(g.g as f32 / 255.0, gl * 0.95, 0.78), contrast);
            let gb = apply_contrast(lerp(g.b as f32 / 255.0, gl * 0.82, 0.84), contrast);
            let lb = blend * (0.55 + gl * 0.45);
            let r = lerp(sr, screen(sr, gr), lb);
            let gv = lerp(sg, screen(sg, gg), lb * 0.92);
            let b = lerp(sb, screen(sb, gb), lb * 0.88);
            px.push(C::new(
                clamp_to_byte(r * 255.0),
                clamp_to_byte(gv * 255.0),
                clamp_to_byte(b * 255.0),
                s.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn dream_glow(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let alpha = (p.f("opacity") / 100.0).clamp(0.0, 1.0);
    let r = p.i("radius");
    if alpha <= 0.0 || r <= 0 {
        return Ok(src.clone());
    }
    let blurred = crate::helpers::blur_bitmap(src, r as f32, r as f32);
    let mut canvas = Canvas::from_bitmap(src);
    canvas.draw_bitmap(
        &blurred,
        0.0,
        0.0,
        &Paint::alpha((255.0 * alpha) as u8).with_blend(Blend::Screen),
    );
    Ok(canvas.into_bitmap())
}

/// `ApplyTransparentBlur`: blur with transparent padding (`2 * radius`, at least 2),
/// sigma `radius / 3`, cut back.
pub fn transparent_blur(src: &Bitmap, radius: f32) -> Bitmap {
    if radius <= 0.01 {
        return src.clone();
    }
    let pad = 2.max((radius * 2.0).ceil() as i32);
    let mut ex = Bitmap::new(src.w + pad * 2, src.h + pad * 2);
    for y in 0..src.h {
        for x in 0..src.w {
            let i = ex.idx(x + pad, y + pad);
            ex.px[i] = src.get(x, y);
        }
    }
    let sigma = 0.001f32.max(radius / 3.0);
    let blurred = crate::helpers::blur_bitmap(&ex, sigma, sigma);
    let mut out = Bitmap::new(src.w, src.h);
    for y in 0..src.h {
        for x in 0..src.w {
            out.px[(y * src.w + x) as usize] = blurred.get(x + pad, y + pad);
        }
    }
    out
}

fn edge_feather(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let radius = p.f("radius").clamp(0.0, 100.0);
    if radius <= 0.01 {
        return Ok(src.clone());
    }
    let mask: Vec<C> = src
        .px
        .iter()
        .map(|c| {
            let a = if c.a > 16 { 255 } else { 0 };
            C::new(a, a, a, a)
        })
        .collect();
    let mask = Bitmap::from_pixels(w, h, mask);
    let fm = transparent_blur(&mask, radius);
    let px = src
        .px
        .iter()
        .zip(&fm.px)
        .map(|(s, m)| {
            C::new(
                s.r,
                s.g,
                s.b,
                clamp_to_byte(s.a as f32 * (m.a as f32 / 255.0)),
            )
        })
        .collect();
    Ok(Bitmap::from_pixels(w, h, px))
}

fn embroidery(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    let size = p.i("stitch_size").clamp(2, 100);
    let colors = p.i("color_count").clamp(2, 256);
    let levels = 2.max((colors as f32).powf(1.0 / 3.0).ceil() as i32);
    let fabric = p.c("fabric_color");
    let mut canvas = Canvas::new(w, h);
    canvas.clear(fabric);
    let cols = (w + size - 1) / size;
    let rows = (h + size - 1) / size;
    let q = |v: u8| {
        let step = 255.0 / (levels - 1) as f32;
        let qi = round(v as f32 / step) as i32;
        (round(qi as f32 * step) as i32).clamp(0, 255) as u8
    };
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
            let color = C::rgb(q((sr / n) as u8), q((sg / n) as u8), q((sb / n) as u8));
            let margin = size as f32 * 0.1;
            let (sx, sy) = (x0 as f32 + margin, y0 as f32 + margin);
            let (ex, ey) = (x1 as f32 - margin, y1 as f32 - margin);
            let paint = Paint {
                style: Style::Stroke,
                stroke_width: 1f32.max(size as f32 * 0.15),
                cap: Cap::Round,
                join: Join::Round,
                ..Paint::fill_aa(color)
            };
            canvas.draw_line(sx, sy, ex, ey, &paint);
            canvas.draw_line(sx, ey, ex, sy, &paint);
        }
    }
    if p.b("show_grid") {
        let gc = C::new(
            (fabric.r as i32 - 30).clamp(0, 255) as u8,
            (fabric.g as i32 - 30).clamp(0, 255) as u8,
            (fabric.b as i32 - 30).clamp(0, 255) as u8,
            80,
        );
        let gp = Paint::stroke(gc, 1.0);
        for gx in 0..=cols {
            let x = (gx * size).min(w - 1) as f32;
            canvas.draw_line(x, 0.0, x, h as f32, &gp);
        }
        for gy in 0..=rows {
            let y = (gy * size).min(h - 1) as f32;
            canvas.draw_line(0.0, y, w as f32, y, &gp);
        }
    }
    Ok(canvas.into_bitmap())
}

fn engraving(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    let spacing = p.f("line_spacing").clamp(1.0, 50.0);
    let max_w = p.f("line_width_max").clamp(0.1, 20.0);
    let ar = p.f("angle") * PI / 180.0;
    let lum: Vec<f32> = src
        .px
        .iter()
        .map(|c| (0.299 * c.r as f32 + 0.587 * c.g as f32 + 0.114 * c.b as f32) / 255.0)
        .collect();
    let mut canvas = Canvas::new(w, h);
    canvas.clear(p.c("paper_color"));
    let (ca, sa) = (ar.cos(), ar.sin());
    let diag = ((w * w + h * h) as f32).sqrt();
    let count = (diag / spacing).ceil() as i32 + 1;
    let line_color = p.c("line_color");
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    for i in -count..=count {
        let off = i as f32 * spacing;
        let bx = cx + ca * off;
        let by = cy + sa * off;
        let (x0, y0) = (bx - sa * diag, by + ca * diag);
        let (x1, y1) = (bx + sa * diag, by - ca * diag);
        let steps = (diag * 2.0 / spacing) as i32 + 1;
        let (mut prx, mut pry) = (x0, y0);
        for s in 1..=steps {
            let t = s as f32 / steps as f32;
            let px = x0 + (x1 - x0) * t;
            let py = y0 + (y1 - y0) * t;
            let sx = round(px) as i32;
            let sy = round(py) as i32;
            if sx >= 0 && sx < w && sy >= 0 && sy < h {
                let sw = (1.0 - lum[(sy * w + sx) as usize]) * max_w;
                if sw > 0.05 {
                    let paint = Paint {
                        style: Style::Stroke,
                        stroke_width: sw,
                        cap: Cap::Round,
                        ..Paint::fill_aa(line_color)
                    };
                    canvas.draw_line(prx, pry, px, py, &paint);
                }
            }
            prx = px;
            pry = py;
        }
    }
    Ok(canvas.into_bitmap())
}

fn etched_glass(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let frost = p.f("frost").clamp(0.0, 100.0) / 100.0;
    let engrave = p.f("engrave").clamp(0.0, 100.0) / 100.0;
    let refraction = p.f("refraction").clamp(0.0, 100.0) / 100.0;
    let highlight = p.f("highlight").clamp(0.0, 100.0) / 100.0;
    let fade = p.f("background_fade").clamp(0.0, 100.0) / 100.0;
    let (w, h) = (src.w, src.h);
    let rp = 0.6 + refraction * 5.4;
    let lum = |c: C| 0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let nx = (hash01(x / 3, y / 3, 2011) * 2.0 - 1.0) * rp;
            let ny = (hash01(x / 3, y / 3, 3001) * 2.0 - 1.0) * rp;
            let refr = bilinear_sample(&src.px, w, h, x as f32 + nx, y as f32 + ny);
            let (l, r, t, b) = (
                (x - 2).max(0),
                (x + 2).min(w - 1),
                (y - 2).max(0),
                (y + 2).min(h - 1),
            );
            let cs = [
                s,
                src.get(l, y),
                src.get(r, y),
                src.get(x, t),
                src.get(x, b),
            ];
            let avg =
                |f: fn(&C) -> u8| clamp_to_byte(cs.iter().map(|c| f(c) as f32).sum::<f32>() / 5.0);
            let blurred = C::new(avg(|c| c.r), avg(|c| c.g), avg(|c| c.b), avg(|c| c.a));
            let li = lum(refr) / 255.0;
            let (l1, r1, t1, b1) = (
                (x - 1).max(0),
                (x + 1).min(w - 1),
                (y - 1).max(0),
                (y + 1).min(h - 1),
            );
            let edge = clamp01(
                ((lum(src.get(r1, y)) / 255.0 - lum(src.get(l1, y)) / 255.0).abs()
                    + (lum(src.get(x, b1)) / 255.0 - lum(src.get(x, t1)) / 255.0).abs())
                    * 1.25,
            );
            let br = lerp(s.r as f32 / 255.0, blurred.r as f32 / 255.0, fade * 0.72);
            let bg = lerp(s.g as f32 / 255.0, blurred.g as f32 / 255.0, fade * 0.72);
            let bb = lerp(s.b as f32 / 255.0, blurred.b as f32 / 255.0, fade * 0.72);
            let gray = li * 0.88 + 0.07;
            let mut gr = lerp(br, gray + 0.07, frost * 0.58);
            let mut gg = lerp(bg, gray + 0.08, frost * 0.60);
            let mut gb = lerp(bb, gray + 0.10, frost * 0.64);
            let eng = edge * engrave;
            gr *= 1.0 - eng * 0.36;
            gg *= 1.0 - eng * 0.38;
            gb *= 1.0 - eng * 0.40;
            let spec = edge * highlight;
            gr = lerp(gr, 1.0, spec * 0.16);
            gg = lerp(gg, 1.0, spec * 0.18);
            gb = lerp(gb, 1.0, spec * 0.22);
            px.push(C::new(
                clamp_to_byte(gr * 255.0),
                clamp_to_byte(gg * 255.0),
                clamp_to_byte(gb * 255.0),
                s.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

/// The xorshift `NextFloat` FakeHair and others use.
pub fn xorshift_float(state: &mut i32) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state & 0x7FFF_FFFF) as f32 / i32::MAX as f32
}

fn fake_hair(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let count = p.i("count").clamp(0, 1000);
    if count == 0 {
        return Ok(src.clone());
    }
    let length = p.f("length").clamp(5.0, 2000.0);
    let thickness = p.f("thickness").clamp(0.2, 20.0);
    let curl = p.f("curliness").clamp(0.0, 100.0) / 100.0;
    let hc = p.c("hair_color");
    let (w, h) = (src.w, src.h);
    let mut canvas = Canvas::from_bitmap(src);
    // Environment.TickCount: a different scatter on every run.
    let mut rng = shared_next(1, i32::MAX);
    if rng == 0 {
        rng = 1;
    }
    for _ in 0..count {
        let sx = xorshift_float(&mut rng) * w as f32;
        let sy = xorshift_float(&mut rng) * h as f32;
        let base = xorshift_float(&mut rng) * PI * 2.0;
        let cv = (xorshift_float(&mut rng) - 0.5) * 0.3;
        let hr = clamp_to_byte(hc.r as f32 + hc.r as f32 * cv);
        let hg = clamp_to_byte(hc.g as f32 + hc.g as f32 * cv);
        let hb = clamp_to_byte(hc.b as f32 + hc.b as f32 * cv);
        let ha = clamp_to_byte(180.0 + xorshift_float(&mut rng) * 75.0);
        let ht = thickness * (0.6 + xorshift_float(&mut rng) * 0.8);
        let hl = length * (0.5 + xorshift_float(&mut rng) * 0.7);
        let segs = 8.max((hl / 6.0) as i32);
        let seg_len = hl / segs as f32;
        let max_turn = curl * 0.35;
        let mut pb = PathBuilder::new();
        let (mut x, mut y) = (sx, sy);
        pb.move_to(x, y);
        let mut angle = base;
        for _ in 1..=segs {
            angle += (xorshift_float(&mut rng) - 0.5) * 2.0 * max_turn;
            x += angle.cos() * seg_len;
            y += angle.sin() * seg_len;
            pb.line_to(x, y);
        }
        let Some(path) = pb.finish() else { continue };
        let paint = Paint {
            style: Style::Stroke,
            stroke_width: ht,
            cap: Cap::Round,
            join: Join::Round,
            ..Paint::fill_aa(C::new(hr, hg, hb, ha))
        };
        canvas.draw_path(&path, &paint);
        if ht > 0.8 {
            let sheen = Paint {
                stroke_width: ht * 0.3,
                color: C::new(
                    clamp_to_byte(hr as f32 + 60.0),
                    clamp_to_byte(hg as f32 + 50.0),
                    clamp_to_byte(hb as f32 + 40.0),
                    ha / 4,
                ),
                ..paint.clone()
            };
            canvas.save();
            canvas.translate(0.3, -0.3);
            canvas.draw_path(&path, &sheen);
            canvas.restore();
        }
    }
    Ok(canvas.into_bitmap())
}

fn film_grain(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let mut rng = DotNetRandom::new(42);
    let range = (p.i("amount") as f32 * 2.55) as i32;
    let mono = p.b("monochrome");
    let cb = |v: i32| v.clamp(0, 255) as u8;
    let px = src
        .px
        .iter()
        .map(|c| {
            if mono {
                let n = rng.next_range(-range, range + 1);
                C::new(
                    cb(c.r as i32 + n),
                    cb(c.g as i32 + n),
                    cb(c.b as i32 + n),
                    c.a,
                )
            } else {
                let r = cb(c.r as i32 + rng.next_range(-range, range + 1));
                let g = cb(c.g as i32 + rng.next_range(-range, range + 1));
                let b = cb(c.b as i32 + rng.next_range(-range, range + 1));
                C::new(r, g, b, c.a)
            }
        })
        .collect();
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}

fn film_strip(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(10, 120);
    let frac = p.f("hole_size").clamp(20.0, 80.0) / 100.0;
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let b = border as f32;
    let mut canvas = Canvas::new(nw, nh);
    let fp = Paint::fill(p.c("frame_color"));
    canvas.draw_rect_xywh(0.0, 0.0, nw as f32, b, &fp);
    canvas.draw_rect_xywh(0.0, (nh - border) as f32, nw as f32, b, &fp);
    canvas.draw_rect_xywh(0.0, b, b, src.h as f32, &fp);
    canvas.draw_rect_xywh((nw - border) as f32, b, b, src.h as f32, &fp);
    canvas.draw_bitmap(src, b, b, &Paint::default());
    if p.b("scratches") {
        let mut seed: i32 = 7391;
        for _ in 0..10 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let x = ((seed as u32) % (nw as u32)) as f32 / nw as f32 * nw as f32;
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let alpha = (((seed as u32) % 100) as f32 / 100.0) * 0.35 + 0.07;
            let paint = Paint::stroke(C::new(140, 140, 140, (alpha * 255.0) as u8), 0.7);
            canvas.draw_line(x, 0.0, x, nh as f32, &paint);
        }
    }
    let hh = b * frac;
    let hw = hh;
    let hr = hh * 0.15;
    let mv = (b - hh) / 2.0;
    let step = hw * 1.75;
    let hole = Paint::default().with_blend(Blend::Clear);
    let mut x = step * 0.4;
    while x + hw <= nw as f32 - step * 0.2 {
        canvas.draw_round_rect(rect_xywh(x, mv, hw, hh), hr, hr, &hole);
        canvas.draw_round_rect(
            rect_xywh(x, (nh - border) as f32 + mv, hw, hh),
            hr,
            hr,
            &hole,
        );
        x += step;
    }
    if p.b("side_strips") {
        let mh = (b - hw) / 2.0;
        let mut y = b + step * 0.4;
        while y + hh <= (nh - border) as f32 - step * 0.2 {
            canvas.draw_round_rect(rect_xywh(mh, y, hw, hh), hr, hr, &hole);
            canvas.draw_round_rect(
                rect_xywh((nw - border) as f32 + mh, y, hw, hh),
                hr,
                hr,
                &hole,
            );
            y += step;
        }
    }
    canvas.draw_rect_xywh(
        b - 0.6,
        b - 0.6,
        src.w as f32 + 1.2,
        src.h as f32 + 1.2,
        &Paint::stroke_aa(C::rgb(60, 60, 60), 1.2),
    );
    Ok(canvas.into_bitmap())
}

/// The cross ratio of the frame effects: 0 at the image, 1 at the outer edge.
pub fn cross_ratio(x: i32, y: i32, border: i32, sw: i32, sh: i32) -> f32 {
    let (il, it) = (border as f32, border as f32);
    let (ir, ib) = ((border + sw) as f32, (border + sh) as f32);
    let cx = (x as f32).clamp(il, ir - 1.0);
    let cy = (y as f32).clamp(it, ib - 1.0);
    let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
    (d / border as f32).clamp(0.0, 1.0)
}

fn frost_ice(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(10, 300);
    let crystal = p.f("crystallization").clamp(0.0, 100.0) / 100.0;
    let transp = p.f("transparency").clamp(0.0, 80.0) / 100.0;
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let ice = p.c("ice_color");
    let (ir, ig, ib) = (
        ice.r as f32 / 255.0,
        ice.g as f32 / 255.0,
        ice.b as f32 / 255.0,
    );
    let seed = 4519;
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
            let cr = cross_ratio(x, y, border, src.w, src.h);
            let thick = smooth_step(0.0, 0.5, cr);
            let coarse = fractal_noise(x as f32 * 0.02, y as f32 * 0.02, 4, 2.5, 0.45, seed);
            let finen = fractal_noise(x as f32 * 0.08, y as f32 * 0.08, 3, 2.0, 0.5, seed ^ 0x3F);
            let pat = coarse + finen * 0.5;
            let cz = if crystal > 0.0 {
                pat.clamp(0.0, 1.0).powf(1.0 - crystal * 0.7)
            } else {
                pat
            };
            let hl = smooth_step(0.55, 0.75, cz);
            let mut r = lerp(ir, 1.0, hl * 0.6);
            let mut g = lerp(ig, 1.0, hl * 0.5);
            let b = lerp(ib, 1.0, hl * 0.35);
            let vein = smooth_step(0.2, 0.35, pat);
            r -= vein * 0.12;
            g -= vein * 0.06;
            let mut a = lerp(1.0 - transp, 1.0, thick);
            a *= 0.85 + cz * 0.15;
            a = a.clamp(0.0, 1.0);
            px[(y * nw + x) as usize] = C::new(
                clamp_to_byte(r * 255.0),
                clamp_to_byte(g * 255.0),
                clamp_to_byte(b * 255.0),
                clamp_to_byte(a * 255.0),
            );
        }
    }
    let base = Bitmap::from_pixels(nw, nh, px);
    let mut canvas = Canvas::from_bitmap(&base);
    canvas.draw_bitmap(src, border as f32, border as f32, &Paint::default());
    if p.b("sparkle") {
        let n = (nw + nh) / 12;
        for i in 0..n {
            let hx = hash01(i, seed ^ 0xAA, seed);
            let hy = hash01(i, seed ^ 0xBB, seed ^ 0x11);
            let hs = hash01(i, seed ^ 0xCC, seed ^ 0x22);
            let sx = (hx * nw as f32) as i32;
            let sy = (hy * nh as f32) as i32;
            let in_border =
                sx < border || sx >= border + src.w || sy < border || sy >= border + src.h;
            if !in_border {
                continue;
            }
            let radius = 0.8 + hs * 2.2;
            canvas.draw_circle(
                sx as f32,
                sy as f32,
                radius,
                &Paint::fill_aa(C::new(255, 255, 255, (140.0 + hs * 115.0) as u8)),
            );
            if hs > 0.5 {
                let len = radius * 2.5;
                let sp = Paint::stroke_aa(C::new(255, 255, 255, (80.0 + hs * 80.0) as u8), 0.5);
                canvas.draw_line(sx as f32 - len, sy as f32, sx as f32 + len, sy as f32, &sp);
                canvas.draw_line(sx as f32, sy as f32 - len, sx as f32, sy as f32 + len, &sp);
            }
        }
    }
    Ok(canvas.into_bitmap())
}
