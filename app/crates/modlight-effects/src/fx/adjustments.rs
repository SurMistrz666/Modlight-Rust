//! `Core/ImageEffects/Adjustments` (32 effects).

use modlight_core::icons as l;
use modlight_editor::effects::Category;

use crate::def::*;
use crate::helpers::{self as h, clamp01, hash01, lerp};
use crate::sk::filters::{color_matrix, color_table};
use crate::sk::{
    Bitmap, C, Canvas, ColorFilter, Paint, Shader, Tile, byte, clamp_to_byte, pixel_op, rect, round,
};

const A: Category = Category::Adjustments;

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "alpha",
            "Alpha",
            A,
            l::DROPLET,
            "Adjusts the alpha transparency.",
            alpha,
        )
        .params(|| vec![float_slider("amount", "Alpha", 0.0, 100.0, 100.0)]),
        Def::new(
            "auto_contrast",
            "Auto contrast",
            A,
            l::WAND_SPARKLES,
            "Automatically adjusts the contrast.",
            auto_contrast,
        )
        .params(|| {
            vec![fine(
                float_slider("clip_percent", "Clip percent", 0.0, 20.0, 0.5),
                0.1,
                "{}{0:0.0}",
            )]
        }),
        Def::new(
            "black_and_white",
            "Black & White",
            A,
            l::CONTRAST,
            "Converts the image to black and white.",
            black_and_white,
        )
        .immediate(),
        Def::new(
            "brightness",
            "Brightness",
            A,
            l::SUN_MEDIUM,
            "Adjusts image brightness.",
            brightness,
        )
        .params(|| vec![float_slider("amount", "Amount", -100.0, 100.0, 0.0)]),
        Def::new(
            "channel_swap",
            "Channel swap",
            A,
            l::SHUFFLE,
            "Swaps color channels for creative color effects.",
            channel_swap,
        )
        .params(|| {
            vec![choice(
                "mode",
                "Swap mode",
                &[
                    ("Red \u{2194} Green", "RedGreen"),
                    ("Red \u{2194} Blue", "RedBlue"),
                    ("Green \u{2194} Blue", "GreenBlue"),
                    ("Rotate R\u{2192}G\u{2192}B", "RotateRGB"),
                    ("Rotate B\u{2192}G\u{2192}R", "RotateBGR"),
                ],
                "RedGreen",
            )]
        }),
        Def::new(
            "clarity",
            "Clarity",
            A,
            l::EYE,
            "Enhances midtone contrast for added depth and punch.",
            clarity,
        )
        .params(|| vec![float_slider("amount", "Amount", -100.0, 100.0, 0.0)]),
        Def::new(
            "color_balance",
            "Color Balance",
            A,
            l::SLIDERS_HORIZONTAL,
            "Adjusts color balance in shadows, midtones, and highlights.",
            color_balance,
        )
        .params(|| {
            vec![
                float_slider("shadow_cyan_red", "Shadows: Cyan-Red", -100.0, 100.0, 0.0),
                float_slider(
                    "shadow_magenta_green",
                    "Shadows: Magenta-Green",
                    -100.0,
                    100.0,
                    0.0,
                ),
                float_slider(
                    "shadow_yellow_blue",
                    "Shadows: Yellow-Blue",
                    -100.0,
                    100.0,
                    0.0,
                ),
                float_slider("mid_cyan_red", "Midtones: Cyan-Red", -100.0, 100.0, 0.0),
                float_slider(
                    "mid_magenta_green",
                    "Midtones: Magenta-Green",
                    -100.0,
                    100.0,
                    0.0,
                ),
                float_slider(
                    "mid_yellow_blue",
                    "Midtones: Yellow-Blue",
                    -100.0,
                    100.0,
                    0.0,
                ),
                float_slider(
                    "highlight_cyan_red",
                    "Highlights: Cyan-Red",
                    -100.0,
                    100.0,
                    0.0,
                ),
                float_slider(
                    "highlight_magenta_green",
                    "Highlights: Magenta-Green",
                    -100.0,
                    100.0,
                    0.0,
                ),
                float_slider(
                    "highlight_yellow_blue",
                    "Highlights: Yellow-Blue",
                    -100.0,
                    100.0,
                    0.0,
                ),
            ]
        }),
        Def::new(
            "color_matrix",
            "Color matrix",
            A,
            l::GRID_3X3,
            "Applies a color matrix transformation.",
            color_matrix_fx,
        )
        .params(|| {
            let n = |k: &str, label: &str, d: f64| double_numeric(k, label, -5.0, 5.0, d).incr(0.1);
            let o = |k: &str, label: &str| {
                double_numeric(k, label, -255.0, 255.0, 0.0)
                    .incr(1.0)
                    .fmt("0")
            };
            vec![
                n("rr", "Rr", 1.0),
                n("rg", "Rg", 0.0),
                n("rb", "Rb", 0.0),
                n("ra", "Ra", 0.0),
                o("ro", "Ro"),
                n("gr", "Gr", 0.0),
                n("gg", "Gg", 1.0),
                n("gb", "Gb", 0.0),
                n("ga", "Ga", 0.0),
                o("go", "Go"),
                n("br", "Br", 0.0),
                n("bg", "Bg", 0.0),
                n("bb", "Bb", 1.0),
                n("ba", "Ba", 0.0),
                o("bo", "Bo"),
                n("ar", "Ar", 0.0),
                n("ag", "Ag", 0.0),
                n("ab", "Ab", 0.0),
                n("aa", "Aa", 1.0),
                o("ao", "Ao"),
            ]
        }),
        Def::new(
            "colorize",
            "Colorize",
            A,
            l::PALETTE,
            "Colorizes the image.",
            colorize,
        )
        .params(|| {
            vec![
                color("color", "Color", C::ORANGE),
                float_slider("strength", "Strength", 0.0, 100.0, 50.0),
            ]
        }),
        Def::new(
            "contrast",
            "Contrast",
            A,
            l::CONTRAST,
            "Adjusts image contrast.",
            contrast,
        )
        .params(|| vec![float_slider("amount", "Amount", -100.0, 100.0, 0.0)]),
        Def::new(
            "cross_process",
            "Cross process",
            A,
            l::CAMERA,
            "Simulates film cross-processing with shifted colors and high contrast.",
            cross_process,
        )
        .params(|| vec![float_slider("strength", "Strength", 0.0, 100.0, 100.0)]),
        Def::new(
            "dehaze",
            "Dehaze",
            A,
            l::SUNSET,
            "Removes or adds atmospheric haze.",
            dehaze,
        )
        .params(|| vec![float_slider("amount", "Amount", -100.0, 100.0, 50.0)]),
        Def::new(
            "duotone_gradient_map",
            "Duotone / Gradient map",
            A,
            l::BLEND,
            "Maps grayscale tones to a custom multi-color gradient.",
            duotone,
        )
        .params(|| {
            vec![
                color("shadow_color", "Shadow color", C::rgb(24, 28, 78)),
                color("midtone_color", "Midtone color", C::rgb(182, 60, 132)),
                color("highlight_color", "Highlight color", C::rgb(255, 224, 132)),
                float_slider("contrast", "Contrast", 50.0, 200.0, 110.0),
                fine(
                    float_slider("gamma", "Gamma", 0.1, 5.0, 1.0),
                    0.1,
                    "{}{0:0.0}",
                ),
                float_slider("blend", "Blend", 0.0, 100.0, 100.0),
            ]
        }),
        Def::new(
            "exposure",
            "Exposure",
            A,
            l::APERTURE,
            "Adjusts the exposure level.",
            exposure,
        )
        .params(|| vec![float_slider("amount", "Exposure", -100.0, 100.0, 0.0)]),
        Def::new(
            "film_emulation",
            "Film emulation",
            A,
            l::FILM,
            "Applies cinematic analog film looks with grain and fade.",
            film_emulation,
        )
        .params(|| {
            vec![
                choice(
                    "preset",
                    "Preset",
                    &[
                        ("Classic", "Classic"),
                        ("Warm", "Warm"),
                        ("Cool", "Cool"),
                        ("Faded", "Faded"),
                        ("Cross processed", "CrossProcessed"),
                    ],
                    "Classic",
                ),
                float_slider("tone_strength", "Tone strength", 0.0, 100.0, 65.0),
                float_slider("grain_amount", "Grain amount", 0.0, 100.0, 12.0),
                float_slider("fade_amount", "Fade amount", 0.0, 100.0, 10.0),
                float_slider("contrast_amount", "Contrast amount", 50.0, 150.0, 110.0),
            ]
        }),
        Def::new(
            "gamma",
            "Gamma",
            A,
            l::GAUGE,
            "Adjusts the gamma level.",
            gamma,
        )
        .params(|| {
            vec![fine(
                float_slider("amount", "Amount", 0.1, 5.0, 1.0),
                0.1,
                "{}{0:0.0}",
            )]
        }),
        Def::new(
            "grayscale",
            "Grayscale",
            A,
            l::ECLIPSE,
            "Converts the image to grayscale.",
            grayscale,
        )
        .params(|| vec![float_slider("strength", "Strength", 0.0, 100.0, 100.0)]),
        Def::new(
            "hue",
            "Hue",
            A,
            l::PIPETTE,
            "Adjusts the hue of the image.",
            hue,
        )
        .params(|| vec![float_slider("amount", "Amount", -180.0, 180.0, 0.0)]),
        Def::new(
            "invert",
            "Invert",
            A,
            l::REFRESH_CCW_DOT,
            "Inverts image colors.",
            invert,
        )
        .immediate(),
        Def::new(
            "levels",
            "Levels",
            A,
            l::SLIDERS_VERTICAL,
            "Adjusts image color levels.",
            levels,
        )
        .params(|| {
            vec![
                int_slider("input_black", "Input black", 0, 255, 0),
                int_slider("input_white", "Input white", 0, 255, 255),
                fine(
                    float_slider("gamma", "Gamma", 0.1, 5.0, 1.0),
                    0.1,
                    "{}{0:0.0}",
                ),
                int_slider("output_black", "Output black", 0, 255, 0),
                int_slider("output_white", "Output white", 0, 255, 255),
            ]
        }),
        Def::new(
            "lomo",
            "Lomo",
            A,
            l::APERTURE,
            "Applies a lomography look with high contrast, warm tones, and vignette.",
            lomo,
        )
        .params(|| vec![float_slider("strength", "Strength", 0.0, 100.0, 100.0)]),
        Def::new(
            "polaroid",
            "Polaroid",
            A,
            l::CAMERA,
            "Applies a Polaroid effect.",
            polaroid,
        )
        .immediate(),
        Def::new(
            "posterize",
            "Posterize",
            A,
            l::LAYERS_3,
            "Reduces the number of colors to create a poster-like effect.",
            posterize,
        )
        .params(|| vec![int_slider("levels", "Levels", 2, 64, 8)]),
        Def::new(
            "replace_color",
            "Replace Color",
            A,
            l::REPLACE,
            "Replaces a specific color.",
            replace_color,
        )
        .params(|| {
            vec![
                color("target_color", "Target color", C::WHITE),
                color("replace_color", "Replace color", C::BLACK),
                float_slider("tolerance", "Tolerance", 0.0, 255.0, 40.0),
            ]
        }),
        Def::new(
            "saturation",
            "Saturation",
            A,
            l::DROPLETS,
            "Adjusts the color saturation.",
            saturation,
        )
        .params(|| vec![float_slider("amount", "Amount", -100.0, 100.0, 0.0)]),
        Def::new(
            "sepia",
            "Sepia",
            A,
            l::COFFEE,
            "Applies a sepia tone effect.",
            sepia,
        )
        .params(|| vec![float_slider("strength", "Strength", 0.0, 100.0, 100.0)]),
        Def::new(
            "shadows_highlights",
            "Shadows / Highlights",
            A,
            l::SUN_MOON,
            "Adjusts shadows and highlights.",
            shadows_highlights,
        )
        .params(|| {
            vec![
                float_slider("shadows", "Shadows", -100.0, 100.0, 0.0),
                float_slider("highlights", "Highlights", -100.0, 100.0, 0.0),
            ]
        }),
        Def::new(
            "solarize",
            "Solarize",
            A,
            l::SUN,
            "Applies a solarize effect.",
            solarize,
        )
        .params(|| vec![int_slider("threshold", "Threshold", 0, 255, 128)]),
        Def::new(
            "split_toning",
            "Split Toning",
            A,
            l::BLEND,
            "Tints shadows and highlights with different colors.",
            split_toning,
        )
        .params(|| {
            vec![
                color("shadow_tint", "Shadow Tint", C::rgb(40, 50, 120)),
                color("highlight_tint", "Highlight Tint", C::rgb(200, 170, 100)),
                float_slider("strength", "Strength", 0.0, 100.0, 30.0),
                float_slider("balance", "Balance", -100.0, 100.0, 0.0),
            ]
        }),
        Def::new(
            "temperature_tint",
            "Temperature / Tint",
            A,
            l::THERMOMETER,
            "Adjusts the color temperature and tint.",
            temperature_tint,
        )
        .params(|| {
            vec![
                float_slider("temperature", "Temperature", -100.0, 100.0, 0.0),
                float_slider("tint", "Tint", -100.0, 100.0, 0.0),
            ]
        }),
        Def::new(
            "threshold",
            "Threshold",
            A,
            l::BINARY,
            "Applies a contrast threshold.",
            threshold,
        )
        .params(|| vec![int_slider("value", "Threshold", 0, 255, 128)]),
        Def::new(
            "vibrance",
            "Vibrance",
            A,
            l::SPARKLES,
            "Adjusts the color vibrance.",
            vibrance,
        )
        .params(|| vec![float_slider("amount", "Amount", -100.0, 100.0, 0.0)]),
    ]
}

