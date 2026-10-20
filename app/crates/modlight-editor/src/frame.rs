//! The preview frame of `EditorView.axaml` and what `GetSnapshot` renders from it: the
//! background (gradient, colour, image or wallpaper), the margin with the output ratio,
//! the drop shadow, the rounded corners, the padding in the image's top-left colour,
//! smart padding (the image's own border cropped away), then the image, the spotlight
//! layer and the annotations, all clipped to the viewport.
//!
//! The background is only part of the picture while the Background panel is open
//! (`AreBackgroundEffectsActive`), as upstream does.

use std::sync::Arc;

use image::{Rgba, RgbaImage};
use tiny_skia::{
    FillRule, FilterQuality, GradientStop, LinearGradient, Mask, Paint, PathBuilder, Pixmap,
    PixmapPaint, Point as SkPoint, Rect as SkRect, SpreadMode, Transform,
};

use crate::color::Color;
use crate::geom::Rect;
use crate::model::{Annotation, AnnotationId};
use crate::render;

/// `CanvasBackgroundMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BackgroundMode {
    Gradient,
    Color,
    #[default]
    Transparent,
    Image,
    Wallpaper,
}

impl BackgroundMode {
    pub fn name(self) -> &'static str {
        match self {
            BackgroundMode::Gradient => "Gradient",
            BackgroundMode::Color => "Color",
            BackgroundMode::Transparent => "Transparent",
            BackgroundMode::Image => "Image",
            BackgroundMode::Wallpaper => "Wallpaper",
        }
    }

    pub fn parse(text: &str) -> BackgroundMode {
        match text.to_ascii_lowercase().as_str() {
            "gradient" => BackgroundMode::Gradient,
            "color" => BackgroundMode::Color,
            "image" => BackgroundMode::Image,
            "wallpaper" => BackgroundMode::Wallpaper,
            _ => BackgroundMode::Transparent,
        }
    }
}

/// `DesktopWallpaperLayout` (how an image background is stretched).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ImageLayout {
    #[default]
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
    Span,
}

/// The brush behind the frame.
#[derive(Clone, Debug, Default)]
pub enum BackgroundBrush {
    #[default]
    Transparent,
    Solid(Color),
    Gradient(Color, Color),
    Image(Arc<RgbaImage>, ImageLayout),
}

/// The background settings of the view model, as far as drawing needs them.
#[derive(Clone, Debug, Default)]
pub struct BackgroundState {
    /// `AreBackgroundEffectsActive`.
    pub active: bool,
    pub margin: f64,
    pub padding: f64,
    pub smart_padding: bool,
    pub rounded_corner: f64,
    pub shadow_radius: f64,
    /// `TargetOutputAspectRatio`.
    pub output_ratio: Option<f64>,
    pub brush: BackgroundBrush,
    /// `Options.ShadowColor`, which the canvas shadow is tinted with.
    pub shadow_color: Color,
    /// The smart padding crop (left, top, right, bottom), detected from the image.
    pub smart_insets: (u32, u32, u32, u32),
}

impl BackgroundState {
    pub fn has_detected_smart_padding(&self) -> bool {
        let (l, t, r, b) = self.smart_insets;
        l > 0 || t > 0 || r > 0 || b > 0
    }

    /// `IsSmartPaddingActive`.
    pub fn smart_padding_active(&self) -> bool {
        self.smart_padding && self.active && self.has_detected_smart_padding()
    }
}

/// `EnsureSmartPaddingCache`: the border of the image's top-left colour (tolerance 30 on
/// every channel), as insets.
pub fn detect_smart_insets(image: &RgbaImage) -> (u32, u32, u32, u32) {
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return (0, 0, 0, 0);
    }
    let target = image.get_pixel(0, 0).0;
    let differs = |p: &Rgba<u8>| (0..4).any(|i| (p.0[i] as i32 - target[i] as i32).abs() > 30);
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (w, h, 0u32, 0u32);
    for y in 0..h {
        let mut row_has_content = false;
        for x in 0..w {
            if differs(image.get_pixel(x, y)) {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
                row_has_content = true;
                break;
            }
        }
        if row_has_content {
            for x in (0..w).rev() {
                if differs(image.get_pixel(x, y)) {
                    max_x = max_x.max(x);
                    break;
                }
            }
        }
    }
    if min_x > max_x || min_y > max_y {
        return (0, 0, 0, 0);
    }
    (
        min_x,
        min_y,
        w.saturating_sub(max_x + 1),
        h.saturating_sub(max_y + 1),
    )
}

