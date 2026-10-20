//! The annotation visuals (`Presentation/Rendering/AnnotationVisuals`, `Controls/*Control`)
//! drawn with tiny-skia, as Avalonia's Skia backend draws them: shapes inset by half
//! their stroke, dash arrays in multiples of the stroke width, figures that are only
//! stroked, `DropShadowEffect`s, rotation about the control's centre, text outlines.
//!
//! Every function takes a `Transform` from document pixels to the target's pixels, so
//! the same code paints the editor's view at any zoom and the exported image at 1:1.

use std::sync::Arc;

use image::RgbaImage;
use tiny_skia::{
    BlendMode, FillRule, FilterQuality, LineCap, LineJoin, Mask, Paint, Path, PathBuilder, Pattern,
    Pixmap, PixmapPaint, Rect as SkRect, Shader, SpreadMode, Stroke, StrokeDash, Transform,
};

use crate::color::Color;
use crate::geom::{Point, Rect};
use crate::model::{
    self, Annotation, AnnotationId, ArrowStyle, BorderStyle, EffectKind, Kind, TextAlign,
};
use crate::text::{self, FontRequest};

/// What the annotation layer needs besides the annotations themselves.
pub struct Scene<'a> {
    pub canvas_size: (f32, f32),
    pub annotations: &'a [Annotation],
    /// An annotation not to draw (the text being edited in its text box).
    pub hidden: Option<AnnotationId>,
}

pub fn paint_color(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color.to_skia());
    paint.anti_alias = true;
    paint
}

fn parse(color: &str) -> Color {
    Color::parse_or_transparent(color)
}

/// A pen as Avalonia's `Pen`: thickness, dashes relative to it, cap, join.
#[derive(Clone, Debug)]
pub struct Pen {
    pub color: Color,
    pub width: f32,
    pub dashes: Vec<f32>,
    pub dash_offset: f32,
    pub cap: LineCap,
    pub join: LineJoin,
}

impl Pen {
    pub fn new(color: Color, width: f32) -> Pen {
        Pen {
            color,
            width,
            dashes: Vec::new(),
            dash_offset: 0.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
        }
    }

    fn border(color: Color, width: f32, style: BorderStyle) -> Pen {
        Pen {
            dashes: style.dash_array().to_vec(),
            cap: if style.round_cap() {
                LineCap::Round
            } else {
                LineCap::Butt
            },
            ..Pen::new(color, width)
        }
    }

    fn stroke(&self) -> Stroke {
        let mut stroke = Stroke {
            width: self.width,
            line_cap: self.cap,
            line_join: self.join,
            miter_limit: 10.0,
            dash: None,
        };
        if !self.dashes.is_empty() && self.dashes.len().is_multiple_of(2) {
            let intervals: Vec<f32> = self
                .dashes
                .iter()
                .map(|d| (d * self.width).max(0.01))
                .collect();
            stroke.dash = StrokeDash::new(intervals, self.dash_offset * self.width);
        }
        stroke
    }
}

pub fn fill_path(
    target: &mut Pixmap,
    path: &Path,
    color: Color,
    transform: Transform,
    mask: Option<&Mask>,
) {
    if color.a == 0 {
        return;
    }
    target.fill_path(
        path,
        &paint_color(color),
        FillRule::Winding,
        transform,
        mask,
    );
}

pub fn stroke_path(
    target: &mut Pixmap,
    path: &Path,
    pen: &Pen,
    transform: Transform,
    mask: Option<&Mask>,
) {
    if pen.color.a == 0 || pen.width <= 0.0 {
        return;
    }
    target.stroke_path(
        path,
        &paint_color(pen.color),
        &pen.stroke(),
        transform,
        mask,
    );
}

/// A geometry whose figures may be stroke-only (`BeginFigure(point, isFilled: false)`).
#[derive(Default)]
pub struct Geometry {
    pub fill: Option<Path>,
    pub stroke: Option<Path>,
}

struct GeometryBuilder {
    fill: PathBuilder,
    stroke: PathBuilder,
    filled: bool,
}

impl GeometryBuilder {
    fn new() -> GeometryBuilder {
        GeometryBuilder {
            fill: PathBuilder::new(),
            stroke: PathBuilder::new(),
            filled: false,
        }
    }

    fn begin(&mut self, p: Point, filled: bool) {
        self.filled = filled;
        self.stroke.move_to(p.x, p.y);
        if filled {
            self.fill.move_to(p.x, p.y);
        }
    }

    fn line(&mut self, p: Point) {
        self.stroke.line_to(p.x, p.y);
        if self.filled {
            self.fill.line_to(p.x, p.y);
        }
    }

    fn quad(&mut self, c: Point, p: Point) {
        self.stroke.quad_to(c.x, c.y, p.x, p.y);
        if self.filled {
            self.fill.quad_to(c.x, c.y, p.x, p.y);
        }
    }

    fn end(&mut self, closed: bool) {
        if closed {
            self.stroke.close();
            if self.filled {
                self.fill.close();
            }
        }
    }

    fn circle(&mut self, center: Point, radius: f32) {
        // The zero-length arrow's dot: two arcs, filled.
        if let Some(path) = PathBuilder::from_circle(center.x, center.y, radius) {
            self.stroke.push_path(&path);
            self.fill.push_path(&path);
        }
    }

    fn finish(self) -> Geometry {
        Geometry {
            fill: self.fill.finish(),
            stroke: self.stroke.finish(),
        }
    }
}

