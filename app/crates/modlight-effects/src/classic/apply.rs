//! `ImageEffect.Apply` of each classic effect and `ImageEffectPreset.ApplyEffects`.

use std::path::{Path, PathBuf};

use image::RgbaImage;
use modlight_core::names::name_parser::{NameParser, NameParserType};
use tiny_skia::{PathBuilder, Transform};

use super::gdi::{self, Fill};
use super::model::{ClassicEffect, FontSpec, Gradient, Kind, Padding, Preset};
use crate::dotnet::shared_random;
use crate::sk::{Bitmap, Blend, C, Canvas, Font, Join, Paint, Sampling, Shader, Style, store};

impl Preset {
    /// `ApplyEffects`: the enabled effects in order on a copy of `image`; `None` when an
    /// effect gives no picture (a crop outside the image), as upstream's null.
    pub fn apply(&self, image: &RgbaImage) -> Option<RgbaImage> {
        let mut bmp = Bitmap::from_image(image);
        for c in bmp.px.iter_mut() {
            *c = store(*c);
        }
        for effect in self.effects.iter().filter(|e| e.enabled) {
            bmp = apply_effect(effect, bmp)?;
            if bmp.w < 1 || bmp.h < 1 {
                return None;
            }
        }
        Some(bmp.to_image())
    }
}

/// `Helpers.GetPosition`.
pub fn position(
    placement: &str,
    offset: (i32, i32),
    bg: (i32, i32),
    size: (i32, i32),
) -> (i32, i32) {
    let mid_x = ((bg.0 as f32 / 2.0 - size.0 as f32 / 2.0) as f64).round_ties_even() as i32;
    let mid_y = ((bg.1 as f32 / 2.0 - size.1 as f32 / 2.0) as f64).round_ties_even() as i32;
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

/// `ImageEffectPathHelpers.TryGetSafeLocalPath`: folder variables expanded, only local
/// paths (no `//` shares, only `file:` URIs), relative ones from the app's folder, and
/// nothing under a symbolic link.
pub fn safe_local_path(path: &str, folder: bool) -> Option<PathBuf> {
    if path.trim().is_empty() {
        return None;
    }
    let mut p = modlight_core::names::file_helpers::expand_folder_variables(path, true);
    if let Some(rest) = p.strip_prefix("file://") {
        if !rest.starts_with('/') {
            return None;
        }
        p = rest.to_string();
    } else if p.contains("://") {
        return None;
    }
    if p.starts_with("//") || p.starts_with("\\\\") {
        return None;
    }
    let mut full = PathBuf::from(&p);
    if !full.is_absolute() {
        let base = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(Path::to_path_buf))?;
        full = base.join(full);
    }
    // `Path.GetFullPath`: `.` and `..` resolved lexically.
    let mut clean = PathBuf::new();
    for part in full.components() {
        match part {
            std::path::Component::ParentDir => {
                clean.pop();
            }
            std::path::Component::CurDir => {}
            other => clean.push(other.as_os_str()),
        }
    }
    let mut check = if folder {
        Some(clean.as_path())
    } else {
        clean.parent()
    };
    while let Some(dir) = check {
        if is_link(dir) && !is_system_link(dir) {
            return None;
        }
        check = dir.parent();
    }
    if !folder && is_link(&clean) {
        return None;
    }
    Some(clean)
}

fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// macOS's own links at the root (`/tmp`, `/var`, `/etc` into `/private`), which every
/// temporary path passes through; Windows has no counterpart, so they are not counted
/// as the reparse points upstream refuses.
fn is_system_link(dir: &Path) -> bool {
    dir.parent() == Some(Path::new("/"))
        && std::fs::read_link(dir).is_ok_and(|target| {
            let name = dir.file_name().unwrap_or_default();
            target == Path::new("private").join(name) || target == Path::new("/private").join(name)
        })
}

/// `ImageEffectPathHelpers.IsPathInFolder`.
pub fn is_path_in_folder(path: &str, folder: &Path) -> bool {
    let Some(full) = safe_local_path(path, false) else {
        return false;
    };
    let Some(folder) = safe_local_path(&folder.to_string_lossy(), true) else {
        return false;
    };
    let lower = |p: &Path| p.to_string_lossy().trim_end_matches('/').to_lowercase();
    let (f, d) = (lower(&full), lower(&folder));
    f == d || f.starts_with(&(d + "/"))
}

