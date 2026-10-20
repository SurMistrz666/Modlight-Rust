//! Filters, part B: Bloom .. Celtic knot border (and Border, Candy cane border, which
//! upstream files here but lists under Drawings).

use std::f32::consts::PI;

use modlight_core::icons as l;
use modlight_editor::effects::Category;
use tiny_skia::PathBuilder;

use crate::def::*;
use crate::dotnet::shared_next;
use crate::helpers::{blurred_clamp, clamp_to_byte, hash01, lerp, smooth_step};
use crate::sk::{Bitmap, C, Canvas, Cap, Join, Paint, Style, rect, round};

const F: Category = Category::Filters;

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "bloom",
            "Bloom",
            F,
            l::SPARKLE,
            "Adds a soft glow bloom effect to bright areas.",
            bloom,
        )
        .params(|| {
            vec![
                float_slider("threshold", "Threshold", 0.0, 100.0, 65.0),
                float_slider("soft_knee", "Soft knee", 0.0, 100.0, 35.0),
                float_slider("radius", "Radius", 1.0, 100.0, 24.0),
                float_slider("intensity", "Intensity", 0.0, 200.0, 85.0),
            ]
        }),
        Def::new(
            "blueprint_drawing",
            "Blueprint drawing",
            F,
            l::DRAFTING_COMPASS,
            "Transforms the image into a blueprint-style drawing.",
            blueprint,
        )
        .params(|| {
            vec![
                float_slider("line_strength", "Line strength", 0.0, 100.0, 75.0),
                float_slider("detail", "Detail", 0.0, 100.0, 45.0),
                float_slider("grid_intensity", "Grid intensity", 0.0, 100.0, 30.0),
                float_slider("texture", "Texture", 0.0, 100.0, 25.0),
                float_slider("glow", "Glow", 0.0, 100.0, 35.0),
            ]
        }),
        Def::new("blur", "Blur", F, l::FOCUS, "Applies a blur effect.", blur)
            .params(|| vec![int_slider("radius", "Radius", 1, 200, 10)]),
        Def::new(
            "border",
            "Border",
            Category::Drawings,
            l::FRAME,
            "Adds a border to the image.",
            border,
        )
        .params(|| {
            vec![
                choice("type", "Type", &[("Outside", "Outside"), ("Inside", "Inside")], "Outside"),
                int_slider("size", "Size", 1, 100, 5),
                choice(
                    "dash_style",
                    "Dash style",
                    &[
                        ("Solid", "Solid"),
                        ("Dash", "Dash"),
                        ("Dot", "Dot"),
                        ("Dash Dot", "DashDot"),
                    ],
                    "Solid",
                ),
                color("color", "Color", C::BLACK),
            ]
        }),
        Def::new(
            "brick_wall_border",
            "Brick wall border",
            F,
            l::BRICK_WALL,
            "Adds a procedural brick wall texture border with mortar lines, shading, and color variation.",
            brick_wall,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 10, 300, 56),
                color("brick_color", "Brick color", C::rgb(178, 90, 62)),
                color("mortar_color", "Mortar color", C::rgb(195, 188, 175)),
                float_slider("mortar_thickness", "Mortar thickness", 1.0, 20.0, 4.0),
                float_slider("color_variation", "Color variation", 0.0, 100.0, 40.0),
                float_slider("bevel_strength", "Bevel strength", 0.0, 100.0, 50.0),
            ]
        }),
        Def::new(
            "crt",
            "CRT",
            F,
            l::MONITOR,
            "Simulates a CRT monitor with scanlines, RGB shift, noise, and vignette.",
            crt,
        )
        .params(|| {
            vec![
                float_slider("scanlineStrength", "Scanline strength", 0.0, 100.0, 35.0),
                int_slider("rgbShift", "RGB shift", 0, 8, 1),
                float_slider("noiseAmount", "Noise amount", 0.0, 100.0, 8.0),
                float_slider("vignetteStrength", "Vignette strength", 0.0, 100.0, 18.0),
            ]
        }),
        Def::new(
            "candy_cane_border",
            "Candy Cane Border",
            Category::Drawings,
            l::CANDY_CANE,
            "Adds a candy cane striped border around the image.",
            candy_cane,
        )
        .params(|| {
            vec![
                int_slider("border_width", "Border Width", 1, 200, 10),
                float_slider("stripe_width", "Stripe Width", 2.0, 60.0, 12.0),
                color("color_a", "Color A", C::rgb(220, 20, 20)),
                color("color_b", "Color B", C::rgb(255, 255, 255)),
                float_slider("gloss", "Gloss", 0.0, 100.0, 50.0),
                checkbox("rounded_corners", "Rounded Corners", true),
            ]
        }),
        Def::new(
            "cartoon_sticker_cutout",
            "Cartoon sticker cutout",
            F,
            l::STICKER,
            "Applies a cartoon sticker cutout effect with quantized colors and ink edges.",
            cartoon,
        )
        .params(|| {
            vec![
                int_slider("colorLevels", "Color levels", 2, 20, 8),
                int_slider("edgeThreshold", "Edge threshold", 5, 120, 32),
                float_slider("inkStrength", "Ink strength", 0.0, 100.0, 85.0),
                int_slider("stickerBorder", "Sticker border", 0, 10, 3),
                float_slider("borderStrength", "Border strength", 0.0, 100.0, 90.0),
            ]
        }),
        Def::new(
            "celtic_knot_border",
            "Celtic knot border",
            F,
            l::CLOVER,
            "Adds an interweaving Celtic knot pattern border inspired by Insular art manuscripts.",
            celtic,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 16, 200, 52),
                color("knot_color", "Knot color", C::rgb(180, 140, 60)),
                color("bg_color", "Background color", C::rgb(35, 55, 25)),
                float_slider("strand_width", "Strand width", 10.0, 80.0, 35.0),
                checkbox("outline_strands", "Outline strands", true),
                checkbox("inner_line", "Inner line", true),
            ]
        }),
    ]
}