/// `ArrowAnnotation.CreateArrowGeometry`.
pub fn arrow_geometry(a: &Annotation) -> Geometry {
    let Kind::Arrow { style, .. } = &a.kind else {
        return Geometry::default();
    };
    let head = a.stroke_width as f64 * arrow_head_width_multiplier(*style);
    let (start, end) = (a.start, a.end);
    let curved = model::has_curve(a);
    let mut g = GeometryBuilder::new();
    let control = model::quadratic_control_point(a);
    let end_tangent = model::tangent_at_end(a);
    match style {
        ArrowStyle::Modern if !curved => match arrow_points(start, end, head) {
            Some(p) => {
                g.begin(start, true);
                g.line(p.0);
                g.line(p.2);
                g.line(end);
                g.line(p.3);
                g.line(p.1);
                g.end(true);
            }
            None => g.circle(start, 2.0),
        },
        ArrowStyle::Modern => {
            g.begin(start, false);
            g.quad(control, end);
            g.end(false);
            if let Some((left, right)) = modern_head(end, end_tangent, head) {
                g.begin(end, true);
                g.line(left);
                g.line(right);
                g.end(true);
            }
        }
        ArrowStyle::Classic | ArrowStyle::Double => {
            let double = *style == ArrowStyle::Double;
            if curved {
                g.begin(start, false);
                g.quad(control, end);
                g.end(false);
                if double {
                    let t = model::tangent_at_start(a);
                    if let Some(cap) = cap_points(start, Point::new(-t.x, -t.y), head) {
                        append_cap(&mut g, start, &cap);
                    }
                }
                if let Some(cap) = cap_points(end, end_tangent, head) {
                    append_cap(&mut g, end, &cap);
                }
            } else {
                let direction = Point::new(end.x - start.x, end.y - start.y);
                match cap_points(end, direction, head) {
                    Some(cap) => {
                        g.begin(start, false);
                        g.line(end);
                        g.end(false);
                        if double
                            && let Some(start_cap) =
                                cap_points(start, Point::new(-direction.x, -direction.y), head)
                        {
                            append_cap(&mut g, start, &start_cap);
                        }
                        append_cap(&mut g, end, &cap);
                    }
                    None => g.circle(start, 2.0),
                }
            }
        }
        ArrowStyle::Basic | ArrowStyle::Line => {
            let tangent = if curved {
                end_tangent
            } else {
                Point::new(end.x - start.x, end.y - start.y)
            };
            match base_points(end, tangent, head) {
                Some((left, right)) => {
                    g.begin(start, false);
                    if curved {
                        g.quad(control, end);
                    } else {
                        g.line(end);
                    }
                    g.end(false);
                    if *style == ArrowStyle::Basic {
                        g.begin(end, true);
                        g.line(left);
                        g.line(right);
                        g.end(true);
                    } else {
                        g.begin(left, false);
                        g.line(end);
                        g.line(right);
                        g.end(false);
                    }
                }
                None if curved => {
                    g.begin(start, false);
                    g.quad(control, end);
                    g.end(false);
                }
                None => g.circle(start, 2.0),
            }
        }
    }
    g.finish()
}

/// `GetArrowHeadWidthMultiplier`.
pub fn arrow_head_width_multiplier(style: ArrowStyle) -> f64 {
    match style {
        ArrowStyle::Line | ArrowStyle::Basic => 1.75,
        ArrowStyle::Modern => 3.0,
        _ => 2.0,
    }
}

struct Cap {
    left: Point,
    right: Point,
    control: Point,
}

fn unit(t: Point) -> Option<(f64, f64)> {
    let length = ((t.x as f64).powi(2) + (t.y as f64).powi(2)).sqrt();
    (length > 0.0).then(|| (t.x as f64 / length, t.y as f64 / length))
}

/// `ComputeArrowCapPointsFromTangent`.
fn cap_points(tip: Point, tangent: Point, half: f64) -> Option<Cap> {
    let (ux, uy) = unit(tangent)?;
    let (px, py) = (-uy, ux);
    let length = half * 3.0;
    let back = half * 2.0;
    let bx = tip.x as f64 - length * ux;
    let by = tip.y as f64 - length * uy;
    Some(Cap {
        left: Point::new((bx + px * half) as f32, (by + py * half) as f32),
        right: Point::new((bx - px * half) as f32, (by - py * half) as f32),
        control: Point::new(
            (tip.x as f64 - ux * back) as f32,
            (tip.y as f64 - uy * back) as f32,
        ),
    })
}

fn append_cap(g: &mut GeometryBuilder, tip: Point, cap: &Cap) {
    g.begin(tip, true);
    g.line(cap.left);
    g.quad(cap.control, cap.right);
    g.end(true);
}

/// `ComputeBasicArrowHeadPointsFromTangent` / `ComputeLineArrowHeadPointsFromTangent`.
fn base_points(tip: Point, tangent: Point, half: f64) -> Option<(Point, Point)> {
    let (ux, uy) = unit(tangent)?;
    let (px, py) = (-uy, ux);
    let length = half * 3.0;
    let bx = tip.x as f64 - length * ux;
    let by = tip.y as f64 - length * uy;
    Some((
        Point::new((bx + px * half) as f32, (by + py * half) as f32),
        Point::new((bx - px * half) as f32, (by - py * half) as f32),
    ))
}

