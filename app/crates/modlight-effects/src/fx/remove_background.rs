//! `RemoveBackgroundImageEffect`: upstream's classic background removal (no model):
//! border colours become background prototypes, centre colours far from them foreground
//! prototypes; every pixel gets a background likelihood and a penalty, a flood fill from
//! the border takes what is cheap enough, and the mask is scaled back and feathered.

use modlight_core::icons as l;
use modlight_editor::effects::Category;

use crate::def::*;
use crate::helpers::{clamp_to_byte, smooth_step};
use crate::sk::{Bitmap, C, Canvas, Paint, Sampling, rect, round};

const TRANSPARENT_ALPHA_THRESHOLD: u8 = 20;
const MAX_PROCESS_DIMENSION: f32 = 256.0;

pub fn def() -> Def {
    Def::new(
        "remove_background",
        "Remove background",
        Category::Manipulations,
        l::SCISSORS,
        "Removes border-connected background colors and turns them transparent.",
        apply,
    )
    .params(|| {
        vec![
            float_slider("sensitivity", "Sensitivity (%)", 0.0, 100.0, 60.0),
            float_slider(
                "center_protection",
                "Center protection (%)",
                0.0,
                100.0,
                65.0,
            ),
            fine(
                float_slider("edge_feather", "Edge feather (px)", 0.0, 24.0, 4.0),
                0.5,
                "{}{0:0.#}",
            ),
        ]
    })
}

#[derive(Clone, Copy, Debug, Default)]
struct Feature {
    luma: f32,
    blue: f32,
    red: f32,
    sat: f32,
}

impl Feature {
    fn from_color(c: C) -> Feature {
        let r = c.r as f32 / 255.0;
        let g = c.g as f32 / 255.0;
        let b = c.b as f32 / 255.0;
        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        Feature {
            luma,
            blue: (b - luma) * 1.15,
            red: (r - luma) * 1.15,
            sat: r.max(g.max(b)) - r.min(g.min(b)),
        }
    }

    fn dist2(a: Feature, b: Feature) -> f32 {
        let l = a.luma - b.luma;
        let bd = a.blue - b.blue;
        let rd = a.red - b.red;
        let s = a.sat - b.sat;
        l * l * 0.52 + bd * bd * 0.94 + rd * rd * 0.94 + s * s * 0.35
    }
}

fn nearest_distance(f: Feature, protos: &[Feature]) -> f32 {
    if protos.is_empty() {
        return 1.0;
    }
    let mut best = f32::MAX;
    for p in protos {
        let d = Feature::dist2(f, *p);
        if d < best {
            best = d;
        }
    }
    best.sqrt()
}

fn nearest_index(f: Feature, protos: &[Feature]) -> usize {
    let mut idx = 0;
    let mut best = f32::MAX;
    for (i, p) in protos.iter().enumerate() {
        let d = Feature::dist2(f, *p);
        if d < best {
            best = d;
            idx = i;
        }
    }
    idx
}

fn build_prototypes(samples: &[Feature], max: usize) -> Vec<Feature> {
    if samples.is_empty() || max == 0 {
        return Vec::new();
    }
    let count = max.min(samples.len());
    let mut protos = vec![Feature::default(); count];
    protos[0] = samples[0];
    for i in 1..count {
        let mut far = 0;
        let mut far_d = -1.0f32;
        for (si, s) in samples.iter().enumerate() {
            let d = nearest_distance(*s, &protos[..i]);
            if d > far_d {
                far_d = d;
                far = si;
            }
        }
        protos[i] = samples[far];
    }
    for _ in 0..5 {
        let mut sums = vec![[0f32; 4]; count];
        let mut counts = vec![0i32; count];
        for s in samples {
            let n = nearest_index(*s, &protos);
            sums[n][0] += s.luma;
            sums[n][1] += s.blue;
            sums[n][2] += s.red;
            sums[n][3] += s.sat;
            counts[n] += 1;
        }
        for pi in 0..count {
            if counts[pi] == 0 {
                protos[pi] = samples[(pi * samples.len()) / count];
                continue;
            }
            let inv = 1.0 / counts[pi] as f32;
            protos[pi] = Feature {
                luma: sums[pi][0] * inv,
                blue: sums[pi][1] * inv,
                red: sums[pi][2] * inv,
                sat: sums[pi][3] * inv,
            };
        }
    }
    protos
}

