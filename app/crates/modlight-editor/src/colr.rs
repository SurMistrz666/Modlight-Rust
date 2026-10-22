//! Colour glyphs of `COLR` fonts (version 0 layers and version 1 paint graphs), painted
//! with tiny-skia: what Direct2D does for upstream's emoji on Windows, where "Segoe UI
//! Emoji" is a `COLR` font. ttf-parser walks the glyph's paint graph and calls the
//! [`Painter`] here; clips are masks, layers are pixmaps composited with their mode, and
//! the gradients are tiny-skia's, their colour lines stretched over the stops' own range
//! (a colour line may run past 0 and 1, which tiny-skia's stops cannot).

use rustybuzz::ttf_parser::{self, GlyphId, RgbaColor, colr};
use tiny_skia::{
    BlendMode, Color, FillRule, GradientStop, LinearGradient, Mask, Paint, Path, PathBuilder,
    Pixmap, PixmapPaint, Point, RadialGradient, Rect, Shader, SpreadMode, SweepGradient, Transform,
};

/// Paint `glyph` of `face` into `pixmap`: font units through `base` (scale, the y flip
/// and the glyph's place) into the pixmap's pixels. `None` when the glyph is not a
/// colour glyph.
pub fn paint_glyph(
    face: &ttf_parser::Face<'_>,
    glyph: GlyphId,
    base: Transform,
    pixmap: &mut Pixmap,
) -> Option<()> {
    if !face.is_color_glyph(glyph) {
        return None;
    }
    let mut painter = ColrPainter {
        face,
        base,
        width: pixmap.width(),
        height: pixmap.height(),
        transforms: vec![Transform::identity()],
        clips: Vec::new(),
        layers: vec![(
            Pixmap::new(pixmap.width(), pixmap.height())?,
            BlendMode::SourceOver,
        )],
        outline: None,
    };
    face.paint_color_glyph(glyph, 0, RgbaColor::new(0, 0, 0, 255), &mut painter)?;
    let (layer, _) = painter.layers.pop()?;
    pixmap.draw_pixmap(
        0,
        0,
        layer.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
    Some(())
}

/// A glyph's outline in font units.
pub fn outline(face: &ttf_parser::Face<'_>, glyph: GlyphId) -> Option<Path> {
    struct Builder(PathBuilder);
    impl ttf_parser::OutlineBuilder for Builder {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.move_to(x, y);
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.0.line_to(x, y);
        }
        fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
            self.0.quad_to(x1, y1, x, y);
        }
        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
            self.0.cubic_to(x1, y1, x2, y2, x, y);
        }
        fn close(&mut self) {
            self.0.close();
        }
    }
    let mut builder = Builder(PathBuilder::new());
    face.outline_glyph(glyph, &mut builder)?;
    builder.0.finish()
}

struct ColrPainter<'f, 'a> {
    face: &'f ttf_parser::Face<'a>,
    /// Font units to pixels.
    base: Transform,
    width: u32,
    height: u32,
    /// The paint graph's transforms, innermost last (font units to font units).
    transforms: Vec<Transform>,
    /// The clips in force, innermost last, each already intersected with the outer ones.
    clips: Vec<Mask>,
    /// The layers being painted, innermost last, with the mode each joins its parent by.
    layers: Vec<(Pixmap, BlendMode)>,
    /// The outline `outline_glyph` stored, in font units.
    outline: Option<Path>,
}

impl ColrPainter<'_, '_> {
    /// Font units under the current transform to pixels.
    fn transform(&self) -> Transform {
        let inner = self
            .transforms
            .last()
            .copied()
            .unwrap_or(Transform::identity());
        self.base.pre_concat(inner)
    }

    fn push_clip_path(&mut self, path: &Path) {
        let transform = self.transform();
        let mask = match self.clips.last() {
            Some(outer) => {
                let mut mask = outer.clone();
                mask.intersect_path(path, FillRule::Winding, true, transform);
                Some(mask)
            }
            None => Mask::new(self.width, self.height).map(|mut mask| {
                mask.fill_path(path, FillRule::Winding, true, transform);
                mask
            }),
        };
        // An empty mask still has to be pushed: its pop must find it.
        let mask = mask.or_else(|| Mask::new(1, 1));
        if let Some(mask) = mask {
            self.clips.push(mask);
        }
    }

