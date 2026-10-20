//! Filters, part O: Windows 11 .. Zoom blur.

use std::f32::consts::PI;

use modlight_core::icons as l;
use modlight_editor::effects::Category;
use tiny_skia::{Path, PathBuilder};

use crate::def::*;
use crate::helpers::{bilinear_sample, clamp_to_byte, clamp01, hash01, smooth_step};
use crate::sk::{Bitmap, C, Canvas, Cap, Font, Paint, Shader, Tile, rect, round};

const F: Category = Category::Filters;

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "windows11_window",
            "Windows 11",
            F,
            l::MONITOR,
            "Wraps the image in a Windows 11-style window frame with a title bar and window controls.",
            windows11,
        )
        .params(|| {
            vec![
                text("title", "Title", ""),
                int_slider("corner_radius", "Corner radius", 0, 16, 8),
                checkbox("dark_mode", "Dark mode", false),
                int_slider("shadow_size", "Shadow size", 0, 60, 16),
                int_slider("padding", "Padding", 0, 100, 20),
            ]
        }),
        Def::new(
            "woodcut_print",
            "Woodcut Print",
            F,
            l::AXE,
            "Simulates a woodcut or linocut print style.",
            woodcut,
        )
        .params(|| {
            vec![
                float_slider("threshold", "Threshold", 0.0, 100.0, 50.0),
                float_slider("edge_strength", "Edge Strength", 0.0, 100.0, 60.0),
                float_slider("grain_amount", "Wood Grain", 0.0, 100.0, 30.0),
                color("ink_color", "Ink Color", C::new(15, 10, 5, 255)),
                color("paper_color", "Paper Color", C::new(235, 225, 200, 255)),
            ]
        }),
        Def::new(
            "wooden_frame",
            "Wooden frame",
            F,
            l::FRAME,
            "Adds a procedural wooden frame around the image.",
            wooden_frame,
        )
        .params(|| {
            vec![
                int_numeric("frame_width", "Frame width", 2, 300, 48),
                float_slider("grain_strength", "Grain strength", 0.0, 100.0, 60.0),
                float_slider("bevel_strength", "Bevel strength", 0.0, 100.0, 65.0),
                color("wood_color", "Wood color", C::rgb(139, 94, 60)),
            ]
        }),
        Def::new(
            "x_ray_scan",
            "X-ray scan",
            F,
            l::SCAN_EYE,
            "Simulates an X-ray scan with edge detection and glow effects.",
            xray,
        )
        .params(|| {
            vec![
                float_slider("contrast", "Contrast", 0.0, 100.0, 70.0),
                float_slider("glow", "Glow", 0.0, 100.0, 60.0),
                float_slider("edge_boost", "Edge boost", 0.0, 100.0, 68.0),
                float_slider("scanlines", "Scanlines", 0.0, 100.0, 40.0),
                float_slider("noise", "Noise", 0.0, 100.0, 14.0),
            ]
        }),
        Def::new(
            "zigzag_border",
            "Zigzag border",
            F,
            l::WAVES,
            "Adds a border with decorative jagged zigzag inner teeth that bite into the image edges.",
            zigzag,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 8, 200, 36),
                float_slider("teeth_height", "Teeth height", 5.0, 80.0, 40.0),
                float_slider("teeth_width", "Teeth width", 4.0, 80.0, 24.0),
                color("frame_color", "Frame color", C::rgb(40, 40, 50)),
                color("accent_color", "Accent color", C::rgb(220, 180, 60)),
                checkbox("accent_stripe", "Accent stripe", true),
            ]
        }),
        Def::new(
            "zoom_blur",
            "Zoom blur",
            F,
            l::ZOOM_IN,
            "Applies a radial zoom blur emanating from a center point.",
            zoom_blur,
        )
        .params(|| {
            vec![
                float_slider("strength", "Strength", 0.0, 100.0, 35.0),
                int_slider("samples", "Samples", 4, 64, 24),
                float_slider("center_x", "Center X", 0.0, 100.0, 50.0),
                float_slider("center_y", "Center Y", 0.0, 100.0, 50.0),
            ]
        }),
    ]
}

const W11_BAR: i32 = 32;
const W11_BUTTON_W: f32 = 46.0;
const W11_BUTTON_H: f32 = 32.0;

