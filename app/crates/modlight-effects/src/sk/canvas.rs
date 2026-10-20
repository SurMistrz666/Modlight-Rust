//! `SKCanvas` and `SKPaint` as the effects use them. Shapes, strokes and gradients are
//! rasterised with tiny-skia (a port of Skia's raster pipeline); bitmaps are sampled and
//! blended here in floats as Skia's highp pipeline does (nearest, linear and the cubic
//! resamplers with their B and C), so `SKSamplingOptions` behave as upstream asks.

use tiny_skia::{
    FillRule, GradientStop, LineCap, LineJoin, Mask, Path, PathBuilder, Pixmap, Point, Rect,
    SpreadMode, Stroke, StrokeDash, Transform,
};

use super::bitmap::{Bitmap, premul, unpremul};
use super::blend::Blend;
use super::blur;
use super::color::C;
use super::filters::{F4, Tile, unorm};
use super::raster;

/// `SKPaintStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Style {
    #[default]
    Fill,
    Stroke,
    StrokeAndFill,
}

/// `SKStrokeCap`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Cap {
    #[default]
    Butt,
    Round,
    Square,
}

/// `SKStrokeJoin`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Join {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// `SKSamplingOptions`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Sampling {
    #[default]
    Nearest,
    Linear,
    /// `SKCubicResampler` (B, C): Mitchell is (1/3, 1/3), CatmullRom (0, 1/2).
    Cubic(f32, f32),
}

impl Sampling {
    pub const CATMULL_ROM: Sampling = Sampling::Cubic(0.0, 0.5);
    pub const MITCHELL: Sampling = Sampling::Cubic(1.0 / 3.0, 1.0 / 3.0);
}

/// `SKShader`.
#[derive(Clone, Debug)]
pub enum Shader {
    Linear {
        p0: (f32, f32),
        p1: (f32, f32),
        colors: Vec<C>,
        pos: Option<Vec<f32>>,
        tile: Tile,
    },
    Radial {
        center: (f32, f32),
        radius: f32,
        colors: Vec<C>,
        pos: Option<Vec<f32>>,
        tile: Tile,
    },
    Sweep {
        center: (f32, f32),
        colors: Vec<C>,
        pos: Option<Vec<f32>>,
    },
    /// `SKShader.CreateBitmap` / `CreateImage`.
    Image {
        bitmap: std::sync::Arc<Bitmap>,
        tile_x: Tile,
        tile_y: Tile,
        matrix: Transform,
        sampling: Sampling,
    },
}

/// `SKPaint`.
#[derive(Clone, Debug)]
pub struct Paint {
    pub color: C,
    pub style: Style,
    pub stroke_width: f32,
    pub cap: Cap,
    pub join: Join,
    pub miter: f32,
    pub aa: bool,
    pub blend: Blend,
    pub shader: Option<Shader>,
    /// `SKMaskFilter.CreateBlur(Normal, sigma)`.
    pub mask_blur: Option<f32>,
    /// `SKImageFilter.CreateBlur(sx, sy)`.
    pub image_blur: Option<(f32, f32)>,
    /// `SKPathEffect.CreateDash(intervals, phase)`.
    pub dash: Option<(Vec<f32>, f32)>,
    pub color_filter: Option<ColorFilter>,
}

impl Default for Paint {
    fn default() -> Self {
        Paint {
            color: C::BLACK,
            style: Style::Fill,
            stroke_width: 0.0,
            cap: Cap::Butt,
            join: Join::Miter,
            miter: 4.0,
            aa: false,
            blend: Blend::SrcOver,
            shader: None,
            mask_blur: None,
            image_blur: None,
            dash: None,
            color_filter: None,
        }
    }
}

impl Paint {
    pub fn fill(color: C) -> Paint {
        Paint {
            color,
            ..Paint::default()
        }
    }

    pub fn fill_aa(color: C) -> Paint {
        Paint {
            color,
            aa: true,
            ..Paint::default()
        }
    }

    pub fn stroke(color: C, width: f32) -> Paint {
        Paint {
            color,
            style: Style::Stroke,
            stroke_width: width,
            ..Paint::default()
        }
    }

    pub fn stroke_aa(color: C, width: f32) -> Paint {
        Paint {
            color,
            style: Style::Stroke,
            stroke_width: width,
            aa: true,
            ..Paint::default()
        }
    }

    pub fn aa(mut self, aa: bool) -> Paint {
        self.aa = aa;
        self
    }

    pub fn with_blend(mut self, blend: Blend) -> Paint {
        self.blend = blend;
        self
    }

    pub fn with_shader(mut self, shader: Shader) -> Paint {
        self.shader = Some(shader);
        self
    }

    pub fn with_cap(mut self, cap: Cap) -> Paint {
        self.cap = cap;
        self
    }

    pub fn with_join(mut self, join: Join) -> Paint {
        self.join = join;
        self
    }

    pub fn with_mask_blur(mut self, sigma: f32) -> Paint {
        self.mask_blur = Some(sigma);
        self
    }

    pub fn with_image_blur(mut self, sx: f32, sy: f32) -> Paint {
        self.image_blur = Some((sx, sy));
        self
    }

    /// The paint used for bitmap draws: only alpha, blend, filters matter.
    pub fn alpha(a: u8) -> Paint {
        Paint {
            color: C::new(255, 255, 255, a),
            ..Paint::default()
        }
    }
}

/// `SKColorFilter`.
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ColorFilter {
    Matrix([f32; 20]),
    /// `CreateBlendMode(color, mode)`.
    Blend(C, Blend),
    /// `CreateCompose(outer, inner)`: inner first.
    Compose(Box<ColorFilter>, Box<ColorFilter>),
    Table([u8; 256], [u8; 256], [u8; 256]),
}

impl ColorFilter {
    /// Applies the filter to one premultiplied pixel.
    pub fn apply(&self, p: F4) -> F4 {
        match self {
            ColorFilter::Matrix(m) => super::filters::color_matrix_pixel(p, m),
            ColorFilter::Blend(c, mode) => super::filters::blend_filter_pixel(p, *c, *mode),
            ColorFilter::Compose(outer, inner) => outer.apply(inner.apply(p)),
            ColorFilter::Table(r, g, b) => {
                let u = p.unpremul();
                F4 {
                    r: r[unorm(u.r) as usize] as f32 / 255.0,
                    g: g[unorm(u.g) as usize] as f32 / 255.0,
                    b: b[unorm(u.b) as usize] as f32 / 255.0,
                    a: u.a,
                }
                .premul()
            }
        }
    }
}

/// `new SKRect(l, t, r, b)`.
pub fn rect(l: f32, t: f32, r: f32, b: f32) -> Rect {
    Rect::from_ltrb(l, t, r.max(l + f32::EPSILON), b.max(t + f32::EPSILON))
        .unwrap_or_else(|| Rect::from_ltrb(0.0, 0.0, 1.0, 1.0).unwrap())
}

/// `SKRect.Create(x, y, w, h)`.
pub fn rect_xywh(x: f32, y: f32, w: f32, h: f32) -> Rect {
    rect(x, y, x + w, y + h)
}

#[derive(Clone)]
struct State {
    matrix: Transform,
    clip: Option<Mask>,
}

/// `SKCanvas` over an 8888 premultiplied surface.
pub struct Canvas {
    pub pm: Pixmap,
    state: State,
    stack: Vec<State>,
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Canvas {
        Canvas {
            pm: Pixmap::new(w.max(1) as u32, h.max(1) as u32).unwrap(),
            state: State {
                matrix: Transform::identity(),
                clip: None,
            },
            stack: Vec::new(),
        }
    }

