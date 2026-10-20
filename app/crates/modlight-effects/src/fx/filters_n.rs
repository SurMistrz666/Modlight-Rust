//! Filters, part N: VHS tape damage .. Wind streaks.

use std::f32::consts::PI;
use std::sync::Arc;

use modlight_core::icons as l;
use modlight_editor::effects::Category;
use tiny_skia::{PathBuilder, Transform};

use crate::def::*;
use crate::helpers::{
    apply_contrast, bilinear_sample, clamp_to_byte, clamp01, fractal_noise, hash01, lerp,
    lerp_color, luminance01, smooth_step,
};
use crate::sk::{Bitmap, C, Canvas, Paint, Sampling, Shader, Tile};

const F: Category = Category::Filters;

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "vhs_tape_damage",
            "VHS tape damage",
            F,
            l::VIDEOTAPE,
            "Simulates VHS tape degradation with distortion, noise, and color bleeding.",
            vhs,
        )
        .params(|| {
            vec![
                float_slider("distortion", "Distortion", 0.0, 100.0, 55.0),
                float_slider("noise", "Noise", 0.0, 100.0, 28.0),
                float_slider("color_bleed", "Color bleed", 0.0, 100.0, 30.0),
                float_slider("tracking", "Tracking", 0.0, 100.0, 24.0),
                float_slider("scanlines", "Scanlines", 0.0, 100.0, 48.0),
            ]
        }),
        Def::new(
            "vignette",
            "Vignette",
            F,
            l::CIRCLE_DASHED,
            "Applies a vignette effect.",
            vignette,
        )
        .params(|| {
            vec![
                fine(
                    float_slider("strength", "Strength", 0.0, 1.0, 0.5),
                    0.05,
                    "{}{0:0.00}",
                ),
                fine(
                    float_slider("radius", "Radius", 0.05, 0.999, 0.75),
                    0.01,
                    "{}{0:0.00}",
                ),
            ]
        }),
        Def::new(
            "vintage_print_damage",
            "Vintage print damage",
            F,
            l::NEWSPAPER,
            "Simulates aged print damage with sepia toning, grain, scratches, and dust.",
            vintage_print,
        )
        .params(|| {
            vec![
                float_slider("sepia_amount", "Sepia amount", 0.0, 100.0, 65.0),
                float_slider("fade_amount", "Fade amount", 0.0, 100.0, 30.0),
                float_slider("grain_amount", "Grain amount", 0.0, 100.0, 25.0),
                float_slider("scratch_amount", "Scratch amount", 0.0, 100.0, 25.0),
                float_slider("dust_amount", "Dust amount", 0.0, 100.0, 20.0),
            ]
        }),
        Def::new(
            "voronoi_mosaic",
            "Voronoi Mosaic",
            F,
            l::HEXAGON,
            "Divides the image into Voronoi cells colored by average region color.",
            voronoi,
        )
        .params(|| {
            vec![
                int_slider("cell_count", "Cell Count", 10, 2000, 300),
                checkbox("draw_edges", "Draw Edges", true),
                fine(
                    float_slider("edge_thickness", "Edge Thickness", 0.5, 5.0, 1.0),
                    0.5,
                    "{}{0:0.0}",
                ),
                color("edge_color", "Edge Color", C::new(0, 0, 0, 255)),
            ]
        }),
        Def::new(
            "watercolor_border",
            "Watercolor border",
            F,
            l::DROPLETS,
            "Adds a soft watercolor wash border with organic, painted edges that bleed into the image.",
            watercolor_border,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 10, 300, 60),
                color("wash_color", "Wash color", C::rgb(185, 160, 130)),
                float_slider("edge_roughness", "Edge roughness", 0.0, 100.0, 65.0),
                float_slider("bleed_amount", "Bleed amount", 0.0, 100.0, 40.0),
                float_slider("opacity", "Opacity", 10.0, 100.0, 90.0),
            ]
        }),
        Def::new(
            "watercolor_kuwahara",
            "Watercolor / Kuwahara",
            F,
            l::PAINTBRUSH,
            "Applies a Kuwahara filter for a watercolor painting effect.",
            kuwahara,
        )
        .params(|| {
            vec![
                int_slider("radius", "Radius", 2, 10, 4),
                float_slider("saturation_boost", "Saturation boost", 0.0, 100.0, 20.0),
                float_slider("detail_blend", "Detail blend", 0.0, 100.0, 18.0),
            ]
        }),
        Def::new(
            "wave_edge",
            "Wave edge",
            F,
            l::WAVES_LADDER,
            "Adds a wavy edge to the image.",
            wave_edge,
        )
        .params(|| {
            vec![
                int_slider("depth", "Depth", 1, 100, 15).snap(false),
                int_slider("range", "Range", 1, 100, 20).snap(false),
                checkbox("top", "Top", true),
                checkbox("right", "Right", true),
                checkbox("bottom", "Bottom", true),
                checkbox("left", "Left", true),
            ]
        }),
        Def::new(
            "wet_plate_collodion",
            "Wet plate collodion",
            F,
            l::FILM,
            "Simulates the wet plate collodion photographic process with stains and silvering.",
            wet_plate,
        )
        .params(|| {
            vec![
                float_slider("contrast", "Contrast", 50.0, 200.0, 132.0),
                float_slider("plate_stains", "Plate stains", 0.0, 100.0, 34.0),
                float_slider("scratches", "Scratches", 0.0, 100.0, 24.0),
                float_slider("silvering", "Silvering", 0.0, 100.0, 36.0),
                float_slider("vignette", "Vignette", 0.0, 100.0, 58.0),
            ]
        }),
        Def::new(
            "wind_streaks",
            "Wind streaks",
            F,
            l::WIND,
            "Adds directional wind streak trails to bright areas.",
            wind_streaks,
        )
        .params(|| {
            vec![
                int_slider("length", "Streak length", 5, 100, 30),
                int_slider("threshold", "Brightness threshold", 100, 254, 200),
                float_slider("opacity", "Opacity", 0.0, 100.0, 40.0),
                choice(
                    "direction",
                    "Direction",
                    &[("Left", "Left"), ("Right", "Right")],
                    "Right",
                ),
            ]
        }),
    ]
}

