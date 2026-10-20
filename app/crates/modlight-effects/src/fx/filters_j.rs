//! Filters, part J: Old camera flash burn .. Plasma energy arcs.

use modlight_core::icons as l;
use modlight_editor::effects::Category;
use tiny_skia::PathBuilder;

use crate::def::*;
use crate::dotnet::sort_by;
use crate::helpers::{clamp_to_byte, clamp01, hash01, lerp, smooth_step};
use crate::sk::filters::morphology_premul;
use crate::sk::{
    Bitmap, Blend, C, Canvas, Cap, ColorFilter, Join, Paint, Sampling, Shader, Style, Tile, premul,
    rect, round,
};

const F: Category = Category::Filters;

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "old_camera_flash_burn",
            "Old camera flash burn",
            F,
            l::FLASHLIGHT,
            "Simulates an old camera flash burn with warm glow and edge burn effects.",
            flash_burn,
        )
        .params(|| {
            vec![
                float_slider("flash_strength", "Flash strength", 0.0, 100.0, 70.0),
                float_slider("flash_radius", "Flash radius", 20.0, 100.0, 68.0),
                float_slider("edge_burn", "Edge burn", 0.0, 100.0, 45.0),
                float_slider("warmth", "Warmth", 0.0, 100.0, 35.0),
                float_slider("grain", "Grain", 0.0, 100.0, 20.0),
                float_slider("center_x", "Center X", 0.0, 100.0, 50.0),
                float_slider("center_y", "Center Y", 0.0, 100.0, 50.0),
            ]
        }),
        Def::new(
            "ornate_scroll_border",
            "Ornate scroll border",
            F,
            l::SCROLL,
            "Adds an elegant Victorian-style ornate border with decorative corner scrollwork and double-line detailing.",
            ornate_scroll,
        )
        .params(|| {
            vec![
                int_numeric("border_size", "Border size", 10, 200, 48),
                color("frame_color", "Frame color", C::rgb(245, 240, 220)),
                color("ornament_color", "Ornament color", C::rgb(160, 130, 60)),
                float_slider("line_thickness", "Line thickness", 0.5, 6.0, 2.0),
                checkbox("fill_background", "Fill background", true),
                checkbox("shadow", "Shadow", true),
            ]
        }),
        Def::new(
            "outline",
            "Outline",
            F,
            l::SQUARE_DASHED,
            "Applies an outline effect.",
            outline,
        )
        .params(|| {
            vec![
                int_slider("size", "Size", 1, 50, 3).snap(false),
                int_slider("padding", "Padding", 0, 100, 0).snap(false),
                checkbox("outline_only", "Outline only", false),
                color("color", "Color", C::BLACK),
            ]
        }),
        Def::new(
            "palette_map",
            "Palette map",
            F,
            l::SWATCH_BOOK,
            "Reduces the image to a limited color palette by quantization.",
            palette_map,
        )
        .params(|| vec![int_slider("levels", "Color levels", 2, 32, 6)]),
        Def::new(
            "paper_stencil_mask",
            "Paper stencil mask",
            F,
            l::STAMP,
            "Applies a paper stencil mask effect with adjustable threshold and edge strength.",
            paper_stencil,
        )
        .params(|| {
            vec![
                float_slider("threshold", "Threshold", 0.0, 255.0, 140.0),
                float_slider("feather_radius", "Feather radius", 0.0, 200.0, 8.0),
                float_slider("edge_strength", "Edge strength", 0.0, 100.0, 70.0),
                float_slider("background_dim", "Background dim", 0.0, 100.0, 35.0),
                checkbox("invert_mask", "Invert mask", false),
                color("stencil_color", "Stencil color", C::new(0, 0, 0, 220)),
            ]
        }),
        Def::new(
            "pencil_sketch",
            "Pencil sketch",
            F,
            l::PENCIL_LINE,
            "Converts the image to a pencil sketch style drawing.",
            pencil_sketch,
        )
        .params(|| {
            vec![
                int_slider("blur_radius", "Blur radius", 1, 24, 8),
                float_slider("edge_strength", "Edge strength", 0.0, 160.0, 65.0),
                float_slider("pencil_darkness", "Pencil darkness", 0.0, 100.0, 70.0),
                float_slider("paper_brightness", "Paper brightness", 40.0, 130.0, 100.0),
            ]
        }),
        Def::new(
            "perlin_noise",
            "Perlin noise",
            F,
            l::WAVES,
            "Generates a Perlin noise pattern and blends it with the image.",
            perlin,
        )
        .params(|| {
            vec![
                fine(float_slider("scale", "Scale", 1.0, 500.0, 64.0), 1.0, "{}{0:0}"),
                int_slider("octaves", "Octaves", 1, 8, 4),
                fine(
                    float_slider("persistence", "Persistence", 0.0, 1.0, 0.5),
                    0.01,
                    "{}{0:0.00}",
                ),
                fine(float_slider("strength", "Strength", 0.0, 100.0, 50.0), 1.0, "{}{0:0}"),
                checkbox("monochrome", "Monochrome", true),
            ]
        }),
        Def::new(
            "pixel_sorting",
            "Pixel sorting",
            F,
            l::ARROW_DOWN_WIDE_NARROW,
            "Sorts pixels along rows or columns based on brightness or hue thresholds.",
            pixel_sort,
        )
        .params(|| {
            vec![
                choice(
                    "direction",
                    "Direction",
                    &[("Horizontal", "Horizontal"), ("Vertical", "Vertical")],
                    "Vertical",
                ),
                choice(
                    "metric",
                    "Metric",
                    &[("Brightness", "Brightness"), ("Hue", "Hue")],
                    "Brightness",
                ),
                float_slider("threshold_low", "Threshold low", 0.0, 100.0, 12.0),
                float_slider("threshold_high", "Threshold high", 0.0, 100.0, 85.0),
                int_slider("min_span_length", "Min span length", 2, 256, 8),
                int_slider("max_span_length", "Max span length", 2, 512, 120),
                float_slider("sort_probability", "Sort probability", 0.0, 100.0, 85.0),
            ]
        }),
        Def::new(
            "pixelate",
            "Pixelate",
            F,
            l::GRID_2X2,
            "Pixelates the image.",
            pixelate,
        )
        .params(|| vec![int_slider("size", "Size", 1, 200, 10)]),
        Def::new(
            "plasma_energy_arcs",
            "Plasma energy arcs",
            F,
            l::ZAP,
            "Generates plasma-like energy arcs overlaid on the image.",
            plasma_arcs,
        )
        .params(|| {
            vec![
                float_slider("energy", "Energy", 0.0, 100.0, 65.0),
                float_slider("arc_density", "Arc density", 0.0, 100.0, 46.0),
                float_slider("glow", "Glow", 0.0, 100.0, 70.0),
                float_slider("turbulence", "Turbulence", 0.0, 100.0, 52.0),
                float_slider("thickness", "Thickness", 0.0, 100.0, 38.0),
            ]
        }),
    ]
}