    /// A canvas drawing into a copy of `bitmap` (`new SKCanvas(bitmap)` on a copy).
    pub fn from_bitmap(bitmap: &Bitmap) -> Canvas {
        Canvas {
            pm: bitmap.to_pixmap(),
            state: State {
                matrix: Transform::identity(),
                clip: None,
            },
            stack: Vec::new(),
        }
    }

    pub fn width(&self) -> i32 {
        self.pm.width() as i32
    }

    pub fn height(&self) -> i32 {
        self.pm.height() as i32
    }

    pub fn into_bitmap(self) -> Bitmap {
        Bitmap::from_pixmap(&self.pm)
    }

    pub fn to_bitmap(&self) -> Bitmap {
        Bitmap::from_pixmap(&self.pm)
    }

    /// `Clear(color)`: replaces every pixel (ignores the clip, as Skia's clear of the
    /// whole device does when nothing is clipped).
    pub fn clear(&mut self, c: C) {
        let p = premul(c);
        if let Some(mask) = &self.state.clip {
            let mask = mask.clone();
            let data = self.pm.data_mut();
            for (i, m) in mask.data().iter().enumerate() {
                if *m == 0 {
                    continue;
                }
                let t = *m as f32 / 255.0;
                for (k, v) in [p.r, p.g, p.b, p.a].iter().enumerate() {
                    let d = data[i * 4 + k] as f32;
                    data[i * 4 + k] = (d + (*v as f32 - d) * t).round() as u8;
                }
            }
            return;
        }
        for px in self.pm.data_mut().chunks_exact_mut(4) {
            px.copy_from_slice(&[p.r, p.g, p.b, p.a]);
        }
    }

    // ---------- matrix and clip ----------

    pub fn save(&mut self) {
        self.stack.push(self.state.clone());
    }

    pub fn restore(&mut self) {
        if let Some(s) = self.stack.pop() {
            self.state = s;
        }
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.state.matrix = self.state.matrix.pre_translate(dx, dy);
    }

    pub fn scale(&mut self, sx: f32, sy: f32) {
        self.state.matrix = self.state.matrix.pre_scale(sx, sy);
    }

    /// `Scale(sx, sy, px, py)`.
    pub fn scale_about(&mut self, sx: f32, sy: f32, px: f32, py: f32) {
        self.translate(px, py);
        self.scale(sx, sy);
        self.translate(-px, -py);
    }

    pub fn rotate_degrees(&mut self, deg: f32) {
        self.state.matrix = self.state.matrix.pre_concat(rotation(deg));
    }

    /// `RotateDegrees(deg, px, py)`.
    pub fn rotate_degrees_about(&mut self, deg: f32, px: f32, py: f32) {
        self.translate(px, py);
        self.rotate_degrees(deg);
        self.translate(-px, -py);
    }

    pub fn skew(&mut self, sx: f32, sy: f32) {
        self.state.matrix = self
            .state
            .matrix
            .pre_concat(Transform::from_row(1.0, sy, sx, 1.0, 0.0, 0.0));
    }

    pub fn concat(&mut self, m: Transform) {
        self.state.matrix = self.state.matrix.pre_concat(m);
    }

    pub fn set_matrix(&mut self, m: Transform) {
        self.state.matrix = m;
    }

    pub fn reset_matrix(&mut self) {
        self.state.matrix = Transform::identity();
    }

    pub fn total_matrix(&self) -> Transform {
        self.state.matrix
    }

    fn clip_with(&mut self, path: &Path, aa: bool) {
        let (w, h) = (self.pm.width(), self.pm.height());
        let mut mask = Mask::new(w, h).unwrap();
        let device = path.clone().transform(self.state.matrix);
        match device {
            Some(d) if aa => {
                mask = if raster::should_use_analytic(&d, is_axis_rect(&d)) {
                    raster::coverage(&raster::flatten_analytic(&d, Transform::identity()), w, h)
                } else {
                    raster::supersampled(&raster::flatten(&d, Transform::identity()), w, h)
                };
            }
            _ => mask.fill_path(path, FillRule::Winding, aa, self.state.matrix),
        }
        if let Some(old) = &self.state.clip {
            for (m, o) in mask.data_mut().iter_mut().zip(old.data()) {
                *m = ((*m as u32 * *o as u32 + 127) / 255) as u8;
            }
        }
        self.state.clip = Some(mask);
    }

    pub fn clip_rect(&mut self, r: Rect, aa: bool) {
        let path = PathBuilder::from_rect(r);
        self.clip_with(&path, aa);
    }

    pub fn clip_path(&mut self, path: &Path, aa: bool) {
        self.clip_with(path, aa);
    }

    pub fn clip_round_rect(&mut self, r: Rect, rx: f32, ry: f32, aa: bool) {
        if let Some(path) = round_rect_path(r, rx, ry) {
            self.clip_with(&path, aa);
        }
    }

    // ---------- shapes ----------

    pub fn draw_rect(&mut self, r: Rect, paint: &Paint) {
        let path = PathBuilder::from_rect(r);
        let m = self.state.matrix;
        // `SkDraw::ComputeRectType`: a plain stroked rect is framed (`AntiFrameRect`), never
        // turned into a hairline unless its width is zero.
        let frame = paint.mask_blur.is_none()
            && paint.dash.is_none()
            && paint.join == Join::Miter
            && paint.miter > std::f32::consts::SQRT_2
            && m.kx == 0.0
            && m.ky == 0.0;
        self.draw_path_impl(&path, paint, m, frame);
    }

    pub fn draw_rect_ltrb(&mut self, l: f32, t: f32, r: f32, b: f32, paint: &Paint) {
        if r <= l || b <= t {
            // Skia sorts the rect; an empty one draws nothing when filled.
            if paint.style == Style::Fill {
                return;
            }
        }
        let (l2, r2) = if l <= r { (l, r) } else { (r, l) };
        let (t2, b2) = if t <= b { (t, b) } else { (b, t) };
        if let Some(rc) = Rect::from_ltrb(l2, t2, r2, b2) {
            self.draw_rect(rc, paint);
        } else if paint.style != Style::Fill {
            let mut pb = PathBuilder::new();
            pb.move_to(l2, t2);
            pb.line_to(r2, t2);
            pb.line_to(r2, b2);
            pb.line_to(l2, b2);
            pb.close();
            if let Some(p) = pb.finish() {
                self.draw_path(&p, paint);
            }
        }
    }

    /// `DrawRect(x, y, w, h, paint)`.
    pub fn draw_rect_xywh(&mut self, x: f32, y: f32, w: f32, h: f32, paint: &Paint) {
        self.draw_rect_ltrb(x, y, x + w, y + h, paint);
    }

    pub fn draw_round_rect(&mut self, r: Rect, rx: f32, ry: f32, paint: &Paint) {
        if let Some(path) = round_rect_path(r, rx, ry) {
            self.draw_path(&path, paint);
        }
    }

    pub fn draw_circle(&mut self, cx: f32, cy: f32, radius: f32, paint: &Paint) {
        if radius <= 0.0 {
            return;
        }
        if let Some(path) = oval_path(cx - radius, cy - radius, cx + radius, cy + radius) {
            self.draw_path(&path, paint);
        }
    }

    pub fn draw_oval(&mut self, r: Rect, paint: &Paint) {
        if let Some(path) = oval_path(r.left(), r.top(), r.right(), r.bottom()) {
            self.draw_path(&path, paint);
        }
    }