/// `ComputeModernArrowHeadPointsFromTangent`.
fn modern_head(tip: Point, tangent: Point, size: f64) -> Option<(Point, Point)> {
    let (ux, uy) = unit(tangent)?;
    let (px, py) = (-uy, ux);
    let height = size * 2.5;
    let wing = size * 1.5 * (std::f64::consts::PI / 5.14).tan();
    let bx = tip.x as f64 - height * ux;
    let by = tip.y as f64 - height * uy;
    Some((
        Point::new((bx + px * wing) as f32, (by + py * wing) as f32),
        Point::new((bx - px * wing) as f32, (by - py * wing) as f32),
    ))
}

/// `ComputeArrowPoints`: shaft end left/right, wing left/right.
fn arrow_points(start: Point, end: Point, size: f64) -> Option<(Point, Point, Point, Point)> {
    let (ux, uy) = unit(Point::new(end.x - start.x, end.y - start.y))?;
    let (px, py) = (-uy, ux);
    let height = size * 2.5;
    let base = size * 1.5;
    let wing = base * (std::f64::consts::PI / 5.14).tan();
    let shaft = base * 0.25;
    let bx = end.x as f64 - height * ux;
    let by = end.y as f64 - height * uy;
    let at = |d: f64| Point::new((bx + px * d) as f32, (by + py * d) as f32);
    Some((at(shaft), at(-shaft), at(wing), at(-wing)))
}

/// `LineAnnotation.CreateLineGeometry`.
pub fn line_path(a: &Annotation) -> Option<Path> {
    let mut b = PathBuilder::new();
    b.move_to(a.start.x, a.start.y);
    if model::has_curve(a) {
        let c = model::quadratic_control_point(a);
        b.quad_to(c.x, c.y, a.end.x, a.end.y);
    } else {
        b.line_to(a.end.x, a.end.y);
    }
    b.finish()
}

/// `FreehandAnnotation.CreateSmoothedGeometry`: midpoints joined by quadratic curves.
pub fn freehand_path(points: &[Point]) -> Option<Path> {
    if points.len() < 2 {
        return None;
    }
    let mut b = PathBuilder::new();
    b.move_to(points[0].x, points[0].y);
    if points.len() == 2 {
        b.line_to(points[1].x, points[1].y);
    } else {
        let mid = model::midpoint(points[0], points[1]);
        b.line_to(mid.x, mid.y);
        for i in 1..points.len() - 1 {
            let c = points[i];
            let next = model::midpoint(c, points[i + 1]);
            b.quad_to(c.x, c.y, next.x, next.y);
        }
        let last = points[points.len() - 1];
        b.line_to(last.x, last.y);
    }
    b.finish()
}

/// `RectangleGeometry(rect, rx, ry)` with Skia's radius clamping.
pub fn rounded_rect_path(r: Rect, rx: f32, ry: f32) -> Option<Path> {
    let w = r.width();
    let h = r.height();
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let mut rx = rx.max(0.0);
    let mut ry = ry.max(0.0);
    if rx > 0.0 && ry > 0.0 {
        let scale = (w / (rx + rx)).min(h / (ry + ry)).min(1.0);
        rx *= scale;
        ry *= scale;
    }
    if rx <= 0.0 || ry <= 0.0 {
        return Some(PathBuilder::from_rect(SkRect::from_ltrb(
            r.left, r.top, r.right, r.bottom,
        )?));
    }
    // Quarter ellipses as cubic Béziers (kappa).
    const K: f32 = 0.552_284_8;
    let (l, t, rt, b) = (r.left, r.top, r.right, r.bottom);
    let mut p = PathBuilder::new();
    p.move_to(l + rx, t);
    p.line_to(rt - rx, t);
    p.cubic_to(rt - rx + K * rx, t, rt, t + ry - K * ry, rt, t + ry);
    p.line_to(rt, b - ry);
    p.cubic_to(rt, b - ry + K * ry, rt - rx + K * rx, b, rt - rx, b);
    p.line_to(l + rx, b);
    p.cubic_to(l + rx - K * rx, b, l, b - ry + K * ry, l, b - ry);
    p.line_to(l, t + ry);
    p.cubic_to(l, t + ry - K * ry, l + rx - K * rx, t, l + rx, t);
    p.close();
    p.finish()
}

pub fn ellipse_path(r: Rect) -> Option<Path> {
    let rect = SkRect::from_ltrb(r.left, r.top, r.right, r.bottom)?;
    PathBuilder::from_oval(rect)
}

/// `Rect(Bounds.Size).Deflate(StrokeThickness / 2)`, the geometry Avalonia's `Rectangle`
/// and `Ellipse` shapes draw.
fn deflated(bounds: Rect, stroke: f32) -> Rect {
    let half = stroke / 2.0;
    let w = (bounds.width() - stroke).max(0.0);
    let h = (bounds.height() - stroke).max(0.0);
    Rect::from_xywh(bounds.left + half, bounds.top + half, w, h)
}

/// The transform of a control rotated about its centre (`RenderTransformOrigin` 0.5).
pub fn rotation_about(center: Point, angle: f32, base: Transform) -> Transform {
    if angle == 0.0 {
        return base;
    }
    base.pre_translate(center.x, center.y)
        .pre_rotate(angle)
        .pre_translate(-center.x, -center.y)
}

