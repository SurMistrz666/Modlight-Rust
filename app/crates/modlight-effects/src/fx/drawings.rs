//! The Drawings category (`Core/ImageEffects/Drawings`): backgrounds, overlays, shapes,
//! text and scattered decorations. Border, Candy cane border and Fake hair live with the
//! filter files they were ported in.

use std::sync::Arc;

use chrono::{Datelike, Local, Timelike};
use modlight_core::culture::Culture;
use modlight_core::icons as l;
use modlight_core::names::name_parser::{
    day_name, machine_name, month_name, user_domain_name, user_name,
};
use modlight_editor::effects::Category;
use tiny_skia::{Path, PathBuilder, Transform};

use crate::def::*;
use crate::dotnet::shared_random;
use crate::fx::manipulations::expand_env;
use crate::sk::{
    Bitmap, Blend, C, Canvas, Cap, ColorFilter, Font, Join, Paint, Sampling, Shader, Style, Tile,
    rect, round, store,
};

const D: Category = Category::Drawings;

const PLACEMENTS: &[(&str, &str)] = &[
    ("Top left", "TopLeft"),
    ("Top center", "TopCenter"),
    ("Top right", "TopRight"),
    ("Middle left", "MiddleLeft"),
    ("Middle center", "MiddleCenter"),
    ("Middle right", "MiddleRight"),
    ("Bottom left", "BottomLeft"),
    ("Bottom center", "BottomCenter"),
    ("Bottom right", "BottomRight"),
];

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "draw_background",
            "Background",
            D,
            l::PAINT_BUCKET,
            "Draws a solid color background behind the image.",
            background,
        )
        .params(|| vec![color("color", "Color", C::BLACK)]),
        Def::new(
            "draw_background_image",
            "Background image",
            D,
            l::IMAGE,
            "Draws a background image behind the source image.",
            background_image,
        )
        .params(|| {
            vec![
                file_path("image_file_path", "Image file path", "", None),
                checkbox("center", "Center", true),
                checkbox("tile", "Tile", false),
            ]
        }),
        Def::new(
            "draw_checkerboard",
            "Checkerboard",
            D,
            l::GRID_2X2_CHECK,
            "Draws a checkerboard pattern behind the image.",
            checkerboard,
        )
        .params(|| {
            vec![
                int_numeric("size", "Size", 1, 100, 10),
                color("color", "Color", C::rgb(211, 211, 211)),
                color("color2", "Color 2", C::WHITE),
            ]
        }),
        Def::new(
            "confetti",
            "Confetti",
            D,
            l::PARTY_POPPER,
            "Scatters colorful confetti pieces over the image.",
            confetti,
        )
        .params(|| {
            vec![
                int_slider("count", "Count", 10, 1000, 150),
                int_slider("min_size", "Min size", 2, 40, 4),
                int_slider("max_size", "Max size", 4, 80, 12),
                float_slider("opacity", "Opacity", 0.0, 100.0, 90.0),
                checkbox("background", "Background", false),
            ]
        }),
        Def::new(
            "fake_cursor",
            "Fake cursor",
            D,
            l::MOUSE_POINTER,
            "Draws fake mouse cursors at random positions on the image.",
            fake_cursor,
        )
        .params(|| {
            vec![
                int_slider("count", "Count", 1, 200, 1),
                float_slider("scale", "Scale", 0.5, 5.0, 1.0).tick(0.5),
                checkbox("random_size", "Random size", false),
                float_slider("random_size_min", "Random size min", 0.5, 5.0, 0.5).tick(0.5),
                float_slider("random_size_max", "Random size max", 0.5, 5.0, 2.0).tick(0.5),
                checkbox("random_angle", "Random angle", false),
                int_numeric("random_angle_min", "Random angle min", 0, 360, 0),
                int_numeric("random_angle_max", "Random angle max", 0, 360, 360),
                color("fill_color", "Fill color", C::WHITE),
                color("border_color", "Border color", C::BLACK),
            ]
        }),
        Def::new(
            "gradient_overlay",
            "Gradient overlay",
            D,
            l::PAINTBRUSH,
            "Blends a two-color gradient overlay on the image.",
            gradient_overlay,
        )
        .params(|| {
            vec![
                color("color1", "Start color", C::new(255, 0, 128, 255)),
                color("color2", "End color", C::new(0, 128, 255, 255)),
                int_slider("angle", "Angle", 0, 360, 0),
                float_slider("opacity", "Opacity", 0.0, 100.0, 50.0),
            ]
        }),
        Def::new(
            "draw_image",
            "Image",
            D,
            l::IMAGE_PLUS,
            "Draws an image overlay on the source image.",
            draw_image,
        )
        .params(|| {
            vec![
                file_path("image_location", "Image location", "", None),
                choice("placement", "Placement", PLACEMENTS, "TopLeft"),
                int_numeric("offset_x", "Offset X", -10000, 10000, 0),
                int_numeric("offset_y", "Offset Y", -10000, 10000, 0),
                choice(
                    "size_mode",
                    "Size mode",
                    &[
                        ("Don't resize", "DontResize"),
                        ("Absolute size", "AbsoluteSize"),
                        ("Percentage of watermark", "PercentageOfWatermark"),
                        ("Percentage of canvas", "PercentageOfCanvas"),
                    ],
                    "DontResize",
                ),
                int_numeric("size_width", "Size width", -1, 10000, 0),
                int_numeric("size_height", "Size height", -1, 10000, 0),
                choice(
                    "rotate_flip",
                    "Rotate / flip",
                    &[
                        ("None", "None"),
                        ("Rotate 90", "Rotate90"),
                        ("Rotate 180", "Rotate180"),
                        ("Rotate 270", "Rotate270"),
                        ("Flip X", "FlipX"),
                        ("Rotate 90 Flip X", "Rotate90FlipX"),
                        ("Flip Y", "FlipY"),
                        ("Rotate 90 Flip Y", "Rotate90FlipY"),
                    ],
                    "None",
                ),
                checkbox("tile", "Tile", false),
                checkbox("auto_hide", "Auto hide", false),
                choice(
                    "interpolation_mode",
                    "Interpolation mode",
                    &[
                        ("High quality bicubic", "HighQualityBicubic"),
                        ("Bicubic", "Bicubic"),
                        ("High quality bilinear", "HighQualityBilinear"),
                        ("Bilinear", "Bilinear"),
                        ("Nearest neighbor", "NearestNeighbor"),
                    ],
                    "HighQualityBicubic",
                ),
                choice(
                    "compositing_mode",
                    "Compositing mode",
                    &[("Source over", "SourceOver"), ("Source copy", "SourceCopy")],
                    "SourceOver",
                ),
                int_slider("opacity", "Opacity", 0, 100, 100),
            ]
        }),
        Def::new(
            "draw_line",
            "Line",
            D,
            l::MINUS,
            "Draws a line on the image.",
            draw_line,
        )
        .params(|| {
            vec![
                int_numeric("start_x", "Start X", -10000, 10000, 0),
                int_numeric("start_y", "Start Y", -10000, 10000, 0),
                int_numeric("end_x", "End X", -10000, 10000, 200),
                int_numeric("end_y", "End Y", -10000, 10000, 0),
                color("color", "Color", C::new(255, 255, 255, 255)),
                float_slider("thickness", "Thickness", 0.5, 100.0, 4.0),
            ]
        }),
        Def::new(
            "draw_particles",
            "Particles",
            D,
            l::SPARKLES,
            "Draws particle images on the source image.",
            particles,
        )
        .params(|| {
            vec![
                file_path("image_folder", "Image folder", "", None),
                int_numeric("image_count", "Image count", 1, 1000, 1),
                checkbox("background", "Background", false),
                checkbox("random_size", "Random size", false),
                int_numeric("random_size_min", "Random size min", 1, 10000, 64),
                int_numeric("random_size_max", "Random size max", 1, 10000, 128),
                checkbox("random_angle", "Random angle", false),
                int_numeric("random_angle_min", "Random angle min", 0, 360, 0),
                int_numeric("random_angle_max", "Random angle max", 0, 360, 360),
                checkbox("random_opacity", "Random opacity", false),
                int_numeric("random_opacity_min", "Random opacity min", 0, 100, 0),
                int_numeric("random_opacity_max", "Random opacity max", 0, 100, 100),
                checkbox("no_overlap", "No overlap", false),
                int_numeric("no_overlap_offset", "No overlap offset", 0, 1000, 0),
                checkbox("edge_overlap", "Edge overlap", false),
            ]
        }),
        Def::new(
            "draw_shape",
            "Shape",
            D,
            l::SHAPES,
            "Draws a shape on the image.",
            draw_shape,
        )
        .params(|| {
            vec![
                choice(
                    "shape",
                    "Shape",
                    &[
                        ("Rectangle", "Rectangle"),
                        ("Rounded rectangle", "RoundedRectangle"),
                        ("Ellipse", "Ellipse"),
                        ("Triangle", "Triangle"),
                        ("Diamond", "Diamond"),
                    ],
                    "Rectangle",
                ),
                choice("placement", "Placement", PLACEMENTS, "TopLeft"),
                int_numeric("offset_x", "Offset X", -10000, 10000, 0),
                int_numeric("offset_y", "Offset Y", -10000, 10000, 0),
                int_numeric("size_width", "Size width", -1, 10000, 100),
                int_numeric("size_height", "Size height", -1, 10000, 100),
                color("color", "Color", C::new(255, 255, 255, 255)),
            ]
        }),
        Def::new(
            "snowflakes",
            "Snowflakes",
            D,
            l::SNOWFLAKE,
            "Scatters snowflakes over the image.",
            snowflakes,
        )
        .params(|| {
            vec![
                int_slider("count", "Count", 10, 500, 80),
                int_slider("min_size", "Min size", 2, 20, 3),
                int_slider("max_size", "Max size", 4, 60, 14),
                float_slider("opacity", "Opacity", 0.0, 100.0, 85.0),
                checkbox("random_angle", "Random angle", true),
                checkbox("background", "Background", false),
            ]
        }),
        Def::new(
            "draw_text",
            "Text",
            D,
            l::TYPE,
            "Draws text on the image.",
            draw_text,
        )
        .params(|| {
            vec![
                text("text", "Text", "Text"),
                choice("placement", "Placement", PLACEMENTS, "TopLeft"),
                int_numeric("offset_x", "Offset X", -10000, 10000, 0),
                int_numeric("offset_y", "Offset Y", -10000, 10000, 0),
                int_numeric("angle", "Angle", -360, 360, 0),
                checkbox("auto_hide", "Auto hide", false),
                text("font_family", "Font family", "Arial"),
                float_slider("font_size", "Font size", 1.0, 500.0, 36.0),
                checkbox("bold", "Bold", false),
                checkbox("italic", "Italic", false),
                color("color", "Color", C::rgb(235, 235, 235)),
                checkbox("outline", "Outline", false),
                int_numeric("outline_size", "Outline size", 1, 100, 5),
                color("outline_color", "Outline color", C::rgb(235, 0, 0)),
                checkbox("shadow", "Shadow", false),
                int_numeric("shadow_offset_x", "Shadow offset X", -1000, 1000, 0),
                int_numeric("shadow_offset_y", "Shadow offset Y", -1000, 1000, 5),
                color("shadow_color", "Shadow color", C::new(0, 0, 0, 125)),
            ]
        }),
        Def::new(
            "text_watermark",
            "Text watermark",
            D,
            l::TEXT_CURSOR,
            "Draws a text watermark with background on the image.",
            text_watermark,
        )
        .params(|| {
            vec![
                text("text", "Text", "Text watermark"),
                choice("placement", "Placement", PLACEMENTS, "BottomRight"),
                int_numeric("offset_x", "Offset X", -10000, 10000, 5),
                int_numeric("offset_y", "Offset Y", -10000, 10000, 5),
                checkbox("auto_hide", "Auto hide", false),
                text("font_family", "Font family", "Arial"),
                float_slider("font_size", "Font size", 1.0, 500.0, 15.0),
                checkbox("bold", "Bold", false),
                checkbox("italic", "Italic", false),
                color("text_color", "Text color", C::rgb(235, 235, 235)),
                checkbox("draw_text_shadow", "Draw text shadow", false),
                color("text_shadow_color", "Text shadow color", C::BLACK),
                int_numeric(
                    "text_shadow_offset_x",
                    "Text shadow offset X",
                    -1000,
                    1000,
                    -1,
                ),
                int_numeric(
                    "text_shadow_offset_y",
                    "Text shadow offset Y",
                    -1000,
                    1000,
                    -1,
                ),
                int_numeric("corner_radius", "Corner radius", 0, 100, 4),
                int_numeric("padding_left", "Padding left", 0, 100, 5),
                int_numeric("padding_top", "Padding top", 0, 100, 5),
                int_numeric("padding_right", "Padding right", 0, 100, 5),
                int_numeric("padding_bottom", "Padding bottom", 0, 100, 5),
                checkbox("draw_border", "Draw border", true),
                color("border_color", "Border color", C::BLACK),
                int_numeric("border_size", "Border size", 0, 50, 1),
                checkbox("draw_background", "Draw background", true),
                color("background_color", "Background color", C::rgb(42, 47, 56)),
            ]
        }),
    ]
}