    pub fn draw_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, paint: &Paint) {
        let mut pb = PathBuilder::new();
        pb.move_to(x0, y0);
        pb.line_to(x1, y1);
        if let Some(path) = pb.finish() {
            let mut p = paint.clone();
            if p.style == Style::Fill {
                // Lines are always stroked.
                p.style = Style::Stroke;
            }
            self.draw_path(&path, &p);
        }
    }

    /// `DrawPaint`: fills the whole canvas.
    pub fn draw_paint(&mut self, paint: &Paint) {
        let (w, h) = (self.pm.width() as f32, self.pm.height() as f32);
        let save = self.state.matrix;
        self.state.matrix = Transform::identity();
        let mut p = paint.clone();
        p.style = Style::Fill;
        if let Some(shader) = &mut p.shader {
            // The shader keeps the canvas matrix.
            let _ = shader;
        }
        let path = PathBuilder::from_rect(rect(0.0, 0.0, w, h));
        self.draw_path_with(&path, &p, save);
        self.state.matrix = save;
    }

    pub fn draw_path(&mut self, path: &Path, paint: &Paint) {
        let m = self.state.matrix;
        self.draw_path_with(path, paint, m);
    }

    fn draw_path_with(&mut self, path: &Path, paint: &Paint, shader_matrix: Transform) {
        self.draw_path_impl(path, paint, shader_matrix, false);
    }

    fn draw_path_impl(
        &mut self,
        path: &Path,
        paint: &Paint,
        shader_matrix: Transform,
        rect_frame: bool,
    ) {
        let matrix = self.state.matrix;
        let (w, h) = (self.pm.width(), self.pm.height());
        if let Some(Shader::Image {
            bitmap,
            tile_x,
            tile_y,
            matrix: local,
            sampling,
        }) = &paint.shader
        {
            self.fill_with_image(path, paint, bitmap, *tile_x, *tile_y, *local, *sampling);
            return;
        }
        let Some(ts_paint) = self.ts_paint(paint, shader_matrix, matrix) else {
            return;
        };
        let stroke = self.ts_stroke(paint);
        let needs_layer =
            paint.mask_blur.is_some() || paint.image_blur.is_some() || paint.color_filter.is_some();
        // The outlines Skia fills, in device space.
        let mut outlines: Vec<Path> = Vec::new();
        if paint.style != Style::Stroke {
            outlines.push(path.clone());
        }
        // `SkDrawTreatAsHairline`: zero-width strokes, and anti-aliased strokes at most one
        // device pixel wide (drawn as hairlines with their width as coverage).
        let hairline = paint.style == Style::Stroke
            && (paint.stroke_width <= 0.0
                || (paint.aa && !rect_frame && thin_stroke(paint.stroke_width, matrix)));
        let stroked = paint.style != Style::Fill && !hairline && paint.stroke_width > 0.0;
        let single_line = if paint.dash.is_none() {
            single_segment(path)
        } else {
            None
        };
        if let (Some((a, b)), true) = (single_line, stroked) {
            if let Some(o) = line_outline(a, b, paint.stroke_width, paint.cap) {
                outlines.push(o);
            }
        } else if stroked {
            let res = matrix_scale(matrix);
            // A dash whose first interval is longer than the contour leaves the closed
            // contour whole in Skia (no butt-capped seam at its start).
            let whole = paint.dash.as_ref().is_some_and(|(i, phase)| {
                *phase == 0.0 && i.first().is_some_and(|f| *f >= path_length(path))
            });
            let dashed = match &stroke.dash {
                Some(d) if !whole => path.dash(d, res),
                _ => Some(path.clone()),
            };
            if let Some(o) = dashed.and_then(|p| {
                let mut st = stroke.clone();
                st.dash = None;
                p.stroke(&st, res)
            }) {
                outlines.push(o);
            }
        }
        if !needs_layer {
            let clip = self.state.clip.clone();
            for o in &outlines {
                self.fill_outline(o, &ts_paint, matrix, clip.as_ref(), paint.aa);
            }
            if hairline {
                stroke_hairline(
                    &mut self.pm,
                    path,
                    &ts_paint,
                    &stroke,
                    matrix,
                    clip.as_ref(),
                );
            }
            return;
        }
        // Draw into a layer, filter it, then composite through the clip.
        let mut lp = ts_paint.clone();
        lp.blend_mode = tiny_skia::BlendMode::SourceOver;
        let saved = std::mem::replace(&mut self.pm, Pixmap::new(w, h).unwrap());
        for o in &outlines {
            self.fill_outline(o, &lp, matrix, None, paint.aa);
        }
        if hairline {
            stroke_hairline(&mut self.pm, path, &lp, &stroke, matrix, None);
        }
        let layer = std::mem::replace(&mut self.pm, saved);
        let mut px = pixmap_pixels(&layer);
        if let Some(sigma) = paint.mask_blur {
            px = blur::mask_blur_layer(&px, w as i32, h as i32, sigma);
        }
        if let Some((sx, sy)) = paint.image_blur {
            px = blur::blur_premul(&px, w as i32, h as i32, 0, 0, w as i32, h as i32, sx, sy);
        }
        self.composite(&px, paint.blend, paint.color_filter.as_ref(), 1.0);
    }

    /// Fills one outline (local coordinates, mapped by `matrix`) with a tiny-skia paint:
    /// analytic coverage where Skia would use it, tiny-skia's rasteriser otherwise.
    fn fill_outline(
        &mut self,
        outline: &Path,
        ts_paint: &tiny_skia::Paint<'static>,
        matrix: Transform,
        clip: Option<&Mask>,
        aa: bool,
    ) {
        let device = match outline.clone().transform(matrix) {
            Some(d) => d,
            None => return,
        };
        if !aa {
            self.pm
                .fill_path(outline, ts_paint, FillRule::Winding, matrix, clip);
            return;
        }
        let analytic = raster::should_use_analytic(&device, is_axis_rect(&device));
        let (cw, ch) = (self.pm.width() as i32, self.pm.height() as i32);
        let b = device.bounds();
        let bx = (b.left().floor() as i32).max(0);
        let by = (b.top().floor() as i32).max(0);
        let bx2 = (b.right().ceil() as i32 + 1).min(cw);
        let by2 = (b.bottom().ceil() as i32 + 1).min(ch);
        if bx2 <= bx || by2 <= by {
            return;
        }
        let (bw, bh) = ((bx2 - bx) as u32, (by2 - by) as u32);
        let shift = Transform::from_translate(-bx as f32, -by as f32);
        let mut cov = if analytic {
            raster::coverage(&raster::flatten_analytic(&device, shift), bw, bh)
        } else {
            raster::supersampled(&raster::flatten(&device, shift), bw, bh)
        };
        if let Some(c) = clip {
            let cd = c.data();
            let data = cov.data_mut();
            for y in 0..bh as usize {
                for x in 0..bw as usize {
                    let m = cd[(by as usize + y) * cw as usize + bx as usize + x] as u32;
                    let v = &mut data[y * bw as usize + x];
                    *v = ((*v as u32 * m + 127) / 255) as u8;
                }
            }
        }
        let mut sub = Pixmap::new(bw, bh).unwrap();
        {
            let src = self.pm.data();
            let dst = sub.data_mut();
            for y in 0..bh as usize {
                let so = ((by as usize + y) * cw as usize + bx as usize) * 4;
                let d0 = y * bw as usize * 4;
                dst[d0..d0 + bw as usize * 4].copy_from_slice(&src[so..so + bw as usize * 4]);
            }
        }
        let mut p = ts_paint.clone();
        p.shader
            .transform(Transform::from_translate(-bx as f32, -by as f32).pre_concat(matrix));
        if let Some(r) = Rect::from_xywh(0.0, 0.0, bw as f32, bh as f32) {
            sub.fill_rect(r, &p, Transform::identity(), Some(&cov));
        }
        let src = sub.data();
        let dst = self.pm.data_mut();
        for y in 0..bh as usize {
            let d0 = ((by as usize + y) * cw as usize + bx as usize) * 4;
            let s0 = y * bw as usize * 4;
            dst[d0..d0 + bw as usize * 4].copy_from_slice(&src[s0..s0 + bw as usize * 4]);
        }
    }

    /// A path filled (or stroked) with an image shader.
    #[allow(clippy::too_many_arguments)]
    fn fill_with_image(
        &mut self,
        path: &Path,
        paint: &Paint,
        bitmap: &Bitmap,
        tile_x: Tile,
        tile_y: Tile,
        local: Transform,
        sampling: Sampling,
    ) {
        let matrix = self.state.matrix;
        let (w, h) = (self.pm.width() as i32, self.pm.height() as i32);
        let mut cover = Mask::new(w as u32, h as u32).unwrap();
        if paint.style != Style::Stroke {
            cover.fill_path(path, FillRule::Winding, paint.aa, matrix);
        }
        if paint.style != Style::Fill {
            // Stroke coverage through a white layer.
            let mut layer = Pixmap::new(w as u32, h as u32).unwrap();
            let p = tiny_skia::Paint {
                anti_alias: paint.aa,
                ..Default::default()
            };
            layer.stroke_path(path, &p, &self.ts_stroke(paint), matrix, None);
            for (m, q) in cover
                .data_mut()
                .iter_mut()
                .zip(layer.data().chunks_exact(4))
            {
                *m = (*m).max(q[3]);
            }
        }
        let Some(inv) = matrix.pre_concat(local).invert() else {
            return;
        };
        let src: Vec<[u8; 4]> = bitmap
            .px
            .iter()
            .map(|c| {
                let p = premul(*c);
                [p.r, p.g, p.b, p.a]
            })
            .collect();
        let alpha = paint.color.a as f32 / 255.0;
        let mut layer = vec![[0u8; 4]; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                let cov = cover.data()[i];
                if cov == 0 {
                    continue;
                }
                let mut pt = Point::from_xy(x as f32 + 0.5, y as f32 + 0.5);
                inv.map_points(std::slice::from_mut(&mut pt));
                let s = sample_tiled(
                    &src, bitmap.w, bitmap.h, pt.x, pt.y, sampling, tile_x, tile_y,
                );
                let k = alpha * cov as f32 / 255.0;
                layer[i] = [
                    unorm(s.r * k),
                    unorm(s.g * k),
                    unorm(s.b * k),
                    unorm(s.a * k),
                ];
            }
        }
        let mut layer_px = layer;
        if let Some(sigma) = paint.mask_blur {
            layer_px = blur::mask_blur_layer(&layer_px, w, h, sigma);
        }
        if let Some((sx, sy)) = paint.image_blur {
            layer_px = blur::blur_premul(&layer_px, w, h, 0, 0, w, h, sx, sy);
        }
        self.composite(&layer_px, paint.blend, paint.color_filter.as_ref(), 1.0);
    }

    fn ts_stroke(&self, paint: &Paint) -> Stroke {
        Stroke {
            width: paint.stroke_width,
            miter_limit: paint.miter,
            line_cap: match paint.cap {
                Cap::Butt => LineCap::Butt,
                Cap::Round => LineCap::Round,
                Cap::Square => LineCap::Square,
            },
            line_join: match paint.join {
                Join::Miter => LineJoin::Miter,
                Join::Round => LineJoin::Round,
                Join::Bevel => LineJoin::Bevel,
            },
            dash: paint
                .dash
                .as_ref()
                .and_then(|(i, phase)| StrokeDash::new(i.clone(), *phase)),
        }
    }

    fn ts_paint(
        &self,
        paint: &Paint,
        shader_matrix: Transform,
        _matrix: Transform,
    ) -> Option<tiny_skia::Paint<'static>> {
        let mut p = tiny_skia::Paint {
            anti_alias: paint.aa,
            blend_mode: paint.blend.to_skia(),
            ..Default::default()
        };
        // tiny-skia applies the paint's transform to the shader through `fill_path`'s
        // transform, so gradients are given in local coordinates.
        let _ = shader_matrix;
        match &paint.shader {
            None => p.shader = tiny_skia::Shader::SolidColor(paint.color.to_skia()),
            Some(shader) => {
                let mut s = make_shader(shader)?;
                if paint.color.a != 255 {
                    s.apply_opacity(paint.color.a as f32 / 255.0);
                }
                p.shader = s;
            }
        }
        Some(p)
    }

    /// Blends premultiplied pixels (canvas-sized) onto the canvas through the clip.
    pub fn composite(
        &mut self,
        src: &[[u8; 4]],
        mode: Blend,
        filter: Option<&ColorFilter>,
        alpha: f32,
    ) {
        let clip = self.state.clip.clone();
        let data = self.pm.data_mut();
        for (i, s) in src.iter().enumerate() {
            let cov = clip.as_ref().map_or(1.0, |m| m.data()[i] as f32 / 255.0);
            if cov == 0.0 {
                continue;
            }
            let mut sp = F4 {
                r: s[0] as f32 / 255.0,
                g: s[1] as f32 / 255.0,
                b: s[2] as f32 / 255.0,
                a: s[3] as f32 / 255.0,
            };
            if alpha != 1.0 {
                sp = F4 {
                    r: sp.r * alpha,
                    g: sp.g * alpha,
                    b: sp.b * alpha,
                    a: sp.a * alpha,
                };
            }
            if let Some(f) = filter {
                sp = f.apply(sp);
            }
            if sp.a == 0.0 && matches!(mode, Blend::SrcOver | Blend::Plus | Blend::Screen) {
                continue;
            }
            blend_into(&mut data[i * 4..i * 4 + 4], sp, mode, cov);
        }
    }

    /// `DrawText(text, x, y, font, paint)`: `y` is the baseline.
    pub fn draw_text(&mut self, s: &str, x: f32, y: f32, font: &super::text::Font, paint: &Paint) {
        if let Some(p) = font.path(s, x, y) {
            self.draw_path(&p, paint);
        }
    }

    // ---------- bitmaps ----------

    /// `DrawBitmap(bitmap, x, y, paint)`.
    pub fn draw_bitmap(&mut self, bitmap: &Bitmap, x: f32, y: f32, paint: &Paint) {
        let m = Transform::from_translate(x, y);
        self.draw_image_matrix(bitmap, m, Sampling::Nearest, paint);
    }

    /// `DrawBitmap(bitmap, srcRect, dstRect, paint)` / `DrawImage(image, src, dst,
    /// sampling, paint)`.
    pub fn draw_bitmap_rect(
        &mut self,
        bitmap: &Bitmap,
        src: Rect,
        dst: Rect,
        sampling: Sampling,
        paint: &Paint,
    ) {
        let sx = dst.width() / src.width();
        let sy = dst.height() / src.height();
        let m = Transform::from_row(
            sx,
            0.0,
            0.0,
            sy,
            dst.left() - src.left() * sx,
            dst.top() - src.top() * sy,
        );
        // Skia's fast constraint: the whole image is sampled (clamped at its edges) and
        // the destination rectangle limits what is drawn.
        self.draw_image_clipped(bitmap, m, sampling, paint, Some(dst));
    }

    /// `DrawImage(image, dst, sampling, paint)`.
    pub fn draw_bitmap_to(
        &mut self,
        bitmap: &Bitmap,
        dst: Rect,
        sampling: Sampling,
        paint: &Paint,
    ) {
        let src = rect(0.0, 0.0, bitmap.w as f32, bitmap.h as f32);
        self.draw_bitmap_rect(bitmap, src, dst, sampling, paint);
    }

    /// Draws `bitmap` mapped by `local` (then the canvas matrix).
    pub fn draw_image_matrix(
        &mut self,
        bitmap: &Bitmap,
        local: Transform,
        sampling: Sampling,
        paint: &Paint,
    ) {
        self.draw_image_clipped(bitmap, local, sampling, paint, None);
    }

    fn draw_image_clipped(
        &mut self,
        bitmap: &Bitmap,
        local: Transform,
        sampling: Sampling,
        paint: &Paint,
        dst_rect: Option<Rect>,
    ) {
        if bitmap.is_empty() {
            return;
        }
        let total = self.state.matrix.pre_concat(local);
        let Some(inv) = total.invert() else {
            return;
        };
        let (w, h) = (self.pm.width() as i32, self.pm.height() as i32);
        let src: Vec<[u8; 4]> = bitmap
            .px
            .iter()
            .map(|c| {
                let p = premul(*c);
                [p.r, p.g, p.b, p.a]
            })
            .collect();
        // Device bounds of the image (or the destination rectangle).
        let bounds = {
            let r = dst_rect.unwrap_or_else(|| rect(0.0, 0.0, bitmap.w as f32, bitmap.h as f32));
            if dst_rect.is_some() {
                // dst_rect is in canvas-local space.
                map_rect(self.state.matrix, r)
            } else {
                map_rect(total, r)
            }
        };
        // Coverage of the image's area (anti-aliasing off: pixel centres inside).
        let area = if let Some(r) = dst_rect {
            let mut pb = PathBuilder::new();
            pb.push_rect(r);
            pb.finish().map(|p| (p, self.state.matrix))
        } else {
            let mut pb = PathBuilder::new();
            pb.push_rect(rect(0.0, 0.0, bitmap.w as f32, bitmap.h as f32));
            pb.finish().map(|p| (p, total))
        };
        let mut cover = Mask::new(w as u32, h as u32).unwrap();
        if let Some((p, m)) = area {
            cover.fill_path(&p, FillRule::Winding, paint.aa, m);
        }
        let x0 = (bounds.left().floor() as i32).max(0);
        let y0 = (bounds.top().floor() as i32).max(0);
        let x1 = (bounds.right().ceil() as i32).min(w);
        let y1 = (bounds.bottom().ceil() as i32).min(h);
        let mut layer = vec![[0u8; 4]; (w * h) as usize];
        let mut layer_f: Vec<F4> = Vec::new();
        let filtered = paint.color_filter.is_some() || paint.color.a != 255;
        if filtered {
            layer_f = vec![F4::default(); (w * h) as usize];
        }
        let alpha = paint.color.a as f32 / 255.0;
        let mut any = false;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y * w + x) as usize;
                let cov = cover.data()[i];
                if cov == 0 {
                    continue;
                }
                let mut pt = Point::from_xy(x as f32 + 0.5, y as f32 + 0.5);
                inv.map_points(std::slice::from_mut(&mut pt));
                let mut s = sample(&src, bitmap.w, bitmap.h, pt.x, pt.y, sampling, Tile::Clamp);
                if s.a == 0.0 && s.r == 0.0 && s.g == 0.0 && s.b == 0.0 && !filtered {
                    continue;
                }
                if alpha != 1.0 {
                    s = F4 {
                        r: s.r * alpha,
                        g: s.g * alpha,
                        b: s.b * alpha,
                        a: s.a * alpha,
                    };
                }
                if let Some(f) = &paint.color_filter {
                    s = f.apply(s);
                }
                if cov != 255 {
                    let c = cov as f32 / 255.0;
                    s = F4 {
                        r: s.r * c,
                        g: s.g * c,
                        b: s.b * c,
                        a: s.a * c,
                    };
                }
                any = true;
                if filtered {
                    layer_f[i] = s;
                } else {
                    layer[i] = [unorm(s.r), unorm(s.g), unorm(s.b), unorm(s.a)];
                }
            }
        }
        if !any {
            return;
        }
        if let Some((sx, sy)) = paint.image_blur {
            if filtered {
                for (d, s) in layer.iter_mut().zip(&layer_f) {
                    *d = [unorm(s.r), unorm(s.g), unorm(s.b), unorm(s.a)];
                }
            }
            let blurred = blur::blur_premul(&layer, w, h, 0, 0, w, h, sx, sy);
            self.composite(&blurred, paint.blend, None, 1.0);
            return;
        }
        let clip = self.state.clip.clone();
        let data = self.pm.data_mut();
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y * w + x) as usize;
                if cover.data()[i] == 0 {
                    continue;
                }
                let ccov = clip.as_ref().map_or(1.0, |m| m.data()[i] as f32 / 255.0);
                if ccov == 0.0 {
                    continue;
                }
                let s = if filtered {
                    layer_f[i]
                } else {
                    let p = layer[i];
                    if p == [0, 0, 0, 0] && paint.blend == Blend::SrcOver {
                        continue;
                    }
                    F4 {
                        r: p[0] as f32 / 255.0,
                        g: p[1] as f32 / 255.0,
                        b: p[2] as f32 / 255.0,
                        a: p[3] as f32 / 255.0,
                    }
                };
                blend_into(&mut data[i * 4..i * 4 + 4], s, paint.blend, ccov);
            }
        }
    }
}