/// The identity colour matrix.
pub const IDENTITY: [f32; 20] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    0.0,
];

/// `identity * (1 - s) + m * s`, as the strength-blended matrices are built.
pub fn mix_matrix(m: &[f32; 20], s: f32) -> [f32; 20] {
    let mut out = [0f32; 20];
    for i in 0..20 {
        out[i] = IDENTITY[i] * (1.0 - s) + m[i] * s;
    }
    out
}

fn alpha(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let a = p.f("amount") / 100.0;
    let mut m = IDENTITY;
    m[18] = a;
    Ok(color_matrix(src, &m))
}

fn auto_contrast(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let clip_percent = p.f("clip_percent").clamp(0.0, 20.0);
    let total = src.px.len();
    if total == 0 {
        return Ok(src.clone());
    }
    let clip = round(total as f32 * (clip_percent / 100.0)) as i32;
    let mut hr = [0i32; 256];
    let mut hg = [0i32; 256];
    let mut hb = [0i32; 256];
    for c in &src.px {
        hr[c.r as usize] += 1;
        hg[c.g as usize] += 1;
        hb[c.b as usize] += 1;
    }
    let range = |hist: &[i32; 256]| {
        let mut min = 0;
        let mut sum = 0;
        for (i, v) in hist.iter().enumerate() {
            sum += v;
            if sum > clip {
                min = i as i32;
                break;
            }
        }
        let mut max = 255;
        sum = 0;
        for i in (0..256).rev() {
            sum += hist[i];
            if sum > clip {
                max = i as i32;
                break;
            }
        }
        (min, max)
    };
    let (min_r, max_r) = range(&hr);
    let (min_g, max_g) = range(&hg);
    let (min_b, max_b) = range(&hb);
    if max_r <= min_r && max_g <= min_g && max_b <= min_b {
        return Ok(src.clone());
    }
    let stretch = |v: u8, min: i32, max: i32| -> u8 {
        if max <= min {
            return v;
        }
        let v32 = v as i32;
        if v32 <= min {
            return 0;
        }
        if v32 >= max {
            return 255;
        }
        round((v32 - min) as f32 * 255.0 / (max - min) as f32) as u8
    };
    let px = src
        .px
        .iter()
        .map(|c| {
            C::new(
                stretch(c.r, min_r, max_r),
                stretch(c.g, min_g, max_g),
                stretch(c.b, min_b, max_b),
                c.a,
            )
        })
        .collect();
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}