/// `ImageHelpers.LoadImage` of a local file, decoded as `SKBitmap`/`Bitmap` stores it.
fn load_image(path: &Path) -> Option<Bitmap> {
    let img = modlight_core::image_decode::open(path).ok()?.to_rgba8();
    if img.width() == 0 || img.height() == 0 {
        return None;
    }
    Some(Bitmap::from_image(&img))
}

fn sampling_for(mode: &str) -> Sampling {
    match mode {
        "NearestNeighbor" => Sampling::Nearest,
        "Bilinear" => Sampling::Linear,
        "Bicubic" | "HighQualityBilinear" => Sampling::MITCHELL,
        _ => Sampling::CATMULL_ROM,
    }
}

fn font_of(spec: &FontSpec) -> Font {
    Font::new(&spec.family, spec.size_px())
        .bold(spec.bold)
        .italic(spec.italic)
}

/// `NameParser(NameParserType.Text)` with the picture's size.
fn parse_text(text: &str, bmp: &Bitmap) -> String {
    let mut parser = NameParser::new(NameParserType::Text);
    parser.image_width = bmp.w;
    parser.image_height = bmp.h;
    parser.parse(text).unwrap_or_else(|_| text.to_string())
}

/// `Graphics.MeasureString` (`StringFormat.GenericDefault` keeps 1/6 em on each side)
/// truncated to whole pixels, and the line height.
fn measure(text: &str, font: &Font) -> ((i32, i32), f32) {
    let (ascent, descent) = font.metrics();
    let line_h = descent - ascent;
    let lines: Vec<&str> = text.split('\n').collect();
    let width = lines.iter().map(|l| font.measure(l)).fold(0f32, f32::max) + font.size / 3.0;
    let height = line_h * lines.len() as f32;
    ((width as i32, height as i32), line_h)
}

/// The outline of `text` laid out from (`x`, `y`) as GDI+'s `AddString`/`DrawString`
/// place it (1/6 em in, the first baseline one ascent down).
fn text_path(text: &str, font: &Font, x: f32, y: f32) -> Option<tiny_skia::Path> {
    let (ascent, descent) = font.metrics();
    let line_h = descent - ascent;
    let mut pb = PathBuilder::new();
    let mut any = false;
    for (i, line) in text.split('\n').enumerate() {
        if let Some(p) = font.path(
            line.trim_end_matches('\r'),
            x + font.size / 6.0,
            y - ascent + i as f32 * line_h,
        ) {
            pb.push_path(&p);
            any = true;
        }
    }
    if any { pb.finish() } else { None }
}

fn gradient_paint(g: &Gradient, x: f32, y: f32, w: f32, h: f32) -> Paint {
    Paint::fill_aa(C::BLACK).with_shader(gdi::gradient_shader(g, x, y, w, h))
}