/// Blends a premultiplied float source into one stored pixel with clip coverage.
#[inline]
pub fn blend_into(dst: &mut [u8], s: F4, mode: Blend, cov: f32) {
    let d = F4 {
        r: dst[0] as f32 / 255.0,
        g: dst[1] as f32 / 255.0,
        b: dst[2] as f32 / 255.0,
        a: dst[3] as f32 / 255.0,
    };
    let mut o = mode.apply(s, d);
    if cov < 1.0 {
        o = F4 {
            r: d.r + (o.r - d.r) * cov,
            g: d.g + (o.g - d.g) * cov,
            b: d.b + (o.b - d.b) * cov,
            a: d.a + (o.a - d.a) * cov,
        };
    }
    let a = unorm(o.a);
    dst[0] = unorm(o.r).min(a);
    dst[1] = unorm(o.g).min(a);
    dst[2] = unorm(o.b).min(a);
    dst[3] = a;
}

/// The two points of a path that is one open line segment.
fn single_segment(path: &Path) -> Option<(Point, Point)> {
    let pts = path.points();
    let verbs = path.verbs();
    if pts.len() == 2 && verbs.len() == 2 {
        Some((pts[0], pts[1]))
    } else {
        None
    }
}

/// `SkStroke` of one line segment: the offset sides and the caps (round caps as two
/// quarter conics, as `RoundCapper` makes them).
pub fn line_outline(a: Point, b: Point, width: f32, cap: Cap) -> Option<Path> {
    let r = width / 2.0;
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt();
    if len <= f32::EPSILON {
        return match cap {
            Cap::Round => oval_path(a.x - r, a.y - r, a.x + r, a.y + r),
            Cap::Square => Some(PathBuilder::from_rect(rect(
                a.x - r,
                a.y - r,
                a.x + r,
                a.y + r,
            ))),
            Cap::Butt => None,
        };
    }
    let (ux, uy) = (dx / len, dy / len);
    let (nx, ny) = (-uy * r, ux * r);
    let (px, py) = (ux * r, uy * r);
    let w = std::f32::consts::FRAC_1_SQRT_2;
    let mut pb = PathBuilder::new();
    pb.move_to(a.x + nx, a.y + ny);
    pb.line_to(b.x + nx, b.y + ny);
    match cap {
        Cap::Butt => pb.line_to(b.x - nx, b.y - ny),
        Cap::Square => {
            pb.line_to(b.x + nx + px, b.y + ny + py);
            pb.line_to(b.x - nx + px, b.y - ny + py);
            pb.line_to(b.x - nx, b.y - ny);
        }
        Cap::Round => {
            conic_to(
                &mut pb,
                (b.x + nx, b.y + ny),
                (b.x + nx + px, b.y + ny + py),
                (b.x + px, b.y + py),
                w,
            );
            conic_to(
                &mut pb,
                (b.x + px, b.y + py),
                (b.x - nx + px, b.y - ny + py),
                (b.x - nx, b.y - ny),
                w,
            );
        }
    }
    pb.line_to(a.x - nx, a.y - ny);
    match cap {
        Cap::Butt => {}
        Cap::Square => {
            pb.line_to(a.x - nx - px, a.y - ny - py);
            pb.line_to(a.x + nx - px, a.y + ny - py);
        }
        Cap::Round => {
            conic_to(
                &mut pb,
                (a.x - nx, a.y - ny),
                (a.x - nx - px, a.y - ny - py),
                (a.x - px, a.y - py),
                w,
            );
            conic_to(
                &mut pb,
                (a.x - px, a.y - py),
                (a.x + nx - px, a.y + ny - py),
                (a.x + nx, a.y + ny),
                w,
            );
        }
    }
    pb.close();
    pb.finish()
}

