//! `Core/ImageEffects/Manipulations` (32 effects; `RemoveBackgroundImageEffect`, filed
//! under Filters upstream but in this category, is in `remove_background.rs`).

use modlight_core::icons as l;
use modlight_editor::effects::Category;

use crate::def::*;
use crate::helpers::{
    self as h, bilinear_sample, clamp_to_byte, multiply_rgb, percentage_to_x, percentage_to_y,
    sample_transparent, smooth_step, wrap_angle,
};
use crate::sk::canvas::{rotation, scale_pixels};
use crate::sk::{Bitmap, C, Canvas, Paint, Sampling, rect, round, roundd};

const M: Category = Category::Manipulations;
use std::f32::consts::PI;

pub fn defs() -> Vec<Def> {
    vec![
        Def::new(
            "auto_crop_image",
            "Auto crop image",
            M,
            l::SCAN,
            "Automatically crops the image using tolerance on edge pixels.",
            auto_crop,
        )
        .params(|| vec![int_slider("tolerance", "Tolerance", 0, 255, 0)]),
        Def::new(
            "crop_image",
            "Crop image",
            M,
            l::CROP,
            "Crops the image.",
            crop,
        )
        .params(|| {
            vec![
                int_numeric("x", "X", 0, 100000, 0),
                int_numeric("y", "Y", 0, 100000, 0),
                int_numeric("width", "Width", 1, 100000, 100),
                int_numeric("height", "Height", 1, 100000, 100),
            ]
        }),
        Def::new(
            "cylinder_wrap",
            "Cylinder wrap",
            M,
            l::CYLINDER,
            "Wraps the image around a cylindrical surface.",
            cylinder_wrap,
        )
        .params(|| {
            vec![
                choice(
                    "orientation",
                    "Orientation",
                    &[("Vertical", "Vertical"), ("Horizontal", "Horizontal")],
                    "Vertical",
                ),
                float_slider("curvature", "Curvature", 0.0, 100.0, 65.0),
                float_slider("edge_shading", "Edge shading", 0.0, 100.0, 35.0),
            ]
        }),
        Def::new(
            "displacement_map",
            "Displacement map",
            M,
            l::MAP,
            "Displaces pixels using a map image.",
            displacement_map,
        )
        .params(|| {
            vec![
                file_path("map_file_path", "Map file path", "", None),
                float_slider("amount_x", "Amount X", -200.0, 200.0, 20.0),
                float_slider("amount_y", "Amount Y", -200.0, 200.0, 20.0),
            ]
        }),
        Def::new(
            "fisheye_lens",
            "Fisheye lens",
            M,
            l::CIRCLE_GAUGE,
            "Applies a fisheye lens distortion effect.",
            fisheye,
        )
        .params(|| {
            vec![
                float_slider("strength", "Strength", 0.0, 100.0, 58.0),
                float_slider("radius_percentage", "Radius percentage", 1.0, 100.0, 100.0),
                float_slider(
                    "center_x_percentage",
                    "Center X percentage",
                    0.0,
                    100.0,
                    50.0,
                ),
                float_slider(
                    "center_y_percentage",
                    "Center Y percentage",
                    0.0,
                    100.0,
                    50.0,
                ),
            ]
        }),
        Def::new(
            "flip",
            "Flip",
            M,
            l::FLIP_HORIZONTAL,
            "Flips the image horizontally or vertically.",
            flip,
        )
        .params(|| {
            vec![
                checkbox("horizontally", "Horizontally", false),
                checkbox("vertically", "Vertically", false),
            ]
        }),
        Def::new(
            "flip_horizontal",
            "Flip horizontal",
            M,
            l::FLIP_HORIZONTAL,
            "Flips the image horizontally.",
            |s, _| Ok(flip_with(s, true, false, false)),
        )
        .immediate(),
        Def::new(
            "flip_vertical",
            "Flip vertical",
            M,
            l::FLIP_VERTICAL,
            "Flips the image vertically.",
            |s, _| Ok(flip_with(s, false, true, false)),
        )
        .immediate(),
        Def::new(
            "fold_crease_warp",
            "Fold crease warp",
            M,
            l::NEWSPAPER,
            "Simulates a folded paper crease distortion.",
            fold_crease,
        )
        .params(|| {
            vec![
                choice(
                    "orientation",
                    "Orientation",
                    &[("Vertical", "Vertical"), ("Horizontal", "Horizontal")],
                    "Vertical",
                ),
                int_slider("fold_count", "Fold count", 1, 10, 3),
                float_slider("fold_depth", "Fold depth", 0.0, 100.0, 30.0),
                float_slider("shadow_strength", "Shadow strength", 0.0, 100.0, 40.0),
            ]
        }),
        Def::new(
            "glass_sphere",
            "Glass sphere",
            M,
            l::GLOBE,
            "Applies a spherical glass lens distortion at the center.",
            glass_sphere,
        )
        .params(|| {
            vec![
                float_slider("radius", "Radius %", 10.0, 100.0, 50.0),
                fine(
                    float_slider("refraction", "Refraction", 0.1, 2.0, 1.5),
                    0.05,
                    "{}{0:0.00}",
                ),
            ]
        }),
        Def::new(
            "kaleidoscope",
            "Kaleidoscope",
            M,
            l::LOADER_PINWHEEL,
            "Creates a kaleidoscope mirror pattern.",
            kaleidoscope,
        )
        .params(|| {
            vec![
                int_slider("segments", "Segments", 2, 24, 8),
                float_slider("rotation", "Rotation", 0.0, 360.0, 0.0),
                float_slider("zoom", "Zoom", 20.0, 300.0, 100.0),
                float_slider(
                    "center_x_percentage",
                    "Center X percentage",
                    0.0,
                    100.0,
                    50.0,
                ),
                float_slider(
                    "center_y_percentage",
                    "Center Y percentage",
                    0.0,
                    100.0,
                    50.0,
                ),
            ]
        }),
        Def::new(
            "liquify_push_smudge",
            "Liquify push / smudge",
            M,
            l::DROPLET,
            "Pushes and smudges pixels in a direction.",
            liquify,
        )
        .params(|| {
            vec![
                float_slider("angle", "Angle", 0.0, 360.0, 0.0),
                float_slider("distance", "Distance", -200.0, 200.0, 40.0),
                float_slider("radius_percentage", "Radius percentage", 1.0, 100.0, 25.0),
                float_slider("smudge", "Smudge", 0.0, 100.0, 35.0),
                float_slider(
                    "center_x_percentage",
                    "Center X percentage",
                    0.0,
                    100.0,
                    50.0,
                ),
                float_slider(
                    "center_y_percentage",
                    "Center Y percentage",
                    0.0,
                    100.0,
                    50.0,
                ),
            ]
        }),
        Def::new(
            "mirror_tiles",
            "Mirror tiles",
            M,
            l::FLIP_HORIZONTAL_2,
            "Tiles the image with alternating mirrored reflections.",
            mirror_tiles,
        )
        .params(|| {
            vec![
                int_slider("columns", "Columns", 1, 16, 3),
                int_slider("rows", "Rows", 1, 16, 3),
                checkbox("mirror_alternate_columns", "Mirror alternate columns", true),
                checkbox("mirror_alternate_rows", "Mirror alternate rows", true),
            ]
        }),
        Def::new(
            "page_curl",
            "Page curl",
            M,
            l::ROTATE_CW_SQUARE,
            "Simulates a page curl at a corner of the image.",
            page_curl,
        )
        .params(|| {
            vec![
                choice(
                    "corner",
                    "Corner",
                    &[
                        ("Top left", "TopLeft"),
                        ("Top right", "TopRight"),
                        ("Bottom right", "BottomRight"),
                        ("Bottom left", "BottomLeft"),
                    ],
                    "BottomRight",
                ),
                float_slider("curl_size", "Curl size", 5.0, 80.0, 28.0),
                float_slider("curl_depth", "Curl depth", 0.0, 100.0, 55.0),
                float_slider("shadow_strength", "Shadow strength", 0.0, 100.0, 60.0),
                color("back_color", "Back color", C::rgb(248, 244, 236)),
            ]
        }),
        Def::new(
            "perspective_warp",
            "Perspective warp",
            M,
            l::MOVE_DIAGONAL,
            "Applies a perspective warp transformation.",
            perspective_warp,
        )
        .params(|| {
            vec![
                int_numeric("top_left_x", "Top left X", -500, 500, 0),
                int_numeric("top_left_y", "Top left Y", -500, 500, 0),
                int_numeric("top_right_x", "Top right X", -500, 500, 0),
                int_numeric("top_right_y", "Top right Y", -500, 500, 0),
                int_numeric("bottom_right_x", "Bottom right X", -500, 500, 0),
                int_numeric("bottom_right_y", "Bottom right Y", -500, 500, 0),
                int_numeric("bottom_left_x", "Bottom left X", -500, 500, 0),
                int_numeric("bottom_left_y", "Bottom left Y", -500, 500, 0),
            ]
        }),
        Def::new(
            "pinch_bulge",
            "Pinch / bulge",
            M,
            l::MAXIMIZE,
            "Applies a pinch or bulge distortion to the image.",
            pinch_bulge,
        )
        .params(|| {
            vec![
                float_slider("strength", "Strength", -100.0, 100.0, 35.0),
                float_slider("radius_percentage", "Radius %", 1.0, 100.0, 50.0),
                float_slider("center_x_percentage", "Center X %", 0.0, 100.0, 50.0),
                float_slider("center_y_percentage", "Center Y %", 0.0, 100.0, 50.0),
            ]
        }),
        Def::new(
            "polar_warp",
            "Polar warp",
            M,
            l::ORBIT,
            "Converts between Cartesian and polar coordinate systems.",
            polar_warp,
        )
        .params(|| {
            vec![
                choice(
                    "mode",
                    "Mode",
                    &[
                        ("Cartesian to polar", "CartesianToPolar"),
                        ("Polar to Cartesian", "PolarToCartesian"),
                    ],
                    "CartesianToPolar",
                ),
                float_slider("rotation", "Rotation", -360.0, 360.0, 0.0),
                float_slider("radius_scale", "Radius scale", 20.0, 200.0, 100.0),
                float_slider("center_x_percentage", "Center X %", 0.0, 100.0, 50.0),
                float_slider("center_y_percentage", "Center Y %", 0.0, 100.0, 50.0),
            ]
        }),
        Def::new(
            "resize_canvas",
            "Resize canvas",
            M,
            l::EXPAND,
            "Resizes the canvas.",
            resize_canvas,
        )
        .params(|| {
            vec![
                int_numeric("top", "Top", -10000, 10000, 0),
                int_numeric("right", "Right", -10000, 10000, 0),
                int_numeric("bottom", "Bottom", -10000, 10000, 0),
                int_numeric("left", "Left", -10000, 10000, 0),
                color("background_color", "Background color", C::TRANSPARENT),
            ]
        }),
        Def::new(
            "resize_image",
            "Resize image",
            M,
            l::SCALING,
            "Resizes the image.",
            resize_image,
        )
        .params(|| {
            vec![
                int_numeric("width", "Width", 0, 10000, 0),
                int_numeric("height", "Height", 0, 10000, 0),
                checkbox("maintain_aspect_ratio", "Maintain aspect ratio", false),
            ]
        }),
        Def::new(
            "ripple_refraction",
            "Ripple refraction",
            M,
            l::WAVES,
            "Applies a ripple refraction distortion effect.",
            ripple_refraction,
        )
        .params(|| {
            vec![
                float_slider("amplitude", "Amplitude", 0.0, 80.0, 12.0),
                float_slider("wavelength", "Wavelength", 4.0, 400.0, 32.0),
                float_slider("phase", "Phase", -360.0, 360.0, 0.0),
                float_slider("refraction", "Refraction", 0.0, 100.0, 35.0),
                float_slider("center_x_percentage", "Center X %", 0.0, 100.0, 50.0),
                float_slider("center_y_percentage", "Center Y %", 0.0, 100.0, 50.0),
            ]
        }),
        Def::new(
            "rotate_custom_angle",
            "Rotate",
            M,
            l::ROTATE_CW,
            "Rotates the image by a custom angle.",
            rotate,
        )
        .params(|| {
            vec![
                float_slider("angle", "Angle", -360.0, 360.0, 0.0),
                checkbox("auto_resize", "Auto resize", true),
            ]
        }),
        Def::new(
            "rotate_180",
            "Rotate 180\u{b0}",
            M,
            l::ROTATE_CW,
            "Rotates the image by 180 degrees.",
            |s, _| Ok(rotate_fixed(s, 180)),
        )
        .immediate(),
        Def::new(
            "rotate_90_clockwise",
            "Rotate 90\u{b0} clockwise",
            M,
            l::ROTATE_CW,
            "Rotates the image 90 degrees clockwise.",
            |s, _| Ok(rotate_fixed(s, 90)),
        )
        .immediate(),
        Def::new(
            "rotate_90_counter_clockwise",
            "Rotate 90\u{b0} counter clockwise",
            M,
            l::ROTATE_CCW,
            "Rotates the image 90 degrees counter-clockwise.",
            |s, _| Ok(rotate_fixed(s, 270)),
        )
        .immediate(),
        Def::new(
            "rounded_corners",
            "Rounded Corners",
            M,
            l::SQUARE_ROUND_CORNER,
            "Rounds the corners of the image.",
            rounded_corners,
        )
        .params(|| vec![int_slider("corner_radius", "Corner radius", 0, 500, 20)]),
        Def::new(
            "scale",
            "Scale",
            M,
            l::SCALING,
            "Scales the image by width and height percentages.",
            scale,
        )
        .params(|| {
            vec![
                float_slider("width_percentage", "Width %", 0.0, 500.0, 100.0),
                float_slider("height_percentage", "Height %", 0.0, 500.0, 0.0),
            ]
        }),
        Def::new(
            "skew",
            "Skew",
            M,
            l::MOVE_DIAGONAL,
            "Skews the image horizontally and vertically.",
            skew,
        )
        .params(|| {
            vec![
                float_slider("horizontally", "Horizontally", -89.0, 89.0, 0.0),
                float_slider("vertically", "Vertically", -89.0, 89.0, 0.0),
                checkbox("auto_resize", "Auto resize", true),
            ]
        }),
        Def::new(
            "symmetry",
            "Symmetry",
            M,
            l::FLIP_HORIZONTAL_2,
            "Creates bilateral symmetry by mirroring one half of the image.",
            symmetry,
        )
        .params(|| {
            vec![choice(
                "direction",
                "Direction",
                &[
                    ("Left \u{2192} Right", "LeftToRight"),
                    ("Right \u{2192} Left", "RightToLeft"),
                    ("Top \u{2192} Bottom", "TopToBottom"),
                    ("Bottom \u{2192} Top", "BottomToTop"),
                ],
                "LeftToRight",
            )]
        }),
        Def::new(
            "tile_repeat",
            "Tile repeat",
            M,
            l::LAYOUT_GRID,
            "Tiles the image into a grid of smaller copies.",
            tile_repeat,
        )
        .params(|| {
            vec![
                int_slider("columns", "Columns", 2, 10, 3),
                int_slider("rows", "Rows", 2, 10, 3),
            ]
        }),
        Def::new(
            "tile_shuffle",
            "Tile Shuffle",
            M,
            l::SHUFFLE,
            "Divides the image into a grid and randomly rearranges the tiles.",
            tile_shuffle,
        )
        .params(|| {
            vec![
                int_slider("columns", "Columns", 2, 20, 4),
                int_slider("rows", "Rows", 2, 20, 4),
                float_slider("gap", "Gap", 0.0, 10.0, 0.0),
                color("gap_color", "Gap Color", C::BLACK),
            ]
        }),
        Def::new(
            "twirl",
            "Twirl",
            M,
            l::LOADER_PINWHEEL,
            "Applies a twirl distortion to the image.",
            twirl,
        )
        .params(|| {
            vec![
                float_slider("angle", "Angle", -360.0, 360.0, 90.0),
                float_slider("radius_percentage", "Radius %", 1.0, 100.0, 50.0),
                float_slider("center_x_percentage", "Center X %", 0.0, 100.0, 50.0),
                float_slider("center_y_percentage", "Center Y %", 0.0, 100.0, 50.0),
            ]
        }),
        Def::new(
            "wave_distortion",
            "Wave distortion",
            M,
            l::WAVES,
            "Applies a sine wave distortion to the image.",
            wave_distortion,
        )
        .params(|| {
            vec![
                int_slider("amplitude", "Amplitude", 1, 100, 20),
                int_slider("wavelength", "Wavelength", 10, 500, 100),
                choice(
                    "direction",
                    "Direction",
                    &[
                        ("Horizontal", "Horizontal"),
                        ("Vertical", "Vertical"),
                        ("Both", "Both"),
                    ],
                    "Horizontal",
                ),
            ]
        }),
    ]
}