fn windows11(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let radius = p.i("corner_radius").clamp(0, 16) as f32;
    let shadow = p.i("shadow_size").clamp(0, 60);
    let padding = p.i("padding").clamp(0, 100);
    let dark = p.b("dark_mode");
    let ww = src.w;
    let wh = W11_BAR + src.h;
    let tw = ww + padding * 2 + shadow * 2;
    let th = wh + padding * 2 + shadow * 2;
    let mut canvas = Canvas::new(tw, th);
    let wx = (padding + shadow) as f32;
    let wy = (padding + shadow) as f32;
    let window = rect(wx, wy, wx + ww as f32, wy + wh as f32);
    if shadow > 0 {
        let paint = Paint::fill_aa(C::new(0, 0, 0, 40)).with_mask_blur(shadow as f32 / 2.5);
        canvas.draw_round_rect(window, radius, radius, &paint);
    }
    canvas.save();
    canvas.clip_round_rect(window, radius, radius, true);
    let bar = if dark {
        C::rgb(32, 32, 32)
    } else {
        C::rgb(243, 243, 243)
    };
    canvas.draw_rect_xywh(wx, wy, ww as f32, W11_BAR as f32, &Paint::fill_aa(bar));
    let border = if dark {
        C::rgb(50, 50, 50)
    } else {
        C::rgb(229, 229, 229)
    };
    let by = wy + W11_BAR as f32;
    canvas.draw_line(wx, by, wx + ww as f32, by, &Paint::stroke_aa(border, 1.0));
    let icon = if dark {
        C::rgb(200, 200, 200)
    } else {
        C::rgb(90, 90, 90)
    };
    let right = wx + ww as f32;
    for (k, bg, ic) in [
        (1.0, C::rgb(196, 43, 28), C::WHITE),
        (2.0, bar, icon),
        (3.0, bar, icon),
    ] {
        let x = right - W11_BUTTON_W * k;
        canvas.draw_rect_xywh(x, wy, W11_BUTTON_W, W11_BUTTON_H, &Paint::fill_aa(bg));
        let (cx, cy) = (x + W11_BUTTON_W / 2.0, wy + W11_BUTTON_H / 2.0);
        let half = 5.0;
        if k == 1.0 {
            let pen = Paint::stroke_aa(C::WHITE, 1.0).with_cap(Cap::Round);
            canvas.draw_line(cx - half, cy - half, cx + half, cy + half, &pen);
            canvas.draw_line(cx + half, cy - half, cx - half, cy + half, &pen);
        } else if k == 2.0 {
            canvas.draw_rect_xywh(cx - half, cy - half, 10.0, 10.0, &Paint::stroke_aa(ic, 1.0));
        } else {
            let pen = Paint::stroke_aa(ic, 1.0).with_cap(Cap::Round);
            canvas.draw_line(cx - half, cy, cx + half, cy, &pen);
        }
    }
    let title = p.s("title");
    if !title.trim().is_empty() {
        let color = if dark {
            C::rgb(220, 220, 220)
        } else {
            C::rgb(50, 50, 50)
        };
        let font = Font::new("Segoe UI", 12.0);
        let ty = wy + W11_BAR as f32 / 2.0 + 12.0 / 3.0;
        canvas.draw_text(&title, wx + 12.0, ty, &font, &Paint::fill_aa(color));
    }
    canvas.draw_bitmap(src, wx, wy + W11_BAR as f32, &Paint::default());
    canvas.restore();
    let wb = if dark {
        C::new(60, 60, 60, 160)
    } else {
        C::new(200, 200, 200, 160)
    };
    canvas.draw_round_rect(window, radius, radius, &Paint::stroke_aa(wb, 1.0));
    if !dark {
        let mica = Paint::fill_aa(C::BLACK).with_shader(Shader::Linear {
            p0: (wx, wy),
            p1: (wx, wy + 2.0),
            colors: vec![C::new(255, 255, 255, 80), C::new(255, 255, 255, 0)],
            pos: None,
            tile: Tile::Clamp,
        });
        canvas.save();
        canvas.clip_round_rect(window, radius, radius, true);
        canvas.draw_rect_xywh(wx, wy, ww as f32, 2.0, &mica);
        canvas.restore();
    }
    Ok(canvas.into_bitmap())
}