/// The length of a path's outline (curves flattened).
fn path_length(path: &Path) -> f32 {
    let lines = raster::flatten(path, Transform::identity());
    lines
        .iter()
        .map(|(a, b)| ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt())
        .sum()
}

/// The scale a matrix applies (the stroker's resolution scale).
/// Draws a hairline the way Skia does near the top and left edges. tiny-skia's
/// anti-aliased hairline blitters clamp rows and columns above or left of the pixmap to
/// zero (Skia keeps them negative and clips them away), which paints stray coverage on the
/// first row or column; so a hairline reaching there is drawn on a copy of that corner of
/// the canvas padded by a few pixels, and the copy is put back.
fn stroke_hairline(
    pm: &mut Pixmap,
    path: &Path,
    paint: &tiny_skia::Paint,
    stroke: &tiny_skia::Stroke,
    m: Transform,
    clip: Option<&Mask>,
) {
    const PAD: i32 = 3;
    let Some(dev) = path.clone().transform(m) else {
        return;
    };
    let b = dev.bounds();
    if b.left() >= 2.0 && b.top() >= 2.0 {
        pm.stroke_path(path, paint, stroke, m, clip);
        return;
    }
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let rw = (b.right().ceil() as i32 + PAD).clamp(0, w);
    let rh = (b.bottom().ceil() as i32 + PAD).clamp(0, h);
    if rw <= 0 || rh <= 0 {
        return;
    }
    let (tw, th) = (rw + PAD, rh + PAD);
    let Some(mut tmp) = Pixmap::new(tw as u32, th as u32) else {
        return;
    };
    let mut tmp_mask = clip.and_then(|_| Mask::new(tw as u32, th as u32));
    {
        let src = pm.data();
        let dst = tmp.data_mut();
        for y in 0..rh {
            let s0 = ((y * w) * 4) as usize;
            let d0 = (((y + PAD) * tw + PAD) * 4) as usize;
            dst[d0..d0 + (rw * 4) as usize].copy_from_slice(&src[s0..s0 + (rw * 4) as usize]);
        }
        if let (Some(c), Some(tm)) = (clip, tmp_mask.as_mut()) {
            let (cs, ds) = (c.data(), tm.data_mut());
            for y in 0..rh {
                let s0 = (y * w) as usize;
                let d0 = ((y + PAD) * tw + PAD) as usize;
                ds[d0..d0 + rw as usize].copy_from_slice(&cs[s0..s0 + rw as usize]);
            }
        }
    }
    let shifted = m.post_translate(PAD as f32, PAD as f32);
    tmp.stroke_path(path, paint, stroke, shifted, tmp_mask.as_ref());
    let src = tmp.data();
    let dst = pm.data_mut();
    for y in 0..rh {
        let d0 = ((y * w) * 4) as usize;
        let s0 = (((y + PAD) * tw + PAD) * 4) as usize;
        dst[d0..d0 + (rw * 4) as usize].copy_from_slice(&src[s0..s0 + (rw * 4) as usize]);
    }
}