/// `ImageHelpers.Crop` (`ExtractSubset` of the part inside the image).
pub fn crop_rect(source: &Bitmap, x: i32, y: i32, w: i32, h: i32) -> Bitmap {
    let l = x.max(0);
    let t = y.max(0);
    let r = (x + w).min(source.w);
    let b = (y + h).min(source.h);
    if r - l <= 0 || b - t <= 0 {
        return Bitmap::new(0, 0);
    }
    let mut out = Bitmap::new(r - l, b - t);
    for yy in t..b {
        for xx in l..r {
            out.px[((yy - t) * (r - l) + (xx - l)) as usize] = source.get(xx, yy);
        }
    }
    out
}

fn auto_crop(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let tol = p.i("tolerance");
    Ok(auto_crop_with(src, C::TRANSPARENT, tol))
}

/// `AutoCropImageEffect(color, tolerance)`.
pub fn auto_crop_with(src: &Bitmap, color: C, tol: i32) -> Bitmap {
    let (w, hh) = (src.w, src.h);
    let match_color = if color == C::TRANSPARENT && w > 0 && hh > 0 {
        src.get(0, 0)
    } else {
        color
    };
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (w, hh, 0, 0);
    let mut has = false;
    for y in 0..hh {
        for x in 0..w {
            if !crate::fx::adjustments::colors_match(src.get(x, y), match_color, tol) {
                has = true;
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    if !has {
        return Bitmap::new(1, 1);
    }
    crop_rect(src, min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
}

fn crop(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let cx = p.i("x").clamp(0, src.w);
    let cy = p.i("y").clamp(0, src.h);
    let cw = p.i("width").clamp(0, src.w - cx);
    let ch = p.i("height").clamp(0, src.h - cy);
    if cw <= 0 || ch <= 0 {
        return Ok(src.clone());
    }
    Ok(crop_rect(src, cx, cy, cw, ch))
}

fn cylinder_wrap(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let c01 = p.f("curvature").clamp(0.0, 100.0) / 100.0;
    if c01 <= 0.0 {
        return Ok(src.clone());
    }
    let s01 = p.f("edge_shading").clamp(0.0, 100.0) / 100.0;
    let max_angle = 0.12 + c01 * 1.28;
    let sin_max = max_angle.sin();
    let (w, hh) = (src.w, src.h);
    let wr = 1f32.max((w - 1) as f32);
    let hr = 1f32.max((hh - 1) as f32);
    let vertical = p.e("orientation") == "Vertical";
    let mut px = vec![C::ZERO; src.px.len()];
    for y in 0..hh {
        for x in 0..w {
            let (sx, sy, shade);
            if vertical {
                let projected = (x as f32 / wr) * 2.0 - 1.0;
                let angle = (projected * sin_max).clamp(-1.0, 1.0).asin();
                let u = (angle / max_angle + 1.0) * 0.5;
                sx = u * wr;
                sy = y as f32;
                shade = 1.0 - s01 * (1.0 - angle.cos()) * 0.65;
            } else {
                let projected = (y as f32 / hr) * 2.0 - 1.0;
                let angle = (projected * sin_max).clamp(-1.0, 1.0).asin();
                let v = (angle / max_angle + 1.0) * 0.5;
                sx = x as f32;
                sy = v * hr;
                shade = 1.0 - s01 * (1.0 - angle.cos()) * 0.65;
            }
            let s = bilinear_sample(&src.px, w, hh, sx, sy);
            px[(y * w + x) as usize] = multiply_rgb(s, shade);
        }
    }
    Ok(Bitmap::from_pixels(w, hh, px))
}

fn displacement_map(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, hh) = (src.w, src.h);
    let (right, bottom) = (w - 1, hh - 1);
    let ax = p.f("amount_x");
    let ay = p.f("amount_y");
    if ax.abs() < 0.0001 && ay.abs() < 0.0001 {
        return Ok(src.clone());
    }
    let map = load_map(&p.s("map_file_path"));
    let (map_px, mw, mh) = match &map {
        Some(m) => (&m.px, m.w, m.h),
        None => (&src.px, w, hh),
    };
    let scale = |c: i32, sl: i32, tl: i32| -> i32 {
        if tl <= 1 || sl <= 1 {
            return 0;
        }
        let s = c as f32 * (tl - 1) as f32 / (sl - 1) as f32;
        (round(s) as i32).clamp(0, tl - 1)
    };
    let mut px = vec![C::ZERO; src.px.len()];
    for y in 0..hh {
        for x in 0..w {
            let m = map_px[(scale(y, hh, mh) * mw + scale(x, w, mw)) as usize];
            let dx = ((m.r as f32 / 255.0) - 0.5) * 2.0 * ax;
            let dy = ((m.g as f32 / 255.0) - 0.5) * 2.0 * ay;
            let sx = (round(x as f32 + dx) as i32).clamp(0, right);
            let sy = (round(y as f32 + dy) as i32).clamp(0, bottom);
            px[(y * w + x) as usize] = src.px[(sy * w + sx) as usize];
        }
    }
    Ok(Bitmap::from_pixels(w, hh, px))
}

/// `SKBitmap.Decode` of a user-chosen file (environment variables expanded); `None` when
/// it is missing or not an image.
pub fn load_map(path: &str) -> Option<Bitmap> {
    let path = expand_env(path);
    if path.trim().is_empty() {
        return None;
    }
    let p = std::path::Path::new(&path);
    if !p.is_file() {
        return None;
    }
    let img = modlight_core::image_decode::open(p).ok()?.to_rgba8();
    if img.width() == 0 || img.height() == 0 {
        return None;
    }
    Some(Bitmap::from_image(&img))
}

/// `Environment.ExpandEnvironmentVariables`: `%NAME%` replaced when NAME is set.
pub fn expand_env(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        if let Some(end) = after.find('%') {
            let name = &after[..end];
            if let Some(v) = std::env::var(name).ok().filter(|_| !name.is_empty()) {
                out.push_str(&rest[..start]);
                out.push_str(&v);
                rest = &after[end + 1..];
                continue;
            }
            out.push_str(&rest[..start + 1]);
            rest = after;
        } else {
            break;
        }
    }
    out.push_str(rest);
    out
}

fn fisheye(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let s01 = p.f("strength").clamp(0.0, 100.0) / 100.0;
    if s01 <= 0.0 {
        return Ok(src.clone());
    }
    let (w, hh) = (src.w, src.h);
    let radius = 1f32.max(w.min(hh) as f32 * p.f("radius_percentage").clamp(1.0, 100.0) / 100.0);
    let cx = percentage_to_x(w, p.f("center_x_percentage"));
    let cy = percentage_to_y(hh, p.f("center_y_percentage"));
    let exponent = 1.0 + s01 * 2.4;
    let mut px = vec![C::ZERO; src.px.len()];
    for y in 0..hh {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            if d <= 0.0001 || d >= radius {
                px[i] = src.px[i];
                continue;
            }
            let n = d / radius;
            let sd = n.powf(exponent) * radius;
            let sc = sd / d;
            px[i] = bilinear_sample(&src.px, w, hh, cx + dx * sc, cy + dy * sc);
        }
    }
    Ok(Bitmap::from_pixels(w, hh, px))
}

fn flip(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let hz = p.b("horizontally");
    let vt = p.b("vertically");
    if !hz && !vt {
        return Ok(src.clone());
    }
    Ok(flip_with(src, hz, vt, true))
}

/// The flips: drawn mirrored about the centre (an exact pixel move).
pub fn flip_with(src: &Bitmap, horizontal: bool, vertical: bool, _clear: bool) -> Bitmap {
    let mut canvas = Canvas::new(src.w, src.h);
    let (cx, cy) = (src.w as f32 / 2.0, src.h as f32 / 2.0);
    let sx = if horizontal { -1.0 } else { 1.0 };
    let sy = if vertical { -1.0 } else { 1.0 };
    canvas.scale_about(sx, sy, cx, cy);
    canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    canvas.into_bitmap()
}

fn fold_crease(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let fold_count = p.i("fold_count").clamp(1, 10);
    let d01 = p.f("fold_depth").clamp(0.0, 100.0) / 100.0;
    if d01 <= 0.0 {
        return Ok(src.clone());
    }
    let sh01 = p.f("shadow_strength").clamp(0.0, 100.0) / 100.0;
    let (w, hh) = (src.w, src.h);
    let vertical = p.e("orientation") == "Vertical";
    let span = if vertical {
        1f32.max((w - 1) as f32)
    } else {
        1f32.max((hh - 1) as f32)
    } / fold_count as f32;
    let disp = span * (0.06 + d01 * 0.22);
    let mut px = vec![C::ZERO; src.px.len()];
    for y in 0..hh {
        for x in 0..w {
            let primary = if vertical { x as f32 } else { y as f32 };
            let phase = (primary / 1f32.max(span)) * PI;
            let offset = phase.sin() * disp;
            let (sx, sy) = if vertical {
                (x as f32 + offset, y as f32)
            } else {
                (x as f32, y as f32 + offset)
            };
            let s = bilinear_sample(&src.px, w, hh, sx, sy);
            let curvature = phase.sin().abs();
            let slope = phase.cos();
            let shade = 1.0 - sh01 * curvature * 0.22 + sh01 * slope * 0.08;
            px[(y * w + x) as usize] = multiply_rgb(s, shade);
        }
    }
    Ok(Bitmap::from_pixels(w, hh, px))
}

fn glass_sphere(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, hh) = (src.w, src.h);
    let (cx, cy) = (w as f32 / 2.0, hh as f32 / 2.0);
    let radius = w.min(hh) as f32 * p.f("radius") / 200.0;
    let r2 = radius * radius;
    let refraction = p.f("refraction");
    let mut dst = src.px.clone();
    for y in 0..hh {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let d2 = dx * dx + dy * dy;
            if d2 >= r2 {
                continue;
            }
            let d = d2.sqrt();
            let nd = d / radius;
            let theta = nd.asin();
            let new_d = radius * (theta * refraction).sin() / (theta + 0.0001).sin();
            let ratio = if d > 0.0001 { new_d / d } else { 1.0 };
            let sx = (cx + dx * ratio) as i32;
            let sy = (cy + dy * ratio) as i32;
            if sx >= 0 && sx < w && sy >= 0 && sy < hh {
                dst[(y * w + x) as usize] = src.px[(sy * w + sx) as usize];
            }
        }
    }
    Ok(Bitmap::from_pixels(w, hh, dst))
}