fn black_and_white(src: &Bitmap, _: &P) -> Result<Bitmap, String> {
    Ok(pixel_op(src, |c| {
        let lum = 0.2126f32 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32;
        if lum > 127.0 { C::WHITE } else { C::BLACK }
    }))
}

fn brightness(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let v = p.f("amount") / 100.0;
    let mut m = IDENTITY;
    m[4] = v;
    m[9] = v;
    m[14] = v;
    Ok(color_matrix(src, &m))
}

fn channel_swap(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let mode = p.e("mode");
    Ok(pixel_op(src, |c| match mode.as_str() {
        "RedGreen" => C::new(c.g, c.r, c.b, c.a),
        "RedBlue" => C::new(c.b, c.g, c.r, c.a),
        "GreenBlue" => C::new(c.r, c.b, c.g, c.a),
        "RotateRGB" => C::new(c.b, c.r, c.g, c.a),
        "RotateBGR" => C::new(c.g, c.b, c.r, c.a),
        _ => c,
    }))
}

fn clarity(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let amount = p.f("amount").clamp(-100.0, 100.0) / 100.0;
    if amount.abs() < 0.001 {
        return Ok(src.clone());
    }
    let radius = (src.w.max(src.h) as f32 * 0.02).max(3.0);
    let blurred = h::blur_bitmap(src, radius, radius);
    let px = src
        .px
        .iter()
        .zip(&blurred.px)
        .map(|(s, b)| {
            let (sr, sg, sb) = (s.r as f32 / 255.0, s.g as f32 / 255.0, s.b as f32 / 255.0);
            let (br, bg, bb) = (b.r as f32 / 255.0, b.g as f32 / 255.0, b.b as f32 / 255.0);
            let lum = 0.299 * sr + 0.587 * sg + 0.114 * sb;
            let diff = lum - 0.5;
            let mask = (-8.0 * diff * diff).exp();
            let strength = amount * mask * 1.5;
            let o = |s: f32, b: f32| round((s + (s - b) * strength).clamp(0.0, 1.0) * 255.0) as u8;
            C::new(o(sr, br), o(sg, bg), o(sb, bb), s.a)
        })
        .collect();
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}