// ---------- DrawingEffectHelpers ----------

/// `ExpandVariables`: environment variables in a trimmed path.
fn expand_path(path: &str) -> String {
    if path.trim().is_empty() {
        return String::new();
    }
    expand_env(path.trim())
}

/// Replaces `token` case-insensitively (`string.Replace(..., OrdinalIgnoreCase)`).
fn replace_ci(text: &str, token: &str, value: &str) -> String {
    let lower = text.to_lowercase();
    let token_l = token.to_lowercase();
    if lower.len() != text.len() || !lower.contains(&token_l) {
        return if lower.len() != text.len() {
            text.replace(token, value)
        } else {
            text.to_string()
        };
    }
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(pos) = lower[i..].find(&token_l) {
        out.push_str(&text[i..i + pos]);
        out.push_str(value);
        i += pos + token_l.len();
    }
    out.push_str(&text[i..]);
    out
}

/// `ExpandTextVariables`: `%width`, `%height`, date and time codes, user and computer.
pub fn expand_text_variables(text: &str, size: (i32, i32)) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut s = expand_env(text);
    let now = Local::now();
    let twelve = s.to_lowercase().contains("%pm");
    let mut hour = now.hour() % 12;
    if hour == 0 {
        hour = 12;
    }
    let culture = Culture::current();
    let month0 = now.month0() as usize;
    let dow = now.weekday().num_days_from_sunday() as usize;
    let replacements: Vec<(&str, String)> = vec![
        ("%width", size.0.to_string()),
        ("%height", size.1.to_string()),
        ("%unix", chrono::Utc::now().timestamp().to_string()),
        ("%mon2", month_name(Culture::english(), month0).to_string()),
        ("%mon", month_name(culture, month0).to_string()),
        ("%yy", format!("{:02}", now.year() % 100)),
        ("%y", now.year().to_string()),
        ("%mo", format!("{:02}", now.month())),
        ("%mi", format!("{:02}", now.minute())),
        (
            "%ms",
            format!("{:03}", now.timestamp_subsec_millis().min(999)),
        ),
        (
            "%pm",
            if now.hour() >= 12 { "PM" } else { "AM" }.to_string(),
        ),
        ("%w2", day_name(Culture::english(), dow).to_string()),
        ("%w", day_name(culture, dow).to_string()),
        ("%d", format!("{:02}", now.day())),
        (
            "%h",
            format!("{:02}", if twelve { hour } else { now.hour() }),
        ),
        ("%s", format!("{:02}", now.second())),
        ("%un", user_name()),
        ("%uln", user_domain_name()),
        ("%cn", machine_name()),
        ("%n", "\n".to_string()),
    ];
    for (token, value) in replacements {
        s = replace_ci(&s, token, &value);
    }
    s
}