fn kaleidoscope(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let segs = p.i("segments").clamp(2, 24);
    let zoom = p.f("zoom").clamp(20.0, 300.0) / 100.0;
    let rot = p.f("rotation") * (PI / 180.0);
    let cx = percentage_to_x(src.w, p.f("center_x_percentage"));
    let cy = percentage_to_y(src.h, p.f("center_y_percentage"));
    let seg_angle = (PI * 2.0) / segs as f32;
    let (w, hh) = (src.w, src.h);
    let mut px = vec![C::ZERO; src.px.len()];
    for y in 0..hh {
        for x in 0..w {
            let dx = (x as f32 - cx) / zoom;
            let dy = (y as f32 - cy) / zoom;
            let d = (dx * dx + dy * dy).sqrt();
            let angle = wrap_angle(dy.atan2(dx) - rot);
            let idx = (angle / seg_angle).floor() as i32;
            let mut local = angle - idx as f32 * seg_angle;
            if idx & 1 == 1 {
                local = seg_angle - local;
            }
            let sa = local + rot;
            px[(y * w + x) as usize] =
                bilinear_sample(&src.px, w, hh, cx + sa.cos() * d, cy + sa.sin() * d);
        }
    }
    Ok(Bitmap::from_pixels(w, hh, px))
}