fn color_balance(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let keys = [
        "shadow_cyan_red",
        "shadow_magenta_green",
        "shadow_yellow_blue",
        "mid_cyan_red",
        "mid_magenta_green",
        "mid_yellow_blue",
        "highlight_cyan_red",
        "highlight_magenta_green",
        "highlight_yellow_blue",
    ];
    let v: Vec<f32> = keys.iter().map(|k| p.f(k)).collect();
    if v.iter().all(|x| x.abs() < 0.01) {
        return Ok(src.clone());
    }
    let n: Vec<f32> = v.iter().map(|x| x / 100.0).collect();
    Ok(pixel_op(src, |c| {
        let mut r = c.r as f32 / 255.0;
        let mut g = c.g as f32 / 255.0;
        let mut b = c.b as f32 / 255.0;
        let lum = 0.299 * r + 0.587 * g + 0.114 * b;
        let sw = (1.0 - lum * 4.0).clamp(0.0, 1.0);
        let hw = ((lum - 0.75) * 4.0).clamp(0.0, 1.0);
        let mw = (1.0 - sw - hw).max(0.0);
        let rs = (n[0] * sw + n[3] * mw + n[6] * hw) * 0.5;
        let gs = (n[1] * sw + n[4] * mw + n[7] * hw) * 0.5;
        let bs = (n[2] * sw + n[5] * mw + n[8] * hw) * 0.5;
        r = (r + rs).clamp(0.0, 1.0);
        g = (g + gs).clamp(0.0, 1.0);
        b = (b + bs).clamp(0.0, 1.0);
        C::new(
            round(r * 255.0) as u8,
            round(g * 255.0) as u8,
            round(b * 255.0) as u8,
            c.a,
        )
    }))
}

fn color_matrix_fx(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let keys = [
        "rr", "rg", "rb", "ra", "ro", "gr", "gg", "gb", "ga", "go", "br", "bg", "bb", "ba", "bo",
        "ar", "ag", "ab", "aa", "ao",
    ];
    let mut m = [0f32; 20];
    for (i, k) in keys.iter().enumerate() {
        m[i] = p.d(k) as f32;
    }
    Ok(color_matrix(src, &m))
}

pub const GRAYSCALE_MATRIX: [f32; 20] = [
    0.2126, 0.7152, 0.0722, 0.0, 0.0, 0.2126, 0.7152, 0.0722, 0.0, 0.0, 0.2126, 0.7152, 0.0722,
    0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
];