fn vhs(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let distortion = p.f("distortion").clamp(0.0, 100.0) / 100.0;
    let noise = p.f("noise").clamp(0.0, 100.0) / 100.0;
    let bleed = p.f("color_bleed").clamp(0.0, 100.0) / 100.0;
    let tracking = p.f("tracking").clamp(0.0, 100.0) / 100.0;
    let scanlines = p.f("scanlines").clamp(0.0, 100.0) / 100.0;
    if distortion <= 0.0001
        && noise <= 0.0001
        && bleed <= 0.0001
        && tracking <= 0.0001
        && scanlines <= 0.0001
    {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let sp = &src.px;
    let mut px = Vec::with_capacity(sp.len());
    for y in 0..h {
        let yf = y as f32;
        let row_noise = hash01(y, 0, 1703) * 2.0 - 1.0;
        let shift =
            ((yf * 0.085 + 0.7).sin() * 0.55 + (yf * 0.024 - 1.6).cos() * 0.45 + row_noise * 0.65)
                * distortion
                * 10.0;
        let dropout = if hash01(y / 3, 4, 4201) > 0.987 {
            0.35 + tracking * 0.45
        } else {
            0.0
        };
        let wave = (yf * 0.022 + 1.1).sin().max(0.0).powf(10.0) * tracking;
        let scan = 1.0 - scanlines * 0.18 * (0.5 + 0.5 * (yf * PI).sin());
        for x in 0..w {
            let bp = 0.5 + bleed * 4.5;
            let sx = x as f32 + shift;
            let rs = bilinear_sample(sp, w, h, sx + bp, yf);
            let gs = bilinear_sample(sp, w, h, sx, yf);
            let bs = bilinear_sample(sp, w, h, sx - bp, yf);
            let (mut r, mut g, mut b, a) = (rs.r as f32, gs.g as f32, bs.b as f32, gs.a as f32);
            let ln = (hash01(x, y, 6113) * 2.0 - 1.0) * noise * 28.0;
            r += ln * 1.05;
            g += ln;
            b += ln * 0.95;
            if wave > 0.0001 {
                let lift = wave * 42.0;
                r = lerp(r, 255.0, wave * 0.18);
                g += lift * 0.22;
                b += lift * 0.30;
            }
            if dropout > 0.0001 && hash01(x / 4, y, 7001) > 0.35 {
                r *= 1.0 - dropout;
                g *= 1.0 - dropout * 0.95;
                b *= 1.0 - dropout * 0.90;
            }
            px.push(C::new(
                clamp_to_byte(r * scan),
                clamp_to_byte(g * scan),
                clamp_to_byte(b * scan),
                clamp_to_byte(a),
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn vignette(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let strength = p.f("strength").clamp(0.0, 1.0);
    let radius = p.f("radius").clamp(0.05, 0.999);
    if strength <= 0.0 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let cx = (w - 1) as f32 * 0.5;
    let cy = (h - 1) as f32 * 0.5;
    let icx = if cx > 0.0 { 1.0 / cx } else { 1.0 };
    let icy = if cy > 0.0 { 1.0 / cy } else { 1.0 };
    let inv_sqrt2 = 0.707_106_8f32;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let dy = (y as f32 - cy) * icy;
        for x in 0..w {
            let dx = (x as f32 - cx) * icx;
            let d = (dx * dx + dy * dy).sqrt() * inv_sqrt2;
            let t = ((d - radius) / (1.0 - radius)).clamp(0.0, 1.0);
            let falloff = t * t * (3.0 - 2.0 * t);
            let f = 1.0 - strength * falloff;
            let c = src.get(x, y);
            px.push(C::new(
                clamp_to_byte(c.r as f32 * f),
                clamp_to_byte(c.g as f32 * f),
                clamp_to_byte(c.b as f32 * f),
                c.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn vintage_print(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let sepia = p.f("sepia_amount").clamp(0.0, 100.0) / 100.0;
    let fade = p.f("fade_amount").clamp(0.0, 100.0) / 100.0;
    let grain = p.f("grain_amount").clamp(0.0, 100.0) / 100.0;
    let scratches = p.f("scratch_amount").clamp(0.0, 100.0) / 100.0;
    let dust = p.f("dust_amount").clamp(0.0, 100.0) / 100.0;
    if sepia <= 0.0 && fade <= 0.0 && grain <= 0.0 && scratches <= 0.0 && dust <= 0.0 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let seed = 1947;
    let cx = (w - 1) as f32 * 0.5;
    let cy = (h - 1) as f32 * 0.5;
    let icx = if cx > 0.0 { 1.0 / cx } else { 1.0 };
    let icy = if cy > 0.0 { 1.0 / cy } else { 1.0 };
    let inv_sqrt2 = 0.707_106_8f32;
    let cell = 8f32.max(26.0 - dust * 18.0);
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let dyn_ = (y as f32 - cy) * icy;
        for x in 0..w {
            let s = src.get(x, y);
            let (mut r, mut g, mut b) =
                (s.r as f32 / 255.0, s.g as f32 / 255.0, s.b as f32 / 255.0);
            let sr = r * 0.393 + g * 0.769 + b * 0.189;
            let sg = r * 0.349 + g * 0.686 + b * 0.168;
            let sb = r * 0.272 + g * 0.534 + b * 0.131;
            r = lerp(r, sr, sepia);
            g = lerp(g, sg, sepia);
            b = lerp(b, sb, sepia);
            let contrast = 1.0 - fade * 0.35;
            let lift = fade * 0.045;
            r = (r - 0.5) * contrast + 0.5 + lift;
            g = (g - 0.5) * contrast + 0.5 + lift;
            b = (b - 0.5) * contrast + 0.5 + lift;
            let paper = 0.08 + fade * 0.10;
            r = lerp(r, 0.93, paper);
            g = lerp(g, 0.86, paper);
            b = lerp(b, 0.74, paper);
            if grain > 0.0 {
                let amp = grain * 0.12;
                r += (hash01(x, y, seed) * 2.0 - 1.0) * amp;
                g += (hash01(x, y, seed ^ 193) * 2.0 - 1.0) * amp;
                b += (hash01(x, y, seed ^ 761) * 2.0 - 1.0) * amp;
            }
            if scratches > 0.0 {
                let column = x / 2;
                let chance = hash01(column, 0, seed ^ 71);
                if chance > 0.985 - scratches * 0.12 {
                    let center = column as f32 * 2.0
                        + (hash01(column, 1, seed ^ 97) - 0.5) * 2.0
                        + (y as f32 * 0.03 + chance * 12.0).sin() * (0.5 + scratches * 2.0);
                    let wpx = 0.5 + hash01(column, 2, seed ^ 131) * 1.8;
                    let d = (x as f32 - center).abs() / wpx.max(0.001);
                    let li = (1.0 - smooth_step(0.1, 1.0, d)) * scratches;
                    if hash01(column, 3, seed ^ 173) > 0.55 {
                        r += li * 0.25;
                        g += li * 0.23;
                        b += li * 0.20;
                    } else {
                        r -= li * 0.22;
                        g -= li * 0.22;
                        b -= li * 0.22;
                    }
                }
            }
            if dust > 0.0 {
                let cxi = (x as f32 / cell) as i32;
                let cyi = (y as f32 / cell) as i32;
                let chance = hash01(cxi, cyi, seed ^ 521);
                if chance > 0.988 - dust * 0.23 {
                    let spx = (cxi as f32 + hash01(cxi, cyi, seed ^ 577)) * cell;
                    let spy = (cyi as f32 + hash01(cxi, cyi, seed ^ 653)) * cell;
                    let rad = cell * (0.10 + 0.24 * hash01(cxi, cyi, seed ^ 997));
                    let (dx, dy) = (x as f32 - spx, y as f32 - spy);
                    let radial = (dx * dx + dy * dy).sqrt() / rad.max(0.001);
                    let st = (1.0 - smooth_step(0.15, 1.0, radial)) * dust;
                    if hash01(cxi, cyi, seed ^ 733) > 0.60 {
                        r += st * 0.18;
                        g += st * 0.16;
                        b += st * 0.14;
                    } else {
                        r -= st * 0.20;
                        g -= st * 0.19;
                        b -= st * 0.18;
                    }
                }
            }
            let dxn = (x as f32 - cx) * icx;
            let d = (dxn * dxn + dyn_ * dyn_).sqrt() * inv_sqrt2;
            let edge = smooth_step(0.62, 1.0, d);
            let burn = edge * (0.10 + fade * 0.32 + scratches * 0.18);
            px.push(C::new(
                clamp_to_byte(r * (1.0 - burn) * 255.0),
                clamp_to_byte(g * (1.0 - burn) * 255.0),
                clamp_to_byte(b * (1.0 - burn) * 255.0),
                s.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn xorshift(mut s: i32) -> i32 {
    s ^= s << 13;
    s ^= s >> 17;
    s ^= s << 5;
    s
}

fn voronoi(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    let n = p.i("cell_count").clamp(2, 5000) as usize;
    let mut state = 42i32;
    let mut seeds = Vec::with_capacity(n);
    for _ in 0..n {
        state = xorshift(state);
        let x = (state & 0x7FFF_FFFF) as f32 / i32::MAX as f32 * w as f32;
        state = xorshift(state);
        let y = (state & 0x7FFF_FFFF) as f32 / i32::MAX as f32 * h as f32;
        seeds.push((x, y));
    }
    let cs = ((w * h) as f32 / n as f32).sqrt() * 2.0;
    let gc = 1.max((w as f32 / cs).ceil() as i32);
    let gr = 1.max((h as f32 / cs).ceil() as i32);
    let mut grid: Vec<Vec<usize>> = vec![Vec::new(); (gc * gr) as usize];
    for (i, s) in seeds.iter().enumerate() {
        let c = ((s.0 / cs) as i32).clamp(0, gc - 1);
        let r = ((s.1 / cs) as i32).clamp(0, gr - 1);
        grid[(r * gc + c) as usize].push(i);
    }
    let mut assignment = vec![0usize; src.px.len()];
    let mut sums = vec![[0i64; 3]; n];
    let mut counts = vec![0i64; n];
    for y in 0..h {
        let r0 = ((y as f32 / cs) as i32).clamp(0, gr - 1);
        for x in 0..w {
            let c0 = ((x as f32 / cs) as i32).clamp(0, gc - 1);
            let mut best = f32::MAX;
            let mut bi = 0usize;
            for r in 0.max(r0 - 2)..=(gr - 1).min(r0 + 2) {
                for c in 0.max(c0 - 2)..=(gc - 1).min(c0 + 2) {
                    for &si in &grid[(r * gc + c) as usize] {
                        let dx = x as f32 - seeds[si].0;
                        let dy = y as f32 - seeds[si].1;
                        let d = dx * dx + dy * dy;
                        if d < best {
                            best = d;
                            bi = si;
                        }
                    }
                }
            }
            let i = (y * w + x) as usize;
            assignment[i] = bi;
            let c = src.px[i];
            sums[bi][0] += c.r as i64;
            sums[bi][1] += c.g as i64;
            sums[bi][2] += c.b as i64;
            counts[bi] += 1;
        }
    }
    let colors: Vec<C> = (0..n)
        .map(|i| {
            if counts[i] > 0 {
                C::new(
                    (sums[i][0] / counts[i]) as u8,
                    (sums[i][1] / counts[i]) as u8,
                    (sums[i][2] / counts[i]) as u8,
                    255,
                )
            } else {
                C::ZERO
            }
        })
        .collect();
    let mut px: Vec<C> = assignment
        .iter()
        .zip(&src.px)
        .map(|(a, s)| {
            let c = colors[*a];
            C::new(c.r, c.g, c.b, s.a)
        })
        .collect();
    if p.b("draw_edges") {
        let edge = p.c("edge_color");
        let ed = 1.max(p.f("edge_thickness").ceil() as i32);
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                let mine = assignment[i];
                let mut is_edge = false;
                let mut d = 1;
                while d <= ed && !is_edge {
                    if x + d < w && assignment[i + d as usize] != mine {
                        is_edge = true;
                    }
                    if y + d < h && assignment[i + (d * w) as usize] != mine {
                        is_edge = true;
                    }
                    d += 1;
                }
                if is_edge {
                    px[i] = edge;
                }
            }
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn watercolor_border(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(10, 300);
    let rough = p.f("edge_roughness").clamp(0.0, 100.0) / 100.0;
    let bleed = p.f("bleed_amount").clamp(0.0, 100.0) / 100.0;
    let opacity = p.f("opacity").clamp(10.0, 100.0) / 100.0;
    let bp = (border as f32 * bleed * 0.5) as i32;
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let wash = p.c("wash_color");
    let (wr, wg, wb) = (
        wash.r as f32 / 255.0,
        wash.g as f32 / 255.0,
        wash.b as f32 / 255.0,
    );
    let mut px = vec![C::ZERO; (nw * nh) as usize];
    for y in 0..nh {
        for x in 0..nw {
            let (sx, sy) = (x - border, y - border);
            let dl = (border - x + bp) as f32;
            let dt = (border - y + bp) as f32;
            let dr = (x - (border + src.w - 1 - bp)) as f32;
            let db = (y - (border + src.h - 1 - bp)) as f32;
            let md = dl.max(dt).max(dr.max(db));
            let i = (y * nw + x) as usize;
            if md <= 0.0 {
                px[i] = src.get(sx, sy);
                continue;
            }
            let cross = (md / (border + bp) as f32).clamp(0.0, 1.0);
            let (xf, yf) = (x as f32, y as f32);
            let n = fractal_noise(xf * 0.015, yf * 0.015, 4, 2.0, 0.55, 37);
            let t = (cross + rough * (n * 2.0 - 1.0) * 0.35).clamp(0.0, 1.0);
            let mut wa = smooth_step(0.0, 0.7, t) * opacity;
            let pn = fractal_noise(xf * 0.04, yf * 0.04, 3, 2.0, 0.4, 99);
            wa *= 0.85 + pn * 0.3;
            wa = wa.clamp(0.0, 1.0);
            px[i] = if wa >= 0.999 {
                let ps = pn * 0.12 - 0.06;
                C::new(
                    clamp_to_byte((wr + ps) * 255.0),
                    clamp_to_byte((wg + ps) * 255.0),
                    clamp_to_byte((wb + ps) * 255.0),
                    255,
                )
            } else if sx >= 0 && sx < src.w && sy >= 0 && sy < src.h {
                let s = src.get(sx, sy);
                let sa = s.a as f32 / 255.0 * (1.0 - wa);
                let ps = pn * 0.10 - 0.05;
                C::new(
                    clamp_to_byte(lerp(s.r as f32 / 255.0, wr + ps, wa) * 255.0),
                    clamp_to_byte(lerp(s.g as f32 / 255.0, wg + ps, wa) * 255.0),
                    clamp_to_byte(lerp(s.b as f32 / 255.0, wb + ps, wa) * 255.0),
                    clamp_to_byte((sa + wa) * 255.0),
                )
            } else {
                let ps = pn * 0.12 - 0.06;
                C::new(
                    clamp_to_byte((wr + ps) * 255.0),
                    clamp_to_byte((wg + ps) * 255.0),
                    clamp_to_byte((wb + ps) * 255.0),
                    clamp_to_byte(wa * 255.0),
                )
            };
        }
    }
    Ok(Bitmap::from_pixels(nw, nh, px))
}

fn kuwahara(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let radius = p.i("radius").clamp(2, 10);
    let boost = p.f("saturation_boost").clamp(0.0, 100.0) / 100.0;
    let detail = p.f("detail_blend").clamp(0.0, 100.0) / 100.0;
    let stride = (w + 1) as usize;
    let size = stride * (h + 1) as usize;
    let mut ir = vec![0f64; size];
    let mut ig = vec![0f64; size];
    let mut ib = vec![0f64; size];
    let mut il = vec![0f64; size];
    let mut il2 = vec![0f64; size];
    for y in 1..=h as usize {
        let (mut rr, mut rg, mut rb, mut rl, mut rl2) = (0f64, 0f64, 0f64, 0f64, 0f64);
        for x in 1..=w as usize {
            let c = src.px[(y - 1) * w as usize + x - 1];
            let l = (0.2126 * c.r as f64 + 0.7152 * c.g as f64 + 0.0722 * c.b as f64) / 255.0;
            rr += c.r as f64;
            rg += c.g as f64;
            rb += c.b as f64;
            rl += l;
            rl2 += l * l;
            let i = y * stride + x;
            let pi = (y - 1) * stride + x;
            ir[i] = ir[pi] + rr;
            ig[i] = ig[pi] + rg;
            ib[i] = ib[pi] + rb;
            il[i] = il[pi] + rl;
            il2[i] = il2[pi] + rl2;
        }
    }
    let rect_sum = |a: &[f64], x0: i32, y0: i32, x1: i32, y1: i32| -> f64 {
        let s = stride as i32;
        a[(y1 * s + x1) as usize] - a[(y0 * s + x1) as usize] - a[(y1 * s + x0) as usize]
            + a[(y0 * s + x0) as usize]
    };
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let mut best = (0f64, 0f64, 0f64);
            let mut best_var = f64::MAX;
            for (x0, y0, x1, y1) in [
                (x - radius, y - radius, x + 1, y + 1),
                (x, y - radius, x + radius + 1, y + 1),
                (x - radius, y, x + 1, y + radius + 1),
                (x, y, x + radius + 1, y + radius + 1),
            ] {
                let (l, t, r, b) = (
                    x0.clamp(0, w),
                    y0.clamp(0, h),
                    x1.clamp(0, w),
                    y1.clamp(0, h),
                );
                if r <= l || b <= t {
                    continue;
                }
                let count = (r - l) * (b - t);
                if count <= 0 {
                    continue;
                }
                let inv = 1.0 / count as f64;
                let ml = rect_sum(&il, l, t, r, b) * inv;
                let var = rect_sum(&il2, l, t, r, b) * inv - ml * ml;
                if var < best_var {
                    best_var = var;
                    best = (
                        rect_sum(&ir, l, t, r, b) * inv,
                        rect_sum(&ig, l, t, r, b) * inv,
                        rect_sum(&ib, l, t, r, b) * inv,
                    );
                }
            }
            let s = src.get(x, y);
            let mut f = C::new(
                clamp_to_byte(best.0 as f32),
                clamp_to_byte(best.1 as f32),
                clamp_to_byte(best.2 as f32),
                s.a,
            );
            if boost > 0.0 {
                let (r, g, b) = (f.r as f32, f.g as f32, f.b as f32);
                let gray = (r + g + b) / 3.0;
                let k = 1.0 + boost;
                f = C::new(
                    clamp_to_byte(gray + (r - gray) * k),
                    clamp_to_byte(gray + (g - gray) * k),
                    clamp_to_byte(gray + (b - gray) * k),
                    f.a,
                );
            }
            if detail > 0.0 {
                f = lerp_color(f, s, detail * 0.55);
            }
            px.push(f);
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn wave_fn(t: i32, max: i32, depth: i32) -> f32 {
    ((1.0 - (t as f64 * std::f64::consts::PI / max as f64).cos()) * depth as f64 / 2.0) as f32
}

fn wave_edge(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (depth, range) = (p.i("depth"), p.i("range"));
    let (top, right, bottom, left) = (p.b("top"), p.b("right"), p.b("bottom"), p.b("left"));
    if depth < 1 || range < 1 || (!top && !right && !bottom && !left) {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let hwc = 2.max((w / range + 1) / 2 * 2) - 1;
    let vwc = 2.max((h / range + 1) / 2 * 2) - 1;
    let hwr = w / hwc;
    let vwr = h / vwc;
    let step = (range / depth).max(1).min(10);
    let mut pts: Vec<(f32, f32)> = Vec::new();
    if top {
        let sx = if left { depth } else { 0 };
        let ex = if right { w - depth } else { w };
        let mut x = sx;
        while x < ex {
            pts.push((x as f32, wave_fn(x, hwr, depth)));
            x += step;
        }
        pts.push((ex as f32, wave_fn(ex, hwr, depth)));
    } else {
        pts.push((0.0, 0.0));
    }
    if right {
        let sy = if top { depth } else { 0 };
        let ey = if bottom { h - depth } else { h };
        let mut y = sy;
        while y < ey {
            pts.push(((w - depth) as f32 + wave_fn(y, vwr, depth), y as f32));
            y += step;
        }
        pts.push(((w - depth) as f32 + wave_fn(ey, vwr, depth), ey as f32));
    } else {
        let last = pts[pts.len() - 1];
        pts.push((w as f32, last.1));
    }
    if bottom {
        let sx = if right { w - depth } else { w };
        let ex = if left { depth } else { 0 };
        let mut x = sx;
        while x >= ex {
            pts.push((x as f32, (h - depth) as f32 + wave_fn(x, hwr, depth)));
            x -= step;
        }
        pts.push((ex as f32, (h - depth) as f32 + wave_fn(ex, hwr, depth)));
    } else {
        let last = pts[pts.len() - 1];
        pts.push((last.0, h as f32));
    }
    if left {
        let sy = if bottom { h - depth } else { h };
        let ey = if top { depth } else { 0 };
        let mut y = sy;
        while y >= ey {
            pts.push((wave_fn(y, vwr, depth), y as f32));
            y -= step;
        }
        pts.push((wave_fn(ey, vwr, depth), ey as f32));
    } else {
        let last = pts[pts.len() - 1];
        pts.push((0.0, last.1));
    }
    if !top {
        let last = pts[pts.len() - 1];
        pts[0] = (last.0, 0.0);
    }
    if pts.len() <= 2 {
        return Ok(src.clone());
    }
    let mut canvas = Canvas::new(w, h);
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].0, pts[0].1);
    for pt in &pts[1..] {
        pb.line_to(pt.0, pt.1);
    }
    pb.close();
    if let Some(path) = pb.finish() {
        let paint = Paint::fill_aa(C::BLACK).with_shader(Shader::Image {
            bitmap: Arc::new(src.clone()),
            tile_x: Tile::Clamp,
            tile_y: Tile::Clamp,
            matrix: Transform::identity(),
            sampling: Sampling::Nearest,
        });
        canvas.draw_path(&path, &paint);
    }
    Ok(canvas.into_bitmap())
}

fn wet_plate(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let contrast = p.f("contrast").clamp(50.0, 200.0) / 100.0;
    let stains = p.f("plate_stains").clamp(0.0, 100.0) / 100.0;
    let scratches = p.f("scratches").clamp(0.0, 100.0) / 100.0;
    let silvering = p.f("silvering").clamp(0.0, 100.0) / 100.0;
    let vignette = p.f("vignette").clamp(0.0, 100.0) / 100.0;
    let (w, h) = (src.w, src.h);
    let seed = 1855;
    let cx = (w - 1) as f32 * 0.5;
    let cy = (h - 1) as f32 * 0.5;
    let max_dist = (cx * cx + cy * cy).sqrt().max(1.0);
    let (shadow, highlight) = (C::rgb(20, 16, 18), C::rgb(220, 213, 202));
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let lum = apply_contrast(luminance01(s), contrast);
            let tone = lerp_color(shadow, highlight, lum.powf(0.88));
            let (xf, yf) = (x as f32, y as f32);
            let chem = fractal_noise(xf * 0.008, yf * 0.008, 5, 2.05, 0.52, seed);
            let streak = fractal_noise(xf * 0.024, yf * 0.0026, 4, 2.2, 0.48, seed ^ 337);
            let mut stain = smooth_step(0.54, 0.92, chem) * stains;
            stain += (streak - 0.72).max(0.0) * stains * 0.45;
            let ed = x.min(w - 1 - x).min(y.min(h - 1 - y)) as f32;
            let edge = 1.0 - (ed / 1f32.max(w.min(h) as f32 * 0.16)).clamp(0.0, 1.0);
            let silver = silvering * edge * lum.max(0.02).powf(0.55);
            let radial = ((xf - cx).powf(2.0) + (yf - cy).powf(2.0)).sqrt() / max_dist;
            let vm = clamp01(1.0 - vignette * radial.powf(1.95) * 0.78);
            let mut scratch = 0.0;
            if hash01(x / 3, y, seed ^ 173) > 0.9975 {
                scratch += scratches * 0.55;
            }
            if hash01(x, y / 5, seed ^ 811) > 0.9984 {
                scratch += scratches * 0.35;
            }
            let mut r = tone.r as f32 / 255.0;
            let mut g = tone.g as f32 / 255.0;
            let mut b = tone.b as f32 / 255.0;
            r = lerp(r, 0.88, silver * 0.34);
            g = lerp(g, 0.84, silver * 0.30);
            b = lerp(b, 0.92, silver * 0.38);
            r = clamp01((r - stain * 0.10 + scratch) * vm);
            g = clamp01((g - stain * 0.08 + scratch) * vm);
            b = clamp01((b - stain * 0.06 + scratch) * vm);
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

fn wind_streaks(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    let len = 1.max(p.i("length"));
    let threshold = p.i("threshold") as f32;
    let alpha = (p.f("opacity") / 100.0).clamp(0.0, 1.0);
    let dir = if p.e("direction") == "Left" { -1 } else { 1 };
    let mut dst = src.px.clone();
    for y in 0..h {
        for x in 0..w {
            let c = src.get(x, y);
            let lum = 0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32;
            if lum < threshold {
                continue;
            }
            let sa = alpha * ((lum - threshold) / (255.0 - threshold));
            for s in 1..=len {
                let tx = x + s * dir;
                if tx < 0 || tx >= w {
                    break;
                }
                let blend = sa * (1.0 - s as f32 / len as f32);
                let i = (y * w + tx) as usize;
                let e = dst[i];
                let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * blend) as u8;
                dst[i] = C::new(mix(e.r, c.r), mix(e.g, c.g), mix(e.b, c.b), e.a);
            }
        }
    }
    Ok(Bitmap::from_pixels(w, h, dst))
}