/// `GetPosition`: where an object of `size` goes on a background of `bg`.
fn position(placement: &str, offset: (i32, i32), bg: (i32, i32), size: (i32, i32)) -> (i32, i32) {
    let mid_x = round(bg.0 as f32 / 2.0 - size.0 as f32 / 2.0) as i32;
    let mid_y = round(bg.1 as f32 / 2.0 - size.1 as f32 / 2.0) as i32;
    let right = bg.0 - size.0;
    let bottom = bg.1 - size.1;
    match placement {
        "TopCenter" => (mid_x, offset.1),
        "TopRight" => (right - offset.0, offset.1),
        "MiddleLeft" => (offset.0, mid_y),
        "MiddleCenter" => (mid_x, mid_y),
        "MiddleRight" => (right - offset.0, mid_y),
        "BottomLeft" => (offset.0, bottom - offset.1),
        "BottomCenter" => (mid_x, bottom - offset.1),
        "BottomRight" => (right - offset.0, bottom - offset.1),
        _ => offset,
    }
}

/// `ApplyAspectRatio`: a zero side follows the other one.
fn aspect(w: i32, h: i32, b: &Bitmap) -> (i32, i32) {
    if w == 0 {
        (round(h as f32 / b.h as f32 * b.w as f32) as i32, h)
    } else if h == 0 {
        (w, round(w as f32 / b.w as f32 * b.h as f32) as i32)
    } else {
        (w, h)
    }
}

fn sampling_for(mode: &str) -> Sampling {
    match mode {
        "Bicubic" | "HighQualityBilinear" => Sampling::MITCHELL,
        "Bilinear" => Sampling::Linear,
        "NearestNeighbor" => Sampling::Nearest,
        _ => Sampling::CATMULL_ROM,
    }
}

/// A picture file decoded as `SKBitmap.Decode` gives it (premultiplied storage).
fn decode(path: &str) -> Option<Bitmap> {
    let img = modlight_core::image_decode::open(std::path::Path::new(path))
        .ok()?
        .to_rgba8();
    if img.width() == 0 || img.height() == 0 {
        return None;
    }
    let mut b = Bitmap::from_image(&img);
    for c in b.px.iter_mut() {
        *c = store(*c);
    }
    Some(b)
}