fn colorize(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let mut strength = p.f("strength");
    if strength <= 0.0 {
        return Ok(src.clone());
    }
    if strength > 100.0 {
        strength = 100.0;
    }
    let filter = ColorFilter::Compose(
        Box::new(ColorFilter::Blend(p.c("color"), crate::sk::Blend::Modulate)),
        Box::new(ColorFilter::Matrix(GRAYSCALE_MATRIX)),
    );
    let mut canvas = Canvas::new(src.w, src.h);
    if strength >= 100.0 {
        let paint = Paint {
            color_filter: Some(filter),
            ..Paint::default()
        };
        canvas.draw_bitmap(src, 0.0, 0.0, &paint);
    } else {
        canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
        let paint = Paint {
            color: C::new(255, 255, 255, byte(255.0 * (strength / 100.0))),
            color_filter: Some(filter),
            ..Paint::default()
        };
        canvas.draw_bitmap(src, 0.0, 0.0, &paint);
    }
    Ok(canvas.into_bitmap())
}

fn contrast(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let mut scale = (100.0 + p.f("amount")) / 100.0;
    scale *= scale;
    let shift = 0.5 * (1.0 - scale);
    let m = [
        scale, 0.0, 0.0, 0.0, shift, 0.0, scale, 0.0, 0.0, shift, 0.0, 0.0, scale, 0.0, shift, 0.0,
        0.0, 0.0, 1.0, 0.0,
    ];
    Ok(color_matrix(src, &m))
}

fn cross_process(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let s = (p.f("strength") / 100.0).clamp(0.0, 1.0);
    if s <= 0.0 {
        return Ok(src.clone());
    }
    let m = [
        1.2, 0.1, -0.1, 0.0, 0.05, -0.1, 1.3, 0.0, 0.0, 0.02, -0.1, 0.0, 0.8, 0.0, 0.1, 0.0, 0.0,
        0.0, 1.0, 0.0,
    ];
    Ok(color_matrix(src, &mix_matrix(&m, s)))
}

fn dehaze(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let amount = p.f("amount").clamp(-100.0, 100.0) / 100.0;
    if amount.abs() < 0.001 {
        return Ok(src.clone());
    }
    let count = src.px.len();
    let top_n = 1.max(count / 1000);
    let brightness: Vec<f32> = src
        .px
        .iter()
        .map(|c| (c.r as f32 + c.g as f32 + c.b as f32) / 3.0)
        .collect();
    let mut sorted = brightness.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let threshold = sorted[count.saturating_sub(top_n)];
    let (mut sr, mut sg, mut sb, mut n) = (0f32, 0f32, 0f32, 0);
    for (i, c) in src.px.iter().enumerate() {
        if brightness[i] >= threshold {
            sr += c.r as f32;
            sg += c.g as f32;
            sb += c.b as f32;
            n += 1;
        }
    }
    let (ar, ag, ab) = if n > 0 {
        (sr / n as f32, sg / n as f32, sb / n as f32)
    } else {
        (200.0, 200.0, 200.0)
    };
    let px = src
        .px
        .iter()
        .map(|c| {
            let (mut r, mut g, mut b) = (c.r as f32, c.g as f32, c.b as f32);
            if amount > 0.0 {
                let dark = (r / ar).min((g / ag).min(b / ab));
                let t = (1.0 - amount * dark).max(0.1);
                r = (r - ar * (1.0 - t)) / t;
                g = (g - ag * (1.0 - t)) / t;
                b = (b - ab * (1.0 - t)) / t;
            } else {
                let hz = -amount;
                r += (ar - r) * hz;
                g += (ag - g) * hz;
                b += (ab - b) * hz;
            }
            C::new(clamp_to_byte(r), clamp_to_byte(g), clamp_to_byte(b), c.a)
        })
        .collect();
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}

fn lerp_c(a: C, b: C, t: f32) -> C {
    C::new(
        clamp_to_byte(lerp(a.r as f32, b.r as f32, t)),
        clamp_to_byte(lerp(a.g as f32, b.g as f32, t)),
        clamp_to_byte(lerp(a.b as f32, b.b as f32, t)),
        255,
    )
}

fn duotone(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let contrast = p.f("contrast").clamp(50.0, 200.0) / 100.0;
    let gamma = p.f("gamma").clamp(0.3, 3.0);
    let blend = p.f("blend").clamp(0.0, 100.0) / 100.0;
    if src.is_empty() {
        return Ok(src.clone());
    }
    let (sh, mi, hi) = (
        p.c("shadow_color"),
        p.c("midtone_color"),
        p.c("highlight_color"),
    );
    let px = src
        .px
        .iter()
        .map(|s| {
            let mut lum = (0.2126 * s.r as f32 + 0.7152 * s.g as f32 + 0.0722 * s.b as f32) / 255.0;
            lum = ((lum - 0.5) * contrast + 0.5).clamp(0.0, 1.0);
            lum = lum.powf(1.0 / gamma);
            let t = lum.clamp(0.0, 1.0);
            let m = if t <= 0.5 {
                lerp_c(sh, mi, t * 2.0)
            } else {
                lerp_c(mi, hi, (t - 0.5) * 2.0)
            };
            C::new(
                clamp_to_byte(lerp(s.r as f32, m.r as f32, blend)),
                clamp_to_byte(lerp(s.g as f32, m.g as f32, blend)),
                clamp_to_byte(lerp(s.b as f32, m.b as f32, blend)),
                s.a,
            )
        })
        .collect();
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}