fn flash_burn(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let strength = p.f("flash_strength").clamp(0.0, 100.0) / 100.0;
    let radius = p.f("flash_radius").clamp(20.0, 100.0) / 100.0;
    let edge_burn = p.f("edge_burn").clamp(0.0, 100.0) / 100.0;
    let warmth = p.f("warmth").clamp(0.0, 100.0);
    let grain = p.f("grain").clamp(0.0, 100.0) / 100.0;
    if strength <= 0.0001 && edge_burn <= 0.0001 && grain <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let right = 1.max(w - 1) as f32;
    let bottom = 1.max(h - 1) as f32;
    let cx = (p.f("center_x").clamp(0.0, 100.0) / 100.0) * right;
    let cy = (p.f("center_y").clamp(0.0, 100.0) / 100.0) * bottom;
    let (iw, ih) = (1.0 / right, 1.0 / bottom);
    let sigma = 0.06 + radius * 0.38;
    let sigma_inv = 1.0 / 0.0001f32.max(2.0 * sigma * sigma);
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let (mut r, mut g, mut b) = (s.r as f32, s.g as f32, s.b as f32);
            let dx = (x as f32 - cx) * iw;
            let dy = (y as f32 - cy) * ih;
            let dist = (dx * dx + dy * dy).sqrt();
            let flash = strength * (-(dist * dist) * sigma_inv).exp();
            if flash > 0.0001 {
                let sh = flash.powf(0.72);
                r = lerp(r, 255.0, sh);
                g = lerp(g, 242.0 + warmth * 0.10, sh * 0.94);
                b = lerp(b, 218.0 - warmth * 0.08, sh * 0.82);
            }
            if edge_burn > 0.0001 {
                let ring = smooth_step(0.54, 1.05, dist);
                if ring > 0.0001 {
                    let na = hash01((x as f32 * 0.12) as i32, (y as f32 * 0.12) as i32, 611);
                    let nb = hash01((x as f32 * 0.032) as i32, (y as f32 * 0.032) as i32, 1201);
                    let burn = ring * edge_burn * (0.68 + 0.32 * (na * 0.6 + nb * 0.4));
                    r = lerp(r, 220.0 + warmth * 0.22, burn * 0.44);
                    g = lerp(g, 98.0 + warmth * 0.18, burn * 0.38);
                    b = lerp(b, 36.0 + warmth * 0.12, burn * 0.30);
                    let dark = burn * 0.45;
                    r *= 1.0 - dark * 0.60;
                    g *= 1.0 - dark * 0.68;
                    b *= 1.0 - dark * 0.78;
                }
            }
            if grain > 0.0001 {
                let n = (hash01(x, y, 1703) * 2.0 - 1.0) * grain * 18.0;
                r += n * 1.05;
                g += n;
                b += n * 0.9;
            }
            px.push(C::new(
                clamp_to_byte(r),
                clamp_to_byte(g),
                clamp_to_byte(b),
                clamp_to_byte(s.a as f32),
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn ornament(canvas: &mut Canvas, paint: &mut Paint, cx: f32, cy: f32, size: f32) {
    let mut pb = PathBuilder::new();
    pb.move_to(cx, cy - size);
    pb.line_to(cx + size * 0.55, cy);
    pb.line_to(cx, cy + size);
    pb.line_to(cx - size * 0.55, cy);
    pb.close();
    if let Some(path) = pb.finish() {
        canvas.draw_path(&path, paint);
    }
    paint.style = Style::Fill;
    canvas.draw_circle(cx, cy, size * 0.18, paint);
}

fn scrollwork(canvas: &mut Canvas, paint: &Paint, x: f32, y: f32, size: f32, corner: i32) {
    let sx = if corner == 0 || corner == 2 {
        1.0
    } else {
        -1.0
    };
    let sy = if corner == 0 || corner == 1 {
        1.0
    } else {
        -1.0
    };
    let mut pb = PathBuilder::new();
    pb.move_to(x, y);
    pb.cubic_to(
        x + sx * size * 0.4,
        y,
        x + sx * size * 0.7,
        y + sy * size * 0.2,
        x + sx * size * 0.8,
        y + sy * size * 0.5,
    );
    pb.cubic_to(
        x + sx * size * 0.85,
        y + sy * size * 0.7,
        x + sx * size * 0.65,
        y + sy * size * 0.85,
        x + sx * size * 0.45,
        y + sy * size * 0.75,
    );
    if let Some(path) = pb.finish() {
        canvas.draw_path(&path, paint);
    }
    let mut pb = PathBuilder::new();
    pb.move_to(x, y);
    pb.cubic_to(
        x,
        y + sy * size * 0.4,
        x + sx * size * 0.2,
        y + sy * size * 0.7,
        x + sx * size * 0.5,
        y + sy * size * 0.8,
    );
    pb.cubic_to(
        x + sx * size * 0.7,
        y + sy * size * 0.85,
        x + sx * size * 0.85,
        y + sy * size * 0.65,
        x + sx * size * 0.75,
        y + sy * size * 0.45,
    );
    if let Some(path) = pb.finish() {
        canvas.draw_path(&path, paint);
    }
}

fn ornate_scroll(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let border = p.i("border_size").clamp(10, 200);
    let lt = p.f("line_thickness").clamp(0.5, 6.0);
    let (nw, nh) = (src.w + border * 2, src.h + border * 2);
    let mut canvas = Canvas::new(nw, nh);
    let (w, h, b) = (nw as f32, nh as f32, border as f32);
    if p.b("shadow") {
        let paint = Paint::fill_aa(C::new(0, 0, 0, 40)).with_mask_blur(border as f32 * 0.18);
        canvas.draw_rect_xywh(
            (border + 3) as f32,
            (border + 4) as f32,
            src.w as f32,
            src.h as f32,
            &paint,
        );
    }
    if p.b("fill_background") {
        canvas.draw_rect_xywh(0.0, 0.0, w, h, &Paint::fill_aa(p.c("frame_color")));
    }
    canvas.draw_bitmap(src, b, b, &Paint::default());
    let inner = Paint::fill_aa(C::BLACK).with_shader(Shader::Linear {
        p0: (b, b),
        p1: (b + b * 0.15, b + b * 0.15),
        colors: vec![C::new(0, 0, 0, 45), C::TRANSPARENT],
        pos: None,
        tile: Tile::Clamp,
    });
    canvas.draw_rect_xywh(b, b, src.w as f32, src.h as f32, &inner);
    let mut paint = Paint::stroke_aa(p.c("ornament_color"), lt)
        .with_cap(Cap::Round)
        .with_join(Join::Round);
    canvas.draw_rect_xywh(lt / 2.0, lt / 2.0, w - lt, h - lt, &paint);
    let inset = b * 0.30;
    canvas.draw_rect_xywh(inset, inset, w - inset * 2.0, h - inset * 2.0, &paint);
    let inset2 = b * 0.70;
    canvas.draw_rect_xywh(inset2, inset2, w - inset2 * 2.0, h - inset2 * 2.0, &paint);
    paint.stroke_width = lt * 0.5;
    let mid = b * 0.50;
    canvas.draw_rect_xywh(mid, mid, w - mid * 2.0, h - mid * 2.0, &paint);
    paint.stroke_width = lt;
    paint.style = Style::Fill;
    let orn = (b * 0.35).min(18.0);
    for (x, y) in [
        (inset, inset),
        (w - inset, inset),
        (inset, h - inset),
        (w - inset, h - inset),
    ] {
        ornament(&mut canvas, &mut paint, x, y, orn);
    }
    let small = orn * 0.65;
    for (x, y) in [
        (w * 0.5, inset),
        (w * 0.5, h - inset),
        (inset, h * 0.5),
        (w - inset, h * 0.5),
    ] {
        ornament(&mut canvas, &mut paint, x, y, small);
    }
    paint.style = Style::Stroke;
    paint.stroke_width = lt * 0.9;
    let ss = b * 0.40;
    let e = lt * 0.5;
    scrollwork(&mut canvas, &paint, e, e, ss, 0);
    scrollwork(&mut canvas, &paint, w - e, e, ss, 1);
    scrollwork(&mut canvas, &paint, e, h - e, ss, 2);
    scrollwork(&mut canvas, &paint, w - e, h - e, ss, 3);
    Ok(canvas.into_bitmap())
}

fn outline(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let size = p.i("size");
    if size <= 0 {
        return Ok(src.clone());
    }
    let padding = p.i("padding");
    let te = size + padding;
    let (nw, nh) = (src.w + te * 2, src.h + te * 2);
    // The source on a transparent canvas of the output's size, premultiplied: what the
    // dilate filter reads (its bounds grow by the radius).
    let mut padded = vec![C::ZERO; (nw * nh).max(0) as usize];
    for y in 0..src.h {
        for x in 0..src.w {
            padded[((y + te) * nw + x + te) as usize] = premul(src.get(x, y));
        }
    }
    let raw = |v: &[C]| -> Vec<[u8; 4]> { v.iter().map(|c| [c.r, c.g, c.b, c.a]).collect() };
    let mut canvas = Canvas::new(nw, nh);
    let dilated = morphology_premul(&padded, nw, nh, te, te, true);
    let tint = ColorFilter::Blend(p.c("color"), Blend::SrcIn);
    canvas.composite(&raw(&dilated), Blend::SrcOver, Some(&tint), 1.0);
    if padding > 0 {
        let gap = morphology_premul(&padded, nw, nh, padding, padding, true);
        canvas.composite(&raw(&gap), Blend::DstOut, None, 1.0);
    }
    let t = te as f32;
    if p.b("outline_only") {
        canvas.draw_bitmap(src, t, t, &Paint::default().with_blend(Blend::DstOut));
    } else {
        canvas.draw_bitmap(src, t, t, &Paint::default());
    }
    Ok(canvas.into_bitmap())
}

fn palette_map(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let levels = p.i("levels").clamp(2, 32);
    let step = 255.0 / (levels - 1) as f32;
    let q = |v: u8| -> u8 {
        let i = (round(v as f32 / step) as i32).clamp(0, levels - 1);
        ((i as f32 * step) as i32).clamp(0, 255) as u8
    };
    let px = src
        .px
        .iter()
        .map(|c| C::new(q(c.r), q(c.g), q(c.b), c.a))
        .collect();
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}

fn paper_stencil(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let t01 = p.f("threshold").clamp(0.0, 255.0) / 255.0;
    let feather = p.f("feather_radius").clamp(0.0, 200.0).clamp(0.0, 30.0) / 30.0;
    let range = 0.02 + 0.13 * feather;
    let es = p.f("edge_strength").clamp(0.0, 100.0) / 100.0;
    let bg_dim = p.f("background_dim");
    if es <= 0.0 && bg_dim <= 0.0 {
        return Ok(src.clone());
    }
    let dim = clamp01(1.0 - bg_dim.clamp(0.0, 100.0) / 100.0);
    let sc = p.c("stencil_color");
    let cap = (sc.a as f32 / 255.0) * es;
    let (sr, sg, sb) = (
        sc.r as f32 / 255.0,
        sc.g as f32 / 255.0,
        sc.b as f32 / 255.0,
    );
    let invert = p.b("invert_mask");
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let seed = 1337;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let lum = (0.2126 * s.r as f32 + 0.7152 * s.g as f32 + 0.0722 * s.b as f32) / 255.0;
            let jitter = (hash01(x, y, seed ^ 0xCAFE) * 2.0 - 1.0) * 0.003;
            let mut mask = smooth_step(t01 - range + jitter, t01 + range + jitter, lum);
            if invert {
                mask = 1.0 - mask;
            }
            let mix = clamp01(mask * cap);
            let or = lerp(s.r as f32 / 255.0 * dim, sr, mix);
            let og = lerp(s.g as f32 / 255.0 * dim, sg, mix);
            let ob = lerp(s.b as f32 / 255.0 * dim, sb, mix);
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

fn box_blur_f(source: &[f32], w: i32, h: i32, radius: i32) -> Vec<f32> {
    let window = (radius * 2 + 1) as f32;
    let (right, bottom) = (w - 1, h - 1);
    let mut hz = vec![0f32; source.len()];
    let mut vt = vec![0f32; source.len()];
    for y in 0..h {
        let row = (y * w) as usize;
        let mut acc = 0f32;
        for k in -radius..=radius {
            acc += source[row + k.clamp(0, right) as usize];
        }
        for x in 0..w {
            hz[row + x as usize] = acc / window;
            let rm = (x - radius).clamp(0, right) as usize;
            let ad = (x + radius + 1).clamp(0, right) as usize;
            acc += source[row + ad] - source[row + rm];
        }
    }
    for x in 0..w {
        let mut acc = 0f32;
        for k in -radius..=radius {
            acc += hz[(k.clamp(0, bottom) * w + x) as usize];
        }
        for y in 0..h {
            vt[(y * w + x) as usize] = acc / window;
            let rm = (y - radius).clamp(0, bottom);
            let ad = (y + radius + 1).clamp(0, bottom);
            acc += hz[(ad * w + x) as usize] - hz[(rm * w + x) as usize];
        }
    }
    vt
}

fn pencil_sketch(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let radius = p.i("blur_radius").clamp(1, 24);
    let es = p.f("edge_strength").clamp(0.0, 160.0) / 100.0;
    let darkness = p.f("pencil_darkness").clamp(0.0, 100.0) / 100.0;
    let paper = p.f("paper_brightness").clamp(40.0, 130.0) / 100.0;
    let gray: Vec<f32> = src
        .px
        .iter()
        .map(|c| (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0)
        .collect();
    let inverse: Vec<f32> = gray.iter().map(|g| 1.0 - g).collect();
    let blurred = box_blur_f(&inverse, w, h, radius);
    let mut edges = vec![0f32; gray.len()];
    for y in 0..h {
        let y0 = if y > 0 { y - 1 } else { 0 };
        let y2 = if y < h - 1 { y + 1 } else { h - 1 };
        for x in 0..w {
            let x0 = if x > 0 { x - 1 } else { 0 };
            let x2 = if x < w - 1 { x + 1 } else { w - 1 };
            let g = |xx: i32, yy: i32| gray[(yy * w + xx) as usize];
            let (tl, tc, tr) = (g(x0, y0), g(x, y0), g(x2, y0));
            let (ml, mr) = (g(x0, y), g(x2, y));
            let (bl, bc, br) = (g(x0, y2), g(x, y2), g(x2, y2));
            let gx = (-tl + tr) + (-2.0 * ml + 2.0 * mr) + (-bl + br);
            let gy = (-tl - 2.0 * tc - tr) + (bl + 2.0 * bc + br);
            edges[(y * w + x) as usize] = ((gx * gx + gy * gy).sqrt() / 4.0).clamp(0.0, 1.0);
        }
    }
    let gamma = 1.0 + darkness * 1.75;
    let px = (0..gray.len())
        .map(|i| {
            let dodge = gray[i] / (1.0 - blurred[i]).max(0.03);
            let sketch = dodge.clamp(0.0, 1.0);
            let mut v = (sketch - edges[i] * es * 0.85).clamp(0.0, 1.0);
            v = v.powf(gamma);
            v = lerp(v, 1.0, (paper - 1.0).clamp(0.0, 0.6));
            let b = clamp_to_byte(v * 255.0);
            C::new(b, b, b, src.px[i].a)
        })
        .collect();
    Ok(Bitmap::from_pixels(w, h, px))
}

const PERM: [u8; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69,
    142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219,
    203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
    74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230,
    220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76,
    132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173,
    186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212, 207, 206,
    59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163,
    70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232,
    178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162,
    241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84, 204,
    176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141,
    128, 195, 78, 66, 215, 61, 156, 180,
];

fn perlin2(x: f32, y: f32) -> f32 {
    let xi = (x.floor() as i32 & 255) as usize;
    let yi = (y.floor() as i32 & 255) as usize;
    let xf = x - x.floor();
    let yf = y - y.floor();
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v) = (fade(xf), fade(yf));
    let pm = |i: usize| PERM[i & 255] as usize;
    let aa = pm(pm(xi) + yi);
    let ab = pm(pm(xi) + yi + 1);
    let ba = pm(pm((xi + 1) & 255) + yi);
    let bb = pm(pm((xi + 1) & 255) + yi + 1);
    let grad = |hash: usize, x: f32, y: f32| match hash & 3 {
        0 => x + y,
        1 => -x + y,
        2 => x - y,
        _ => -x - y,
    };
    let g00 = grad(aa, xf, yf);
    let g10 = grad(ba, xf - 1.0, yf);
    let g01 = grad(ab, xf, yf - 1.0);
    let g11 = grad(bb, xf - 1.0, yf - 1.0);
    lerp(lerp(g00, g10, u), lerp(g01, g11, u), v)
}

fn fractal_perlin(x: f32, y: f32, octaves: i32, persistence: f32, seed: i32) -> f32 {
    let (mut value, mut amp, mut freq, mut max) = (0f32, 1f32, 1f32, 0f32);
    for i in 0..octaves {
        let ox = seed as f32 * 137.3 + i as f32 * 47.7;
        let oy = seed as f32 * 251.1 + i as f32 * 89.3;
        value += perlin2(x * freq + ox, y * freq + oy) * amp;
        max += amp;
        freq *= 2.0;
        amp *= persistence;
    }
    if max > 0.0 { value / max } else { 0.0 }
}

fn perlin(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let strength = p.f("strength").clamp(0.0, 100.0) / 100.0;
    if strength <= 0.0 {
        return Ok(src.clone());
    }
    let scale = p.f("scale").max(1.0);
    let octaves = p.i("octaves").clamp(1, 8);
    let persistence = p.f("persistence").clamp(0.0, 1.0);
    let mono = p.b("monochrome");
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let inv = 1.0 / scale;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        for x in 0..w {
            let s = src.get(x, y);
            let (fx, fy) = (x as f32 * inv, y as f32 * inv);
            let (nr, ng, nb) = if mono {
                let n = (fractal_perlin(fx, fy, octaves, persistence, 0) + 1.0) * 0.5;
                (n, n, n)
            } else {
                (
                    (fractal_perlin(fx, fy, octaves, persistence, 0) + 1.0) * 0.5,
                    (fractal_perlin(fx, fy, octaves, persistence, 1) + 1.0) * 0.5,
                    (fractal_perlin(fx, fy, octaves, persistence, 2) + 1.0) * 0.5,
                )
            };
            px.push(C::new(
                clamp_to_byte(lerp(s.r as f32 / 255.0, nr, strength) * 255.0),
                clamp_to_byte(lerp(s.g as f32 / 255.0, ng, strength) * 255.0),
                clamp_to_byte(lerp(s.b as f32 / 255.0, nb, strength) * 255.0),
                s.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn sort_hue(c: C) -> f32 {
    let (r, g, b) = (c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0);
    let max = r.max(g.max(b));
    let min = r.min(g.min(b));
    let delta = max - min;
    if delta <= 0.0001 {
        return 0.0;
    }
    let mut hue = if max == r {
        ((g - b) / delta) % 6.0
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    hue /= 6.0;
    if hue < 0.0 {
        hue += 1.0;
    }
    hue
}

fn pixel_sort(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 {
        return Ok(src.clone());
    }
    let mut low = p.f("threshold_low").clamp(0.0, 100.0) / 100.0;
    let mut high = p.f("threshold_high").clamp(0.0, 100.0) / 100.0;
    if high < low {
        std::mem::swap(&mut low, &mut high);
    }
    let min_span = p.i("min_span_length").clamp(2, 256);
    let max_span = p.i("max_span_length").clamp(min_span, 512);
    let prob = p.f("sort_probability").clamp(0.0, 100.0) / 100.0;
    let hue = p.e("metric") == "Hue";
    let metric = move |c: C| -> f32 {
        if hue {
            sort_hue(c)
        } else {
            (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0
        }
    };
    let seed = 3110;
    let mut px = src.px.clone();
    let horizontal = p.e("direction") == "Horizontal";
    // (outer, inner) loops: rows then x for horizontal, columns then y for vertical.
    let (outer, inner) = if horizontal { (h, w) } else { (w, h) };
    let index = |o: i32, i: i32| -> usize {
        if horizontal {
            (o * w + i) as usize
        } else {
            (i * w + o) as usize
        }
    };
    let cmp = |a: &C, b: &C| {
        metric(*a)
            .partial_cmp(&metric(*b))
            .unwrap_or(std::cmp::Ordering::Equal)
    };
    let mut segment = Vec::new();
    for o in 0..outer {
        let mut i = 0;
        while i < inner {
            let m = metric(px[index(o, i)]);
            if m < low || m > high {
                i += 1;
                continue;
            }
            let start = i;
            i += 1;
            while i < inner {
                let m = metric(px[index(o, i)]);
                if m < low || m > high {
                    break;
                }
                i += 1;
            }
            if i - start < min_span {
                continue;
            }
            let mut cursor = start;
            while cursor < i {
                let len = (i - cursor).min(max_span);
                if len < min_span {
                    break;
                }
                let selector = if horizontal {
                    hash01(cursor, o, seed ^ 1187)
                } else {
                    hash01(o, cursor, seed ^ 7723)
                };
                if selector <= prob {
                    segment.clear();
                    segment.extend((0..len).map(|k| px[index(o, cursor + k)]));
                    sort_by(&mut segment, &cmp);
                    for (k, c) in segment.iter().enumerate() {
                        px[index(o, cursor + k as i32)] = *c;
                    }
                }
                cursor += len;
            }
        }
    }
    Ok(Bitmap::from_pixels(w, h, px))
}

fn pixelate(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let size = p.i("size");
    if size <= 1 {
        return Ok(src.clone());
    }
    let sw = 1.max(src.w / size);
    let sh = 1.max(src.h / size);
    let mut small = Canvas::new(sw, sh);
    small.draw_bitmap_to(
        src,
        rect(0.0, 0.0, sw as f32, sh as f32),
        Sampling::Nearest,
        &Paint::default(),
    );
    let small = small.into_bitmap();
    let mut canvas = Canvas::new(src.w, src.h);
    canvas.draw_bitmap_to(
        &small,
        rect(0.0, 0.0, src.w as f32, src.h as f32),
        Sampling::Nearest,
        &Paint::default(),
    );
    Ok(canvas.into_bitmap())
}

fn plasma_arcs(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let energy = p.f("energy").clamp(0.0, 100.0) / 100.0;
    let density = p.f("arc_density").clamp(0.0, 100.0) / 100.0;
    let glow = p.f("glow").clamp(0.0, 100.0) / 100.0;
    let turbulence = p.f("turbulence").clamp(0.0, 100.0) / 100.0;
    let thickness = p.f("thickness").clamp(0.0, 100.0) / 100.0;
    if energy <= 0.0001 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w, src.h);
    let iw = 1.0 / 1.max(w - 1) as f32;
    let ih = 1.0 / 1.max(h - 1) as f32;
    let lw = 0.020 + (1.0 - thickness) * 0.10;
    let df = 16.0 + density * 18.0;
    let tf = 2.0 + turbulence * 6.0;
    let l01 = |c: C| (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0;
    let scr = |s: f32, o: f32| 1.0 - (1.0 - s) * (1.0 - clamp01(o));
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..h {
        let v = y as f32 * ih;
        for x in 0..w {
            let s = src.get(x, y);
            let (sr, sg, sb) = (s.r as f32 / 255.0, s.g as f32 / 255.0, s.b as f32 / 255.0);
            let u = x as f32 * iw;
            let fa = (u * df + (v * tf * 2.4 + 0.7).sin() * (1.8 + turbulence * 4.5) + 0.8).sin();
            let fb =
                (v * (df * 0.92) - (u * tf * 1.6 - 1.1).sin() * (1.4 + turbulence * 4.2) - 1.3)
                    .cos();
            let fc = ((u + v) * (10.0 + density * 12.0) + 2.1).sin();
            let core_f = fa * 0.46 + fb * 0.34 + fc * 0.20;
            let branch_f = fa * 0.28 - fb * 0.44 + fc * 0.28;
            let core = 1.0 - smooth_step(lw, lw + 0.08, core_f.abs());
            let branch = 1.0 - smooth_step(lw * 0.70, lw * 0.70 + 0.09, branch_f.abs());
            let mut arc = core.max(branch * 0.72);
            let edge = clamp01(
                ((l01(src.get((w - 1).min(x + 1), y)) - l01(src.get(0.max(x - 1), y))).abs()
                    + (l01(src.get(x, (h - 1).min(y + 1))) - l01(src.get(x, 0.max(y - 1)))).abs())
                    * 1.1,
            );
            arc *= 0.72 + edge * 0.28;
            let gm = arc.max(0.0).powf(0.34) * glow;
            let cm = arc * energy;
            let mut or = scr(sr, cm * 0.20 + gm * 0.10);
            let mut og = scr(sg, cm * 0.70 + gm * 0.26);
            let mut ob = scr(sb, cm * 1.00 + gm * 0.34);
            or = lerp(or, 1.0, cm * 0.18);
            og = lerp(og, 1.0, cm * 0.22);
            ob = lerp(ob, 1.0, cm * 0.28);
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