fn liquify(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let distance = p.f("distance").clamp(-200.0, 200.0);
    if distance.abs() <= 0.01 {
        return Ok(src.clone());
    }
    let (w, hh) = (src.w, src.h);
    let radius = 1f32.max(w.min(hh) as f32 * p.f("radius_percentage").clamp(1.0, 100.0) / 100.0);
    let sm01 = p.f("smudge").clamp(0.0, 100.0) / 100.0;
    let samples = 1 + round(sm01 * 5.0) as i32;
    let angle = p.f("angle") * (PI / 180.0);
    let (dir_x, dir_y) = (angle.cos(), angle.sin());
    let cx = percentage_to_x(w, p.f("center_x_percentage"));
    let cy = percentage_to_y(hh, p.f("center_y_percentage"));
    let mut px = vec![C::ZERO; src.px.len()];
    for y in 0..hh {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let pd = (dx * dx + dy * dy).sqrt();
            if pd >= radius {
                px[i] = src.px[i];
                continue;
            }
            let n = pd / radius;
            let mut infl = 1.0 - smooth_step(0.0, 1.0, n);
            infl *= infl;
            let push = distance * infl;
            if samples <= 1 {
                px[i] = bilinear_sample(
                    &src.px,
                    w,
                    hh,
                    x as f32 - dir_x * push,
                    y as f32 - dir_y * push,
                );
                continue;
            }
            let (mut sr, mut sg, mut sb, mut sa) = (0f32, 0f32, 0f32, 0f32);
            for k in 0..samples {
                let t = if samples == 1 {
                    0.0
                } else {
                    k as f32 / (samples - 1) as f32
                };
                let off = push * (1.0 - t * sm01);
                let s = bilinear_sample(
                    &src.px,
                    w,
                    hh,
                    x as f32 - dir_x * off,
                    y as f32 - dir_y * off,
                );
                sr += s.r as f32;
                sg += s.g as f32;
                sb += s.b as f32;
                sa += s.a as f32;
            }
            let inv = 1.0 / samples as f32;
            px[i] = C::new(
                clamp_to_byte(sr * inv),
                clamp_to_byte(sg * inv),
                clamp_to_byte(sb * inv),
                clamp_to_byte(sa * inv),
            );
        }
    }
    Ok(Bitmap::from_pixels(w, hh, px))
}