fn exposure(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let amount = p.f("amount").clamp(-10.0, 10.0);
    if amount.abs() < 0.0001 {
        return Ok(src.clone());
    }
    let gain = 2f32.powf(amount);
    Ok(pixel_op(src, |c| {
        C::new(
            clamp_to_byte(c.r as f32 * gain),
            clamp_to_byte(c.g as f32 * gain),
            clamp_to_byte(c.b as f32 * gain),
            c.a,
        )
    }))
}

fn film_emulation(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let tone = p.f("tone_strength").clamp(0.0, 100.0) / 100.0;
    let grain = p.f("grain_amount").clamp(0.0, 100.0) / 100.0;
    let fade = p.f("fade_amount").clamp(0.0, 100.0) / 100.0;
    let contrast = p.f("contrast_amount").clamp(50.0, 150.0) / 100.0;
    if tone <= 0.0 && grain <= 0.0 && fade <= 0.0 && (contrast - 1.0).abs() < 0.0001 {
        return Ok(src.clone());
    }
    let preset = p.e("preset");
    let seed = 1977;
    let c01 = clamp01;
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..src.h {
        for x in 0..src.w {
            let s = src.get(x, y);
            let mut r = s.r as f32 / 255.0;
            let mut g = s.g as f32 / 255.0;
            let mut b = s.b as f32 / 255.0;
            let (tr, tg, tb) = match preset.as_str() {
                "Warm" => (r * 1.10 + g * 0.015, g * 1.04, b * 0.88 + r * 0.01),
                "Cool" => (
                    r * 0.94 + b * 0.02,
                    g * 1.01 + b * 0.01,
                    b * 1.10 + g * 0.01,
                ),
                "Faded" => {
                    let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                    (
                        lerp(r, lum + 0.04, 0.22),
                        lerp(g, lum + 0.03, 0.18),
                        lerp(b, lum - 0.01, 0.26),
                    )
                }
                "CrossProcessed" => (
                    r.max(0.0001).powf(0.88) * 1.08,
                    g.max(0.0001).powf(1.05) * 0.98,
                    b.max(0.0001).powf(0.82) * 1.12,
                ),
                _ => (
                    r * 1.05 + g * 0.02,
                    g * 1.02 + r * 0.01,
                    b * 0.94 + g * 0.02,
                ),
            };
            let (tr, tg, tb) = (c01(tr), c01(tg), c01(tb));
            r = lerp(r, tr, tone);
            g = lerp(g, tg, tone);
            b = lerp(b, tb, tone);
            r = (r - 0.5) * contrast + 0.5;
            g = (g - 0.5) * contrast + 0.5;
            b = (b - 0.5) * contrast + 0.5;
            let fs = 1.0 - fade * 0.35;
            let lift = fade * 0.06;
            r = r * fs + lift;
            g = g * fs + lift;
            b = b * fs + lift;
            if grain > 0.0 {
                let n0 = hash01(x, y, seed) * 2.0 - 1.0;
                let n1 = hash01(x, y, seed ^ 117) * 2.0 - 1.0;
                let n2 = hash01(x, y, seed ^ 911) * 2.0 - 1.0;
                let amp = grain * 0.10;
                r += n0 * amp;
                g += n1 * amp;
                b += n2 * amp;
            }
            px.push(C::new(
                clamp_to_byte(c01(r) * 255.0),
                clamp_to_byte(c01(g) * 255.0),
                clamp_to_byte(c01(b) * 255.0),
                s.a,
            ));
        }
    }
    Ok(Bitmap::from_pixels(src.w, src.h, px))
}

fn gamma(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let amount = p.f("amount");
    let mut table = [0u8; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let v = i as f32 / 255.0;
        let corrected = (v as f64).powf(1.0 / amount as f64) as f32;
        *t = byte(corrected.clamp(0.0, 1.0) * 255.0);
    }
    Ok(color_table(src, &table, &table, &table))
}

fn grayscale(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let strength = p.f("strength");
    if strength >= 100.0 {
        return Ok(color_matrix(src, &GRAYSCALE_MATRIX));
    }
    if strength <= 0.0 {
        return Ok(src.clone());
    }
    let s = strength / 100.0;
    let i = 1.0 - s;
    let (r, g, b) = (0.2126f32, 0.7152f32, 0.0722f32);
    let m = [
        r * s + i,
        g * s,
        b * s,
        0.0,
        0.0,
        r * s,
        g * s + i,
        b * s,
        0.0,
        0.0,
        r * s,
        g * s,
        b * s + i,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
    ];
    Ok(color_matrix(src, &m))
}