/// Approximate device-space bounds of what an annotation draws, for layers.
fn visual_bounds(a: &Annotation) -> Rect {
    let base = match &a.kind {
        Kind::Number(_) | Kind::SpeechBalloon(_) => model::interaction_bounds(a, 0.0),
        _ => a.bounds(),
    };
    let reach = a.stroke_width * 4.0 + 4.0;
    let r = base.inflate(reach, reach);
    if a.rotation == 0.0 {
        return r;
    }
    let c = a.bounds().center();
    let corners = [
        Point::new(r.left, r.top),
        Point::new(r.right, r.top),
        Point::new(r.left, r.bottom),
        Point::new(r.right, r.bottom),
    ]
    .map(|p| p.rotate_around(c, a.rotation));
    let mut out = Rect::new(corners[0].x, corners[0].y, corners[0].x, corners[0].y);
    for p in corners {
        out.left = out.left.min(p.x);
        out.top = out.top.min(p.y);
        out.right = out.right.max(p.x);
        out.bottom = out.bottom.max(p.y);
    }
    out
}

/// Avalonia's `SkBlurRadiusToSigma`.
pub fn blur_radius_to_sigma(radius: f64) -> f32 {
    if radius <= 0.0 {
        0.0
    } else {
        0.288_675 * radius as f32 + 0.5
    }
}

fn transform_scale(t: &Transform) -> f32 {
    ((t.sx * t.sx + t.ky * t.ky).sqrt() + (t.kx * t.kx + t.sy * t.sy).sqrt()) / 2.0
}

fn map_rect(t: &Transform, r: Rect) -> Rect {
    let mut pts = [
        tiny_skia::Point::from_xy(r.left, r.top),
        tiny_skia::Point::from_xy(r.right, r.top),
        tiny_skia::Point::from_xy(r.left, r.bottom),
        tiny_skia::Point::from_xy(r.right, r.bottom),
    ];
    t.map_points(&mut pts);
    let mut out = Rect::new(pts[0].x, pts[0].y, pts[0].x, pts[0].y);
    for p in pts {
        out.left = out.left.min(p.x);
        out.top = out.top.min(p.y);
        out.right = out.right.max(p.x);
        out.bottom = out.bottom.max(p.y);
    }
    out
}

/// Draws one annotation; a drop shadow, when enabled, is painted under it the way
/// `DropShadowEffect` does: the content's alpha, blurred, tinted and offset.
pub fn draw_annotation(target: &mut Pixmap, a: &Annotation, transform: Transform) {
    let shadow_capable = !matches!(
        a.kind,
        Kind::Effect(_)
            | Kind::Spotlight(_)
            | Kind::SmartEraser { .. }
            | Kind::Crop
            | Kind::CutOut { .. }
    ) && !a.is_image();
    if !(a.shadow.enabled && shadow_capable) {
        draw_content(target, a, transform);
        return;
    }
    let shadow_color = Color::parse(&a.shadow.color).unwrap_or(Color::argb(0x80, 0, 0, 0));
    let alpha = (shadow_color.a as f64 * a.shadow.opacity.clamp(0.0, 1.0)).clamp(0.0, 255.0) as u8;
    let scale = transform_scale(&transform);
    let sigma = blur_radius_to_sigma(a.shadow.blur_radius.max(0.0)) * scale;
    // The offset is in the control's coordinates: it follows rotation and zoom.
    let local = rotation_about(a.bounds().center(), a.rotation, Transform::identity());
    let mut offset = [tiny_skia::Point::from_xy(
        a.shadow.offset_x as f32,
        a.shadow.offset_y as f32,
    )];
    let linear = Transform::from_row(
        transform.sx,
        transform.ky,
        transform.kx,
        transform.sy,
        0.0,
        0.0,
    )
    .pre_concat(Transform::from_row(
        local.sx, local.ky, local.kx, local.sy, 0.0, 0.0,
    ));
    linear.map_points(&mut offset);
    let (ox, oy) = (offset[0].x, offset[0].y);

    let device = map_rect(&transform, visual_bounds(a));
    let reach = sigma * 3.0 + 2.0;
    let left = (device.left - reach).floor().max(-(ox.abs() + reach));
    let top = (device.top - reach).floor().max(-(oy.abs() + reach));
    let right = (device.right + reach).ceil();
    let bottom = (device.bottom + reach).ceil();
    let (w, h) = (
        (right - left).max(1.0) as u32,
        (bottom - top).max(1.0) as u32,
    );
    if w > 16384 || h > 16384 {
        draw_content(target, a, transform);
        return;
    }
    let Some(mut layer) = Pixmap::new(w, h) else {
        return;
    };
    let layer_transform = transform.post_translate(-left, -top);
    draw_content(&mut layer, a, layer_transform);
    if alpha > 0 {
        let mut shadow = layer.clone();
        for p in shadow.pixels_mut() {
            let la = p.alpha() as u32 * alpha as u32 / 255;
            let mul = |c: u8| (c as u32 * la / 255) as u8;
            *p = tiny_skia::PremultipliedColorU8::from_rgba(
                mul(shadow_color.r),
                mul(shadow_color.g),
                mul(shadow_color.b),
                la as u8,
            )
            .unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT);
        }
        crate::raster::gaussian_blur_pixmap(&mut shadow, sigma);
        target.draw_pixmap(
            0,
            0,
            shadow.as_ref(),
            &PixmapPaint {
                quality: FilterQuality::Bilinear,
                ..PixmapPaint::default()
            },
            Transform::from_translate(left + ox, top + oy),
            None,
        );
    }
    target.draw_pixmap(
        0,
        0,
        layer.as_ref(),
        &PixmapPaint::default(),
        Transform::from_translate(left, top),
        None,
    );
}