fn woodcut(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    let threshold = p.f("threshold").clamp(0.0, 100.0) / 100.0;
    let ew = p.f("edge_strength").clamp(0.0, 100.0) / 100.0;
    let gw = p.f("grain_amount").clamp(0.0, 100.0) / 100.0;
    let n = src.px.len();
    let lum: Vec<f32> = src
        .px
        .iter()
        .map(|c| (0.299 * c.r as f32 + 0.587 * c.g as f32 + 0.114 * c.b as f32) / 255.0)
        .collect();
    let mut edges = vec![0f32; n];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let l = |xx: i32, yy: i32| lum[(yy * w + xx) as usize];
            let gx = -l(x - 1, y - 1) + l(x + 1, y - 1) - 2.0 * l(x - 1, y) + 2.0 * l(x + 1, y)
                - l(x - 1, y + 1)
                + l(x + 1, y + 1);
            let gy = -l(x - 1, y - 1) - 2.0 * l(x, y - 1) - l(x + 1, y - 1)
                + l(x - 1, y + 1)
                + 2.0 * l(x, y + 1)
                + l(x + 1, y + 1);
            edges[(y * w + x) as usize] = (gx * gx + gy * gy).sqrt();
        }
    }
    let mut grain = vec![0f32; n];
    if gw > 0.001 {
        let mut rng: i32 = 7919;
        let mut phase = Vec::with_capacity(h as usize);
        for _ in 0..h {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            phase.push((rng & 0x7FFF_FFFF) as f32 / i32::MAX as f32 * PI * 2.0);
        }
        for y in 0..h {
            let ph = phase[y as usize];
            for x in 0..w {
                let wave = (x as f32 * 0.08 + ph).sin() * 0.5 + 0.5;
                let fine = (x as f32 * 0.3 + y as f32 * 0.15 + ph * 2.0).sin() * 0.5 + 0.5;
                grain[(y * w + x) as usize] = wave * 0.6 + fine * 0.4;
            }
        }
    }
    let (ink, paper) = (p.c("ink_color"), p.c("paper_color"));
    let f = |c: C| (c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0);
    let (ir, ig, ib) = f(ink);
    let (pr, pg, pb) = f(paper);
    let px = (0..n)
        .map(|i| {
            let local = threshold + (grain[i] - 0.5) * gw * 0.4;
            let score = lum[i] - edges[i] * ew;
            let ink_amt = if score < local - 0.05 {
                1.0
            } else if score > local + 0.05 {
                0.0
            } else {
                1.0 - (score - (local - 0.05)) / 0.1
            };
            let ch = |a: f32, b: f32| {
                round((a * ink_amt + b * (1.0 - ink_amt)).clamp(0.0, 1.0) * 255.0) as u8
            };
            C::new(ch(ir, pr), ch(ig, pg), ch(ib, pb), src.px[i].a)
        })
        .collect();
    Ok(Bitmap::from_pixels(w, h, px))
}

#[derive(Clone, Copy, PartialEq)]
enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

fn wood_grain(along: f32, across: f32, strength: f32, seed: i32) -> f32 {
    let coarse = hash01((along * 0.20) as i32, (across * 0.08) as i32, seed) * 2.0 - 1.0;
    let medium = hash01((along * 0.55) as i32, (across * 0.16) as i32, seed ^ 0x57) * 2.0 - 1.0;
    let fine = hash01((along * 1.30) as i32, (across * 0.42) as i32, seed ^ 0xA1) * 2.0 - 1.0;
    let w1 = (along * 0.075 + coarse * 5.6 + across * 0.028).sin();
    let w2 = (along * 0.19 - medium * 3.1 + across * 0.012).sin();
    let sign = if w1 > 0.0 {
        1.0
    } else if w1 < 0.0 {
        -1.0
    } else {
        0.0
    };
    let ridge = sign * w1.abs().powf(1.75);
    let pore = fine * 0.12;
    let tint = (across * 0.55 + coarse * 2.2).sin() * 0.04;
    (ridge * 0.20 + w2 * 0.08 + pore + tint) * strength
}

fn wood_bevel(side: Side, ratio: f32, outer: f32, inner: f32, fw: i32, strength: f32) -> f32 {
    let span = 2f32.max(fw as f32 * (0.16 + 0.10 * strength));
    let om = 1.0 - smooth_step(0.0, span, outer);
    let im = 1.0 - smooth_step(0.0, span, inner);
    let slope = 0.5 - ratio;
    let shade = match side {
        Side::Top => om * 0.24 - im * 0.28 + slope * 0.12,
        Side::Left => om * 0.18 - im * 0.22 + slope * 0.10,
        Side::Right => -(om * 0.20) + im * 0.08 - slope * 0.08,
        Side::Bottom => -(om * 0.26) + im * 0.10 - slope * 0.12,
    };
    shade * strength
}