    fn shader(&self, paint: &colr::Paint<'_>) -> Option<Shader<'static>> {
        let transform = self.transform();
        match paint {
            colr::Paint::Solid(color) => Some(Shader::SolidColor(color_of(*color))),
            colr::Paint::LinearGradient(gradient) => {
                let (stops, range) = stops(gradient.stops(0, &[]))?;
                let p0 = Point::from_xy(gradient.x0, gradient.y0);
                let p1 = Point::from_xy(gradient.x1, gradient.y1);
                let p2 = Point::from_xy(gradient.x2, gradient.y2);
                // The gradient runs from p0 along p0->p1 projected onto the normal of
                // p0->p2 (the spec's rotation point).
                let d1 = p1 - p0;
                let d2 = p2 - p0;
                let normal = Point::from_xy(d2.y, -d2.x);
                let length = normal.x * normal.x + normal.y * normal.y;
                let p3 = if length > f32::EPSILON {
                    let along = (d1.x * normal.x + d1.y * normal.y) / length;
                    Point::from_xy(p0.x + normal.x * along, p0.y + normal.y * along)
                } else {
                    p1
                };
                let at =
                    |t: f32| Point::from_xy(p0.x + (p3.x - p0.x) * t, p0.y + (p3.y - p0.y) * t);
                LinearGradient::new(
                    at(range.0),
                    at(range.1),
                    skia_stops(stops),
                    spread(gradient.extend),
                    transform,
                )
            }
            colr::Paint::RadialGradient(gradient) => {
                let (stops, range) = stops(gradient.stops(0, &[]))?;
                let center = |t: f32| {
                    Point::from_xy(
                        gradient.x0 + (gradient.x1 - gradient.x0) * t,
                        gradient.y0 + (gradient.y1 - gradient.y0) * t,
                    )
                };
                let radius = |t: f32| (gradient.r0 + (gradient.r1 - gradient.r0) * t).max(0.0);
                RadialGradient::new(
                    center(range.0),
                    radius(range.0),
                    center(range.1),
                    radius(range.1),
                    skia_stops(stops),
                    spread(gradient.extend),
                    transform,
                )
            }
            colr::Paint::SweepGradient(gradient) => {
                let (mut stops, range) = stops(gradient.stops(0, &[]))?;
                // Angles come in half turns, counter-clockwise in the font's own (y up)
                // space, which is tiny-skia's sense in the shader's space.
                let angle = |t: f32| {
                    (gradient.start_angle + (gradient.end_angle - gradient.start_angle) * t) * 180.0
                };
                let (mut start, mut end) = (angle(range.0), angle(range.1));
                if start > end {
                    // tiny-skia sweeps one way only: the same colours the other way.
                    std::mem::swap(&mut start, &mut end);
                    stops = stops.into_iter().rev().map(|(p, c)| (1.0 - p, c)).collect();
                }
                SweepGradient::new(
                    Point::from_xy(gradient.center_x, gradient.center_y),
                    start,
                    end,
                    skia_stops(stops),
                    spread(gradient.extend),
                    transform,
                )
            }
        }
    }
}

impl<'a> colr::Painter<'a> for ColrPainter<'_, 'a> {
    fn outline_glyph(&mut self, glyph_id: GlyphId) {
        self.outline = outline(self.face, glyph_id);
    }

    fn paint(&mut self, paint: colr::Paint<'a>) {
        let Some(shader) = self.shader(&paint) else {
            return;
        };
        let Some(rect) = Rect::from_xywh(0.0, 0.0, self.width as f32, self.height as f32) else {
            return;
        };
        let mask = self.clips.last();
        if mask.is_some_and(|m| m.width() != self.width) {
            // A clip that came out empty: nothing shows.
            return;
        }
        let paint = Paint {
            shader,
            anti_alias: false,
            ..Paint::default()
        };
        if let Some((layer, _)) = self.layers.last_mut() {
            layer.fill_rect(rect, &paint, Transform::identity(), mask);
        }
    }

    fn push_clip(&mut self) {
        match self.outline.take() {
            Some(path) => {
                self.push_clip_path(&path);
                self.outline = Some(path);
            }
            None => {
                // No outline: an empty clip.
                if let Some(mask) = Mask::new(1, 1) {
                    self.clips.push(mask);
                }
            }
        }
    }