fn hue(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let radians = (p.f("amount") as f64 * std::f64::consts::PI / 180.0) as f32;
    let c = (radians as f64).cos() as f32;
    let s = (radians as f64).sin() as f32;
    let m = [
        0.213 + c * 0.787 - s * 0.213,
        0.715 - c * 0.715 - s * 0.715,
        0.072 - c * 0.072 + s * 0.928,
        0.0,
        0.0,
        0.213 - c * 0.213 + s * 0.143,
        0.715 + c * 0.285 + s * 0.140,
        0.072 - c * 0.072 - s * 0.283,
        0.0,
        0.0,
        0.213 - c * 0.213 - s * 0.787,
        0.715 - c * 0.715 + s * 0.715,
        0.072 + c * 0.928 + s * 0.072,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
    ];
    Ok(color_matrix(src, &m))
}

fn invert(src: &Bitmap, _: &P) -> Result<Bitmap, String> {
    let m = [
        -1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 0.0,
        0.0, 1.0, 0.0,
    ];
    Ok(color_matrix(src, &m))
}

fn levels(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let in_black = p.i("input_black").clamp(0, 255);
    let mut in_white = p.i("input_white").clamp(0, 255);
    let mut out_black = p.i("output_black").clamp(0, 255);
    let mut out_white = p.i("output_white").clamp(0, 255);
    let gamma = p.f("gamma").clamp(0.1, 5.0);
    if in_white <= in_black {
        in_white = 255.min(in_black + 1);
    }
    if out_white < out_black {
        std::mem::swap(&mut out_black, &mut out_white);
    }
    if in_black == 0
        && in_white == 255
        && (gamma - 1.0).abs() < 0.0001
        && out_black == 0
        && out_white == 255
    {
        return Ok(src.clone());
    }
    let in_range = (in_white - in_black) as f32;
    let out_range = (out_white - out_black) as f32;
    let map = |v: u8| -> u8 {
        let n = ((v as i32 - in_black) as f32 / in_range).clamp(0.0, 1.0);
        let corrected = n.powf(gamma);
        let o = out_black as f32 + corrected * out_range;
        clamp_to_byte(o)
    };
    Ok(pixel_op(src, |c| C::new(map(c.r), map(c.g), map(c.b), c.a)))
}

fn lomo(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let s = (p.f("strength") / 100.0).clamp(0.0, 1.0);
    if s <= 0.0 {
        return Ok(src.clone());
    }
    let (w, hh) = (src.w, src.h);
    let m = [
        1.3, 0.05, -0.05, 0.0, 0.02, -0.02, 1.1, -0.02, 0.0, 0.01, -0.05, 0.0, 0.9, 0.0, -0.02,
        0.0, 0.0, 0.0, 1.0, 0.0,
    ];
    let colored = color_matrix(src, &mix_matrix(&m, s));
    let mut canvas = Canvas::from_bitmap(&colored);
    let (cx, cy) = (w as f32 / 2.0, hh as f32 / 2.0);
    let radius = ((w * w + hh * hh) as f32).sqrt() * 0.5;
    let paint = Paint::default().with_shader(Shader::Radial {
        center: (cx, cy),
        radius,
        colors: vec![C::TRANSPARENT, C::new(0, 0, 0, byte(180.0 * s))],
        pos: Some(vec![0.5, 1.0]),
        tile: Tile::Clamp,
    });
    canvas.draw_rect(rect(0.0, 0.0, w as f32, hh as f32), &paint);
    Ok(canvas.into_bitmap())
}

fn polaroid(src: &Bitmap, _: &P) -> Result<Bitmap, String> {
    let m = [
        1.438, -0.062, -0.062, 0.0, 0.0, -0.122, 1.378, -0.122, 0.0, 0.0, -0.016, -0.016, 1.483,
        0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
    ];
    Ok(color_matrix(src, &m))
}

fn posterize(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let levels = p.i("levels").clamp(2, 64);
    let scale = (levels - 1) as f32;
    let q = |v: u8| -> u8 {
        let bucket = round(v as f32 * scale / 255.0);
        let mapped = bucket * 255.0 / scale;
        clamp_to_byte(mapped)
    };
    Ok(pixel_op(src, |c| C::new(q(c.r), q(c.g), q(c.b), c.a)))
}

fn replace_color(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let tol = (p.f("tolerance") * 2.55) as i32;
    let target = p.c("target_color");
    let replace = p.c("replace_color");
    Ok(pixel_op(src, |c| {
        if colors_match(c, target, tol) {
            replace
        } else {
            c
        }
    }))
}

/// `ImageHelpers.ColorsMatch`.
pub fn colors_match(a: C, b: C, tolerance: i32) -> bool {
    (a.r as i32 - b.r as i32).abs() <= tolerance
        && (a.g as i32 - b.g as i32).abs() <= tolerance
        && (a.b as i32 - b.b as i32).abs() <= tolerance
        && (a.a as i32 - b.a as i32).abs() <= tolerance
}

fn saturation(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let x = 1.0 + p.f("amount") / 100.0;
    let inv = 1.0 - x;
    let r = inv * 0.3086f32;
    let g = inv * 0.6094f32;
    let b = inv * 0.0820f32;
    let m = [
        r + x,
        g,
        b,
        0.0,
        0.0,
        r,
        g + x,
        b,
        0.0,
        0.0,
        r,
        g,
        b + x,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
    ];
    Ok(color_matrix(src, &m))
}

pub const SEPIA_MATRIX: [f32; 20] = [
    0.393, 0.769, 0.189, 0.0, 0.0, 0.349, 0.686, 0.168, 0.0, 0.0, 0.272, 0.534, 0.131, 0.0, 0.0,
    0.0, 0.0, 0.0, 1.0, 0.0,
];