fn draw_content(target: &mut Pixmap, a: &Annotation, transform: Transform) {
    let stroke_color = parse(&a.stroke_color);
    let fill_color = parse(&a.fill_color);
    match &a.kind {
        Kind::Rectangle {
            border_style,
            corner_radius,
        } => {
            let bounds = a.bounds();
            let t = rotation_about(bounds.center(), a.rotation, transform);
            let r = deflated(bounds, a.stroke_width);
            let radius = (*corner_radius).max(0) as f32;
            if let Some(path) = rounded_rect_path(r, radius, radius) {
                fill_path(target, &path, fill_color, t, None);
                stroke_path(
                    target,
                    &path,
                    &Pen::border(stroke_color, a.stroke_width, *border_style),
                    t,
                    None,
                );
            }
        }
        Kind::SmartEraser { .. } => {
            let bounds = a.bounds();
            let r = deflated(bounds, a.stroke_width);
            match crate::annot_effects::smart_eraser_fill_bitmap(a) {
                Some(bitmap) => {
                    draw_bitmap_fill(target, &bitmap, r, transform, FilterQuality::Nearest, None)
                }
                None => {
                    if let Some(path) = rounded_rect_path(r, 0.0, 0.0) {
                        fill_path(target, &path, fill_color, transform, None);
                    }
                }
            }
        }
        Kind::Ellipse { border_style } => {
            let bounds = a.bounds();
            let t = rotation_about(bounds.center(), a.rotation, transform);
            if let Some(path) = ellipse_path(deflated(bounds, a.stroke_width)) {
                fill_path(target, &path, fill_color, t, None);
                stroke_path(
                    target,
                    &path,
                    &Pen::border(stroke_color, a.stroke_width, *border_style),
                    t,
                    None,
                );
            }
        }
        Kind::Line { border_style, .. } => {
            if let Some(path) = line_path(a) {
                let mut pen = Pen::border(stroke_color, a.stroke_width, *border_style);
                pen.join = LineJoin::Round;
                stroke_path(target, &path, &pen, transform, None);
            }
        }
        Kind::Arrow { .. } => {
            let g = arrow_geometry(a);
            if let Some(fill) = &g.fill {
                fill_path(target, fill, stroke_color, transform, None);
            }
            if let Some(stroke) = &g.stroke {
                let mut pen = Pen::new(stroke_color, a.stroke_width);
                pen.cap = LineCap::Round;
                pen.join = LineJoin::Round;
                stroke_path(target, stroke, &pen, transform, None);
            }
        }
        Kind::Freehand {
            points,
            border_style,
            ..
        } => {
            if let Some(path) = freehand_path(points) {
                let mut pen = Pen::border(stroke_color, a.stroke_width, *border_style);
                pen.join = LineJoin::Round;
                stroke_path(target, &path, &pen, transform, None);
            }
        }
        Kind::Text(data) => draw_outlined_text(target, a, data, transform),
        Kind::SpeechBalloon(data) => draw_speech_balloon(target, a, data, transform),
        Kind::Number(data) => draw_step(target, a, data, transform),
        Kind::Image { bitmap, .. } | Kind::Cursor { bitmap, .. } => {
            if let Some(bitmap) = bitmap {
                draw_image_uniform(target, bitmap, a.bounds(), a.rotation, transform);
            }
        }
        Kind::Emoji {
            unicode, bitmap, ..
        } => {
            let bounds = a.bounds();
            let size = bounds.width().max(bounds.height()).ceil().max(1.0) as u32;
            let image = bitmap
                .clone()
                .filter(|b| b.width() == size)
                .or_else(|| crate::emoji::sticker(unicode, size));
            if let Some(image) = image {
                draw_image_uniform(target, &image, bounds, a.rotation, transform);
            }
        }
        Kind::Effect(data) => draw_effect(target, a, data, transform),
        Kind::Spotlight(_) | Kind::Crop | Kind::CutOut { .. } => {}
    }
}

/// An `ImageBrush` with `Stretch.Fill` filling `rect`.
fn draw_bitmap_fill(
    target: &mut Pixmap,
    bitmap: &RgbaImage,
    rect: Rect,
    transform: Transform,
    quality: FilterQuality,
    mask: Option<&Mask>,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 || bitmap.width() == 0 {
        return;
    }
    let pixmap = crate::raster::to_pixmap(bitmap);
    let sx = rect.width() / bitmap.width() as f32;
    let sy = rect.height() / bitmap.height() as f32;
    let pattern = Pattern::new(
        pixmap.as_ref(),
        SpreadMode::Pad,
        quality,
        1.0,
        Transform::from_row(sx, 0.0, 0.0, sy, rect.left, rect.top),
    );
    let paint = Paint {
        shader: pattern,
        anti_alias: true,
        ..Paint::default()
    };
    if let Some(r) = SkRect::from_ltrb(rect.left, rect.top, rect.right, rect.bottom) {
        let path = PathBuilder::from_rect(r);
        target.fill_path(&path, &paint, FillRule::Winding, transform, mask);
    }
}