/// `RotateFlip` through a canvas, as upstream draws it.
fn rotate_flip(src: &Bitmap, mode: &str) -> Bitmap {
    let draw = |w: i32, h: i32, f: &dyn Fn(&mut Canvas)| {
        let mut c = Canvas::new(w, h);
        f(&mut c);
        c.draw_bitmap(src, 0.0, 0.0, &Paint::default());
        c.into_bitmap()
    };
    let (w, h) = (src.w as f32, src.h as f32);
    let r90 = || {
        draw(src.h, src.w, &|c| {
            c.translate(h, 0.0);
            c.rotate_degrees(90.0);
        })
    };
    let flip_x = |b: &Bitmap| {
        let mut c = Canvas::new(b.w, b.h);
        c.translate(b.w as f32, 0.0);
        c.scale(-1.0, 1.0);
        c.draw_bitmap(b, 0.0, 0.0, &Paint::default());
        c.into_bitmap()
    };
    let flip_y = |b: &Bitmap| {
        let mut c = Canvas::new(b.w, b.h);
        c.translate(0.0, b.h as f32);
        c.scale(1.0, -1.0);
        c.draw_bitmap(b, 0.0, 0.0, &Paint::default());
        c.into_bitmap()
    };
    match mode {
        "Rotate90" => r90(),
        "Rotate180" => draw(src.w, src.h, &|c| {
            c.translate(w, h);
            c.rotate_degrees(180.0);
        }),
        "Rotate270" => draw(src.h, src.w, &|c| {
            c.translate(0.0, w);
            c.rotate_degrees(270.0);
        }),
        "FlipX" => flip_x(src),
        "FlipY" => flip_y(src),
        "Rotate90FlipX" => flip_x(&r90()),
        "Rotate90FlipY" => flip_y(&r90()),
        _ => src.clone(),
    }
}

// ---------- effects ----------

fn background(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let mut canvas = Canvas::new(src.w, src.h);
    canvas.draw_rect_xywh(
        0.0,
        0.0,
        src.w as f32,
        src.h as f32,
        &Paint::fill_aa(p.c("color")),
    );
    canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    Ok(canvas.into_bitmap())
}

fn background_image(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let path = expand_path(&p.s("image_file_path"));
    if path.trim().is_empty() || !std::path::Path::new(&path).is_file() {
        return Ok(src.clone());
    }
    let Some(bg) = decode(&path) else {
        return Ok(src.clone());
    };
    let (rw, rh) = (src.w, src.h);
    let center = p.b("center");
    let mut canvas = Canvas::new(rw, rh);
    if p.b("tile") {
        let (mut tx, mut ty) = (0, 0);
        if center {
            tx = (rw - bg.w) / 2 % bg.w;
            ty = (rh - bg.h) / 2 % bg.h;
        }
        let paint = Paint::fill_aa(C::BLACK).with_shader(Shader::Image {
            bitmap: Arc::new(bg),
            tile_x: Tile::Repeat,
            tile_y: Tile::Repeat,
            matrix: Transform::identity(),
            sampling: Sampling::CATMULL_ROM,
        });
        if center {
            canvas.save();
            canvas.translate(tx as f32, ty as f32);
            canvas.draw_rect_xywh(-tx as f32, -ty as f32, rw as f32, rh as f32, &paint);
            canvas.restore();
        } else {
            canvas.draw_rect_xywh(0.0, 0.0, rw as f32, rh as f32, &paint);
        }
    } else {
        let ratio = bg.w as f32 / bg.h as f32;
        let mut w = rw;
        let mut h = (w as f32 / ratio) as i32;
        if h < rh {
            h = rh;
            w = (h as f32 * ratio) as i32;
        }
        let x = if center { (rw - w) / 2 } else { 0 };
        let y = if center { (rh - h) / 2 } else { 0 };
        canvas.draw_bitmap_to(
            &bg,
            rect(x as f32, y as f32, (x + w) as f32, (y + h) as f32),
            Sampling::CATMULL_ROM,
            &Paint::default().aa(true),
        );
    }
    canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    Ok(canvas.into_bitmap())
}

fn checkerboard(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let size = 1.max(p.i("size"));
    let (c1, c2) = (p.c("color"), p.c("color2"));
    let mut pattern = Canvas::new(size * 2, size * 2);
    let s = size as f32;
    pattern.draw_rect_xywh(0.0, 0.0, s, s, &Paint::fill(c1));
    pattern.draw_rect_xywh(s, s, s, s, &Paint::fill(c1));
    pattern.draw_rect_xywh(s, 0.0, s, s, &Paint::fill(c2));
    pattern.draw_rect_xywh(0.0, s, s, s, &Paint::fill(c2));
    let paint = Paint::fill_aa(C::BLACK).with_shader(Shader::Image {
        bitmap: Arc::new(pattern.into_bitmap()),
        tile_x: Tile::Repeat,
        tile_y: Tile::Repeat,
        matrix: Transform::identity(),
        sampling: Sampling::Nearest,
    });
    let mut canvas = Canvas::new(src.w, src.h);
    canvas.draw_rect_xywh(0.0, 0.0, src.w as f32, src.h as f32, &paint);
    canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    Ok(canvas.into_bitmap())
}

const CONFETTI: [C; 10] = [
    C::rgb(255, 56, 96),
    C::rgb(255, 165, 0),
    C::rgb(255, 215, 0),
    C::rgb(0, 200, 83),
    C::rgb(0, 176, 255),
    C::rgb(156, 39, 176),
    C::rgb(255, 64, 129),
    C::rgb(0, 230, 118),
    C::rgb(255, 234, 0),
    C::rgb(29, 233, 182),
];