    fn push_clip_box(&mut self, clipbox: colr::ClipBox) {
        let rect = Rect::from_ltrb(clipbox.x_min, clipbox.y_min, clipbox.x_max, clipbox.y_max);
        match rect.map(PathBuilder::from_rect) {
            Some(path) => self.push_clip_path(&path),
            None => {
                if let Some(mask) = Mask::new(1, 1) {
                    self.clips.push(mask);
                }
            }
        }
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn push_layer(&mut self, mode: colr::CompositeMode) {
        if let Some(layer) = Pixmap::new(self.width, self.height) {
            self.layers.push((layer, blend_mode(mode)));
        }
    }

    fn pop_layer(&mut self) {
        if self.layers.len() < 2 {
            return;
        }
        let Some((layer, mode)) = self.layers.pop() else {
            return;
        };
        if let Some((parent, _)) = self.layers.last_mut() {
            parent.draw_pixmap(
                0,
                0,
                layer.as_ref(),
                &PixmapPaint {
                    blend_mode: mode,
                    ..PixmapPaint::default()
                },
                Transform::identity(),
                None,
            );
        }
    }

    fn push_transform(&mut self, transform: ttf_parser::Transform) {
        let inner = Transform::from_row(
            transform.a,
            transform.b,
            transform.c,
            transform.d,
            transform.e,
            transform.f,
        );
        let current = self
            .transforms
            .last()
            .copied()
            .unwrap_or(Transform::identity());
        self.transforms.push(current.pre_concat(inner));
    }

    fn pop_transform(&mut self) {
        if self.transforms.len() > 1 {
            self.transforms.pop();
        }
    }
}

fn skia_stops(stops: Vec<(f32, Color)>) -> Vec<GradientStop> {
    stops
        .into_iter()
        .map(|(position, color)| GradientStop::new(position, color))
        .collect()
}

fn color_of(color: RgbaColor) -> Color {
    Color::from_rgba8(color.red, color.green, color.blue, color.alpha)
}

/// A colour line's stops, stretched to 0..1, and the offsets it really spans.
type ColourLine = (Vec<(f32, Color)>, (f32, f32));

/// The colour line as (offset, colour) pairs sorted by offset and stretched to 0..1,
/// with the offsets the line really spans (`range`), which the geometry follows.
fn stops<'a, 'b>(iter: colr::GradientStopsIter<'a, 'b>) -> Option<ColourLine> {
    let mut stops: Vec<(f32, Color)> = iter
        .map(|stop| (stop.stop_offset, color_of(stop.color)))
        .collect();
    if stops.is_empty() {
        return None;
    }
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    let first = stops[0].0;
    let last = stops[stops.len() - 1].0;
    let span = last - first;
    if span <= f32::EPSILON {
        // One offset: a solid colour from there on, as a gradient of two equal stops.
        let color = stops[stops.len() - 1].1;
        return Some((vec![(0.0, color), (1.0, color)], (0.0, 1.0)));
    }
    let stretched = stops
        .into_iter()
        .map(|(offset, color)| ((offset - first) / span, color))
        .collect();
    Some((stretched, (first, last)))
}

fn spread(extend: colr::GradientExtend) -> SpreadMode {
    match extend {
        colr::GradientExtend::Pad => SpreadMode::Pad,
        colr::GradientExtend::Repeat => SpreadMode::Repeat,
        colr::GradientExtend::Reflect => SpreadMode::Reflect,
    }
}

fn blend_mode(mode: colr::CompositeMode) -> BlendMode {
    use colr::CompositeMode as C;
    match mode {
        C::Clear => BlendMode::Clear,
        C::Source => BlendMode::Source,
        C::Destination => BlendMode::Destination,
        C::SourceOver => BlendMode::SourceOver,
        C::DestinationOver => BlendMode::DestinationOver,
        C::SourceIn => BlendMode::SourceIn,
        C::DestinationIn => BlendMode::DestinationIn,
        C::SourceOut => BlendMode::SourceOut,
        C::DestinationOut => BlendMode::DestinationOut,
        C::SourceAtop => BlendMode::SourceAtop,
        C::DestinationAtop => BlendMode::DestinationAtop,
        C::Xor => BlendMode::Xor,
        C::Plus => BlendMode::Plus,
        C::Screen => BlendMode::Screen,
        C::Overlay => BlendMode::Overlay,
        C::Darken => BlendMode::Darken,
        C::Lighten => BlendMode::Lighten,
        C::ColorDodge => BlendMode::ColorDodge,
        C::ColorBurn => BlendMode::ColorBurn,
        C::HardLight => BlendMode::HardLight,
        C::SoftLight => BlendMode::SoftLight,
        C::Difference => BlendMode::Difference,
        C::Exclusion => BlendMode::Exclusion,
        C::Multiply => BlendMode::Multiply,
        C::Hue => BlendMode::Hue,
        C::Saturation => BlendMode::Saturation,
        C::Color => BlendMode::Color,
        C::Luminosity => BlendMode::Luminosity,
    }
}