fn wood_seam(x: i32, y: i32, w: i32, h: i32, fw: i32) -> f32 {
    let sw = 1.5f32.max(fw as f32 * 0.08);
    let s = |d: f32| (1.0 - smooth_step(0.0, sw, d.abs())) * 0.10;
    let f1 = fw as f32 - 1.0;
    if x < fw && y < fw {
        return s((x + y) as f32 - f1);
    }
    if x >= w - fw && y < fw {
        return s(((w - 1 - x) + y) as f32 - f1);
    }
    if x < fw && y >= h - fw {
        return s((x + (h - 1 - y)) as f32 - f1);
    }
    if x >= w - fw && y >= h - fw {
        return s(((w - 1 - x) + (h - 1 - y)) as f32 - f1);
    }
    0.0
}

fn wooden_frame(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let fw = p.i("frame_width").clamp(2, 300);
    let gs = p.f("grain_strength").clamp(0.0, 100.0) / 100.0;
    let bs = p.f("bevel_strength").clamp(0.0, 100.0) / 100.0;
    let wood = p.c("wood_color");
    let seed = 1649;
    let (nw, nh) = (src.w + fw * 2, src.h + fw * 2);
    let mut px = vec![C::ZERO; (nw * nh) as usize];
    for y in 0..nh {
        let top = y < fw;
        let bottom = y >= fw + src.h;
        for x in 0..nw {
            let left = x < fw;
            let right = x >= fw + src.w;
            if !top && !bottom && !left && !right {
                continue;
            }
            let (side, along, across, outer, inner, bseed) = if top {
                (Side::Top, x, y, y, (fw - 1) - y, seed ^ 0x1C3)
            } else if bottom {
                (
                    Side::Bottom,
                    x,
                    y - (fw + src.h),
                    (nh - 1) - y,
                    y - (fw + src.h),
                    seed ^ 0x29B,
                )
            } else if left {
                (Side::Left, y, x, x, (fw - 1) - x, seed ^ 0x35F)
            } else {
                (
                    Side::Right,
                    y,
                    x - (fw + src.w),
                    (nw - 1) - x,
                    x - (fw + src.w),
                    seed ^ 0x41D,
                )
            };
            let span = 1f32.max(fw as f32 - 1.0);
            let ratio = across as f32 / span;
            let variation = (hash01(bseed, fw, seed) * 2.0 - 1.0) * 0.06;
            let grain = wood_grain(along as f32, across as f32, gs, bseed);
            let mut shade = variation + wood_bevel(side, ratio, outer as f32, inner as f32, fw, bs);
            shade -= wood_seam(x, y, nw, nh, fw);
            px[(y * nw + x) as usize] = C::new(
                clamp_to_byte(wood.r as f32 * (1.0 + grain + shade + 0.03)),
                clamp_to_byte(wood.g as f32 * (1.0 + grain * 0.82 + shade)),
                clamp_to_byte(wood.b as f32 * (1.0 + grain * 0.48 + shade - 0.03)),
                255,
            );
        }
    }
    let mut canvas = Canvas::from_bitmap(&Bitmap::from_pixels(nw, nh, px));
    canvas.draw_bitmap(src, fw as f32, fw as f32, &Paint::default());
    Ok(canvas.into_bitmap())
}