fn select_foreground(candidates: &[Feature], bg: &[Feature]) -> Vec<Feature> {
    if candidates.is_empty() || bg.is_empty() {
        return candidates.to_vec();
    }
    let mut distances = Vec::with_capacity(candidates.len());
    let mut sum = 0.0f32;
    let mut max = 0.0f32;
    for c in candidates {
        let d = nearest_distance(*c, bg);
        distances.push(d);
        sum += d;
        if d > max {
            max = d;
        }
    }
    let average = sum / 1.max(candidates.len()) as f32;
    let cutoff = average + (max - average) * 0.18;
    let by = |cut: f32| -> Vec<Feature> {
        candidates
            .iter()
            .zip(&distances)
            .filter(|(_, d)| **d >= cut)
            .map(|(c, _)| *c)
            .collect()
    };
    let selected = by(cutoff);
    if selected.len() >= 16.max(candidates.len() / 8) {
        return selected;
    }
    let selected = by(average);
    if selected.len() >= 8 {
        selected
    } else {
        candidates.to_vec()
    }
}

fn affinity_scale(samples: &[Feature], protos: &[Feature], fallback: f32) -> f32 {
    if samples.is_empty() || protos.is_empty() {
        return fallback;
    }
    let mut sum = 0.0f32;
    for s in samples {
        sum += nearest_distance(*s, protos);
    }
    let average = sum / samples.len() as f32;
    fallback.max(average * 2.8 + fallback * 0.4)
}

fn edge_magnitude(luma: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut edges = vec![0f32; luma.len()];
    let (right, bottom) = (w - 1, h - 1);
    for y in 0..h {
        let y0 = y.saturating_sub(1);
        let y2 = if y < bottom { y + 1 } else { bottom };
        for x in 0..w {
            let x0 = x.saturating_sub(1);
            let x2 = if x < right { x + 1 } else { right };
            let l = |xx: usize, yy: usize| luma[yy * w + xx];
            let gx = (-l(x0, y0) + l(x2, y0))
                + (-2.0 * l(x0, y) + 2.0 * l(x2, y))
                + (-l(x0, y2) + l(x2, y2));
            let gy = (-l(x0, y0) - 2.0 * l(x, y0) - l(x2, y0))
                + (l(x0, y2) + 2.0 * l(x, y2) + l(x2, y2));
            edges[y * w + x] = ((gx * gx + gy * gy).sqrt() / 1.35).clamp(0.0, 1.0);
        }
    }
    edges
}