fn confetti(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let count = p.i("count");
    if count <= 0 {
        return Ok(src.clone());
    }
    let alpha = (p.f("opacity") / 100.0).clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return Ok(src.clone());
    }
    let (min_s, max_s) = (p.i("min_size"), p.i("max_size"));
    let min_size = 2.max(min_s.min(max_s));
    let max_size = min_size.max(max_s);
    let background = p.b("background");
    let mut canvas = if background {
        Canvas::new(src.w, src.h)
    } else {
        Canvas::from_bitmap(src)
    };
    let mut rng = shared_random();
    for _ in 0..count {
        let x = rng.next_range(0, src.w) as f32;
        let y = rng.next_range(0, src.h) as f32;
        let w = rng.next_range(min_size, max_size + 1) as f32;
        let h = rng.next_range(min_size, max_size + 1) as f32;
        let angle = rng.next_range(0, 360) as f32;
        let c = CONFETTI[rng.next_max(CONFETTI.len() as i32) as usize];
        let paint = Paint::fill_aa(c.with_alpha((c.a as f32 * alpha) as u8));
        canvas.save();
        canvas.translate(x, y);
        canvas.rotate_degrees(angle);
        match rng.next_max(3) {
            0 => canvas.draw_rect_xywh(-w / 2.0, -h / 2.0, w, h, &paint),
            1 => canvas.draw_oval(rect(-w / 2.0, -h / 2.0, w / 2.0, h / 2.0), &paint),
            _ => {
                let mut pb = PathBuilder::new();
                pb.move_to(0.0, -h / 2.0);
                pb.line_to(w / 2.0, h / 2.0);
                pb.line_to(-w / 2.0, h / 2.0);
                pb.close();
                if let Some(path) = pb.finish() {
                    canvas.draw_path(&path, &paint);
                }
            }
        }
        canvas.restore();
    }
    if background {
        canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    }
    Ok(canvas.into_bitmap())
}

const CURSOR: [(f32, f32); 7] = [
    (0.0, 0.0),
    (0.0, 20.4),
    (5.7, 15.6),
    (9.6, 23.4),
    (12.6, 22.2),
    (8.7, 14.4),
    (15.3, 14.4),
];

fn fake_cursor(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let count = p.i("count").clamp(1, 200);
    let (rmin, rmax) = (p.f("random_size_min"), p.f("random_size_max"));
    let mut canvas = Canvas::from_bitmap(src);
    let mut rng = shared_random();
    let mut pb = PathBuilder::new();
    pb.move_to(CURSOR[0].0, CURSOR[0].1);
    for pt in &CURSOR[1..] {
        pb.line_to(pt.0, pt.1);
    }
    pb.close();
    let Some(path) = pb.finish() else {
        return Ok(src.clone());
    };
    let border = Paint::stroke_aa(p.c("border_color"), 1.5).with_join(Join::Round);
    let fill = Paint::fill_aa(p.c("fill_color"));
    for _ in 0..count {
        let scale = if p.b("random_size") {
            rmin + rng.next_single() * (rmin.max(rmax) - rmin)
        } else {
            p.f("scale")
        }
        .clamp(0.5, 5.0);
        let cw = 15.3 * scale;
        let ch = 23.4 * scale;
        let x = rng.next_range(0, 1.max(src.w - cw as i32)) as f32;
        let y = rng.next_range(0, 1.max(src.h - ch as i32)) as f32;
        canvas.save();
        canvas.translate(x, y);
        if p.b("random_angle") {
            let (a, b) = (p.i("random_angle_min"), p.i("random_angle_max"));
            let (lo, hi) = (a.min(b), a.max(b));
            let angle = if lo == hi { lo } else { rng.next_range(lo, hi) };
            canvas.translate(cw / 2.0, ch / 2.0);
            canvas.rotate_degrees(angle as f32);
            canvas.translate(-cw / 2.0, -ch / 2.0);
        }
        canvas.scale(scale, scale);
        canvas.draw_path(&path, &border);
        canvas.draw_path(&path, &fill);
        canvas.restore();
    }
    Ok(canvas.into_bitmap())
}

fn gradient_overlay(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let alpha = (p.f("opacity") / 100.0).clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return Ok(src.clone());
    }
    let (w, h) = (src.w as f32, src.h as f32);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let rad = p.i("angle") as f32 * std::f32::consts::PI / 180.0;
    let diag = ((src.w * src.w + src.h * src.h) as f32).sqrt() / 2.0;
    let (c1, c2) = (p.c("color1"), p.c("color2"));
    let paint = Paint::default()
        .with_shader(Shader::Linear {
            p0: (cx - rad.cos() * diag, cy - rad.sin() * diag),
            p1: (cx + rad.cos() * diag, cy + rad.sin() * diag),
            colors: vec![
                c1.with_alpha((c1.a as f32 * alpha) as u8),
                c2.with_alpha((c2.a as f32 * alpha) as u8),
            ],
            pos: Some(vec![0.0, 1.0]),
            tile: Tile::Clamp,
        })
        .with_blend(Blend::Overlay);
    let mut canvas = Canvas::from_bitmap(src);
    canvas.draw_rect_xywh(0.0, 0.0, w, h, &paint);
    Ok(canvas.into_bitmap())
}

fn draw_image(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let opacity = p.i("opacity").clamp(0, 100);
    let size_mode = p.e("size_mode");
    let (sw, sh) = (p.i("size_width"), p.i("size_height"));
    if opacity < 1 || (size_mode != "DontResize" && sw <= 0 && sh <= 0) {
        return Ok(src.clone());
    }
    let path = expand_path(&p.s("image_location"));
    if path.trim().is_empty() || !std::path::Path::new(&path).is_file() {
        return Ok(src.clone());
    }
    let Some(raw) = decode(&path) else {
        return Ok(src.clone());
    };
    let mark = rotate_flip(&raw, &p.e("rotate_flip"));
    let pct = |v: i32, of: i32| round(v as f32 / 100.0 * of as f32) as i32;
    let size = match size_mode.as_str() {
        "AbsoluteSize" => aspect(
            if sw == -1 { src.w } else { sw },
            if sh == -1 { src.h } else { sh },
            &mark,
        ),
        "PercentageOfWatermark" => aspect(pct(sw, mark.w), pct(sh, mark.h), &mark),
        "PercentageOfCanvas" => aspect(pct(sw, src.w), pct(sh, src.h), &mark),
        _ => (mark.w, mark.h),
    };
    if size.0 <= 0 || size.1 <= 0 {
        return Ok(src.clone());
    }
    let pos = position(
        &p.e("placement"),
        (p.i("offset_x"), p.i("offset_y")),
        (src.w, src.h),
        size,
    );
    let r = (pos.0, pos.1, pos.0 + size.0, pos.1 + size.1);
    if p.b("auto_hide") && !(r.0 >= 0 && r.1 >= 0 && r.2 <= src.w && r.3 <= src.h) {
        return Ok(src.clone());
    }
    let sampling = sampling_for(&p.e("interpolation_mode"));
    let blend = if p.e("compositing_mode") == "SourceCopy" {
        Blend::Src
    } else {
        Blend::SrcOver
    };
    let mut paint = Paint::default().aa(true).with_blend(blend);
    if opacity < 100 {
        let a = round(255.0 * (opacity as f32 / 100.0)) as u8;
        paint.color_filter = Some(ColorFilter::Blend(
            C::new(255, 255, 255, a),
            Blend::Modulate,
        ));
    }
    let mut canvas = Canvas::from_bitmap(src);
    if p.b("tile") {
        let paint = paint.with_shader(Shader::Image {
            bitmap: Arc::new(mark),
            tile_x: Tile::Repeat,
            tile_y: Tile::Repeat,
            matrix: Transform::identity(),
            sampling,
        });
        canvas.save();
        canvas.translate(r.0 as f32, r.1 as f32);
        canvas.draw_rect_xywh(0.0, 0.0, size.0 as f32, size.1 as f32, &paint);
        canvas.restore();
    } else {
        canvas.draw_bitmap_to(
            &mark,
            rect(r.0 as f32, r.1 as f32, r.2 as f32, r.3 as f32),
            sampling,
            &paint,
        );
    }
    Ok(canvas.into_bitmap())
}