fn mirror_tiles(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let cols = p.i("columns").clamp(1, 16);
    let rows = p.i("rows").clamp(1, 16);
    let mc = p.b("mirror_alternate_columns");
    let mr = p.b("mirror_alternate_rows");
    if cols == 1 && rows == 1 && !mc && !mr {
        return Ok(src.clone());
    }
    let (w, hh) = (src.w, src.h);
    let mut px = vec![C::ZERO; src.px.len()];
    for y in 0..hh {
        let ty = ((y as f32 + 0.5) / hh as f32) * rows as f32;
        let tile_y = (rows - 1).min(0.max(ty.floor() as i32));
        let mut ly = ty - tile_y as f32;
        if tile_y == rows - 1 {
            ly = ly.min(1.0);
        }
        if mr && tile_y & 1 == 1 {
            ly = 1.0 - ly;
        }
        let sy = ly * 0.max(hh - 1) as f32;
        for x in 0..w {
            let tx = ((x as f32 + 0.5) / w as f32) * cols as f32;
            let tile_x = (cols - 1).min(0.max(tx.floor() as i32));
            let mut lx = tx - tile_x as f32;
            if tile_x == cols - 1 {
                lx = lx.min(1.0);
            }
            if mc && tile_x & 1 == 1 {
                lx = 1.0 - lx;
            }
            let sx = lx * 0.max(w - 1) as f32;
            px[(y * w + x) as usize] = bilinear_sample(&src.px, w, hh, sx, sy);
        }
    }
    Ok(Bitmap::from_pixels(w, hh, px))
}

fn page_curl(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, hh) = (src.w, src.h);
    let curl_size = 12f32.max(w.min(hh) as f32 * p.f("curl_size").clamp(5.0, 80.0) / 100.0);
    let depth = p.f("curl_depth").clamp(0.0, 100.0) / 100.0;
    let shadow = p.f("shadow_strength").clamp(0.0, 100.0) / 100.0;
    let fold = curl_size * (0.78 + depth * 0.22);
    let shadow_w = 4f32.max(curl_size * (0.08 + shadow * 0.20 + depth * 0.10));
    let back = p.c("back_color");
    let corner = p.e("corner");
    let to_local = |x: f32, y: f32| -> (f32, f32) {
        match corner.as_str() {
            "TopLeft" => (x, y),
            "TopRight" => ((w - 1) as f32 - x, y),
            "BottomRight" => ((w - 1) as f32 - x, (hh - 1) as f32 - y),
            _ => (x, (hh - 1) as f32 - y),
        }
    };
    let mut px = vec![C::ZERO; src.px.len()];
    for y in 0..hh {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let original = src.px[i];
            let (lx, ly) = to_local(x as f32, y as f32);
            if lx > curl_size + shadow_w || ly > curl_size + shadow_w {
                px[i] = original;
                continue;
            }
            let diag = lx + ly;
            if diag <= fold {
                let rx = fold - ly;
                let ry = fold - lx;
                // FromCornerLocal is the same mapping (an involution).
                let (sx, sy) = to_local(rx, ry);
                let reflected = bilinear_sample(&src.px, w, hh, sx, sy);
                let mix = 0.28 + depth * 0.28 + (1.0 - diag / fold) * 0.18;
                let curled = h::blend(reflected, back, mix);
                let highlight = (0.05 + depth * 0.09) * smooth_step(fold * 0.55, 0.0, diag);
                px[i] = h::blend(curled, C::new(255, 255, 255, curled.a), highlight);
            } else if diag < fold + shadow_w {
                let fade = 1.0 - (diag - fold) / shadow_w;
                let shade = 1.0 - shadow * fade * 0.42;
                px[i] = multiply_rgb(original, shade);
            } else {
                px[i] = original;
            }
        }
    }
    Ok(Bitmap::from_pixels(w, hh, px))
}