fn xray(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let contrast = p.f("contrast").clamp(0.0, 100.0) / 100.0;
    let glow = p.f("glow").clamp(0.0, 100.0) / 100.0;
    let eb = p.f("edge_boost").clamp(0.0, 100.0) / 100.0;
    let scanlines = p.f("scanlines").clamp(0.0, 100.0) / 100.0;
    let noise = p.f("noise").clamp(0.0, 100.0) / 100.0;
    let (w, h) = (src.w, src.h);
    let l = |c: C| (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let lum = l(s);
            let edge = clamp01(
                ((l(src.get((w - 1).min(x + 1), y)) - l(src.get(0.max(x - 1), y))).abs()
                    + (l(src.get(x, (h - 1).min(y + 1))) - l(src.get(x, 0.max(y - 1)))).abs())
                    * (0.85 + eb * 1.45),
            );
            let density = clamp01(1.0 - lum).powf(1.85 - contrast * 1.25);
            let energy = clamp01(density * 0.48 + edge * 0.92);
            let gm = clamp01(edge * (0.25 + glow * 0.95) + energy * glow * 0.18);
            let scan = 1.0 - scanlines * 0.16 * (0.5 + 0.5 * (y as f32 * PI).sin());
            let grain = (hash01(x, y, 8849) * 2.0 - 1.0) * noise * 0.08;
            let r = clamp01((energy * 0.14 + gm * 0.08 + grain * 0.30) * scan);
            let g = clamp01((energy * 0.72 + gm * 0.16 + grain * 0.40) * scan);
            let b = clamp01((energy * 1.00 + gm * 0.26 + grain * 0.48) * scan);
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

fn zigzag_path(cw: i32, ch: i32, border: i32, tw: f32, th: f32) -> Option<Path> {
    let (l, t) = (border as f32, border as f32);
    let (r, b) = ((cw - border) as f32, (ch - border) as f32);
    let mut pb = PathBuilder::new();
    pb.move_to(l, t);
    let mut x = l;
    while x < r {
        let nx = (x + tw).min(r);
        let mx = x + tw * 0.5;
        if mx < r {
            pb.line_to(mx, t + th);
        }
        pb.line_to(nx, t);
        x = nx;
    }
    let mut y = t;
    while y < b {
        let ny = (y + tw).min(b);
        let my = y + tw * 0.5;
        if my < b {
            pb.line_to(r - th, my);
        }
        pb.line_to(r, ny);
        y = ny;
    }
    let mut x = r;
    while x > l {
        let nx = (x - tw).max(l);
        let mx = x - tw * 0.5;
        if mx > l {
            pb.line_to(mx, b - th);
        }
        pb.line_to(nx, b);
        x = nx;
    }
    let mut y = b;
    while y > t {
        let ny = (y - tw).max(t);
        let my = y - tw * 0.5;
        if my > t {
            pb.line_to(l + th, my);
        }
        pb.line_to(l, ny);
        y = ny;
    }
    pb.close();
    pb.finish()
}

fn zigzag(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(8, 200);
    let th = p.f("teeth_height").clamp(5.0, 80.0);
    let tw = p.f("teeth_width").clamp(4.0, 80.0);
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let mut canvas = Canvas::new(nw, nh);
    canvas.clear(p.c("frame_color"));
    if let Some(path) = zigzag_path(nw, nh, border, tw, th) {
        canvas.save();
        canvas.clip_path(&path, true);
        canvas.draw_bitmap(src, border as f32, border as f32, &Paint::default());
        canvas.restore();
        if p.b("accent_stripe") {
            let sw = 2f32.max(border as f32 * 0.1);
            canvas.draw_path(&path, &Paint::stroke_aa(p.c("accent_color"), sw));
        }
    }
    Ok(canvas.into_bitmap())
}

fn zoom_blur(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let strength = p.f("strength").clamp(0.0, 100.0) / 100.0;
    let n = p.i("samples").clamp(4, 64);
    if strength <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let cx = (p.f("center_x").clamp(0.0, 100.0) / 100.0) * (w - 1) as f32;
    let cy = (p.f("center_y").clamp(0.0, 100.0) / 100.0) * (h - 1) as f32;
    let inv = if n <= 1 { 1.0 } else { 1.0 / (n - 1) as f32 };
    let table: Vec<(f32, f32)> = (0..n)
        .map(|i| {
            let t = i as f32 * inv;
            (1.0 - t * strength, 1.0 - t * 0.6)
        })
        .collect();
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let (mut sr, mut sg, mut sb, mut sa, mut sw) = (0f32, 0f32, 0f32, 0f32, 0f32);
            for &(f, wt) in &table {
                let c = bilinear_sample(&src.px, w, h, cx + dx * f, cy + dy * f);
                sr += c.r as f32 * wt;
                sg += c.g as f32 * wt;
                sb += c.b as f32 * wt;
                sa += c.a as f32 * wt;
                sw += wt;
            }
            let k = 1.0 / 0.0001f32.max(sw);
            px.push(C::new(
                clamp_to_byte(sr * k),
                clamp_to_byte(sg * k),
                clamp_to_byte(sb * k),
                clamp_to_byte(sa * k),
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}