fn draw_line(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let thickness = p.f("thickness");
    let color = p.c("color");
    if thickness <= 0.0 || color.a == 0 {
        return Ok(src.clone());
    }
    let mut canvas = Canvas::from_bitmap(src);
    let paint = Paint::stroke_aa(color, thickness)
        .with_cap(Cap::Round)
        .with_join(Join::Round);
    canvas.draw_line(
        p.i("start_x") as f32,
        p.i("start_y") as f32,
        p.i("end_x") as f32,
        p.i("end_y") as f32,
        &paint,
    );
    Ok(canvas.into_bitmap())
}

fn particles(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let folder = expand_path(&p.s("image_folder"));
    let dir = std::path::Path::new(&folder);
    if folder.trim().is_empty() || !dir.is_dir() {
        return Ok(src.clone());
    }
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|f| {
            f.is_file()
                && f.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| matches!(e.to_lowercase().as_str(), "png" | "jpg" | "jpeg"))
        })
        .collect();
    files.sort();
    if files.is_empty() {
        return Ok(src.clone());
    }
    let count = p.i("image_count").clamp(1, 1000);
    let background = p.b("background");
    let mut canvas = if background {
        Canvas::new(src.w, src.h)
    } else {
        Canvas::from_bitmap(src)
    };
    let mut rng = shared_random();
    let mut next = |a: i32, b: i32| -> i32 {
        let (lo, hi) = (a.min(b), a.max(b));
        if lo == hi { lo } else { rng.next_range(lo, hi) }
    };
    let mut cache: std::collections::HashMap<std::path::PathBuf, Option<Bitmap>> =
        std::collections::HashMap::new();
    let mut placed: Vec<(i32, i32, i32, i32)> = Vec::new();
    let edge = p.b("edge_overlap");
    let no_overlap = p.b("no_overlap");
    let gap = p.i("no_overlap_offset");
    for _ in 0..count {
        let file = files[next(0, files.len() as i32) as usize].clone();
        let particle = cache
            .entry(file.clone())
            .or_insert_with(|| decode(&file.to_string_lossy()));
        let Some(particle) = particle.clone() else {
            continue;
        };
        let (mut w, mut h) = (particle.w, particle.h);
        if p.b("random_size") {
            let size = next(p.i("random_size_min"), p.i("random_size_max"));
            w = size;
            h = size;
            if particle.w > particle.h {
                h = (size as f64 * (particle.h as f64 / particle.w as f64)).round_ties_even()
                    as i32;
            } else if particle.w < particle.h {
                w = (size as f64 * (particle.w as f64 / particle.h as f64)).round_ties_even()
                    as i32;
            }
        }
        if w < 1 || h < 1 {
            continue;
        }
        let min_x = if edge { -w + 1 } else { 0 };
        let min_y = if edge { -h + 1 } else { 0 };
        let max_x = src.w - if edge { 0 } else { w } - 1;
        let max_y = src.h - if edge { 0 } else { h } - 1;
        let mut attempts = 0;
        let r = loop {
            attempts += 1;
            if attempts > 1000 {
                break None;
            }
            let x = next(min_x, max_x);
            let y = next(min_y, max_y);
            let r = (x, y, x + w, y + h);
            let inflated = (r.0 - gap, r.1 - gap, r.2 + gap, r.3 + gap);
            let hit = placed.iter().any(|o| {
                o.0 < inflated.2 && o.2 > inflated.0 && o.1 < inflated.3 && o.3 > inflated.1
            });
            if !(no_overlap && hit) {
                break Some(r);
            }
        };
        let Some(r) = r else {
            continue;
        };
        placed.push(r);
        let mut alpha = 255;
        if p.b("random_opacity") {
            let o = next(p.i("random_opacity_min"), p.i("random_opacity_max")).clamp(0, 100);
            alpha = round(o as f32 / 100.0 * 255.0) as i32;
        }
        canvas.save();
        if p.b("random_angle") {
            let mx = r.0 as f32 + w as f32 / 2.0;
            let my = r.1 as f32 + h as f32 / 2.0;
            let rot = next(p.i("random_angle_min"), p.i("random_angle_max"));
            canvas.translate(mx, my);
            canvas.rotate_degrees(rot as f32);
            canvas.translate(-mx, -my);
        }
        let mut paint = Paint::default().aa(true);
        if alpha < 255 {
            paint.color_filter = Some(ColorFilter::Blend(
                C::new(255, 255, 255, alpha as u8),
                Blend::Modulate,
            ));
        }
        canvas.draw_bitmap_to(
            &particle,
            rect(r.0 as f32, r.1 as f32, r.2 as f32, r.3 as f32),
            Sampling::CATMULL_ROM,
            &paint,
        );
        canvas.restore();
    }
    if background {
        canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    }
    Ok(canvas.into_bitmap())
}