fn lum01(c: C) -> f32 {
    (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0
}

fn bloom(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let threshold = p.f("threshold").clamp(0.0, 100.0) / 100.0;
    let knee = p.f("soft_knee").clamp(0.0, 100.0) / 100.0;
    let radius = p.f("radius").clamp(1.0, 100.0);
    let intensity = p.f("intensity").clamp(0.0, 200.0) / 100.0;
    if intensity <= 0.0 {
        return Ok(src.clone());
    }
    let half = knee * 0.5;
    let low = (threshold - half).clamp(0.0, 1.0);
    let high = (threshold + half).clamp(0.0, 1.0);
    let bright: Vec<C> = src
        .px
        .iter()
        .map(|c| {
            let lum = lum01(*c);
            let mask = if knee > 0.0 {
                smooth_step(low, high, lum)
            } else if lum >= threshold {
                1.0
            } else {
                0.0
            };
            if mask <= 0.0 {
                return C::ZERO;
            }
            let gain = mask * (0.35 + 0.65 * lum);
            C::new(
                clamp_to_byte(c.r as f32 * gain),
                clamp_to_byte(c.g as f32 * gain),
                clamp_to_byte(c.b as f32 * gain),
                clamp_to_byte(c.a as f32 * gain),
            )
        })
        .collect();
    let bright = Bitmap::from_pixels(src.w, src.h, bright);
    let blurred = blurred_clamp(&bright, radius);
    let ch = |s: u8, b: u8| {
        let s = s as f32 / 255.0;
        let b = (b as f32 / 255.0) * intensity;
        let mut screened = 1.0 - (1.0 - s) * (1.0 - b.clamp(0.0, 1.0));
        if b > 1.0 {
            screened += (b - 1.0) * 0.40;
        }
        clamp_to_byte(screened * 255.0)
    };
    let px = src
        .px
        .iter()
        .zip(&blurred.px)
        .map(|(s, b)| C::new(ch(s.r, b.r), ch(s.g, b.g), ch(s.b, b.b), s.a))
        .collect();
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}

fn blueprint(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let line_strength = p.f("line_strength").clamp(0.0, 100.0) / 100.0;
    let detail = p.f("detail").clamp(0.0, 100.0) / 100.0;
    let grid = p.f("grid_intensity").clamp(0.0, 100.0) / 100.0;
    let texture = p.f("texture").clamp(0.0, 100.0) / 100.0;
    let glow = p.f("glow").clamp(0.0, 100.0) / 100.0;
    let seed = shared_next(1, i32::MAX);
    let (w, h) = (src.w, src.h);
    let (right, bottom) = (w - 1, h - 1);
    let cx = right as f32 * 0.5;
    let cy = bottom as f32 * 0.5;
    let inv_spacing = 1.0 / (10.0 + (1.0 - detail) * 20.0);
    let lum: Vec<f32> = src.px.iter().map(|c| lum01(*c)).collect();
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let row = y * w;
        let yu = if y > 0 { y - 1 } else { 0 };
        let yd = if y < bottom { y + 1 } else { bottom };
        for x in 0..w {
            let xl = if x > 0 { x - 1 } else { 0 };
            let xr = if x < right { x + 1 } else { right };
            let i = (row + x) as usize;
            let l0 = lum[i];
            let gx = (lum[(row + xr) as usize] - lum[(row + xl) as usize]).abs();
            let gy = (lum[(yd * w + x) as usize] - lum[(yu * w + x) as usize]).abs();
            let edge = ((gx + gy) * (1.7 + line_strength * 2.4)).clamp(0.0, 1.0);
            let tonal = (1.0 - l0).powf(0.65 + (1.0 - detail) * 1.5) * (0.2 + detail * 0.35);
            let line = (edge * (0.72 + line_strength * 0.75) + tonal).clamp(0.0, 1.0);
            let nx = x as f32 / 1f32.max(right as f32);
            let ny = y as f32 / 1f32.max(bottom as f32);
            let dx = (x as f32 - cx) / 1f32.max(right as f32);
            let dy = (y as f32 - cy) / 1f32.max(bottom as f32);
            let dist = (dx * dx + dy * dy).sqrt();
            let vignette = 1.0 - smooth_step(0.35, 0.95, dist);
            let shade = 0.42 + 0.38 * vignette;
            let gxl = (x as f32 * inv_spacing - round(x as f32 * inv_spacing)).abs();
            let gyl = (y as f32 * inv_spacing - round(y as f32 * inv_spacing)).abs();
            let grid_line = (1.0 - gxl * 18.0).max(1.0 - gyl * 18.0).clamp(0.0, 1.0) * grid * 0.45;
            let na =
                (hash01((nx * 1024.0) as i32, (ny * 1024.0) as i32, seed) * 2.0 - 1.0) * texture;
            let nb = (hash01((nx * 320.0) as i32, (ny * 320.0) as i32, seed ^ 431) * 2.0 - 1.0)
                * texture
                * 0.6;
            let paper = (na + nb) * 12.0;
            let br = 10.0 + 12.0 * shade + 16.0 * grid_line + paper;
            let bg = 52.0 + 46.0 * shade + 28.0 * grid_line + paper;
            let bb = 122.0 + 88.0 * shade + 46.0 * grid_line + paper;
            let gm = line * (0.55 + glow * 0.45);
            let mut r = lerp(br, 230.0, gm);
            let mut g = lerp(bg, 242.0, gm);
            let mut b = lerp(bb, 255.0, gm);
            let cyan = line * glow * 0.28;
            r = lerp(r, 136.0, cyan);
            g = lerp(g, 224.0, cyan);
            b = lerp(b, 255.0, cyan);
            px.push(C::new(
                clamp_to_byte(r),
                clamp_to_byte(g),
                clamp_to_byte(b),
                src.px[i].a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

/// The clamp-padded blur: edge pixels extended by `padding`, blurred with `sigma`, cut
/// back.
pub fn padded_blur(src: &Bitmap, padding: i32, sigma: f32) -> Bitmap {
    let (ew, eh) = (src.w + padding * 2, src.h + padding * 2);
    let mut expanded = Bitmap::new(ew, eh);
    for y in 0..eh {
        let sy = (y - padding).clamp(0, src.h - 1);
        for x in 0..ew {
            let sx = (x - padding).clamp(0, src.w - 1);
            expanded.px[(y * ew + x) as usize] = src.get(sx, sy);
        }
    }
    let blurred = crate::helpers::blur_bitmap(&expanded, sigma, sigma);
    let mut out = Bitmap::new(src.w, src.h);
    for y in 0..src.h {
        for x in 0..src.w {
            out.px[(y * src.w + x) as usize] = blurred.get(x + padding, y + padding);
        }
    }
    out
}

fn blur(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let r = p.i("radius");
    if r <= 0 {
        return Ok(src.clone());
    }
    Ok(padded_blur(src, r * 2, r as f32))
}

fn border(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let size = p.i("size");
    if size <= 0 {
        return Ok(src.clone());
    }
    let outside = p.e("type") == "Outside";
    let (nw, nh) = if outside {
        (src.w + size * 2, src.h + size * 2)
    } else {
        (src.w, src.h)
    };
    let mut canvas = Canvas::new(nw, nh);
    let off = if outside { size } else { 0 };
    canvas.draw_bitmap(src, off as f32, off as f32, &Paint::default());
    let s = size as f32;
    let mut paint = Paint::stroke_aa(p.c("color"), s);
    paint.dash = match p.e("dash_style").as_str() {
        "Dash" => Some((vec![s * 3.0, s], 0.0)),
        "Dot" => Some((vec![s, s], 0.0)),
        "DashDot" => Some((vec![s * 3.0, s, s, s], 0.0)),
        _ => None,
    };
    let half = s / 2.0;
    let r = if outside {
        rect(half, half, nw as f32 - half, nh as f32 - half)
    } else {
        rect(half, half, src.w as f32 - half, src.h as f32 - half)
    };
    canvas.draw_rect(r, &paint);
    Ok(canvas.into_bitmap())
}

fn brick_wall(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(10, 300);
    let mortar = p.f("mortar_thickness").clamp(1.0, 20.0);
    let var = p.f("color_variation").clamp(0.0, 100.0) / 100.0;
    let bevel = p.f("bevel_strength").clamp(0.0, 100.0) / 100.0;
    let brick = p.c("brick_color");
    let mortar_c = p.c("mortar_color");
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let brick_h = (border as f32 * 0.45).max(mortar * 2.0 + 4.0);
    let brick_w = brick_h * 2.4;
    let seed = 3271;
    let mut px = vec![C::ZERO; (nw * nh) as usize];
    for py in 0..nh {
        let top = py < border;
        let bottom = py >= border + src.h;
        for pxx in 0..nw {
            let left = pxx < border;
            let rightb = pxx >= border + src.w;
            if !top && !bottom && !left && !rightb {
                continue;
            }
            let (lx, ly) = if top {
                (pxx as f32, py as f32)
            } else if bottom {
                ((nw - 1 - pxx) as f32, (nh - 1 - py) as f32)
            } else if left {
                (py as f32, pxx as f32)
            } else {
                ((nh - 1 - py) as f32, (nw - 1 - pxx) as f32)
            };
            let row = (ly / brick_h) as i32;
            let row_frac = (ly % brick_h) / brick_h;
            let offset = if row % 2 == 0 { 0.0 } else { brick_w * 0.5 };
            let col = ((lx + offset) / brick_w) as i32;
            let col_frac = ((lx + offset) % brick_w) / brick_w;
            let mfh = mortar / brick_h;
            let mfw = mortar / brick_w;
            let i = (py * nw + pxx) as usize;
            if row_frac < mfh || col_frac < mfw {
                px[i] = mortar_c;
                continue;
            }
            let vh = hash01(col, row ^ (row.wrapping_mul(31).wrapping_add(seed)), seed);
            let vh2 = hash01(col ^ col.wrapping_mul(13), row, seed ^ 0x5A);
            let red_var = (vh - 0.5) * var * 0.5;
            let green_var = (vh2 - 0.5) * var * 0.25;
            let bf = (row_frac - mfh)
                .min(1.0 - row_frac)
                .min((col_frac - mfw).min(1.0 - col_frac));
            let shade = if bevel > 0.0 {
                (1.0 - smooth_step(0.0, 0.18, bf)) * bevel * 0.35
            } else {
                0.0
            };
            let noise = hash01((lx * 2.0) as i32, (ly * 2.0) as i32, seed ^ 0x99) * 0.08 - 0.04;
            let r = brick.r as f32 / 255.0 + red_var + noise - shade;
            let g = brick.g as f32 / 255.0 + green_var * 0.5 + noise * 0.4 - shade;
            let b = brick.b as f32 / 255.0 - red_var * 0.2 - shade;
            px[i] = C::new(
                clamp_to_byte(r * 255.0),
                clamp_to_byte(g * 255.0),
                clamp_to_byte(b * 255.0),
                255,
            );
        }
    }
    let base = Bitmap::from_pixels(nw, nh, px);
    let mut canvas = Canvas::from_bitmap(&base);
    canvas.draw_bitmap(src, border as f32, border as f32, &Paint::default());
    canvas.draw_rect_xywh(
        border as f32 - mortar * 0.5,
        border as f32 - mortar * 0.5,
        src.w as f32 + mortar,
        src.h as f32 + mortar,
        &Paint::stroke(mortar_c, mortar),
    );
    Ok(canvas.into_bitmap())
}

fn hash_u32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846CA68B);
    x ^= x >> 16;
    x
}

fn crt(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let scan = p.f("scanlineStrength").clamp(0.0, 100.0) / 100.0;
    let shift = p.i("rgbShift").clamp(0, 8);
    let noise = p.f("noiseAmount").clamp(0.0, 100.0);
    let vig = p.f("vignetteStrength").clamp(0.0, 100.0) / 100.0;
    if scan <= 0.0 && shift == 0 && noise <= 0.0 && vig <= 0.0 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let right = w - 1;
    let amp = round(noise * 0.6) as i32;
    let cx = (w - 1) as f32 * 0.5;
    let cy = (h - 1) as f32 * 0.5;
    let icx = if cx > 0.0 { 1.0 / cx } else { 1.0 };
    let icy = if cy > 0.0 { 1.0 / cy } else { 1.0 };
    let inv_sqrt2 = 0.70710678f32;
    let seed = 1337u32.wrapping_mul(747796405);
    let cb = |v: i32| v.clamp(0, 255) as u8;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let row = y * w;
        let line = if scan > 0.0 {
            1.0 - scan * if y & 1 == 0 { 0.62 } else { 0.18 }
        } else {
            1.0
        };
        let dy = (y as f32 - cy) * icy;
        for x in 0..w {
            let idx = row + x;
            let sr = src.px[(row + (x + shift).clamp(0, right)) as usize];
            let sg = src.px[idx as usize];
            let sb = src.px[(row + (x - shift).clamp(0, right)) as usize];
            let (mr, mg, mb) = match x % 3 {
                0 => (1.06, 0.97, 0.93),
                1 => (0.93, 1.06, 0.97),
                _ => (0.97, 0.93, 1.06),
            };
            let mut vf = 1.0;
            if vig > 0.0 {
                let dx = (x as f32 - cx) * icx;
                let d = (dx * dx + dy * dy).sqrt() * inv_sqrt2;
                let t = ((d - 0.55) / 0.45).clamp(0.0, 1.0);
                vf = 1.0 - vig * (t * t * (3.0 - 2.0 * t));
            }
            let br = line * vf;
            let (mut nr, mut ng, mut nb) = (0, 0, 0);
            if amp > 0 {
                let mut hh = hash_u32(idx as u32 ^ seed);
                nr = ((hh & 0xFF) as i32 - 128) * amp / 128;
                hh = hash_u32(hh ^ 0x9E3779B9);
                ng = ((hh & 0xFF) as i32 - 128) * amp / 128;
                hh = hash_u32(hh ^ 0x85EBCA6B);
                nb = ((hh & 0xFF) as i32 - 128) * amp / 128;
            }
            let r = round(sr.r as f32 * mr * br) as i32 + nr;
            let g = round(sg.g as f32 * mg * br) as i32 + ng;
            let b = round(sb.b as f32 * mb * br) as i32 + nb;
            px.push(C::new(cb(r), cb(g), cb(b), sg.a));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn candy_cane(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let bw = p.i("border_width").clamp(1, 500);
    let sw = p.f("stripe_width").clamp(1.0, 200.0);
    let gloss = p.f("gloss").clamp(0.0, 100.0) / 100.0;
    let rounded = p.b("rounded_corners");
    let (ca, cbc) = (p.c("color_a"), p.c("color_b"));
    let (nw, nh) = (src.w + bw * 2, src.h + bw * 2);
    let (ar, ag, ab) = (
        ca.r as f32 / 255.0,
        ca.g as f32 / 255.0,
        ca.b as f32 / 255.0,
    );
    let (br_, bg_, bb_) = (
        cbc.r as f32 / 255.0,
        cbc.g as f32 / 255.0,
        cbc.b as f32 / 255.0,
    );
    let corner = if rounded { bw as f32 * 0.4 } else { 0.0 };
    let mut px = vec![C::ZERO; (nw * nh) as usize];
    let (il, it) = (bw as f32, bw as f32);
    let ir = (bw + src.w - 1) as f32;
    let ib = (bw + src.h - 1) as f32;
    let (iw, ih) = (src.w as f32, src.h as f32);
    for y in 0..nh {
        let in_top = y < bw;
        let in_bottom = y >= bw + src.h;
        for x in 0..nw {
            let in_left = x < bw;
            let in_right = x >= bw + src.w;
            if !in_top && !in_bottom && !in_left && !in_right {
                continue;
            }
            let (xf, yf) = (x as f32, y as f32);
            let cxp = xf.clamp(il, ir);
            let cyp = yf.clamp(it, ib);
            let d = ((xf - cxp) * (xf - cxp) + (yf - cyp) * (yf - cyp)).sqrt();
            let cross = (d / bw as f32).clamp(0.0, 1.0);
            let along = if cyp == it && cxp > il {
                cxp - il
            } else if cxp == ir && cyp > it {
                iw + (cyp - it)
            } else if cyp == ib && cxp < ir {
                iw + ih + (ir - cxp)
            } else if cxp == il && cyp < ib {
                iw * 2.0 + ih + (ib - cyp)
            } else if cxp == ir && cyp == it {
                iw
            } else if cxp == ir && cyp == ib {
                iw + ih
            } else if cxp == il && cyp == ib {
                iw * 2.0 + ih
            } else {
                0.0
            };
            let i = (y * nw + x) as usize;
            if rounded && corner > 0.0 {
                let icx = xf < corner || xf > (nw - 1) as f32 - corner;
                let icy = yf < corner || yf > (nh - 1) as f32 - corner;
                if icx && icy {
                    let ocx = if xf < corner {
                        corner
                    } else {
                        (nw - 1) as f32 - corner
                    };
                    let ocy = if yf < corner {
                        corner
                    } else {
                        (nh - 1) as f32 - corner
                    };
                    let dd = ((xf - ocx) * (xf - ocx) + (yf - ocy) * (yf - ocy)).sqrt();
                    if dd > corner {
                        px[i] = C::TRANSPARENT;
                        continue;
                    }
                }
            }
            let phase = (along + cross * bw as f32) / sw;
            let mut t = ((phase * PI * 2.0).sin() + 1.0) * 0.5;
            t = if t < 0.5 {
                smooth_step(0.15, 0.45, t)
            } else {
                1.0 - smooth_step(0.55, 0.85, 1.0 - t)
            };
            let mut r = lerp(ar, br_, t);
            let mut g = lerp(ag, bg_, t);
            let mut b = lerp(ab, bb_, t);
            if gloss > 0.001 {
                let c = (cross - 0.5) * 2.0;
                let hl = (-3.0 * c * c).exp();
                let dark = 1.0 - 0.15 * (1.0 - hl);
                let comb = dark + hl * 0.25 * gloss;
                r = (r * comb).clamp(0.0, 1.0);
                g = (g * comb).clamp(0.0, 1.0);
                b = (b * comb).clamp(0.0, 1.0);
            }
            px[i] = C::new(
                round(r * 255.0) as u8,
                round(g * 255.0) as u8,
                round(b * 255.0) as u8,
                255,
            );
        }
    }
    let base = Bitmap::from_pixels(nw, nh, px);
    let mut canvas = Canvas::from_bitmap(&base);
    canvas.draw_bitmap(src, bw as f32, bw as f32, &Paint::default());
    Ok(canvas.into_bitmap())
}

fn cartoon(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let levels = p.i("colorLevels").clamp(2, 20);
    let thr = p.i("edgeThreshold").clamp(5, 120);
    let ink = p.f("inkStrength").clamp(0.0, 100.0) / 100.0;
    let br = p.i("stickerBorder").clamp(0, 10);
    let bs = p.f("borderStrength").clamp(0.0, 100.0) / 100.0;
    let (w, h) = (src.w, src.h);
    let (right, bottom) = (w - 1, h - 1);
    let step = 255.0 / (levels - 1) as f32;
    let q = |v: u8| clamp_to_byte(round(v as f32 / step) * step);
    let quant: Vec<C> = src
        .px
        .iter()
        .map(|c| C::new(q(c.r), q(c.g), q(c.b), c.a))
        .collect();
    let mut edge = vec![0u8; quant.len()];
    for y in 0..h {
        let row = y * w;
        let yu = if y > 0 { y - 1 } else { 0 };
        let yd = if y < bottom { y + 1 } else { bottom };
        for x in 0..w {
            let xl = if x > 0 { x - 1 } else { 0 };
            let xr = if x < right { x + 1 } else { right };
            let l = quant[(row + xl) as usize];
            let r = quant[(row + xr) as usize];
            let u = quant[(yu * w + x) as usize];
            let d = quant[(yd * w + x) as usize];
            let gx = (r.r as i32 - l.r as i32).abs()
                + (r.g as i32 - l.g as i32).abs()
                + (r.b as i32 - l.b as i32).abs();
            let gy = (d.r as i32 - u.r as i32).abs()
                + (d.g as i32 - u.g as i32).abs()
                + (d.b as i32 - u.b as i32).abs();
            let e = (gx + gy) / 6;
            let n = (e - thr) * 255 / 1.max(255 - thr);
            edge[(row + x) as usize] = if n <= 0 { 0 } else { 255.min(n) as u8 };
        }
    }
    let border = if br > 0 && bs > 0.001 {
        let r2 = br * br;
        let mut m = vec![0u8; edge.len()];
        for y in 0..h {
            for x in 0..w {
                let mut max = 0u8;
                'outer: for ky in -br..=br {
                    let sy = (y + ky).clamp(0, bottom);
                    for kx in -br..=br {
                        if kx * kx + ky * ky > r2 {
                            continue;
                        }
                        let sx = (x + kx).clamp(0, right);
                        let v = edge[(sy * w + sx) as usize];
                        if v > max {
                            max = v;
                            if max == 255 {
                                break 'outer;
                            }
                        }
                    }
                }
                m[(y * w + x) as usize] = max;
            }
        }
        Some(m)
    } else {
        None
    };
    let px = quant
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let (mut r, mut g, mut b) = (c.r as f32, c.g as f32, c.b as f32);
            let e = edge[i] as f32 / 255.0;
            let im = e * ink;
            if let Some(bm) = &border {
                let bo = (bm[i] as f32 / 255.0 - e * 0.65).max(0.0);
                let mix = bo * bs;
                if mix > 0.001 {
                    r = lerp(r, 255.0, mix);
                    g = lerp(g, 255.0, mix);
                    b = lerp(b, 255.0, mix);
                }
            }
            if im > 0.001 {
                r = lerp(r, 0.0, im);
                g = lerp(g, 0.0, im);
                b = lerp(b, 0.0, im);
            }
            C::new(clamp_to_byte(r), clamp_to_byte(g), clamp_to_byte(b), c.a)
        })
        .collect();
    Ok(Bitmap::from_pixels(w, h, px))
}

fn celtic(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(16, 200);
    let frac = p.f("strand_width").clamp(10.0, 80.0) / 100.0;
    let knot = p.c("knot_color");
    let bg = p.c("bg_color");
    let outline = p.b("outline_strands");
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let b = border as f32;
    let mut canvas = Canvas::new(nw, nh);
    let bgp = Paint::fill(bg);
    canvas.draw_rect_xywh(0.0, 0.0, nw as f32, b, &bgp);
    canvas.draw_rect_xywh(0.0, (nh - border) as f32, nw as f32, b, &bgp);
    canvas.draw_rect_xywh(0.0, b, b, src.h as f32, &bgp);
    canvas.draw_rect_xywh((nw - border) as f32, b, b, src.h as f32, &bgp);
    let sw = b * frac;
    let knot_paint = Paint {
        style: Style::Stroke,
        stroke_width: sw,
        cap: Cap::Round,
        join: Join::Round,
        ..Paint::fill_aa(knot)
    };
    let dark = C::new(
        (bg.r as f32 * 0.7) as u8,
        (bg.g as f32 * 0.7) as u8,
        (bg.b as f32 * 0.7) as u8,
        bg.a,
    );
    let outline_paint = Paint {
        stroke_width: sw + 3.0,
        color: dark,
        ..knot_paint.clone()
    };
    let cell = b * 1.2;
    let band = |canvas: &mut Canvas, rx: f32, ry: f32, rw: f32, rh: f32, horizontal: bool| {
        canvas.save();
        canvas.clip_rect(rect(rx, ry, rx + rw, ry + rh), false);
        let primary = if horizontal { rw } else { rh };
        let mid = if horizontal {
            ry + rh * 0.5
        } else {
            rx + rw * 0.5
        };
        let amp = if horizontal { rh * 0.25 } else { rw * 0.25 };
        let cells = 2.max((primary / cell) as i32 + 1);
        for strand in 0..2 {
            let phase = strand as f32 * PI;
            let segs = cells * 8;
            let seg_len = primary / segs as f32;
            let mut pb = PathBuilder::new();
            for s in 0..=segs {
                let along = s as f32 * seg_len;
                let wave = ((along / cell) * PI * 2.0 + phase).sin() * amp;
                let (x, y) = if horizontal {
                    (rx + along, mid + wave)
                } else {
                    (mid + wave, ry + along)
                };
                if s == 0 {
                    pb.move_to(x, y);
                } else {
                    pb.line_to(x, y);
                }
            }
            if let Some(path) = pb.finish() {
                if outline {
                    canvas.draw_path(&path, &outline_paint);
                }
                canvas.draw_path(&path, &knot_paint);
            }
        }
        canvas.restore();
    };
    band(&mut canvas, 0.0, 0.0, nw as f32, b, true);
    band(&mut canvas, 0.0, (nh - border) as f32, nw as f32, b, true);
    band(&mut canvas, 0.0, b, b, src.h as f32, false);
    band(&mut canvas, (nw - border) as f32, b, b, src.h as f32, false);
    canvas.draw_bitmap(src, b, b, &Paint::default());
    if p.b("inner_line") {
        canvas.draw_rect_xywh(
            b - 1.25,
            b - 1.25,
            src.w as f32 + 2.5,
            src.h as f32 + 2.5,
            &Paint::stroke_aa(knot, 2.5),
        );
    }
    canvas.draw_rect_xywh(
        1.0,
        1.0,
        nw as f32 - 2.0,
        nh as f32 - 2.0,
        &Paint::stroke_aa(knot, 2.0),
    );
    Ok(canvas.into_bitmap())
}