fn perspective_warp(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let keys = [
        "top_left_x",
        "top_left_y",
        "top_right_x",
        "top_right_y",
        "bottom_right_x",
        "bottom_right_y",
        "bottom_left_x",
        "bottom_left_y",
    ];
    let v: Vec<f32> = keys.iter().map(|k| p.i(k) as f32).collect();
    if v.iter().all(|x| *x == 0.0) {
        return Ok(src.clone());
    }
    let (w, hh) = (src.w, src.h);
    let (right, bottom) = (w - 1, hh - 1);
    let (rf, bf) = (right as f32, bottom as f32);
    let src_quad = [(0.0, 0.0), (rf, 0.0), (rf, bf), (0.0, bf)];
    let dst_quad = [
        (v[0], v[1]),
        (rf + v[2], v[3]),
        (rf + v[4], bf + v[5]),
        (v[6], bf + v[7]),
    ];
    let Some(inv) = homography(&dst_quad, &src_quad) else {
        return Ok(src.clone());
    };
    let mut px = vec![C::ZERO; src.px.len()];
    for y in 0..hh {
        for x in 0..w {
            let (xd, yd) = (x as f64, y as f64);
            let wv = inv[6] * xd + inv[7] * yd + inv[8];
            let i = (y * w + x) as usize;
            if wv.abs() < 1e-8 {
                px[i] = C::TRANSPARENT;
                continue;
            }
            let sx = ((inv[0] * xd + inv[1] * yd + inv[2]) / wv) as f32;
            let sy = ((inv[3] * xd + inv[4] * yd + inv[5]) / wv) as f32;
            if sx < 0.0 || sy < 0.0 || sx > rf || sy > bf {
                px[i] = C::TRANSPARENT;
                continue;
            }
            let ix = (roundd(sx as f64) as i32).clamp(0, right);
            let iy = (roundd(sy as f64) as i32).clamp(0, bottom);
            px[i] = src.px[(iy * w + ix) as usize];
        }
    }
    Ok(Bitmap::from_pixels(w, hh, px))
}

/// `TryComputeHomography`: the 3x3 projective map taking `src` corners to `dst`.
pub fn homography(src: &[(f32, f32); 4], dst: &[(f32, f32); 4]) -> Option<[f64; 9]> {
    let mut aug = [[0f64; 9]; 8];
    for i in 0..4 {
        let (x, y) = (src[i].0 as f64, src[i].1 as f64);
        let (u, v) = (dst[i].0 as f64, dst[i].1 as f64);
        aug[i * 2] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        aug[i * 2 + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    let n = 8;
    for col in 0..n {
        let mut pivot = col;
        let mut max = aug[pivot][col].abs();
        for row in col + 1..n {
            let value = aug[row][col].abs();
            if value > max {
                max = value;
                pivot = row;
            }
        }
        if max < 1e-12 {
            return None;
        }
        if pivot != col {
            aug.swap(col, pivot);
        }
        let div = aug[col][col];
        for j in col..=n {
            aug[col][j] /= div;
        }
        for row in 0..n {
            if row == col {
                continue;
            }
            let factor = aug[row][col];
            if factor.abs() < 1e-12 {
                continue;
            }
            for j in col..=n {
                aug[row][j] -= factor * aug[col][j];
            }
        }
    }
    let mut h = [0f64; 9];
    for i in 0..8 {
        h[i] = aug[i][n];
    }
    h[8] = 1.0;
    Some(h)
}

fn pinch_bulge(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, hh) = (src.w, src.h);
    let (right, bottom) = (w - 1, hh - 1);
    let radius = 1f32.max(w.min(hh) as f32 * p.f("radius_percentage").clamp(1.0, 100.0) / 100.0);
    let cx = p.f("center_x_percentage").clamp(0.0, 100.0) / 100.0 * right as f32;
    let cy = p.f("center_y_percentage").clamp(0.0, 100.0) / 100.0 * bottom as f32;
    let strength = p.f("strength").clamp(-100.0, 100.0) / 100.0;
    if strength.abs() < 0.0001 {
        return Ok(src.clone());
    }
    let mut px = vec![C::ZERO; src.px.len()];
    for y in 0..hh {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            if d >= radius || d <= 0.0001 {
                px[i] = src.px[i];
                continue;
            }
            let n = d / radius;
            let mut amount = 1.0 + strength * (1.0 - n * n);
            if amount < 0.05 {
                amount = 0.05;
            }
            let sd = d / amount;
            let sc = sd / d;
            let sx = (round(cx + dx * sc) as i32).clamp(0, right);
            let sy = (round(cy + dy * sc) as i32).clamp(0, bottom);
            px[i] = src.px[(sy * w + sx) as usize];
        }
    }
    Ok(Bitmap::from_pixels(w, hh, px))
}

fn polar_warp(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, hh) = (src.w, src.h);
    let rs = p.f("radius_scale").clamp(20.0, 200.0) / 100.0;
    let rot = p.f("rotation") * (PI / 180.0);
    let cx = percentage_to_x(w, p.f("center_x_percentage"));
    let cy = percentage_to_y(hh, p.f("center_y_percentage"));
    let max_r = ((w - 1).min(hh - 1) as f32 * 0.5) * rs;
    if max_r <= 0.5 {
        return Ok(src.clone());
    }
    let wr = 1f32.max((w - 1) as f32);
    let hr = 1f32.max((hh - 1) as f32);
    let to_polar = p.e("mode") == "CartesianToPolar";
    let mut px = vec![C::ZERO; src.px.len()];
    for y in 0..hh {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if to_polar {
                let a = (x as f32 / wr) * PI * 2.0 + rot;
                let r = (y as f32 / hr) * max_r;
                px[i] = sample_transparent(&src.px, w, hh, cx + a.cos() * r, cy + a.sin() * r);
                continue;
            }
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            if d > max_r {
                px[i] = C::TRANSPARENT;
                continue;
            }
            let a = wrap_angle(dy.atan2(dx) - rot);
            let sx = (a / (PI * 2.0)) * wr;
            let sy = (d / max_r) * hr;
            px[i] = bilinear_sample(&src.px, w, hh, sx, sy);
        }
    }
    Ok(Bitmap::from_pixels(w, hh, px))
}

fn resize_canvas(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (t, r, b, l) = (p.i("top"), p.i("right"), p.i("bottom"), p.i("left"));
    let nw = src.w + l + r;
    let nh = src.h + t + b;
    if nw <= 0 || nh <= 0 {
        return Ok(src.clone());
    }
    let mut canvas = Canvas::new(nw, nh);
    canvas.clear(p.c("background_color"));
    canvas.draw_bitmap(src, l as f32, t as f32, &Paint::default());
    Ok(canvas.into_bitmap())
}