#[allow(clippy::too_many_arguments)]
fn flood_background(
    alpha: &[u8],
    likelihood: &[f32],
    edges: &[f32],
    penalty: &[f32],
    w: usize,
    h: usize,
    sensitivity: f32,
) -> Vec<bool> {
    let mut visited = vec![false; penalty.len()];
    let mut queue = std::collections::VecDeque::new();
    let base = 0.19 + sensitivity * 0.28;
    let seed = base + 0.04;
    let boost = 0.04 + sensitivity * 0.06;
    let try_seed = |i: usize,
                    visited: &mut Vec<bool>,
                    queue: &mut std::collections::VecDeque<usize>| {
        if visited[i] {
            return;
        }
        if alpha[i] <= TRANSPARENT_ALPHA_THRESHOLD || penalty[i] <= seed || likelihood[i] >= 0.82 {
            visited[i] = true;
            queue.push_back(i);
        }
    };
    for x in 0..w {
        try_seed(x, &mut visited, &mut queue);
        try_seed((h - 1) * w + x, &mut visited, &mut queue);
    }
    for y in 1..h.saturating_sub(1) {
        try_seed(y * w, &mut visited, &mut queue);
        try_seed(y * w + (w - 1), &mut visited, &mut queue);
    }
    while let Some(i) = queue.pop_front() {
        let x = (i % w) as i32;
        let y = (i / w) as i32;
        for oy in -1..=1 {
            let ny = y + oy;
            if ny < 0 || ny >= h as i32 {
                continue;
            }
            for ox in -1..=1 {
                if ox == 0 && oy == 0 {
                    continue;
                }
                let nx = x + ox;
                if nx < 0 || nx >= w as i32 {
                    continue;
                }
                let ni = ny as usize * w + nx as usize;
                if visited[ni] {
                    continue;
                }
                if alpha[ni] <= TRANSPARENT_ALPHA_THRESHOLD {
                    visited[ni] = true;
                    queue.push_back(ni);
                    continue;
                }
                let local = base + (1.0 - edges[ni]) * 0.05 + likelihood[ni] * boost;
                if penalty[ni] <= local {
                    visited[ni] = true;
                    queue.push_back(ni);
                }
            }
        }
    }
    visited
}

fn center_weight(x: usize, y: usize, w: usize, h: usize) -> f32 {
    let cx = (w as f32 - 1.0) * 0.5;
    let cy = (h as f32 - 1.0) * 0.5;
    let dx = (x as f32 - cx) / 1f32.max(w as f32 * 0.5);
    let dy = (y as f32 - cy) / 1f32.max(h as f32 * 0.5);
    let d = (dx * dx + dy * dy).sqrt();
    1.0 - smooth_step(0.22, 1.0, d)
}

/// `ResizeBitmap`: Catmull-Rom onto a cleared bitmap (anti-aliased paint).
fn resize_bitmap(src: &Bitmap, w: i32, h: i32) -> Bitmap {
    if src.w == w && src.h == h {
        return src.clone();
    }
    let mut canvas = Canvas::new(w, h);
    canvas.draw_bitmap_to(
        src,
        rect(0.0, 0.0, w as f32, h as f32),
        Sampling::CATMULL_ROM,
        &Paint::default().aa(true),
    );
    canvas.into_bitmap()
}

/// `ApplyTransparentBlur`: blur with transparent padding, then cut back.
fn transparent_blur(src: &Bitmap, radius: f32) -> Bitmap {
    if radius <= 0.01 {
        return src.clone();
    }
    let pad = 2.max((radius * 2.0).ceil() as i32);
    let (ew, eh) = (src.w + pad * 2, src.h + pad * 2);
    let mut expanded = Canvas::new(ew, eh);
    expanded.draw_bitmap(src, pad as f32, pad as f32, &Paint::default());
    let expanded = expanded.into_bitmap();
    let sigma = 0.001f32.max(radius / 3.0);
    let blurred = crate::helpers::blur_bitmap(&expanded, sigma, sigma);
    let mut out = Bitmap::new(src.w, src.h);
    for y in 0..src.h {
        for x in 0..src.w {
            out.px[(y * src.w + x) as usize] = blurred.get(x + pad, y + pad);
        }
    }
    out
}