/// One effect's `Apply`.
pub fn apply_effect(e: &ClassicEffect, bmp: Bitmap) -> Option<Bitmap> {
    use Kind::*;
    let f = |p: &str| e.float(p);
    Some(match e.kind {
        Alpha => gdi::color_matrix(&bmp, &gdi::m_alpha(f("Value"), f("Addition"))),
        BlackWhite => gdi::color_matrix(&bmp, &gdi::m_black_white()),
        Brightness => gdi::color_matrix(&bmp, &gdi::m_brightness(f("Value"))),
        MatrixColor => {
            let v: Vec<f32> = [
                "Rr", "Rg", "Rb", "Ra", "Ro", "Gr", "Gg", "Gb", "Ga", "Go", "Br", "Bg", "Bb", "Ba",
                "Bo", "Ar", "Ag", "Ab", "Aa", "Ao",
            ]
            .iter()
            .map(|n| f(n))
            .collect();
            // Rows are the inputs (r, g, b, a, offset); columns the outputs.
            let m = [
                [v[0], v[5], v[10], v[15], 0.0],
                [v[1], v[6], v[11], v[16], 0.0],
                [v[2], v[7], v[12], v[17], 0.0],
                [v[3], v[8], v[13], v[18], 0.0],
                [v[4], v[9], v[14], v[19], 1.0],
            ];
            gdi::color_matrix(&bmp, &m)
        }
        Colorize => gdi::color_matrix(&bmp, &gdi::m_colorize(e.color("Color"), f("Value"))),
        Contrast => gdi::color_matrix(&bmp, &gdi::m_contrast(f("Value"))),
        Gamma => gdi::gamma(&bmp, f("Value")),
        Grayscale => gdi::color_matrix(&bmp, &gdi::m_grayscale(f("Value"))),
        Hue => gdi::color_matrix(&bmp, &gdi::m_hue(f("Angle"))),
        Inverse => gdi::color_matrix(&bmp, &gdi::m_inverse()),
        Polaroid => gdi::color_matrix(&bmp, &gdi::m_polaroid()),
        ReplaceColor => {
            let mut b = bmp;
            gdi::replace_color(
                &mut b,
                e.color("SourceColor"),
                e.color("TargetColor"),
                e.bool("AutoSourceColor"),
                e.int("Threshold"),
            );
            b
        }
        Saturation => gdi::color_matrix(&bmp, &gdi::m_saturation(f("Value"))),
        SelectiveColor => {
            let mut b = bmp;
            gdi::selective_color(
                &mut b,
                e.color("LightColor"),
                e.color("DarkColor"),
                e.int("PaletteSize").clamp(2, 100),
            );
            b
        }
        Sepia => gdi::color_matrix(&bmp, &gdi::m_sepia(f("Value"))),
        Blur => {
            let mut b = bmp;
            gdi::box_blur(&mut b, e.int("Radius"));
            b
        }
        ColorDepth => {
            let mut b = bmp;
            gdi::color_depth(&mut b, e.int("BitsPerChannel"));
            b
        }
        MatrixConvolution => {
            let factor = e.double("Factor");
            let mut k = gdi::Kernel::new(3, 3);
            let names = [
                ["X0Y0", "X1Y0", "X2Y0"],
                ["X0Y1", "X1Y1", "X2Y1"],
                ["X0Y2", "X1Y2", "X2Y2"],
            ];
            for (y, row) in names.iter().enumerate() {
                for (x, n) in row.iter().enumerate() {
                    k.rows[y][x] = e.int(n) as f64 / factor;
                }
            }
            k.offset = e.int("Offset").clamp(0, 255) as u8;
            gdi::convolve(&bmp, &k)
        }
        EdgeDetect => gdi::convolve(&bmp, &gdi::k_edge_detect()),
        Emboss => gdi::convolve(&bmp, &gdi::k_emboss()),
        GaussianBlur => gdi::gaussian_blur(&bmp, e.int("Radius")),
        Glow => gdi::add_glow(
            &bmp,
            e.int("Size"),
            f("Strength"),
            e.color("Color"),
            e.point("Offset"),
            if e.bool("UseGradient") {
                e.gradient("Gradient")
            } else {
                None
            },
        ),
        MeanRemoval => gdi::convolve(&bmp, &gdi::k_mean_removal()),
        Outline => gdi::outline(
            bmp,
            e.int("Size"),
            e.color("Color"),
            e.int("Padding"),
            e.bool("OutlineOnly"),
        ),
        Pixelate => {
            let mut b = bmp;
            gdi::pixelate(
                &mut b,
                e.int("Size"),
                e.int("BorderSize"),
                e.color("BorderColor"),
            );
            b
        }
        Reflection => gdi::draw_reflection(
            &bmp,
            e.int("Percentage"),
            e.int("MaxAlpha"),
            e.int("MinAlpha"),
            e.int("Offset"),
            e.bool("Skew"),
            e.int("SkewSize"),
        ),
        RGBSplit => gdi::rgb_split(
            &bmp,
            e.point("OffsetRed"),
            e.point("OffsetGreen"),
            e.point("OffsetBlue"),
        ),
        Shadow => gdi::add_shadow(
            &bmp,
            f("Opacity"),
            e.int("Size"),
            f("Darkness") + 1.0,
            e.color("Color"),
            e.point("Offset"),
            e.bool("AutoResize"),
        ),
        Sharpen => gdi::convolve(&bmp, &gdi::k_sharpen()),
        Slice => {
            let (a, b) = (e.int("MinSliceHeight"), e.int("MaxSliceHeight"));
            let (c, d) = (e.int("MinSliceShift"), e.int("MaxSliceShift"));
            gdi::slice(&bmp, a.min(b), a.max(b), c.min(d), c.max(d))
        }
        Smooth => gdi::convolve(&bmp, &gdi::k_smooth()),
        TornEdge => gdi::torn_edges(
            &bmp,
            e.int("Depth"),
            e.int("Range"),
            gdi::sides_or_all(e.flags("Sides")),
            e.bool("CurvedEdges"),
        ),
        WaveEdge => gdi::wavy_edges(
            &bmp,
            e.int("Depth"),
            e.int("Range"),
            gdi::sides_or_all(e.flags("Sides")),
        ),
        AutoCrop => gdi::auto_crop(bmp, true, e.flags("Sides"), e.int("Padding")),
        Canvas => {
            let m = e.padding("Margin");
            let margin = if e.text("MarginMode") == "PercentageOfCanvas" {
                let r = |v: i32, of: i32| {
                    ((v as f32 / 100.0 * of as f32) as f64).round_ties_even() as i32
                };
                Padding::new(
                    r(m.left, bmp.w),
                    r(m.top, bmp.h),
                    r(m.right, bmp.w),
                    r(m.bottom, bmp.h),
                )
            } else {
                m
            };
            gdi::add_canvas(&bmp, margin, e.color("Color")).unwrap_or(bmp)
        }
        Crop => {
            let m = e.padding("Margin");
            if m.all() == 0 {
                bmp
            } else {
                gdi::crop(
                    &bmp,
                    m.left,
                    m.top,
                    bmp.w - m.horizontal(),
                    bmp.h - m.vertical(),
                )?
            }
        }
        Flip => {
            let (h, v) = (e.bool("Horizontally"), e.bool("Vertically"));
            if h || v { gdi::flip(&bmp, h, v) } else { bmp }
        }
        ForceProportions => force_proportions(e, bmp)?,
        Resize => {
            let (w, h) = (e.int("Width"), e.int("Height"));
            if w <= 0 && h <= 0 {
                bmp
            } else {
                let (nw, nh) = gdi::aspect_ratio(w, h, &bmp);
                let mode = e.text("Mode");
                if (mode == "ResizeIfBigger" && bmp.w <= nw && bmp.h <= nh)
                    || (mode == "ResizeIfSmaller" && bmp.w >= nw && bmp.h >= nh)
                {
                    bmp
                } else {
                    gdi::resize(bmp, nw, nh)
                }
            }
        }
        Rotate => {
            if f("Angle") == 0.0 {
                bmp
            } else {
                gdi::rotate(&bmp, f("Angle"), e.bool("Upsize"), e.bool("Clip"))
            }
        }
        RoundedCorners => gdi::rounded_corners(&bmp, e.int("CornerRadius")),
        Scale => {
            let (wp, hp) = (f("WidthPercentage"), f("HeightPercentage"));
            if wp <= 0.0 && hp <= 0.0 {
                bmp
            } else {
                let w = ((wp / 100.0 * bmp.w as f32) as f64).round_ties_even() as i32;
                let h = ((hp / 100.0 * bmp.h as f32) as f64).round_ties_even() as i32;
                let (nw, nh) = gdi::aspect_ratio(w, h, &bmp);
                gdi::resize(bmp, nw, nh)
            }
        }
        Skew => {
            let (h, v) = (e.int("Horizontally"), e.int("Vertically"));
            if h == 0 && v == 0 {
                bmp
            } else {
                gdi::add_skew(&bmp, h, v)
            }
        }
        DrawBackground => {
            let fill = match e.gradient("Gradient") {
                Some(g) if e.bool("UseGradient") && g.is_valid() => Fill::Gradient(g),
                _ => Fill::Color(e.color("Color")),
            };
            gdi::fill_background(&bmp, fill)
        }
        DrawBackgroundImage => {
            match safe_local_path(&e.text("ImageFilePath"), false).and_then(|p| load_image(&p)) {
                Some(bg) => gdi::draw_background_image(&bmp, &bg, e.bool("Center"), e.bool("Tile")),
                None => bmp,
            }
        }
        DrawBorder => {
            let fill = match e.gradient("Gradient") {
                Some(g) if e.bool("UseGradient") && g.is_valid() => Fill::Gradient(g),
                _ => Fill::Color(e.color("Color")),
            };
            gdi::draw_border(
                bmp,
                fill,
                e.int("Size"),
                e.text("Type") != "Inside",
                &e.text("DashStyle"),
            )
        }
        DrawCheckerboard => {
            gdi::draw_checkers(&bmp, e.int("Size"), e.color("Color"), e.color("Color2"))
        }
        DrawImage => draw_image(e, bmp),
        DrawParticles => draw_particles(e, bmp),
        DrawText => draw_text(e, bmp),
        DrawTextEx => draw_text_ex(e, bmp),
    })
}