/// `ResizeImageEffect.ResolveTargetSize`.
pub fn resolve_target_size(
    sw: i32,
    sh: i32,
    rw: i32,
    rh: i32,
    keep: bool,
    anchor: modlight_editor::effects::ResizeAnchor,
) -> (i32, i32) {
    use modlight_editor::effects::ResizeAnchor as A;
    let mut w = rw.max(0);
    let mut hh = rh.max(0);
    if !keep {
        return (if w > 0 { w } else { sw }, if hh > 0 { hh } else { sh });
    }
    if w <= 0 && hh <= 0 {
        return (sw, sh);
    }
    let r = |v: f64| (v.round_ties_even() as i32).max(1);
    if w <= 0 {
        w = r(hh as f64 / sh as f64 * sw as f64);
        return (w, hh);
    }
    if hh <= 0 {
        hh = r(w as f64 / sw as f64 * sh as f64);
        return (w, hh);
    }
    match anchor {
        A::Width => (w, r(w as f64 / sw as f64 * sh as f64)),
        A::Height => (r(hh as f64 / sh as f64 * sw as f64), hh),
        A::LargestDimension => {
            let s = (w as f64 / sw as f64).max(hh as f64 / sh as f64);
            (r(sw as f64 * s), r(sh as f64 * s))
        }
    }
}

fn resize_image(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    use modlight_editor::effects::ResizeAnchor;
    let ws = p.values.sequence.get("width").copied().unwrap_or(0);
    let hs = p.values.sequence.get("height").copied().unwrap_or(0);
    let anchor = if ws == hs {
        ResizeAnchor::LargestDimension
    } else if ws > hs {
        ResizeAnchor::Width
    } else {
        ResizeAnchor::Height
    };
    let (w, hh) = resolve_target_size(
        src.w,
        src.h,
        p.i("width"),
        p.i("height"),
        p.b("maintain_aspect_ratio"),
        anchor,
    );
    Ok(scale_pixels(src, w, hh, Sampling::CATMULL_ROM))
}

fn ripple_refraction(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let amplitude = p.f("amplitude").clamp(0.0, 80.0);
    if amplitude <= 0.01 {
        return Ok(src.clone());
    }
    let wavelength = p.f("wavelength").clamp(4.0, 400.0);
    let phase = p.f("phase") * (PI / 180.0);
    let refr = p.f("refraction").clamp(0.0, 100.0) / 100.0;
    let cx = percentage_to_x(src.w, p.f("center_x_percentage"));
    let cy = percentage_to_y(src.h, p.f("center_y_percentage"));
    let wave_scale = (PI * 2.0) / wavelength;
    let (w, hh) = (src.w, src.h);
    let mut px = vec![C::ZERO; src.px.len()];
    for y in 0..hh {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            if d <= 0.0001 {
                px[i] = src.px[i];
                continue;
            }
            let wp = d * wave_scale + phase;
            let radial = wp.sin() * amplitude;
            let refracted = wp.cos() * amplitude * refr * 0.35;
            let sd = d + radial + refracted;
            let sc = sd / d;
            px[i] = bilinear_sample(&src.px, w, hh, cx + dx * sc, cy + dy * sc);
        }
    }
    Ok(Bitmap::from_pixels(w, hh, px))
}

fn rotate(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let angle = p.f("angle");
    let auto = p.b("auto_resize");
    Ok(rotate_with(src, angle, auto))
}

/// `RotateImageEffect` (`Custom(angle, autoResize)`).
pub fn rotate_with(src: &Bitmap, angle: f32, auto: bool) -> Bitmap {
    if angle % 90.0 == 0.0 && auto {
        return rotate_fixed(src, angle as i32);
    }
    let (w, hh) = (src.w as f32, src.h as f32);
    if auto {
        let m = rotation(angle);
        let m = tiny_skia::Transform::from_translate(w / 2.0, hh / 2.0)
            .pre_concat(m)
            .pre_translate(-w / 2.0, -hh / 2.0);
        let mut pts = [
            tiny_skia::Point::from_xy(0.0, 0.0),
            tiny_skia::Point::from_xy(w, 0.0),
            tiny_skia::Point::from_xy(w, hh),
            tiny_skia::Point::from_xy(0.0, hh),
        ];
        m.map_points(&mut pts);
        let minx = pts.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
        let maxx = pts.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
        let miny = pts.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
        let maxy = pts.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
        let nw = (maxx - minx).ceil() as i32;
        let nh = (maxy - miny).ceil() as i32;
        let mut canvas = Canvas::new(nw, nh);
        canvas.translate(nw as f32 / 2.0, nh as f32 / 2.0);
        canvas.rotate_degrees(angle);
        canvas.translate(-w / 2.0, -hh / 2.0);
        canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
        return canvas.into_bitmap();
    }
    let mut canvas = Canvas::new(src.w, src.h);
    canvas.translate(w / 2.0, hh / 2.0);
    canvas.rotate_degrees(angle);
    canvas.translate(-w / 2.0, -hh / 2.0);
    canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    canvas.into_bitmap()
}

/// `RotateOrthogonal`: multiples of 90 degrees.
pub fn rotate_fixed(src: &Bitmap, angle: i32) -> Bitmap {
    let mut angle = angle % 360;
    if angle < 0 {
        angle += 360;
    }
    let (mut w, mut hh) = (src.w, src.h);
    if angle == 90 || angle == 270 {
        std::mem::swap(&mut w, &mut hh);
    }
    let mut canvas = Canvas::new(w, hh);
    match angle {
        90 => {
            canvas.translate(w as f32, 0.0);
            canvas.rotate_degrees(90.0);
        }
        180 => {
            canvas.translate(w as f32, hh as f32);
            canvas.rotate_degrees(180.0);
        }
        270 => {
            canvas.translate(0.0, hh as f32);
            canvas.rotate_degrees(270.0);
        }
        _ => {}
    }
    canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    canvas.into_bitmap()
}

fn rounded_corners(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let r = p.i("corner_radius");
    if r <= 0 {
        return Ok(src.clone());
    }
    let mut canvas = Canvas::new(src.w, src.h);
    canvas.clip_round_rect(
        rect(0.0, 0.0, src.w as f32, src.h as f32),
        r as f32,
        r as f32,
        true,
    );
    canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    Ok(canvas.into_bitmap())
}

fn scale(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let wp = p.f("width_percentage");
    let hp = p.f("height_percentage");
    if wp <= 0.0 && hp <= 0.0 {
        return Ok(src.clone());
    }
    let mut w = roundd((wp / 100.0 * src.w as f32) as f64) as i32;
    let mut hh = roundd((hp / 100.0 * src.h as f32) as f64) as i32;
    if w == 0 {
        w = roundd((hh as f32 / src.h as f32 * src.w as f32) as f64) as i32;
    } else if hh == 0 {
        hh = roundd((w as f32 / src.w as f32 * src.h as f32) as f64) as i32;
    }
    if w <= 0 || hh <= 0 {
        return Ok(src.clone());
    }
    Ok(scale_pixels(src, w, hh, Sampling::CATMULL_ROM))
}