fn apply(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (width, height) = (src.w, src.h);
    if width <= 0 || height <= 0 {
        return Ok(src.clone());
    }
    let sensitivity = p.f("sensitivity").clamp(0.0, 100.0) / 100.0;
    let protection = p.f("center_protection").clamp(0.0, 100.0) / 100.0;
    let feather = p.f("edge_feather").clamp(0.0, 24.0);
    if !src.px.iter().any(|c| c.a > TRANSPARENT_ALPHA_THRESHOLD) {
        return Ok(src.clone());
    }
    let scale = 1f32.min(MAX_PROCESS_DIMENSION / width.max(height) as f32);
    let pw = 1.max(round(width as f32 * scale) as i32);
    let ph = 1.max(round(height as f32 * scale) as i32);
    let working = if scale < 0.999 {
        resize_bitmap(src, pw, ph)
    } else {
        src.clone()
    };
    let (w, h) = (pw as usize, ph as usize);
    let count = w * h;
    let mut features = vec![Feature::default(); count];
    let mut luma = vec![0f32; count];
    let mut alpha = vec![0u8; count];
    let mut border = Vec::new();
    let mut center = Vec::new();
    let band = (round(pw.min(ph) as f32 * 0.08) as i32).clamp(1, 24);
    let inset_x = (round(pw as f32 * 0.22) as i32).clamp(band, band.max(pw / 2));
    let inset_y = (round(ph as f32 * 0.18) as i32).clamp(band, band.max(ph / 2));
    let (cl, ct) = (inset_x, inset_y);
    let cr = cl.max(pw - inset_x - 1);
    let cb = ct.max(ph - inset_y - 1);
    for y in 0..ph {
        let border_row = y < band || y >= ph - band;
        let center_row = y >= ct && y <= cb;
        for x in 0..pw {
            let i = (y * pw + x) as usize;
            let c = working.px[i];
            alpha[i] = c.a;
            let f = Feature::from_color(c);
            features[i] = f;
            luma[i] = f.luma;
            if c.a <= TRANSPARENT_ALPHA_THRESHOLD {
                continue;
            }
            if border_row || x < band || x >= pw - band {
                border.push(f);
            }
            if center_row && x >= cl && x <= cr {
                center.push(f);
            }
        }
    }
    if border.len() < 12 {
        return Ok(src.clone());
    }
    let bg = build_prototypes(&border, 4);
    let fg_samples = select_foreground(&center, &bg);
    let fg = build_prototypes(&fg_samples, 3);
    let bg_scale = affinity_scale(&border, &bg, 0.045);
    let fg_scale = if !fg.is_empty() {
        affinity_scale(&fg_samples, &fg, 0.055)
    } else {
        0.08
    };
    let edges = edge_magnitude(&luma, w, h);
    let mut likelihood = vec![0f32; count];
    let mut penalty = vec![0f32; count];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if alpha[i] <= TRANSPARENT_ALPHA_THRESHOLD {
                likelihood[i] = 1.0;
                penalty[i] = 0.0;
                continue;
            }
            let f = features[i];
            let bd = nearest_distance(f, &bg);
            let fd = if !fg.is_empty() {
                nearest_distance(f, &fg)
            } else {
                bd + 0.16
            };
            let ba = (-bd / bg_scale).exp();
            let fa = if !fg.is_empty() {
                (-fd / fg_scale).exp()
            } else {
                0.16
            };
            let lk = ba / (ba + fa + 0.0001);
            likelihood[i] = lk;
            let cw = center_weight(x, y, w, h);
            let lift = 1.0 - alpha[i] as f32 / 255.0;
            let local = (1.0 - lk) * 0.72 + edges[i] * 0.18 + cw * protection * 0.22 - lift * 0.12;
            penalty[i] = local.max(0.0);
        }
    }
    let mask = flood_background(&alpha, &likelihood, &edges, &penalty, w, h, sensitivity);
    let small: Vec<C> = mask
        .iter()
        .map(|bgm| {
            let a = if *bgm { 0 } else { 255 };
            C::new(a, a, a, a)
        })
        .collect();
    let small = Bitmap::from_pixels(pw, ph, small);
    let scaled = if pw == width && ph == height {
        small
    } else {
        resize_bitmap(&small, width, height)
    };
    let fin = if feather > 0.05 {
        transparent_blur(&scaled, feather)
    } else {
        scaled
    };
    let px = src
        .px
        .iter()
        .zip(&fin.px)
        .map(|(s, m)| {
            let ma = m.a as f32 / 255.0;
            C::new(s.r, s.g, s.b, clamp_to_byte(s.a as f32 * ma))
        })
        .collect();
    Ok(Bitmap::from_pixels(width, height, px))
}