/// Where everything sits in the frame, in output pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameLayout {
    pub width: f32,
    pub height: f32,
    /// `CanvasPadding` (margin, widened to the output ratio).
    pub canvas_padding: (f32, f32, f32, f32),
    /// `SmartPaddingThickness`.
    pub padding: f32,
    /// `SmartPaddingViewportWidth/Height`.
    pub viewport: (f32, f32),
    /// `SmartPaddingOffsetX/Y`: where the image sits inside the viewport.
    pub image_offset: (f32, f32),
    pub corner_radius: f32,
}

impl FrameLayout {
    /// The rectangle of the bordered card (inside the margin).
    pub fn card(&self) -> Rect {
        let (l, t, r, b) = self.canvas_padding;
        Rect::new(l, t, self.width - r, self.height - b)
    }

    /// The viewport rectangle in frame coordinates.
    pub fn viewport_rect(&self) -> Rect {
        let card = self.card();
        Rect::from_xywh(
            card.left + self.padding,
            card.top + self.padding,
            self.viewport.0,
            self.viewport.1,
        )
    }

    /// The transform from image pixels to frame pixels.
    pub fn image_transform(&self) -> Transform {
        let v = self.viewport_rect();
        Transform::from_translate(v.left + self.image_offset.0, v.top + self.image_offset.1)
    }
}

/// `CalculateOutputPadding`.
fn output_padding(base: f64, viewport: (f64, f64), ratio: Option<f64>) -> (f64, f64, f64, f64) {
    let (w, h) = viewport;
    let Some(target) = ratio.filter(|_| w > 0.0 && h > 0.0) else {
        return (base, base, base, base);
    };
    let total_w = w + base * 2.0;
    let total_h = h + base * 2.0;
    let current = total_w / total_h;
    let mut extra_x = 0.0;
    let mut extra_y = 0.0;
    const EPSILON: f64 = 0.0001;
    if current > target + EPSILON {
        extra_y = ((total_w / target) - total_h).max(0.0) / 2.0;
    } else if current + EPSILON < target {
        extra_x = ((total_h * target) - total_w).max(0.0) / 2.0;
    }
    (
        base + extra_x,
        base + extra_y,
        base + extra_x,
        base + extra_y,
    )
}

/// The frame's layout for an image of `image_size`.
pub fn layout(state: &BackgroundState, image_size: (f32, f32)) -> FrameLayout {
    let (mut vw, mut vh) = image_size;
    let mut offset = (0.0, 0.0);
    if state.smart_padding_active() {
        let (l, t, r, b) = state.smart_insets;
        vw = (vw - (l + r) as f32).max(0.0);
        vh = (vh - (t + b) as f32).max(0.0);
        offset = (-(l as f32), -(t as f32));
    }
    if !state.active {
        return FrameLayout {
            width: vw,
            height: vh,
            canvas_padding: (0.0, 0.0, 0.0, 0.0),
            padding: 0.0,
            viewport: (vw, vh),
            image_offset: offset,
            corner_radius: 0.0,
        };
    }
    let cp = output_padding(state.margin, (vw as f64, vh as f64), state.output_ratio);
    let padding = state.padding as f32;
    let width = vw + padding * 2.0 + (cp.0 + cp.2) as f32;
    let height = vh + padding * 2.0 + (cp.1 + cp.3) as f32;
    FrameLayout {
        width,
        height,
        canvas_padding: (cp.0 as f32, cp.1 as f32, cp.2 as f32, cp.3 as f32),
        padding,
        viewport: (vw, vh),
        image_offset: offset,
        corner_radius: state.rounded_corner.max(0.0) as f32,
    }
}