fn skew(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let hz = p.f("horizontally");
    let vt = p.f("vertically");
    if hz == 0.0 && vt == 0.0 {
        return Ok(src.clone());
    }
    let (w, hh) = (src.w as f32, src.h as f32);
    let kx = (hz as f64 * std::f64::consts::PI / 180.0).tan() as f32;
    let ky = (vt as f64 * std::f64::consts::PI / 180.0).tan() as f32;
    let m = tiny_skia::Transform::from_row(1.0, ky, kx, 1.0, 0.0, 0.0);
    let mut pts = [
        tiny_skia::Point::from_xy(0.0, 0.0),
        tiny_skia::Point::from_xy(w, 0.0),
        tiny_skia::Point::from_xy(w, hh),
        tiny_skia::Point::from_xy(0.0, hh),
    ];
    m.map_points(&mut pts);
    let minx = pts.iter().map(|p| p.x).fold(f32::MAX, f32::min);
    let miny = pts.iter().map(|p| p.y).fold(f32::MAX, f32::min);
    let maxx = pts.iter().map(|p| p.x).fold(f32::MIN, f32::max);
    let maxy = pts.iter().map(|p| p.y).fold(f32::MIN, f32::max);
    let (nw, nh, ox, oy) = if p.b("auto_resize") {
        (
            1.max((maxx - minx).ceil() as i32),
            1.max((maxy - miny).ceil() as i32),
            -minx,
            -miny,
        )
    } else {
        (w as i32, hh as i32, 0.0, 0.0)
    };
    let mut canvas = Canvas::new(nw, nh);
    canvas.translate(ox, oy);
    canvas.concat(m);
    canvas.draw_bitmap(src, 0.0, 0.0, &Paint::default());
    Ok(canvas.into_bitmap())
}

fn symmetry(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, hh) = (src.w as f32, src.h as f32);
    let mut canvas = Canvas::from_bitmap(src);
    let r = match p.e("direction").as_str() {
        "RightToLeft" => {
            canvas.scale_about(-1.0, 1.0, w / 2.0, 0.0);
            rect(w / 2.0, 0.0, w, hh)
        }
        "TopToBottom" => {
            canvas.scale_about(1.0, -1.0, 0.0, hh / 2.0);
            rect(0.0, 0.0, w, hh / 2.0)
        }
        "BottomToTop" => {
            canvas.scale_about(1.0, -1.0, 0.0, hh / 2.0);
            rect(0.0, hh / 2.0, w, hh)
        }
        _ => {
            canvas.scale_about(-1.0, 1.0, w / 2.0, 0.0);
            rect(0.0, 0.0, w / 2.0, hh)
        }
    };
    canvas.draw_bitmap_rect(src, r, r, Sampling::Nearest, &Paint::default());
    Ok(canvas.into_bitmap())
}

fn tile_repeat(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let cols = 1.max(p.i("columns"));
    let rows = 1.max(p.i("rows"));
    let (w, hh) = (src.w, src.h);
    let tw = w as f32 / cols as f32;
    let th = hh as f32 / rows as f32;
    let mut canvas = Canvas::new(w, hh);
    let full = rect(0.0, 0.0, w as f32, hh as f32);
    for r in 0..rows {
        for c in 0..cols {
            let dst = rect(
                c as f32 * tw,
                r as f32 * th,
                (c + 1) as f32 * tw,
                (r + 1) as f32 * th,
            );
            canvas.draw_bitmap_rect(src, full, dst, Sampling::Nearest, &Paint::default());
        }
    }
    Ok(canvas.into_bitmap())
}

fn tile_shuffle(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let cols = p.i("columns").clamp(1, 100);
    let rows = p.i("rows").clamp(1, 100);
    let (w, hh) = (src.w, src.h);
    let gap = p.f("gap").clamp(0.0, 50.0);
    let total = cols * rows;
    let mut idx: Vec<i32> = (0..total).collect();
    let mut rng: i32 = 42;
    for i in (1..total).rev() {
        rng ^= rng.wrapping_shl(13);
        rng ^= rng >> 17;
        rng ^= rng.wrapping_shl(5);
        let j = ((rng & 0x7FFF_FFFF) as u32 % (i + 1) as u32) as usize;
        idx.swap(i as usize, j);
    }
    let tw = w as f32 / cols as f32;
    let th = hh as f32 / rows as f32;
    let mut canvas = Canvas::new(w, hh);
    canvas.clear(p.c("gap_color"));
    for d in 0..total {
        let s = idx[d as usize];
        let (sc, sr) = (s % cols, s / cols);
        let (dc, dr) = (d % cols, d / cols);
        let src_r = rect(
            sc as f32 * tw,
            sr as f32 * th,
            (sc + 1) as f32 * tw,
            (sr + 1) as f32 * th,
        );
        let (l, t, r, b) = (
            dc as f32 * tw + gap * 0.5,
            dr as f32 * th + gap * 0.5,
            (dc + 1) as f32 * tw - gap * 0.5,
            (dr + 1) as f32 * th - gap * 0.5,
        );
        if r - l <= 0.0 || b - t <= 0.0 {
            continue;
        }
        canvas.draw_bitmap_rect(
            src,
            src_r,
            rect(l, t, r, b),
            Sampling::Nearest,
            &Paint::default(),
        );
    }
    Ok(canvas.into_bitmap())
}

fn twirl(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let (w, hh) = (src.w, src.h);
    let (right, bottom) = (w - 1, hh - 1);
    let radius = 1f32.max(w.min(hh) as f32 * p.f("radius_percentage").clamp(1.0, 100.0) / 100.0);
    let cx = p.f("center_x_percentage").clamp(0.0, 100.0) / 100.0 * right as f32;
    let cy = p.f("center_y_percentage").clamp(0.0, 100.0) / 100.0 * bottom as f32;
    let ar = p.f("angle") * PI / 180.0;
    if ar.abs() < 0.0001 {
        return Ok(src.clone());
    }
    let mut px = vec![C::ZERO; src.px.len()];
    for y in 0..hh {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            if d >= radius || d <= 0.0001 {
                px[i] = src.px[i];
                continue;
            }
            let n = d / radius;
            let tw = ar * (1.0 - n);
            let theta = dy.atan2(dx) - tw;
            let sx = (round(cx + d * theta.cos()) as i32).clamp(0, right);
            let sy = (round(cy + d * theta.sin()) as i32).clamp(0, bottom);
            px[i] = src.px[(sy * w + sx) as usize];
        }
    }
    Ok(Bitmap::from_pixels(w, hh, px))
}

fn wave_distortion(src: &Bitmap, p: &P) -> Result<Bitmap, String> {
    let amp = p.i("amplitude");
    if amp <= 0 {
        return Ok(src.clone());
    }
    let (w, hh) = (src.w, src.h);
    let freq = PI * 2.0 / 10.max(p.i("wavelength")) as f32;
    let dir = p.e("direction");
    let hz = dir == "Horizontal" || dir == "Both";
    let vt = dir == "Vertical" || dir == "Both";
    let mut px = vec![C::ZERO; src.px.len()];
    for y in 0..hh {
        for x in 0..w {
            let mut sx = x;
            let mut sy = y;
            if hz {
                sx = x + (amp as f32 * (y as f32 * freq).sin()) as i32;
            }
            if vt {
                sy = y + (amp as f32 * (x as f32 * freq).sin()) as i32;
            }
            px[(y * w + x) as usize] = if sx >= 0 && sx < w && sy >= 0 && sy < hh {
                src.px[(sy * w + sx) as usize]
            } else {
                C::TRANSPARENT
            };
        }
    }
    Ok(Bitmap::from_pixels(w, hh, px))
}
