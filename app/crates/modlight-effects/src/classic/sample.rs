//! `ImageEffectsViewModel.CreateSampleImage`: the picture the presets window previews on
//! when it has none (720 x 480, a cube of the skin's accent colours on its background).

use image::RgbaImage;
use tiny_skia::PathBuilder;

use crate::sk::{C, Canvas, Paint, Shader, Tile, rect};

/// The skin colours the sample is drawn with (`GetThemeColor`).
#[derive(Clone, Copy, Debug)]
pub struct SampleColors {
    pub background_main: C,
    pub background_panel: C,
    pub accent_start: C,
    pub accent_end: C,
    pub accent_foreground: C,
}

impl Default for SampleColors {
    /// Upstream's fallbacks.
    fn default() -> Self {
        SampleColors {
            background_main: C::rgb(39, 39, 39),
            background_panel: C::rgb(36, 36, 36),
            accent_start: C::rgb(62, 131, 242),
            accent_end: C::rgb(57, 117, 213),
            accent_foreground: C::rgb(216, 218, 219),
        }
    }
}

/// `BlendColors`.
fn blend(first: C, second: C, amount: f32) -> C {
    let amount = amount.clamp(0.0, 1.0);
    let inverse = 1.0 - amount;
    let mix = |a: u8, b: u8| (a as f32 * inverse + b as f32 * amount) as u8;
    C::new(
        mix(first.r, second.r),
        mix(first.g, second.g),
        mix(first.b, second.b),
        mix(first.a, second.a),
    )
}

/// `CreateHexagon(bounds, 0)`.
fn hexagon(left: f32, top: f32, size: f32) -> [(f32, f32); 6] {
    let quarter = size / 4.0;
    let cx = left + size / 2.0;
    let (right, bottom) = (left + size, top + size);
    [
        (cx, top),
        (right, top + quarter),
        (right, bottom - quarter),
        (cx, bottom),
        (left, bottom - quarter),
        (left, top + quarter),
    ]
}

fn polygon(points: &[(f32, f32)]) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(points[0].0, points[0].1);
    for p in &points[1..] {
        pb.line_to(p.0, p.1);
    }
    pb.close();
    pb.finish()
}

/// The sample picture.
pub fn sample_image(colors: SampleColors) -> RgbaImage {
    let (w, h) = (720.0f32, 480.0f32);
    let mut canvas = Canvas::new(w as i32, h as i32);
    // `LinearGradientBrush(rect, main, panel, 35f)`: the band runs along 35 degrees and
    // spans the rectangle's corners.
    let angle = 35f32.to_radians();
    let (cos, sin) = (angle.cos(), angle.sin());
    let length = w * cos + h * sin;
    canvas.draw_rect(
        rect(0.0, 0.0, w, h),
        &Paint::default().with_shader(Shader::Linear {
            p0: (0.0, 0.0),
            p1: (length * cos, length * sin),
            colors: vec![colors.background_main, colors.background_panel],
            pos: None,
            tile: Tile::Clamp,
        }),
    );
    let size = 200.0;
    let (left, top) = ((w - size) / 2.0, (h - size) / 2.0);
    let outer = hexagon(left, top, size);
    let center = (left + size / 2.0, top + size / 2.0);
    let top_face = [outer[0], outer[1], center, outer[5]];
    let left_face = [outer[5], center, outer[3], outer[4]];
    let right_face = [outer[1], outer[2], outer[3], center];
    let faces = [
        (
            &top_face[..],
            blend(colors.accent_start, colors.accent_foreground, 0.16),
        ),
        (&left_face[..], colors.accent_start),
        (&right_face[..], colors.accent_end),
    ];
    for (points, color) in faces {
        if let Some(path) = polygon(points) {
            canvas.draw_path(&path, &Paint::fill_aa(color));
        }
    }
    let f = colors.accent_foreground;
    let outline = Paint::stroke_aa(C::new(f.r, f.g, f.b, 100), 2.0);
    if let Some(path) = polygon(&outer) {
        canvas.draw_path(&path, &outline);
    }
    for corner in [outer[3], outer[1], outer[5]] {
        canvas.draw_line(corner.0, corner.1, center.0, center.1, &outline);
    }
    canvas.into_bitmap().to_image()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_has_upstreams_size_and_cube() {
        let image = sample_image(SampleColors::default());
        assert_eq!(image.dimensions(), (720, 480));
        // The left face of the cube, accent start.
        let p = image.get_pixel(300, 260);
        assert_eq!([p[0], p[1], p[2]], [62, 131, 242]);
        assert_eq!(image.get_pixel(5, 5)[3], 255);
    }
}