/// `GetSnapshot`'s pixel size (`Math.Round(..., AwayFromZero)`).
pub fn snapshot_size(layout: &FrameLayout) -> (u32, u32) {
    (
        (layout.width.round() as i64).max(1) as u32,
        (layout.height.round() as i64).max(1) as u32,
    )
}

fn rect_mask(
    width: u32,
    height: u32,
    path: &tiny_skia::Path,
    transform: Transform,
) -> Option<Mask> {
    let mut mask = Mask::new(width, height)?;
    mask.fill_path(path, FillRule::Winding, true, transform);
    Some(mask)
}

/// Fills `rect` with the background brush.
fn draw_brush(target: &mut Pixmap, brush: &BackgroundBrush, rect: Rect, transform: Transform) {
    let Some(sk) = SkRect::from_ltrb(rect.left, rect.top, rect.right, rect.bottom) else {
        return;
    };
    let path = PathBuilder::from_rect(sk);
    match brush {
        BackgroundBrush::Transparent => {}
        BackgroundBrush::Solid(color) => {
            render::fill_path(target, &path, *color, transform, None);
        }
        BackgroundBrush::Gradient(c1, c2) => {
            let Some(shader) = LinearGradient::new(
                SkPoint::from_xy(rect.left, rect.top),
                SkPoint::from_xy(rect.right, rect.bottom),
                vec![
                    GradientStop::new(0.0, c1.to_skia()),
                    GradientStop::new(1.0, c2.to_skia()),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            ) else {
                return;
            };
            let paint = Paint {
                shader,
                anti_alias: true,
                ..Paint::default()
            };
            target.fill_path(&path, &paint, FillRule::Winding, transform, None);
        }
        BackgroundBrush::Image(image, layout) => {
            draw_image_brush(target, image, *layout, rect, transform);
        }
    }
}

/// `CreateImageBrush`: the wallpaper layouts as Avalonia stretches.
fn draw_image_brush(
    target: &mut Pixmap,
    image: &RgbaImage,
    layout: ImageLayout,
    rect: Rect,
    transform: Transform,
) {
    let (iw, ih) = (image.width() as f32, image.height() as f32);
    if iw <= 0.0 || ih <= 0.0 {
        return;
    }
    let (w, h) = (rect.width(), rect.height());
    let pixmap = crate::raster::to_pixmap(image);
    let (sx, sy, spread) = match layout {
        ImageLayout::Fill => {
            let s = (w / iw).max(h / ih);
            (s, s, SpreadMode::Pad)
        }
        ImageLayout::Fit => {
            let s = (w / iw).min(h / ih);
            (s, s, SpreadMode::Pad)
        }
        ImageLayout::Stretch | ImageLayout::Span => (w / iw, h / ih, SpreadMode::Pad),
        ImageLayout::Center => (1.0, 1.0, SpreadMode::Pad),
        ImageLayout::Tile => (1.0, 1.0, SpreadMode::Repeat),
    };
    let (dw, dh) = (iw * sx, ih * sy);
    let (ox, oy) = if layout == ImageLayout::Tile {
        (rect.left, rect.top)
    } else {
        (rect.left + (w - dw) / 2.0, rect.top + (h - dh) / 2.0)
    };
    let shader = tiny_skia::Pattern::new(
        pixmap.as_ref(),
        spread,
        FilterQuality::Bilinear,
        1.0,
        Transform::from_row(sx, 0.0, 0.0, sy, ox, oy),
    );
    let paint = Paint {
        shader,
        anti_alias: true,
        ..Paint::default()
    };
    // Uniform layouts leave the rest of the rectangle empty.
    let draw = match layout {
        ImageLayout::Fit | ImageLayout::Center => Rect::new(
            ox.max(rect.left),
            oy.max(rect.top),
            (ox + dw).min(rect.right),
            (oy + dh).min(rect.bottom),
        ),
        _ => rect,
    };
    if let Some(sk) = SkRect::from_ltrb(draw.left, draw.top, draw.right, draw.bottom) {
        target.fill_path(
            &PathBuilder::from_rect(sk),
            &paint,
            FillRule::Winding,
            transform,
            None,
        );
    }
}

/// Everything inside the viewport: the image, the spotlight layer and the annotations.
pub struct Content<'a> {
    pub image: Option<&'a Pixmap>,
    pub canvas_size: (f32, f32),
    pub annotations: &'a [Annotation],
    pub hidden: Option<AnnotationId>,
    pub spotlight_blur: Option<&'a RgbaImage>,
    /// Draw the image itself (off when the view paints it as a texture).
    pub draw_image: bool,
}