/// An `Image` control (`Stretch.Uniform`): the bitmap fitted into the bounds, centred.
fn draw_image_uniform(
    target: &mut Pixmap,
    bitmap: &Arc<RgbaImage>,
    bounds: Rect,
    rotation: f32,
    transform: Transform,
) {
    if bitmap.width() == 0 || bitmap.height() == 0 {
        return;
    }
    let w = bounds.width().max(1.0);
    let h = bounds.height().max(1.0);
    let scale = (w / bitmap.width() as f32).min(h / bitmap.height() as f32);
    let dw = bitmap.width() as f32 * scale;
    let dh = bitmap.height() as f32 * scale;
    let rect = Rect::from_xywh(
        bounds.left + (w - dw) / 2.0,
        bounds.top + (h - dh) / 2.0,
        dw,
        dh,
    );
    let t = rotation_about(
        Point::new(bounds.left + w / 2.0, bounds.top + h / 2.0),
        rotation,
        transform,
    );
    draw_bitmap_fill(target, bitmap, rect, t, FilterQuality::Bilinear, None);
}

fn draw_effect(
    target: &mut Pixmap,
    a: &Annotation,
    data: &model::EffectData,
    transform: Transform,
) {
    let bounds = a.bounds();
    let t = rotation_about(bounds.center(), a.rotation, transform);
    let r = deflated(bounds, a.stroke_width);
    let ellipse_clip = data.effect == Some(EffectKind::Magnify) && data.is_ellipse;
    let mask = if ellipse_clip {
        ellipse_path(Rect::from_xywh(
            bounds.left,
            bounds.top,
            bounds.width().max(1.0),
            bounds.height().max(1.0),
        ))
        .and_then(|path| {
            let mut mask = Mask::new(target.width(), target.height())?;
            mask.fill_path(&path, FillRule::Winding, true, t);
            Some(mask)
        })
    } else {
        None
    };
    match &data.bitmap {
        Some(bitmap) => {
            draw_bitmap_fill(target, bitmap, r, t, FilterQuality::Bilinear, mask.as_ref())
        }
        None => {
            // The visual's own fill until the effect is computed.
            let placeholder = match data.effect {
                Some(EffectKind::Blur) => Color::argb(0x20, 0, 0, 0xFF),
                Some(EffectKind::Pixelate) => Color::argb(0x20, 0, 0xFF, 0),
                Some(EffectKind::Highlight) => {
                    let base = Color::parse(&a.fill_color).unwrap_or(Color::rgb(255, 255, 0));
                    base.with_alpha(0x55)
                }
                _ => Color::TRANSPARENT,
            };
            if let Some(path) = rounded_rect_path(r, 0.0, 0.0) {
                fill_path(target, &path, placeholder, t, mask.as_ref());
            }
        }
    }
}

/// `OutlinedTextControl.Render`: padded, wrapped, aligned text whose outline is stroked
/// (round joins) and then filled, clipped to the control.
fn draw_outlined_text(
    target: &mut Pixmap,
    a: &Annotation,
    data: &model::TextData,
    transform: Transform,
) {
    if data.text.is_empty() {
        return;
    }
    const PADDING: f32 = 8.0;
    let bounds = a.bounds();
    let t = rotation_about(bounds.center(), a.rotation, transform);
    let pad = PADDING + a.stroke_width;
    let max_width = (bounds.width() - pad * 2.0).max(0.0);
    let request = FontRequest {
        family: data.font_family.clone(),
        bold: data.bold,
        italic: data.italic,
    };
    let laid = text::layout(
        &data.text,
        &request,
        data.font_size,
        (max_width > 0.0).then_some(max_width),
        data.align,
    );
    let mut origin_y = pad.max((bounds.height() - laid.height) / 2.0);
    origin_y = origin_y.min(pad.max(bounds.height() - laid.height - pad));
    let Some(path) = text::build_geometry(&laid, bounds.left + pad, bounds.top + origin_y) else {
        return;
    };
    let clip =
        SkRect::from_ltrb(bounds.left, bounds.top, bounds.right, bounds.bottom).and_then(|r| {
            let mut mask = Mask::new(target.width(), target.height())?;
            mask.fill_path(&PathBuilder::from_rect(r), FillRule::Winding, false, t);
            Some(mask)
        });
    let embolden = laid.embolden_width();
    let text_color = parse(&data.text_color);
    if a.stroke_width > 0.0 {
        let mut pen = Pen::new(parse(&a.stroke_color), a.stroke_width + embolden);
        pen.join = LineJoin::Round;
        stroke_path(target, &path, &pen, t, clip.as_ref());
    }
    fill_text(target, &path, text_color, embolden, t, clip.as_ref());
}

/// Fills glyph outlines, emboldened like Skia's fake bold when the face was simulated.
fn fill_text(
    target: &mut Pixmap,
    path: &Path,
    color: Color,
    embolden: f32,
    transform: Transform,
    clip: Option<&Mask>,
) {
    if color.a == 0 {
        return;
    }
    fill_path(target, path, color, transform, clip);
    if embolden > 0.0 {
        let mut pen = Pen::new(color, embolden);
        pen.join = LineJoin::Miter;
        stroke_path(target, path, &pen, transform, clip);
    }
}

/// The union of polygons (`CombinedGeometry(Union)`), as one path.
fn union_path(polygons: Vec<Vec<[f64; 2]>>) -> Option<Path> {
    use i_overlay::core::fill_rule::FillRule as OverlayFill;
    use i_overlay::core::overlay_rule::OverlayRule;
    use i_overlay::float::single::SingleFloatOverlay;
    let mut iter = polygons.into_iter();
    let first = iter.next()?;
    let mut shapes: Vec<Vec<Vec<[f64; 2]>>> = vec![vec![first]];
    for polygon in iter {
        let subject: Vec<Vec<[f64; 2]>> = shapes.into_iter().flatten().collect();
        shapes = subject.overlay(&polygon, OverlayRule::Union, OverlayFill::NonZero);
    }
    let mut b = PathBuilder::new();
    for shape in shapes {
        for contour in shape {
            if contour.len() < 3 {
                continue;
            }
            b.move_to(contour[0][0] as f32, contour[0][1] as f32);
            for p in &contour[1..] {
                b.line_to(p[0] as f32, p[1] as f32);
            }
            b.close();
        }
    }
    b.finish()
}