/// `SkDrawTreatAAStrokeAsHairline`: both mapped width vectors at most one pixel long
/// (Skia's fast length: the longer side plus half the shorter).
fn thin_stroke(width: f32, m: Transform) -> bool {
    let fast_len = |x: f32, y: f32| {
        let (mut a, mut b) = (x.abs(), y.abs());
        if a < b {
            std::mem::swap(&mut a, &mut b);
        }
        a + b * 0.5
    };
    let l0 = fast_len(m.sx * width, m.ky * width);
    let l1 = fast_len(m.kx * width, m.sy * width);
    l0 <= 1.0 && l1 <= 1.0
}

fn matrix_scale(m: Transform) -> f32 {
    let sx = (m.sx * m.sx + m.ky * m.ky).sqrt();
    let sy = (m.kx * m.kx + m.sy * m.sy).sqrt();
    sx.max(sy).max(1e-6)
}

/// `SkPath::isRect` for the simple case: one closed axis-aligned quadrilateral.
fn is_axis_rect(path: &Path) -> bool {
    let pts = path.points();
    if pts.len() != 4 && pts.len() != 5 {
        return false;
    }
    let b = path.bounds();
    pts.iter()
        .all(|p| (p.x == b.left() || p.x == b.right()) && (p.y == b.top() || p.y == b.bottom()))
}

fn pixmap_pixels(pm: &Pixmap) -> Vec<[u8; 4]> {
    pm.data()
        .chunks_exact(4)
        .map(|q| [q[0], q[1], q[2], q[3]])
        .collect()
}

fn map_rect(m: Transform, r: Rect) -> Rect {
    let mut pts = [
        Point::from_xy(r.left(), r.top()),
        Point::from_xy(r.right(), r.top()),
        Point::from_xy(r.right(), r.bottom()),
        Point::from_xy(r.left(), r.bottom()),
    ];
    m.map_points(&mut pts);
    let l = pts.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let t = pts.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let rr = pts.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
    let b = pts.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
    rect(l, t, rr, b)
}

/// `SKMatrix.CreateRotationDegrees`.
pub fn rotation(deg: f32) -> Transform {
    let rad = deg.to_radians();
    let (s, c) = sin_cos_snap(rad);
    Transform::from_row(c, s, -s, c, 0.0, 0.0)
}

/// Skia's `SkScalarSinSnapToZero` / `CosSnapToZero`.
fn sin_cos_snap(rad: f32) -> (f32, f32) {
    let s = rad.sin();
    let c = rad.cos();
    let snap = |v: f32| {
        if v.abs() <= 1.0 / 4096.0 { 0.0 } else { v }
    };
    (snap(s), snap(c))
}