fn force_proportions(e: &ClassicEffect, bmp: Bitmap) -> Option<Bitmap> {
    let pw = e.int("ProportionalWidth").max(1);
    let ph = e.int("ProportionalHeight").max(1);
    let current = bmp.w as f32 / bmp.h as f32;
    let target = pw as f32 / ph as f32;
    let wider = target > current;
    let (mut tw, mut th) = (bmp.w, bmp.h);
    let round = |v: f32| (v as f64).round_ties_even() as i32;
    match e.text("Method").as_str() {
        "Crop" => {
            let (mut ml, mut mt) = (0, 0);
            if wider {
                th = round(bmp.w as f32 / target);
                mt = (bmp.h - th) / 2;
            } else {
                tw = round(bmp.h as f32 * target);
                ml = (bmp.w - tw) / 2;
            }
            gdi::crop(&bmp, ml, mt, tw, th)
        }
        _ => {
            if wider {
                tw = round(bmp.h as f32 * target);
            } else {
                th = round(bmp.w as f32 / target);
            }
            Some(gdi::resize_fit(
                &bmp,
                tw,
                th,
                false,
                true,
                e.color("GrowFillColor"),
            ))
        }
    }
}

fn draw_image(e: &ClassicEffect, bmp: Bitmap) -> Bitmap {
    let opacity = e.int("Opacity");
    let size_mode = e.text("SizeMode");
    let (sw, sh) = e.point("Size");
    if opacity < 1 || (size_mode != "DontResize" && sw <= 0 && sh <= 0) {
        return bmp;
    }
    let Some(path) = safe_local_path(&e.text("ImageLocation"), false).filter(|p| p.is_file())
    else {
        return bmp;
    };
    let Some(mut mark) = load_image(&path) else {
        return bmp;
    };
    let rf = e.text("RotateFlip");
    if rf != "None" {
        mark = gdi::rotate_flip(&mark, &rf);
    }
    let pct = |v: i32, of: i32| ((v as f32 / 100.0 * of as f32) as f64).round_ties_even() as i32;
    let size = match size_mode.as_str() {
        "AbsoluteSize" => gdi::aspect_ratio(
            if sw == -1 { bmp.w } else { sw },
            if sh == -1 { bmp.h } else { sh },
            &mark,
        ),
        "PercentageOfWatermark" => gdi::aspect_ratio(pct(sw, mark.w), pct(sh, mark.h), &mark),
        "PercentageOfCanvas" => gdi::aspect_ratio(pct(sw, bmp.w), pct(sh, bmp.h), &mark),
        _ => (mark.w, mark.h),
    };
    let pos = position(
        &e.text("Placement"),
        e.point("Offset"),
        (bmp.w, bmp.h),
        size,
    );
    let inside = pos.0 >= 0 && pos.1 >= 0 && pos.0 + size.0 <= bmp.w && pos.1 + size.1 <= bmp.h;
    if e.bool("AutoHide") && !inside {
        return bmp;
    }
    let blend = if e.text("CompositingMode") == "SourceCopy" {
        Blend::Src
    } else {
        Blend::SrcOver
    };
    let mut canvas = Canvas::from_bitmap(&bmp);
    if e.bool("Tile") {
        let paint = Paint::default()
            .with_blend(blend)
            .with_shader(Shader::Image {
                bitmap: std::sync::Arc::new(mark),
                tile_x: crate::sk::Tile::Repeat,
                tile_y: crate::sk::Tile::Repeat,
                matrix: Transform::from_translate(pos.0 as f32, pos.1 as f32),
                sampling: Sampling::Nearest,
            });
        canvas.draw_rect(
            crate::sk::rect(
                pos.0 as f32,
                pos.1 as f32,
                (pos.0 + size.0) as f32,
                (pos.1 + size.1) as f32,
            ),
            &paint,
        );
    } else {
        let alpha = if opacity < 100 {
            opacity as f32 / 100.0
        } else {
            1.0
        };
        gdi::draw_image(
            &mut canvas,
            &mark,
            (pos.0, pos.1, size.0, size.1),
            sampling_for(&e.text("InterpolationMode")),
            blend,
            alpha,
        );
    }
    canvas.into_bitmap()
}