fn draw_shape(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let dim = |v: i32, full: i32| if v == -1 { full } else { v };
    let w = dim(p.i("size_width"), src.w);
    let h = dim(p.i("size_height"), src.h);
    let color = p.c("color");
    if w <= 0 || h <= 0 || color.a == 0 {
        return Ok(src.clone());
    }
    let pos = position(
        &p.e("placement"),
        (p.i("offset_x"), p.i("offset_y")),
        (src.w, src.h),
        (w, h),
    );
    let r = rect(
        pos.0 as f32,
        pos.1 as f32,
        (pos.0 + w) as f32,
        (pos.1 + h) as f32,
    );
    let paint = Paint::fill_aa(color);
    let mut canvas = Canvas::from_bitmap(src);
    let (mx, my) = ((r.left() + r.right()) / 2.0, (r.top() + r.bottom()) / 2.0);
    match p.e("shape").as_str() {
        "RoundedRectangle" => {
            let radius = 1f32.max(r.width().min(r.height()) * 0.2);
            canvas.draw_round_rect(r, radius, radius, &paint);
        }
        "Ellipse" => canvas.draw_oval(r, &paint),
        "Triangle" | "Diamond" => {
            let mut pb = PathBuilder::new();
            pb.move_to(mx, r.top());
            if p.e("shape") == "Triangle" {
                pb.line_to(r.right(), r.bottom());
                pb.line_to(r.left(), r.bottom());
            } else {
                pb.line_to(r.right(), my);
                pb.line_to(mx, r.bottom());
                pb.line_to(r.left(), my);
            }
            pb.close();
            if let Some(path) = pb.finish() {
                canvas.draw_path(&path, &paint);
            }
        }
        _ => canvas.draw_rect(r, &paint),
    }
    Ok(canvas.into_bitmap())
}

fn snowflake(canvas: &mut Canvas, cx: f32, cy: f32, size: f32, alpha: f32, rotation: f32) {
    let a = (255.0 * alpha) as u8;
    canvas.save();
    canvas.translate(cx, cy);
    canvas.rotate_degrees(rotation);
    canvas.translate(-cx, -cy);
    let glow =
        Paint::fill_aa(C::new(200, 220, 255, (a as f32 * 0.3) as u8)).with_mask_blur(size * 0.4);
    canvas.draw_circle(cx, cy, size * 0.6, &glow);
    let line =
        Paint::stroke_aa(C::new(240, 248, 255, a), 1f32.max(size * 0.1)).with_cap(Cap::Round);
    let half = size / 2.0;
    for b in 0..6 {
        let angle = b as f32 * 60.0 * std::f32::consts::PI / 180.0;
        let (cos, sin) = (angle.cos(), angle.sin());
        canvas.draw_line(cx, cy, cx + cos * half, cy + sin * half, &line);
        if size >= 6.0 {
            let len = half * 0.35;
            let mx = cx + cos * half * 0.55;
            let my = cy + sin * half * 0.55;
            for side in [-1.0f32, 1.0] {
                let sa = angle + side * std::f32::consts::PI / 3.0;
                canvas.draw_line(mx, my, mx + sa.cos() * len, my + sa.sin() * len, &line);
            }
        }
    }
    let dot = Paint::fill_aa(C::new(255, 255, 255, a));
    canvas.draw_circle(cx, cy, 1f32.max(size * 0.08), &dot);
    canvas.restore();
}

fn snowflakes(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let count = p.i("count");
    if count <= 0 {
        return Ok(src.clone());
    }
    let alpha = (p.f("opacity") / 100.0).clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return Ok(src.clone());
    }
    let (min_s, max_s) = (p.i("min_size"), p.i("max_size"));
    let min_size = 2.max(min_s.min(max_s));
    let max_size = min_size.max(max_s);
    let background = p.b("background");
    let random_angle = p.b("random_angle");
    let mut canvas = if background {
        Canvas::new(src.w, src.h)
    } else {
        Canvas::from_bitmap(src)
    };
    let mut rng = shared_random();
    for _ in 0..count {
        let x = rng.next_range(0, src.w) as f32;
        let y = rng.next_range(0, src.h) as f32;
        let size = rng.next_range(min_size, max_size + 1) as f32;
        let fa = alpha * (0.5 + rng.next_single() * 0.5);
        let angle = if random_angle {
            rng.next_single() * 360.0
        } else {
            0.0
        };
        snowflake(&mut canvas, x, y, size, fa, angle);
    }
    if background {
        canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    }
    Ok(canvas.into_bitmap())
}

fn font_for(p: &P, size: f32) -> Font {
    Font::new(&p.s("font_family"), size)
        .bold(p.b("bold"))
        .italic(p.b("italic"))
}

/// `SKFont.Metrics`: ascent (negative), descent, leading.
fn metrics(font: &Font) -> (f32, f32, f32) {
    let (ascent, descent) = font.metrics();
    (ascent, descent, 0.0)
}

fn split_lines(text: &str) -> Vec<String> {
    text.replace("\r\n", "\n")
        .split('\n')
        .map(str::to_string)
        .collect()
}

fn text_path(text: &str, font: &Font) -> Option<Path> {
    let lines = split_lines(text);
    let (ascent, descent, leading) = metrics(font);
    let line_h = (descent - ascent + leading).max(font.size);
    let base = -ascent;
    let mut pb = PathBuilder::new();
    let mut any = false;
    for (i, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        if let Some(p) = font.path(line, 0.0, base + i as f32 * line_h) {
            pb.push_path(&p);
            any = true;
        }
    }
    if any { pb.finish() } else { None }
}