/// Draws the frame under `transform` (frame pixels to target pixels).
pub fn draw(
    target: &mut Pixmap,
    transform: Transform,
    state: &BackgroundState,
    layout: &FrameLayout,
    content: &Content,
    top_left: Option<Color>,
) {
    if state.active {
        let frame = Rect::new(0.0, 0.0, layout.width, layout.height);
        draw_brush(target, &state.brush, frame, transform);
        let card = layout.card();
        let radius = layout.corner_radius;
        if let Some(card_path) = render::rounded_rect_path(card, radius, radius) {
            // `BoxShadow`: blur, offset (0, 10), alpha scaled from the default 0x80 to 80,
            // never painted under the card itself.
            draw_box_shadow(target, transform, card, radius, state);
            if let Some(color) = top_left {
                render::fill_path(target, &card_path, color, transform, None);
            }
        }
    }
    let viewport = layout.viewport_rect();
    let mut clip = SkRect::from_ltrb(viewport.left, viewport.top, viewport.right, viewport.bottom)
        .and_then(|r| {
            rect_mask(
                target.width(),
                target.height(),
                &PathBuilder::from_rect(r),
                transform,
            )
        });
    if state.active
        && layout.corner_radius > 0.0
        && let Some(card) =
            render::rounded_rect_path(layout.card(), layout.corner_radius, layout.corner_radius)
        && let Some(card_mask) = rect_mask(target.width(), target.height(), &card, transform)
        && let Some(mask) = clip.as_mut()
    {
        intersect_masks(mask, &card_mask);
    }
    let Some(mut layer) = Pixmap::new(target.width(), target.height()) else {
        return;
    };
    let image_transform = transform.pre_concat(layout.image_transform());
    if content.draw_image
        && let Some(image) = content.image
    {
        layer.draw_pixmap(
            0,
            0,
            image.as_ref(),
            &PixmapPaint {
                quality: if is_pixel_aligned(&image_transform) {
                    FilterQuality::Nearest
                } else {
                    FilterQuality::Bilinear
                },
                ..PixmapPaint::default()
            },
            image_transform,
            None,
        );
    }
    render::draw_spotlight_overlay(
        &mut layer,
        image_transform,
        content.canvas_size,
        content.annotations,
        content.spotlight_blur,
    );
    render::draw_annotations(
        &mut layer,
        &render::Scene {
            canvas_size: content.canvas_size,
            annotations: content.annotations,
            hidden: content.hidden,
        },
        image_transform,
    );
    target.draw_pixmap(
        0,
        0,
        layer.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        clip.as_ref(),
    );
}

fn is_pixel_aligned(t: &Transform) -> bool {
    t.sx == 1.0
        && t.sy == 1.0
        && t.kx == 0.0
        && t.ky == 0.0
        && t.tx.fract() == 0.0
        && t.ty.fract() == 0.0
}

fn intersect_masks(a: &mut Mask, b: &Mask) {
    for (x, y) in a.data_mut().iter_mut().zip(b.data()) {
        *x = ((*x as u16 * *y as u16 + 127) / 255) as u8;
    }
}