fn draw_particles(e: &ClassicEffect, bmp: Bitmap) -> Bitmap {
    let Some(folder) = safe_local_path(&e.text("ImageFolder"), true).filter(|p| p.is_dir()) else {
        return bmp;
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&folder)
        .map(|rd| {
            rd.filter_map(|d| d.ok().map(|d| d.path()))
                .filter(|p| {
                    p.is_file()
                        && p.extension().and_then(|x| x.to_str()).is_some_and(|x| {
                            x.eq_ignore_ascii_case("png") || x.eq_ignore_ascii_case("jpg")
                        })
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    if files.is_empty() {
        return bmp;
    }
    let background = e.bool("Background");
    let mut canvas = if background {
        Canvas::new(bmp.w, bmp.h)
    } else {
        Canvas::from_bitmap(&bmp)
    };
    let mut rng = shared_random();
    let mut cache: std::collections::HashMap<PathBuf, Option<Bitmap>> = Default::default();
    let mut placed: Vec<(i32, i32, i32, i32)> = Vec::new();
    let edge = e.bool("EdgeOverlap");
    for _ in 0..e.int("ImageCount") {
        let file = files[gdi::rand_incl(&mut rng, 0, files.len() as i32 - 1) as usize].clone();
        let Some(img) = cache
            .entry(file.clone())
            .or_insert_with(|| load_image(&file))
            .clone()
        else {
            continue;
        };
        let (mut w, mut h) = (img.w, img.h);
        if e.bool("RandomSize") {
            let (a, b) = (e.int("RandomSizeMin"), e.int("RandomSizeMax"));
            let size = gdi::rand_incl(&mut rng, a.min(b), a.max(b));
            w = size;
            h = size;
            if img.w > img.h {
                h = (size as f64 * (img.h as f64 / img.w as f64)).round_ties_even() as i32;
            } else if img.w < img.h {
                w = (size as f64 * (img.w as f64 / img.h as f64)).round_ties_even() as i32;
            }
        }
        if w < 1 || h < 1 {
            continue;
        }
        let min_x = if edge { -w + 1 } else { 0 };
        let min_y = if edge { -h + 1 } else { 0 };
        let max_x = bmp.w - if edge { 0 } else { w } - 1;
        let max_y = bmp.h - if edge { 0 } else { h } - 1;
        let gap = e.int("NoOverlapOffset");
        let mut attempts = 0;
        let rect = loop {
            attempts += 1;
            if attempts > 1000 {
                break None;
            }
            let x = gdi::rand_incl(&mut rng, min_x.min(max_x), min_x.max(max_x));
            let y = gdi::rand_incl(&mut rng, min_y.min(max_y), min_y.max(max_y));
            let r = (x, y, w, h);
            let o = (x - gap, y - gap, w + gap * 2, h + gap * 2);
            let hit = placed
                .iter()
                .any(|p| p.0 < o.0 + o.2 && o.0 < p.0 + p.2 && p.1 < o.1 + o.3 && o.1 < p.1 + p.3);
            if !(e.bool("NoOverlap") && hit) {
                break Some(r);
            }
        };
        let Some(r) = rect else {
            return canvas.into_bitmap();
        };
        placed.push(r);
        canvas.save();
        if e.bool("RandomAngle") {
            let mx = r.0 as f32 + r.2 as f32 / 2.0;
            let my = r.1 as f32 + r.3 as f32 / 2.0;
            let (a, b) = (e.int("RandomAngleMin"), e.int("RandomAngleMax"));
            let rot = gdi::rand_incl(&mut rng, a.min(b), a.max(b));
            canvas.translate(mx, my);
            canvas.rotate_degrees(rot as f32);
            canvas.translate(-mx, -my);
        }
        let mut alpha = 1.0;
        if e.bool("RandomOpacity") {
            let (a, b) = (e.int("RandomOpacityMin"), e.int("RandomOpacityMax"));
            alpha = gdi::rand_incl(&mut rng, a.min(b), a.max(b)).clamp(0, 100) as f32 / 100.0;
        }
        gdi::draw_image(
            &mut canvas,
            &img,
            r,
            Sampling::CATMULL_ROM,
            Blend::SrcOver,
            alpha,
        );
        canvas.restore();
    }
    if background {
        canvas.draw_bitmap(&bmp, 0.0, 0.0, &Paint::default());
    }
    canvas.into_bitmap()
}

fn draw_text(e: &ClassicEffect, bmp: Bitmap) -> Bitmap {
    let text = e.text("Text");
    if text.is_empty() {
        return bmp;
    }
    let spec = e.font("TextFont");
    if spec.size_pt < 1.0 {
        return bmp;
    }
    let font = font_of(&spec);
    let parsed = parse_text(&text, &bmp);
    let ((tw, th), _) = measure(&parsed, &font);
    let pad = e.padding("Padding");
    let size = (pad.left + tw + pad.right, pad.top + th + pad.bottom);
    let pos = position(
        &e.text("Placement"),
        e.point("Offset"),
        (bmp.w, bmp.h),
        size,
    );
    let inside = pos.0 >= 0 && pos.1 >= 0 && pos.0 + size.0 <= bmp.w && pos.1 + size.1 <= bmp.h;
    if e.bool("AutoHide") && !inside {
        return bmp;
    }
    let mut canvas = Canvas::from_bitmap(&bmp);
    let (x, y, w, h) = (pos.0 as f32, pos.1 as f32, size.0 as f32, size.1 as f32);
    // `AddRoundedRectangleProper`: one pixel smaller, for a one-pixel pen.
    let path = gdi::rounded_rect_path(x, y, w - 1.0, h - 1.0, e.int("CornerRadius") as f32);
    if let Some(path) = &path {
        if e.bool("DrawBackground") {
            let paint = match e.gradient("Gradient") {
                Some(g) if e.bool("UseGradient") && g.is_valid() => gradient_paint(g, x, y, w, h),
                _ => Paint::fill_aa(e.color("BackgroundColor")),
            };
            canvas.draw_path(path, &paint);
        }
        if e.bool("DrawBorder") {
            let size = e.int("BorderSize").max(1);
            let mut paint = Paint::stroke_aa(e.color("BorderColor"), size as f32);
            paint.join = Join::Miter;
            canvas.save();
            if size % 2 == 0 {
                // `PixelOffsetMode.Half`.
                canvas.translate(-0.5, -0.5);
            }
            canvas.draw_path(path, &paint);
            canvas.restore();
        }
    }
    let tx = x + pad.left as f32;
    let ty = y + pad.top as f32;
    if e.bool("DrawTextShadow") {
        let (ox, oy) = e.point("TextShadowOffset");
        if let Some(p) = text_path(&parsed, &font, tx + ox as f32, ty + oy as f32) {
            canvas.draw_path(&p, &Paint::fill_aa(e.color("TextShadowColor")));
        }
    }
    if let Some(p) = text_path(&parsed, &font, tx, ty) {
        canvas.draw_path(&p, &Paint::fill_aa(e.color("TextColor")));
    }
    canvas.into_bitmap()
}

fn draw_text_ex(e: &ClassicEffect, bmp: Bitmap) -> Bitmap {
    let text = e.text("Text");
    if text.is_empty() {
        return bmp;
    }
    let spec = e.font("Font");
    if spec.size_pt < 1.0 {
        return bmp;
    }
    let font = font_of(&spec);
    let parsed = parse_text(&text, &bmp);
    let Some(mut path) = text_path(&parsed, &font, 0.0, 0.0) else {
        return bmp;
    };
    let angle = e.int("Angle");
    if angle != 0 {
        match path.transform(Transform::from_rotate(angle as f32)) {
            Some(p) => path = p,
            None => return bmp,
        }
    }
    let b = path.bounds();
    if b.width() <= 0.0 && b.height() <= 0.0 {
        return bmp;
    }
    // `pathRect.Size.ToSize().Offset(1)`.
    let size = (b.width() as i32 + 1, b.height() as i32 + 1);
    let pos = position(
        &e.text("Placement"),
        e.point("Offset"),
        (bmp.w, bmp.h),
        size,
    );
    let inside = pos.0 >= 0 && pos.1 >= 0 && pos.0 + size.0 <= bmp.w && pos.1 + size.1 <= bmp.h;
    if e.bool("AutoHide") && !inside {
        return bmp;
    }
    let Some(path) = path.transform(Transform::from_translate(
        pos.0 as f32 - b.left(),
        pos.1 as f32 - b.top(),
    )) else {
        return bmp;
    };
    let mut canvas = Canvas::from_bitmap(&bmp);
    let rect_off = |o: i32, shift: (i32, i32)| {
        (
            (pos.0 - o + shift.0) as f32,
            (pos.1 - o + shift.1) as f32,
            (size.0 + o * 2) as f32,
            (size.1 + o * 2) as f32,
        )
    };
    let outline = e.bool("Outline") && e.int("OutlineSize") > 0;
    let outline_size = e.int("OutlineSize");
    let stroke = |paint: Paint| Paint {
        style: Style::Stroke,
        stroke_width: outline_size as f32,
        join: Join::Round,
        ..paint
    };
    // `PixelOffsetMode.Half`.
    canvas.save();
    canvas.translate(-0.5, -0.5);
    let shadow_on = e.bool("Shadow")
        && if e.bool("ShadowUseGradient") {
            e.gradient("ShadowGradient")
                .is_some_and(Gradient::is_visible)
        } else {
            e.color("ShadowColor").a > 0
        };
    if shadow_on {
        let off = e.point("ShadowOffset");
        if let Some(sp) = path
            .clone()
            .transform(Transform::from_translate(off.0 as f32, off.1 as f32))
        {
            let grow = if outline { outline_size + 1 } else { 1 };
            let paint = match e.gradient("ShadowGradient") {
                Some(g) if e.bool("ShadowUseGradient") => {
                    let (x, y, w, h) = rect_off(grow, off);
                    gradient_paint(g, x, y, w, h)
                }
                _ => Paint::fill_aa(e.color("ShadowColor")),
            };
            canvas.draw_path(&sp, &if outline { stroke(paint) } else { paint });
        }
    }
    if outline {
        if e.bool("OutlineUseGradient") {
            if let Some(g) = e.gradient("OutlineGradient").filter(|g| g.is_visible()) {
                let (x, y, w, h) = rect_off(outline_size + 1, (0, 0));
                canvas.draw_path(&path, &stroke(gradient_paint(g, x, y, w, h)));
            }
        } else if e.color("OutlineColor").a > 0 {
            canvas.draw_path(&path, &stroke(Paint::fill_aa(e.color("OutlineColor"))));
        }
    }
    if e.bool("UseGradient") {
        if let Some(g) = e.gradient("Gradient").filter(|g| g.is_visible()) {
            let (x, y, w, h) = rect_off(1, (0, 0));
            canvas.draw_path(&path, &gradient_paint(g, x, y, w, h));
        }
    } else if e.color("Color").a > 0 {
        canvas.draw_path(&path, &Paint::fill_aa(e.color("Color")));
    }
    canvas.restore();
    canvas.into_bitmap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classic::model::Value;

    fn picture(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([(x * 20) as u8, (y * 20) as u8, 128, 255])
        })
    }

    #[test]
    fn every_class_applies_with_its_defaults() {
        let img = picture(40, 30);
        for kind in Kind::all() {
            let preset = Preset {
                name: String::new(),
                effects: vec![ClassicEffect::new(kind)],
            };
            let out = preset.apply(&img);
            assert!(out.is_some(), "{kind:?} gave no picture");
        }
    }

    #[test]
    fn default_preset_adds_the_canvas() {
        let out = Preset::default_preset().apply(&picture(60, 40)).unwrap();
        assert_eq!(out.dimensions(), (60, 70));
    }

    #[test]
    fn crop_outside_the_picture_stops_the_chain() {
        let mut crop = ClassicEffect::new(Kind::Crop);
        crop.set("Margin", Value::Padding(Padding::new(30, 0, 30, 0)));
        let preset = Preset {
            name: String::new(),
            effects: vec![crop],
        };
        assert!(preset.apply(&picture(40, 30)).is_none());
    }

    #[test]
    fn disabled_effects_are_skipped() {
        let mut inv = ClassicEffect::new(Kind::Inverse);
        inv.enabled = false;
        let img = picture(4, 4);
        let preset = Preset {
            name: String::new(),
            effects: vec![inv],
        };
        assert_eq!(preset.apply(&img).unwrap(), img);
    }

    #[test]
    fn positions_follow_content_alignment() {
        assert_eq!(
            position("BottomRight", (5, 5), (100, 80), (20, 10)),
            (75, 65)
        );
        assert_eq!(
            position("MiddleCenter", (0, 0), (100, 80), (20, 10)),
            (40, 35)
        );
    }

    #[test]
    fn unsafe_paths_are_refused() {
        assert!(safe_local_path("//server/share/a.png", false).is_none());
        assert!(safe_local_path("https://example.com/a.png", false).is_none());
        assert_eq!(
            safe_local_path("/tmp/../tmp/x.png", false),
            Some(PathBuf::from("/tmp/x.png"))
        );
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real, dir.path().join("link")).unwrap();
            let through = dir.path().join("link/a.png");
            assert!(safe_local_path(&through.to_string_lossy(), false).is_none());
        }
        let inside = real.join("a.png");
        assert!(is_path_in_folder(&inside.to_string_lossy(), &real));
        assert!(!is_path_in_folder(
            &dir.path().join("b.png").to_string_lossy(),
            &real
        ));
    }
}