fn draw_text(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let text = p.s("text");
    let size = p.f("font_size");
    if text.trim().is_empty() || size < 1.0 {
        return Ok(src.clone());
    }
    let font = font_for(p, size);
    let Some(mut path) = text_path(&text, &font) else {
        return Ok(src.clone());
    };
    let angle = p.i("angle");
    if angle != 0 {
        match path.transform(Transform::from_rotate(angle as f32)) {
            Some(t) => path = t,
            None => return Ok(src.clone()),
        }
    }
    let b = path.bounds();
    if b.width() <= 0.0 || b.height() <= 0.0 {
        return Ok(src.clone());
    }
    let ts = (b.width().ceil() as i32 + 1, b.height().ceil() as i32 + 1);
    let pos = position(
        &p.e("placement"),
        (p.i("offset_x"), p.i("offset_y")),
        (src.w, src.h),
        ts,
    );
    let r = (pos.0, pos.1, pos.0 + ts.0, pos.1 + ts.1);
    if p.b("auto_hide") && !(r.0 >= 0 && r.1 >= 0 && r.2 <= src.w && r.3 <= src.h) {
        return Ok(src.clone());
    }
    let Some(path) = path.transform(Transform::from_translate(
        r.0 as f32 - b.left(),
        r.1 as f32 - b.top(),
    )) else {
        return Ok(src.clone());
    };
    let mut canvas = Canvas::from_bitmap(src);
    let outline = p.b("outline") && p.i("outline_size") > 0;
    let stroke = |canvas: &mut Canvas, path: &Path, color: C| {
        if color.a == 0 {
            return;
        }
        let paint = Paint {
            style: Style::Stroke,
            stroke_width: 1.max(p.i("outline_size")) as f32,
            join: Join::Round,
            ..Paint::fill_aa(color)
        };
        canvas.draw_path(path, &paint);
    };
    let fill = |canvas: &mut Canvas, path: &Path, color: C| {
        if color.a != 0 {
            canvas.draw_path(path, &Paint::fill_aa(color));
        }
    };
    let shadow = p.c("shadow_color");
    if p.b("shadow") && shadow.a > 0 {
        let off =
            Transform::from_translate(p.i("shadow_offset_x") as f32, p.i("shadow_offset_y") as f32);
        if let Some(sp) = path.clone().transform(off) {
            if outline {
                stroke(&mut canvas, &sp, shadow);
            } else {
                fill(&mut canvas, &sp, shadow);
            }
        }
    }
    if outline {
        stroke(&mut canvas, &path, p.c("outline_color"));
    }
    fill(&mut canvas, &path, p.c("color"));
    Ok(canvas.into_bitmap())
}

fn text_watermark(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let size = p.f("font_size");
    if size < 1.0 {
        return Ok(src.clone());
    }
    let text = expand_text_variables(&p.s("text"), (src.w, src.h));
    if text.trim().is_empty() {
        return Ok(src.clone());
    }
    let pl = 0.max(p.i("padding_left"));
    let pt = 0.max(p.i("padding_top"));
    let pr = 0.max(p.i("padding_right"));
    let pbm = 0.max(p.i("padding_bottom"));
    let font = font_for(p, size);
    let lines = split_lines(&text);
    let (ascent, descent, leading) = metrics(&font);
    let raw_h = (descent - ascent).max(font.size);
    let line_h = (raw_h + leading).max(font.size);
    let base = -ascent;
    let max_w = lines.iter().map(|l| font.measure(l)).fold(0f32, f32::max);
    let total_h = raw_h + (lines.len().max(1) - 1) as f32 * line_h;
    if max_w <= 0.0 && total_h <= 0.0 {
        return Ok(src.clone());
    }
    let ws = (
        (max_w + (pl + pr) as f32).ceil() as i32,
        (total_h + (pt + pbm) as f32).ceil() as i32,
    );
    if ws.0 <= 0 || ws.1 <= 0 {
        return Ok(src.clone());
    }
    let pos = position(
        &p.e("placement"),
        (p.i("offset_x"), p.i("offset_y")),
        (src.w, src.h),
        ws,
    );
    let r = (pos.0, pos.1, pos.0 + ws.0, pos.1 + ws.1);
    if p.b("auto_hide") && !(r.0 >= 0 && r.1 >= 0 && r.2 <= src.w && r.3 <= src.h) {
        return Ok(src.clone());
    }
    let mut canvas = Canvas::from_bitmap(src);
    let radius = (0.max(p.i("corner_radius")) as f32).min(ws.0.min(ws.1) as f32 / 2.0);
    let bg_rect = rect(r.0 as f32, r.1 as f32, r.2 as f32, r.3 as f32);
    let bg = p.c("background_color");
    if p.b("draw_background") && bg.a > 0 {
        canvas.draw_round_rect(bg_rect, radius, radius, &Paint::fill_aa(bg));
    }
    let border = p.c("border_color");
    if p.b("draw_border") && border.a > 0 {
        let w = 1.max(0.max(p.i("border_size"))) as f32;
        canvas.draw_round_rect(bg_rect, radius, radius, &Paint::stroke_aa(border, w));
    }
    let tx = (r.0 + pl) as f32;
    let ty = (r.1 + pt) as f32 + base;
    let draw_lines = |canvas: &mut Canvas, x: f32, y: f32, color: C| {
        for (i, line) in lines.iter().enumerate() {
            if !line.is_empty() {
                canvas.draw_text(
                    line,
                    x,
                    y + i as f32 * line_h,
                    &font,
                    &Paint::fill_aa(color),
                );
            }
        }
    };
    let shadow = p.c("text_shadow_color");
    if p.b("draw_text_shadow") && shadow.a > 0 {
        draw_lines(
            &mut canvas,
            tx + p.i("text_shadow_offset_x") as f32,
            ty + p.i("text_shadow_offset_y") as f32,
            shadow,
        );
    }
    draw_lines(&mut canvas, tx, ty, p.c("text_color"));
    Ok(canvas.into_bitmap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_variables_expand() {
        let s = expand_text_variables("%WIDTH x %height%n%y", (640, 480));
        let year = Local::now().year().to_string();
        assert_eq!(s, format!("640 x 480\n{year}"));
    }

    #[test]
    fn positions_follow_placement() {
        assert_eq!(position("TopLeft", (5, 6), (100, 80), (10, 10)), (5, 6));
        assert_eq!(
            position("BottomRight", (5, 5), (100, 80), (10, 10)),
            (85, 65)
        );
        assert_eq!(
            position("MiddleCenter", (0, 0), (101, 80), (10, 10)),
            (46, 35)
        );
    }
}