fn flatten_ellipse(center: Point, rx: f32, ry: f32, segments: usize) -> Vec<[f64; 2]> {
    (0..segments)
        .map(|i| {
            let a = i as f64 / segments as f64 * std::f64::consts::TAU;
            [
                center.x as f64 + rx as f64 * a.cos(),
                center.y as f64 + ry as f64 * a.sin(),
            ]
        })
        .collect()
}

fn segments_for(radius: f32, scale: f32) -> usize {
    ((radius * scale * std::f32::consts::TAU / 2.0).ceil() as usize).clamp(24, 720)
}

/// `StepControl.Render`.
fn draw_step(target: &mut Pixmap, a: &Annotation, data: &model::NumberData, transform: Transform) {
    let body = a.bounds();
    let width = body.width().max(2.0);
    let height = body.height().max(2.0);
    let center = Point::new(body.left + width / 2.0, body.top + height / 2.0);
    let path = match model::number_tail_polygon(a) {
        Some((s, tip, e)) => {
            let segments = segments_for(width.max(height) / 2.0, transform_scale(&transform));
            union_path(vec![
                flatten_ellipse(center, width / 2.0, height / 2.0, segments),
                vec![
                    [s.x as f64, s.y as f64],
                    [tip.x as f64, tip.y as f64],
                    [e.x as f64, e.y as f64],
                ],
            ])
        }
        None => ellipse_path(Rect::from_xywh(body.left, body.top, width, height)),
    };
    if let Some(path) = &path {
        fill_path(target, path, parse(&a.fill_color), transform, None);
        stroke_path(
            target,
            path,
            &Pen::new(parse(&a.stroke_color), a.stroke_width),
            transform,
            None,
        );
    }
    let label = model::number_display_text(data);
    let request = FontRequest {
        family: text::DEFAULT_FAMILY.into(),
        bold: data.bold,
        italic: false,
    };
    let laid = text::layout(
        &label,
        &request,
        data.font_size * 0.6,
        None,
        TextAlign::Left,
    );
    let x = body.left + (width - laid.width) / 2.0;
    let y = body.top + (height - laid.height) / 2.0;
    if let Some(glyphs) = text::build_geometry(&laid, x, y) {
        fill_text(
            target,
            &glyphs,
            parse(&data.text_color),
            laid.embolden_width(),
            transform,
            None,
        );
    }
}

/// `SpeechBalloonControl.Render`.
fn draw_speech_balloon(
    target: &mut Pixmap,
    a: &Annotation,
    data: &model::BalloonData,
    transform: Transform,
) {
    let body = a.bounds();
    let t = rotation_about(body.center(), a.rotation, transform);
    let width = body.width().max(20.0);
    let height = body.height().max(20.0);
    let radius = (data.corner_radius as f32).clamp(0.0, ((width.min(height)) / 2.0).floor());
    let body_rect = Rect::from_xywh(body.left, body.top, width, height);
    let path = match model::balloon_tail_polygon(a) {
        Some((s, tip, e)) => {
            let segments = segments_for(radius, transform_scale(&transform));
            union_path(vec![
                rounded_rect_polygon(body_rect, radius, segments / 4 + 2),
                vec![
                    [s.x as f64, s.y as f64],
                    [tip.x as f64, tip.y as f64],
                    [e.x as f64, e.y as f64],
                ],
            ])
        }
        None => rounded_rect_path(body_rect, radius, radius),
    };
    if let Some(path) = &path {
        fill_path(target, path, parse(&a.fill_color), t, None);
        stroke_path(
            target,
            path,
            &Pen::new(parse(&a.stroke_color), a.stroke_width),
            t,
            None,
        );
    }
    let text_data = &data.text;
    if text_data.text.is_empty() {
        return;
    }
    const PADDING: f32 = 12.0;
    let max_width = (width - PADDING * 2.0).max(0.0);
    let request = FontRequest {
        family: if text_data.font_family.trim().is_empty() {
            "Segoe UI".into()
        } else {
            text_data.font_family.clone()
        },
        bold: text_data.bold,
        italic: text_data.italic,
    };
    let laid = text::layout(
        &text_data.text,
        &request,
        text_data.font_size,
        (max_width > 0.0).then_some(max_width),
        text_data.align,
    );
    let x = body.left + PADDING;
    let mut y = body.top + PADDING.max((height - laid.height) / 2.0);
    y = y.min(body.top + height - laid.height - PADDING);
    if let Some(glyphs) = text::build_geometry(&laid, x, y) {
        fill_text(
            target,
            &glyphs,
            parse(&text_data.text_color),
            laid.embolden_width(),
            t,
            None,
        );
    }
}