/// `AddRoundRect` / `DrawRoundRect`: straight edges and quarter conics (weight
/// sqrt(2)/2), as `SkPath::addRRect` builds them.
pub fn round_rect_path(r: Rect, rx: f32, ry: f32) -> Option<Path> {
    // SkRRect::setRectXY: radii too large for the rect shrink together.
    let (mut rx, mut ry) = (rx.max(0.0), ry.max(0.0));
    if rx <= 0.0 || ry <= 0.0 {
        return Some(PathBuilder::from_rect(r));
    }
    if r.width() < rx + rx || r.height() < ry + ry {
        let scale = (r.width() / (rx + rx)).min(r.height() / (ry + ry));
        rx *= scale;
        ry *= scale;
    }
    let w = std::f32::consts::FRAC_1_SQRT_2;
    let (l, t, rr, b) = (r.left(), r.top(), r.right(), r.bottom());
    let mut pb = PathBuilder::new();
    pb.move_to(l + rx, t);
    pb.line_to(rr - rx, t);
    conic_to(&mut pb, (rr - rx, t), (rr, t), (rr, t + ry), w);
    pb.line_to(rr, b - ry);
    conic_to(&mut pb, (rr, b - ry), (rr, b), (rr - rx, b), w);
    pb.line_to(l + rx, b);
    conic_to(&mut pb, (l + rx, b), (l, b), (l, b - ry), w);
    pb.line_to(l, t + ry);
    conic_to(&mut pb, (l, t + ry), (l, t), (l + rx, t), w);
    pb.close();
    pb.finish()
}

/// `SKPath.AddOval` (clockwise from the right-hand point) in quarter conics, which the
/// edge builder turns into quads the way Skia does (so small circles are as round, and as
/// slightly bulging, as upstream's).
pub fn oval_path(l: f32, t: f32, r: f32, b: f32) -> Option<Path> {
    if !(r > l && b > t) {
        return None;
    }
    let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
    let w = std::f32::consts::FRAC_1_SQRT_2;
    let mut pb = PathBuilder::new();
    pb.move_to(r, cy);
    conic_to(&mut pb, (r, cy), (r, b), (cx, b), w);
    conic_to(&mut pb, (cx, b), (l, b), (l, cy), w);
    conic_to(&mut pb, (l, cy), (l, t), (cx, t), w);
    conic_to(&mut pb, (cx, t), (r, t), (r, cy), w);
    pb.close();
    pb.finish()
}

/// A conic from `p0` (the current point) through control `p1` to `p2`, as the quads
/// `SkAutoConicToQuads` makes of it (tolerance 0.25).
pub fn conic_to(pb: &mut PathBuilder, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), w: f32) {
    let a = w - 1.0;
    let k = a / (4.0 * (2.0 + a));
    let x = k * (p0.0 - 2.0 * p1.0 + p2.0);
    let y = k * (p0.1 - 2.0 * p1.1 + p2.1);
    let mut error = (x * x + y * y).sqrt();
    let mut pow2 = 0;
    while pow2 < 5 {
        if error <= 0.25 {
            break;
        }
        error *= 0.25;
        pow2 += 1;
    }
    fn chop(
        p0: (f32, f32),
        p1: (f32, f32),
        p2: (f32, f32),
        w: f32,
        level: i32,
        out: &mut Vec<((f32, f32), (f32, f32))>,
    ) {
        if level == 0 {
            out.push((p1, p2));
            return;
        }
        let scale = 1.0 / (1.0 + w);
        let nw = (0.5 + w * 0.5).sqrt();
        let m = (
            (p0.0 + 2.0 * w * p1.0 + p2.0) * scale * 0.5,
            (p0.1 + 2.0 * w * p1.1 + p2.1) * scale * 0.5,
        );
        let a1 = ((p0.0 + w * p1.0) * scale, (p0.1 + w * p1.1) * scale);
        let b1 = ((w * p1.0 + p2.0) * scale, (w * p1.1 + p2.1) * scale);
        chop(p0, a1, m, nw, level - 1, out);
        chop(m, b1, p2, nw, level - 1, out);
    }
    let mut quads = Vec::new();
    chop(p0, p1, p2, w, pow2, &mut quads);
    for (c, e) in quads {
        pb.quad_to(c.0, c.1, e.0, e.1);
    }
}

/// `SKPath.AddArc(oval, startAngle, sweepAngle)`: a new contour along the ellipse, in
/// conic segments of at most 90 degrees.
pub fn add_arc(pb: &mut PathBuilder, oval: (f32, f32, f32, f32), start: f32, sweep: f32) {
    let (l, t, r, b) = oval;
    if r <= l || b <= t || sweep == 0.0 {
        return;
    }
    let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
    let (rx, ry) = ((r - l) / 2.0, (b - t) / 2.0);
    let sweep = sweep.clamp(-360.0, 360.0);
    let n = (sweep.abs() / 90.0).ceil().max(1.0) as i32;
    let step = sweep / n as f32;
    let pt = |deg: f32| {
        let a = deg.to_radians();
        (cx + rx * a.cos(), cy + ry * a.sin())
    };
    let mut cur = pt(start);
    pb.move_to(cur.0, cur.1);
    let mut a = start;
    for _ in 0..n {
        let a1 = a + step;
        let mid = ((a + a1) / 2.0).to_radians();
        let half = (step / 2.0).to_radians();
        let k = 1.0 / half.cos();
        let c = (cx + rx * mid.cos() * k, cy + ry * mid.sin() * k);
        let e = pt(a1);
        conic_to(pb, cur, c, e, half.cos());
        cur = e;
        a = a1;
    }
}

/// `SKBitmap.Resize(info, sampling)` (`SkPixmap::scalePixels`): the image scaled onto the
/// new size with a clamped image shader and `Src` blending.
pub fn scale_pixels(bitmap: &Bitmap, w: i32, h: i32, sampling: Sampling) -> Bitmap {
    if w == bitmap.w && h == bitmap.h {
        return bitmap.clone();
    }
    let src: Vec<[u8; 4]> = bitmap
        .px
        .iter()
        .map(|c| {
            let p = premul(*c);
            [p.r, p.g, p.b, p.a]
        })
        .collect();
    let sx = bitmap.w as f32 / w as f32;
    let sy = bitmap.h as f32 / h as f32;
    let mut px = Vec::with_capacity((w.max(0) * h.max(0)) as usize);
    for y in 0..h {
        for x in 0..w {
            let u = (x as f32 + 0.5) * sx;
            let v = (y as f32 + 0.5) * sy;
            let s = sample(&src, bitmap.w, bitmap.h, u, v, sampling, Tile::Clamp);
            px.push(unpremul(C::new(
                unorm(s.r),
                unorm(s.g),
                unorm(s.b),
                unorm(s.a),
            )));
        }
    }
    Bitmap { w, h, px }
}

fn stops(colors: &[C], pos: &Option<Vec<f32>>) -> Vec<GradientStop> {
    let n = colors.len();
    colors
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let p = match pos {
                Some(p) => p.get(i).copied().unwrap_or(1.0),
                None => {
                    if n <= 1 {
                        0.0
                    } else {
                        i as f32 / (n - 1) as f32
                    }
                }
            };
            GradientStop::new(p, c.to_skia())
        })
        .collect()
}

fn spread(t: Tile) -> SpreadMode {
    match t {
        Tile::Clamp | Tile::Decal => SpreadMode::Pad,
        Tile::Repeat => SpreadMode::Repeat,
        Tile::Mirror => SpreadMode::Reflect,
    }
}