fn sepia(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let s = (p.f("strength") / 100.0).clamp(0.0, 1.0);
    if s <= 0.0 {
        return Ok(src.clone());
    }
    Ok(color_matrix(src, &mix_matrix(&SEPIA_MATRIX, s)))
}

fn shadows_highlights(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let shadows = p.f("shadows").clamp(-100.0, 100.0);
    let highlights = p.f("highlights").clamp(-100.0, 100.0);
    if shadows.abs() < 0.0001 && highlights.abs() < 0.0001 {
        return Ok(src.clone());
    }
    let ss = shadows / 100.0;
    let hs = highlights / 100.0;
    Ok(pixel_op(src, |c| {
        let luma = (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0;
        let sw = (1.0 - luma) * (1.0 - luma);
        let hw = luma * luma;
        let delta = (ss * sw - hs * hw) * 255.0;
        C::new(
            clamp_to_byte(c.r as f32 + delta),
            clamp_to_byte(c.g as f32 + delta),
            clamp_to_byte(c.b as f32 + delta),
            c.a,
        )
    }))
}

fn solarize(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let t = p.i("threshold").clamp(0, 255);
    let f = |v: u8| if v as i32 > t { 255 - v } else { v };
    Ok(pixel_op(src, |c| C::new(f(c.r), f(c.g), f(c.b), c.a)))
}

fn split_toning(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let strength = p.f("strength").clamp(0.0, 100.0) / 100.0;
    if strength < 0.001 {
        return Ok(src.clone());
    }
    let balance = p.f("balance").clamp(-100.0, 100.0) / 200.0 + 0.5;
    let st = p.c("shadow_tint");
    let ht = p.c("highlight_tint");
    let (sr, sg, sb) = (
        st.r as f32 / 255.0,
        st.g as f32 / 255.0,
        st.b as f32 / 255.0,
    );
    let (hr, hg, hb) = (
        ht.r as f32 / 255.0,
        ht.g as f32 / 255.0,
        ht.b as f32 / 255.0,
    );
    let smooth = |e0: f32, e1: f32, x: f32| {
        let range = e1 - e0;
        if range <= 0.0 {
            return if x >= e1 { 1.0 } else { 0.0 };
        }
        let t = ((x - e0) / range).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let soft = |base: f32, blend: f32| (1.0 - 2.0 * blend) * base * base + 2.0 * blend * base;
    Ok(pixel_op(src, |c| {
        let mut r = c.r as f32 / 255.0;
        let mut g = c.g as f32 / 255.0;
        let mut b = c.b as f32 / 255.0;
        let lum = 0.299 * r + 0.587 * g + 0.114 * b;
        let sw = 1.0 - smooth(0.0, balance, lum);
        let hw = smooth(balance, 1.0, lum);
        let tr = sr * sw + hr * hw;
        let tg = sg * sw + hg * hw;
        let tb = sb * sw + hb * hw;
        r = lerp(r, soft(r, tr), strength);
        g = lerp(g, soft(g, tg), strength);
        b = lerp(b, soft(b, tb), strength);
        C::new(
            clamp_to_byte(r * 255.0),
            clamp_to_byte(g * 255.0),
            clamp_to_byte(b * 255.0),
            c.a,
        )
    }))
}

fn temperature_tint(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let temperature = p.f("temperature").clamp(-100.0, 100.0);
    let tint = p.f("tint").clamp(-100.0, 100.0);
    if temperature.abs() < 0.0001 && tint.abs() < 0.0001 {
        return Ok(src.clone());
    }
    let td = temperature / 100.0 * 64.0;
    let nd = tint / 100.0 * 64.0;
    Ok(pixel_op(src, |c| {
        let r = c.r as f32 + td - nd * 0.25;
        let g = c.g as f32 + nd;
        let b = c.b as f32 - td - nd * 0.25;
        C::new(clamp_to_byte(r), clamp_to_byte(g), clamp_to_byte(b), c.a)
    }))
}

fn threshold(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let t = p.i("value").clamp(0, 255);
    Ok(pixel_op(src, |c| {
        let luma = (c.r as i32 * 77 + c.g as i32 * 150 + c.b as i32 * 29) >> 8;
        let bw = if luma >= t { 255 } else { 0 };
        C::new(bw, bw, bw, c.a)
    }))
}

fn vibrance(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let amount = p.f("amount").clamp(-100.0, 100.0) / 100.0;
    if amount.abs() < 0.0001 {
        return Ok(src.clone());
    }
    Ok(pixel_op(src, |c| {
        let (r, g, b) = (c.r as f32, c.g as f32, c.b as f32);
        let max = r.max(g.max(b));
        let min = r.min(g.min(b));
        let sat = (max - min) / 255.0;
        let gray = (r + g + b) / 3.0;
        let factor = if amount >= 0.0 {
            1.0 + amount * (1.0 - sat)
        } else {
            1.0 + amount
        };
        C::new(
            clamp_to_byte(gray + (r - gray) * factor),
            clamp_to_byte(gray + (g - gray) * factor),
            clamp_to_byte(gray + (b - gray) * factor),
            c.a,
        )
    }))
}