fn rounded_rect_polygon(r: Rect, radius: f32, arc_segments: usize) -> Vec<[f64; 2]> {
    if radius <= 0.0 {
        return vec![
            [r.left as f64, r.top as f64],
            [r.right as f64, r.top as f64],
            [r.right as f64, r.bottom as f64],
            [r.left as f64, r.bottom as f64],
        ];
    }
    let mut out = Vec::new();
    let corners = [
        (r.right - radius, r.top + radius, -90.0f64),
        (r.right - radius, r.bottom - radius, 0.0),
        (r.left + radius, r.bottom - radius, 90.0),
        (r.left + radius, r.top + radius, 180.0),
    ];
    for (cx, cy, start) in corners {
        for i in 0..=arc_segments {
            let angle = (start + 90.0 * i as f64 / arc_segments as f64).to_radians();
            out.push([
                cx as f64 + radius as f64 * angle.cos(),
                cy as f64 + radius as f64 * angle.sin(),
            ]);
        }
    }
    out
}

/// `SpotlightOverlayControl`: the (optionally blurred) image darkened by the strongest
/// spotlight, with every spotlight's area cleared. One layer for all of them, painted
/// above the image and below the annotations.
pub fn draw_spotlight_overlay(
    target: &mut Pixmap,
    transform: Transform,
    canvas_size: (f32, f32),
    annotations: &[Annotation],
    blurred_source: Option<&RgbaImage>,
) {
    let spotlights: Vec<(&Annotation, &model::SpotlightData)> = annotations
        .iter()
        .filter_map(|a| match &a.kind {
            Kind::Spotlight(s) => Some((a, s)),
            _ => None,
        })
        .collect();
    if spotlights.is_empty() {
        return;
    }
    let darken = spotlights
        .iter()
        .map(|(_, s)| s.darken_opacity)
        .max()
        .unwrap_or(0);
    let Some(mut layer) = Pixmap::new(target.width(), target.height()) else {
        return;
    };
    let (w, h) = (canvas_size.0.max(1.0).ceil(), canvas_size.1.max(1.0).ceil());
    let canvas_rect = Rect::new(0.0, 0.0, w, h);
    if let Some(blurred) = blurred_source {
        draw_bitmap_fill(
            &mut layer,
            blurred,
            canvas_rect,
            transform,
            FilterQuality::Bilinear,
            None,
        );
    }
    if let Some(path) = rounded_rect_path(canvas_rect, 0.0, 0.0) {
        fill_path(
            &mut layer,
            &path,
            Color::argb(darken, 0, 0, 0),
            transform,
            None,
        );
    }
    let clear = Paint {
        blend_mode: BlendMode::Clear,
        anti_alias: true,
        shader: Shader::SolidColor(tiny_skia::Color::BLACK),
        ..Paint::default()
    };
    for (a, s) in spotlights {
        let bounds = a.bounds();
        let path = if s.is_ellipse {
            ellipse_path(bounds)
        } else {
            rounded_rect_path(bounds, 0.0, 0.0)
        };
        if let Some(path) = path {
            layer.fill_path(&path, &clear, FillRule::Winding, transform, None);
        }
    }
    target.draw_pixmap(
        0,
        0,
        layer.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
}

/// The strongest spotlight blur (`spotlights.Max(BlurAmount)`).
pub fn spotlight_blur_amount(annotations: &[Annotation]) -> f32 {
    annotations
        .iter()
        .filter_map(|a| match &a.kind {
            Kind::Spotlight(s) => Some(s.blur_amount),
            _ => None,
        })
        .fold(0.0, f32::max)
}

/// Every annotation in order, the hidden one left out.
pub fn draw_annotations(target: &mut Pixmap, scene: &Scene, transform: Transform) {
    for a in scene.annotations {
        if Some(a.id) == scene.hidden {
            continue;
        }
        draw_annotation(target, a, transform);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(p: &Pixmap, x: u32, y: u32) -> [u8; 4] {
        let c = p.pixel(x, y).unwrap().demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    }

    #[test]
    fn rectangle_stroke_stays_inside_its_bounds() {
        let mut target = Pixmap::new(40, 40).unwrap();
        let mut r = Annotation::rectangle();
        r.start = Point::new(10.0, 10.0);
        r.end = Point::new(30.0, 30.0);
        r.stroke_width = 4.0;
        r.stroke_color = "#FFFF0000".into();
        draw_annotation(&mut target, &r, Transform::identity());
        assert_eq!(pixel(&target, 11, 20), [255, 0, 0, 255]);
        assert_eq!(pixel(&target, 9, 20)[3], 0);
        assert_eq!(pixel(&target, 20, 20)[3], 0);
    }

    #[test]
    fn classic_arrow_has_a_filled_head() {
        let mut target = Pixmap::new(120, 40).unwrap();
        let mut a = Annotation::arrow();
        a.start = Point::new(10.0, 20.0);
        a.end = Point::new(110.0, 20.0);
        a.stroke_color = "#FF0000FF".into();
        draw_annotation(&mut target, &a, Transform::identity());
        // The head is 8 px wide each side at width 4; behind the tip it is solid.
        assert_eq!(pixel(&target, 95, 24)[3], 255);
        assert_eq!(pixel(&target, 50, 26)[3], 0);
    }

    #[test]
    fn spotlight_darkens_outside_only() {
        let mut target = Pixmap::new(50, 50).unwrap();
        target.fill(tiny_skia::Color::WHITE);
        let mut s = Annotation::spotlight();
        s.start = Point::new(10.0, 10.0);
        s.end = Point::new(30.0, 30.0);
        draw_spotlight_overlay(&mut target, Transform::identity(), (50.0, 50.0), &[s], None);
        assert_eq!(pixel(&target, 20, 20), [255, 255, 255, 255]);
        let outside = pixel(&target, 40, 40);
        assert!(outside[0] < 100, "{outside:?}");
    }
}