fn make_shader(shader: &Shader) -> Option<tiny_skia::Shader<'static>> {
    match shader {
        Shader::Linear {
            p0,
            p1,
            colors,
            pos,
            tile,
        } => {
            if (p0.0 - p1.0).abs() < 1e-6 && (p0.1 - p1.1).abs() < 1e-6 {
                // Degenerate: Skia draws the last colour (clamp) or nothing.
                return Some(tiny_skia::Shader::SolidColor(
                    colors.last().copied().unwrap_or(C::TRANSPARENT).to_skia(),
                ));
            }
            tiny_skia::LinearGradient::new(
                Point::from_xy(p0.0, p0.1),
                Point::from_xy(p1.0, p1.1),
                stops(colors, pos),
                spread(*tile),
                Transform::identity(),
            )
        }
        Shader::Radial {
            center,
            radius,
            colors,
            pos,
            tile,
        } => {
            if *radius <= 0.0 {
                return Some(tiny_skia::Shader::SolidColor(
                    colors.last().copied().unwrap_or(C::TRANSPARENT).to_skia(),
                ));
            }
            tiny_skia::RadialGradient::new(
                Point::from_xy(center.0, center.1),
                0.0,
                Point::from_xy(center.0, center.1),
                *radius,
                stops(colors, pos),
                spread(*tile),
                Transform::identity(),
            )
        }
        Shader::Sweep {
            center,
            colors,
            pos,
        } => tiny_skia::SweepGradient::new(
            Point::from_xy(center.0, center.1),
            0.0,
            360.0,
            stops(colors, pos),
            SpreadMode::Pad,
            Transform::identity(),
        ),
        Shader::Image { .. } => None,
    }
}

// ---------- sampling ----------

fn cubic_weights(t: f32, b: f32, c: f32) -> [f32; 4] {
    // Mitchell-Netravali for the four taps at distances 1+t, t, 1-t, 2-t.
    let k = |x: f32| -> f32 {
        let x = x.abs();
        if x < 1.0 {
            ((12.0 - 9.0 * b - 6.0 * c) * x * x * x
                + (-18.0 + 12.0 * b + 6.0 * c) * x * x
                + (6.0 - 2.0 * b))
                / 6.0
        } else if x < 2.0 {
            ((-b - 6.0 * c) * x * x * x
                + (6.0 * b + 30.0 * c) * x * x
                + (-12.0 * b - 48.0 * c) * x
                + (8.0 * b + 24.0 * c))
                / 6.0
        } else {
            0.0
        }
    };
    [k(1.0 + t), k(t), k(1.0 - t), k(2.0 - t)]
}

fn tile_coord(v: i32, n: i32, tile: Tile) -> Option<i32> {
    match tile {
        Tile::Clamp => Some(v.clamp(0, n - 1)),
        Tile::Repeat => Some(v.rem_euclid(n)),
        Tile::Mirror => {
            let period = 2 * n;
            let m = v.rem_euclid(period);
            Some(if m < n { m } else { period - 1 - m })
        }
        Tile::Decal => {
            if v < 0 || v >= n {
                None
            } else {
                Some(v)
            }
        }
    }
}

/// Samples premultiplied pixels at `(u, v)` (pixel-centre convention), clamping texel
/// reads to the edge for filtering and treating points outside as transparent when
/// `outside` is `Decal`.
pub fn sample(
    src: &[[u8; 4]],
    w: i32,
    h: i32,
    u: f32,
    v: f32,
    sampling: Sampling,
    outside: Tile,
) -> F4 {
    sample_tiled(src, w, h, u, v, sampling, outside, outside)
}

#[allow(clippy::too_many_arguments)]
pub fn sample_tiled(
    src: &[[u8; 4]],
    w: i32,
    h: i32,
    u: f32,
    v: f32,
    sampling: Sampling,
    tile_x: Tile,
    tile_y: Tile,
) -> F4 {
    let texel = |x: i32, y: i32| -> F4 {
        let (Some(x), Some(y)) = (tile_coord(x, w, tile_x), tile_coord(y, h, tile_y)) else {
            return F4::default();
        };
        let p = src[(y * w + x) as usize];
        F4 {
            r: p[0] as f32 / 255.0,
            g: p[1] as f32 / 255.0,
            b: p[2] as f32 / 255.0,
            a: p[3] as f32 / 255.0,
        }
    };
    match sampling {
        Sampling::Nearest => {
            // SkBitmapProcState's nearest mapper biases by one 16.16 step so that exact
            // integer sample points (a scaled image's pixel centres) round down.
            let x = (u - 1.0 / 65536.0).floor() as i32;
            let y = (v - 1.0 / 65536.0).floor() as i32;
            texel(x, y)
        }
        Sampling::Linear => {
            let sx = u - 0.5;
            let sy = v - 0.5;
            let x0 = sx.floor();
            let y0 = sy.floor();
            let fx = sx - x0;
            let fy = sy - y0;
            let (x0, y0) = (x0 as i32, y0 as i32);
            let c00 = texel(x0, y0);
            let c10 = texel(x0 + 1, y0);
            let c01 = texel(x0, y0 + 1);
            let c11 = texel(x0 + 1, y0 + 1);
            let l = |a: f32, b: f32, c: f32, d: f32| {
                (a * (1.0 - fx) + b * fx) * (1.0 - fy) + (c * (1.0 - fx) + d * fx) * fy
            };
            F4 {
                r: l(c00.r, c10.r, c01.r, c11.r),
                g: l(c00.g, c10.g, c01.g, c11.g),
                b: l(c00.b, c10.b, c01.b, c11.b),
                a: l(c00.a, c10.a, c01.a, c11.a),
            }
        }
        Sampling::Cubic(b, c) => {
            let sx = u - 0.5;
            let sy = v - 0.5;
            let x0 = sx.floor();
            let y0 = sy.floor();
            let wx = cubic_weights(sx - x0, b, c);
            let wy = cubic_weights(sy - y0, b, c);
            let (x0, y0) = (x0 as i32, y0 as i32);
            let mut acc = F4::default();
            for (j, wyj) in wy.iter().enumerate() {
                for (i, wxi) in wx.iter().enumerate() {
                    let t = texel(x0 - 1 + i as i32, y0 - 1 + j as i32);
                    let k = wxi * wyj;
                    acc.r += t.r * k;
                    acc.g += t.g * k;
                    acc.b += t.b * k;
                    acc.a += t.a * k;
                }
            }
            let a = acc.a.clamp(0.0, 1.0);
            F4 {
                r: acc.r.clamp(0.0, a),
                g: acc.g.clamp(0.0, a),
                b: acc.b.clamp(0.0, a),
                a,
            }
        }
    }
}

/// A straight-colour helper over [`sample`] for effects that resample whole bitmaps.
pub fn sample_bitmap(bitmap: &Bitmap, u: f32, v: f32, sampling: Sampling, tile: Tile) -> C {
    let src: Vec<[u8; 4]> = bitmap
        .px
        .iter()
        .map(|c| {
            let p = premul(*c);
            [p.r, p.g, p.b, p.a]
        })
        .collect();
    let s = sample(&src, bitmap.w, bitmap.h, u, v, sampling, tile);
    unpremul(C::new(unorm(s.r), unorm(s.g), unorm(s.b), unorm(s.a)))
}

/// `SKBitmap.Resize(info, sampling)` / scaling a bitmap onto a new canvas of `w` x `h`.
pub fn resize(bitmap: &Bitmap, w: i32, h: i32, sampling: Sampling) -> Bitmap {
    let mut canvas = Canvas::new(w, h);
    canvas.draw_bitmap_to(
        bitmap,
        rect(0.0, 0.0, w as f32, h as f32),
        sampling,
        &Paint::default(),
    );
    canvas.into_bitmap()
}