fn draw_box_shadow(
    target: &mut Pixmap,
    transform: Transform,
    card: Rect,
    radius: f32,
    state: &BackgroundState,
) {
    const OFFSET_Y: f32 = 10.0;
    const DEFAULT_ALPHA: f64 = 80.0;
    let base = state.shadow_color;
    let alpha = ((base.a as f64 * (DEFAULT_ALPHA / 128.0)).round()).clamp(0.0, 255.0) as u8;
    if alpha == 0 {
        return;
    }
    let scale = ((transform.sx * transform.sx + transform.ky * transform.ky).sqrt()).max(0.0001);
    let sigma = render::blur_radius_to_sigma(state.shadow_radius) * scale;
    let Some(shadow_path) = render::rounded_rect_path(card.offset(0.0, OFFSET_Y), radius, radius)
    else {
        return;
    };
    let Some(mut layer) = Pixmap::new(target.width(), target.height()) else {
        return;
    };
    render::fill_path(
        &mut layer,
        &shadow_path,
        base.with_alpha(alpha),
        transform,
        None,
    );
    crate::raster::gaussian_blur_pixmap(&mut layer, sigma);
    let outside = render::rounded_rect_path(card, radius, radius)
        .and_then(|p| rect_mask(target.width(), target.height(), &p, transform))
        .map(|mut m| {
            m.invert();
            m
        });
    target.draw_pixmap(
        0,
        0,
        layer.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        outside.as_ref(),
    );
}

/// `GetSnapshot`: the frame at 1:1 (96 DPI), selection chrome left out.
pub fn snapshot(
    state: &BackgroundState,
    source: &RgbaImage,
    annotations: &[Annotation],
    spotlight_blur: Option<&RgbaImage>,
) -> RgbaImage {
    let canvas_size = (source.width() as f32, source.height() as f32);
    let frame = layout(state, canvas_size);
    let (w, h) = snapshot_size(&frame);
    let Some(mut target) = Pixmap::new(w, h) else {
        return source.clone();
    };
    let pixmap = crate::raster::to_pixmap(source);
    let top_left = source
        .get_pixel_checked(0, 0)
        .map(|p| Color::rgba(p.0[0], p.0[1], p.0[2], p.0[3]));
    draw(
        &mut target,
        Transform::identity(),
        state,
        &frame,
        &Content {
            image: Some(&pixmap),
            canvas_size,
            annotations,
            hidden: None,
            spotlight_blur,
            draw_image: true,
        },
        top_left,
    );
    crate::raster::from_pixmap(&target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Point;

    #[test]
    fn inactive_background_exports_the_image_size() {
        let state = BackgroundState::default();
        let image = RgbaImage::from_pixel(30, 20, Rgba([0, 0, 255, 255]));
        let mut r = Annotation::rectangle();
        r.start = Point::new(2.0, 2.0);
        r.end = Point::new(40.0, 10.0);
        let out = snapshot(&state, &image, &[r], None);
        assert_eq!(out.dimensions(), (30, 20));
        assert_eq!(out.get_pixel(3, 5).0, [0xef, 0x44, 0x44, 255]);
    }

    #[test]
    fn active_background_adds_margin_padding_and_ratio() {
        let state = BackgroundState {
            active: true,
            margin: 10.0,
            padding: 5.0,
            smart_padding: false,
            rounded_corner: 0.0,
            shadow_radius: 0.0,
            output_ratio: Some(1.0),
            brush: BackgroundBrush::Solid(Color::rgb(0, 255, 0)),
            shadow_color: Color::argb(0x80, 0, 0, 0),
            smart_insets: (0, 0, 0, 0),
        };
        let image = RgbaImage::from_pixel(40, 20, Rgba([0, 0, 255, 255]));
        let frame = layout(&state, (40.0, 20.0));
        assert_eq!(snapshot_size(&frame), (70, 70));
        let out = snapshot(&state, &image, &[], None);
        assert_eq!(out.get_pixel(1, 1).0, [0, 255, 0, 255]);
        // The padding takes the image's top-left colour.
        assert_eq!(out.get_pixel(17, 23).0, [0, 0, 255, 255]);
        assert_eq!(out.get_pixel(35, 35).0, [0, 0, 255, 255]);
    }

    #[test]
    fn smart_insets_find_the_border() {
        let mut img = RgbaImage::from_pixel(20, 10, Rgba([255, 255, 255, 255]));
        for x in 5..12 {
            img.put_pixel(x, 3, Rgba([0, 0, 0, 255]));
        }
        assert_eq!(detect_smart_insets(&img), (5, 3, 8, 6));
    }
}
