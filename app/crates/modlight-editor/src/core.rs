//! `Core/Editor/EditorCore`: the document (source image and annotations), the selection
//! the core knows about, z-order, the destructive operations (crop, cut-out, resize,
//! rotate, flip, canvas size, auto crop, flatten) with their annotation transforms, and
//! the undo history.
//!
//! Like upstream, moving, resizing or restyling an annotation records nothing: only
//! adding, removing, reordering and clearing annotations and the image operations do.

use std::sync::Arc;

use image::{Rgba, RgbaImage};

use crate::color::Color;
use crate::geom::{Point, Rect};
use crate::history::{History, Memento};
use crate::model::{
    Annotation, AnnotationId, EditorTool, Kind, balloon_effective_tail_point, set_balloon_tail,
    set_number_tail,
};
use crate::ops;

/// The events `EditorCore` raises, collected for the view to drain once per frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoreEvents {
    pub invalidate: bool,
    pub image_changed: bool,
    pub annotations_restored: bool,
    pub history_changed: bool,
    pub order_changed: bool,
}

impl CoreEvents {
    fn merge(&mut self, other: CoreEvents) {
        self.invalidate |= other.invalidate;
        self.image_changed |= other.image_changed;
        self.annotations_restored |= other.annotations_restored;
        self.history_changed |= other.history_changed;
        self.order_changed |= other.order_changed;
    }
}

#[derive(Default)]
pub struct EditorCore {
    source: Option<Arc<RgbaImage>>,
    /// Bumped whenever the source image changes, for caches keyed on it.
    generation: u64,
    canvas_size: (f32, f32),
    pub active_tool: EditorTool,
    annotations: Vec<Annotation>,
    selected: Option<AnnotationId>,
    history: History,
    events: CoreEvents,
}

impl EditorCore {
    pub fn new() -> EditorCore {
        EditorCore::default()
    }

    pub fn source(&self) -> Option<&Arc<RgbaImage>> {
        self.source.as_ref()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn canvas_size(&self) -> (f32, f32) {
        self.canvas_size
    }

    pub fn annotations(&self) -> &[Annotation] {
        &self.annotations
    }

    pub fn annotations_mut(&mut self) -> &mut [Annotation] {
        &mut self.annotations
    }

    pub fn annotation(&self, id: AnnotationId) -> Option<&Annotation> {
        self.annotations.iter().find(|a| a.id == id)
    }

    pub fn annotation_mut(&mut self, id: AnnotationId) -> Option<&mut Annotation> {
        self.annotations.iter_mut().find(|a| a.id == id)
    }

    pub fn index_of(&self, id: AnnotationId) -> Option<usize> {
        self.annotations.iter().position(|a| a.id == id)
    }

    pub fn selected(&self) -> Option<AnnotationId> {
        self.selected
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    pub fn take_events(&mut self) -> CoreEvents {
        std::mem::take(&mut self.events)
    }

    fn raise(&mut self, events: CoreEvents) {
        self.events.merge(events);
    }

    fn set_source(&mut self, image: Arc<RgbaImage>) {
        self.canvas_size = (image.width() as f32, image.height() as f32);
        self.source = Some(image);
        self.generation += 1;
    }

    /// `LoadImage`: a new document; annotations and history are cleared. The picture
    /// changed (upstream's view model invalidates what it derived from the old one,
    /// such as smart padding, when the preview changes).
    pub fn load_image(&mut self, image: RgbaImage) {
        self.set_source(Arc::new(image));
        self.clear_all(true);
        self.raise(CoreEvents {
            invalidate: true,
            image_changed: true,
            ..CoreEvents::default()
        });
    }

    /// `UpdateSourceImage`: replace the image, keep history and annotations.
    pub fn update_source_image(&mut self, image: RgbaImage) {
        self.set_source(Arc::new(image));
        self.raise(CoreEvents {
            invalidate: true,
            image_changed: true,
            ..CoreEvents::default()
        });
    }

    fn memento(&self, with_canvas: bool, exclude: Option<AnnotationId>) -> Memento {
        Memento {
            annotations: self
                .annotations
                .iter()
                .filter(|a| Some(a.id) != exclude)
                .map(Annotation::clone_for_history)
                .collect(),
            canvas_size: self.canvas_size,
            canvas: if with_canvas {
                self.source.clone()
            } else {
                None
            },
            selected: self.selected,
        }
    }

    /// `CreateCanvasMemento`.
    pub fn create_canvas_memento(&mut self) {
        let m = self.memento(true, None);
        self.history.add(m);
    }

    /// `CreateAnnotationsMemento`. Upstream skips it while the Crop tool is active
    /// (unless forced), for the crop annotation its core drew; the port's crop never
    /// enters the document, and the skip only lost the undo step of a picture, emoji or
    /// paste added with the Crop tool active.
    pub fn create_annotations_memento(&mut self, exclude: Option<AnnotationId>, _force: bool) {
        let m = self.memento(false, exclude);
        self.history.add(m);
    }

    /// `ApplyImageOperation`: record the canvas, replace the image, then either clear the
    /// annotations or transform them to follow the image.
    pub fn apply_image_operation(
        &mut self,
        operation: impl FnOnce(&RgbaImage) -> Option<RgbaImage>,
        clear_annotations: bool,
        transform: Option<&dyn Fn(&mut EditorCore)>,
    ) -> bool {
        let Some(source) = self.source.clone() else {
            return false;
        };
        let Some(result) = operation(&source) else {
            return false;
        };
        if result.width() == 0 || result.height() == 0 {
            return false;
        }
        self.create_canvas_memento();
        self.set_source(Arc::new(result));
        if clear_annotations {
            self.clear_annotations_for_image_mutation();
        } else if let Some(transform) = transform {
            transform(self);
            self.raise(CoreEvents {
                annotations_restored: true,
                ..CoreEvents::default()
            });
        }
        self.refresh_spotlights();
        self.raise(CoreEvents {
            invalidate: true,
            image_changed: true,
            history_changed: true,
            ..CoreEvents::default()
        });
        true
    }

    /// `ApplyImageEffect`.
    pub fn apply_image_effect(
        &mut self,
        effect: impl FnOnce(&RgbaImage) -> Option<RgbaImage>,
    ) -> bool {
        self.apply_image_operation(effect, false, None)
    }

    pub fn resize_image(&mut self, width: u32, height: u32) -> bool {
        if width == 0 || height == 0 {
            return false;
        }
        let Some(source) = &self.source else {
            return false;
        };
        let sx = width as f32 / source.width() as f32;
        let sy = height as f32 / source.height() as f32;
        self.apply_image_operation(
            |img| Some(ops::resize(img, width, height)),
            false,
            Some(&move |core: &mut EditorCore| core.scale_annotations(sx, sy)),
        )
    }

    pub fn resize_canvas(
        &mut self,
        top: i32,
        right: i32,
        bottom: i32,
        left: i32,
        color: Color,
    ) -> bool {
        self.apply_image_operation(
            |img| ops::resize_canvas(img, left, top, right, bottom, color),
            false,
            Some(&move |core: &mut EditorCore| {
                core.transform_annotations(&|p| Point::new(p.x + left as f32, p.y + top as f32))
            }),
        )
    }

    fn old_size(&self) -> (f32, f32) {
        self.source
            .as_ref()
            .map(|s| (s.width() as f32, s.height() as f32))
            .unwrap_or((0.0, 0.0))
    }

    pub fn rotate_90_clockwise(&mut self) -> bool {
        let (w, h) = self.old_size();
        self.apply_image_operation(
            |img| Some(ops::rotate_orthogonal(img, 90)),
            false,
            Some(&move |core: &mut EditorCore| core.rotate_annotations_orthogonal(90, w, h)),
        )
    }

    pub fn rotate_90_counter_clockwise(&mut self) -> bool {
        let (w, h) = self.old_size();
        self.apply_image_operation(
            |img| Some(ops::rotate_orthogonal(img, -90)),
            false,
            Some(&move |core: &mut EditorCore| core.rotate_annotations_orthogonal(270, w, h)),
        )
    }

    pub fn rotate_180(&mut self) -> bool {
        let (w, h) = self.old_size();
        self.apply_image_operation(
            |img| Some(ops::rotate_orthogonal(img, 180)),
            false,
            Some(&move |core: &mut EditorCore| core.rotate_annotations_orthogonal(180, w, h)),
        )
    }

    pub fn rotate_custom_angle(&mut self, angle: f32, auto_resize: bool) -> bool {
        let (w, h) = self.old_size();
        let Some(source) = self.source.clone() else {
            return false;
        };
        let rotated = ops::rotate_custom(&source, angle, auto_resize);
        let (nw, nh) = (rotated.width() as f32, rotated.height() as f32);
        self.apply_image_operation(
            move |_| Some(rotated),
            false,
            Some(&move |core: &mut EditorCore| {
                core.rotate_annotations_arbitrary(angle, w, h, nw, nh, auto_resize)
            }),
        )
    }

    pub fn flip_horizontal(&mut self) -> bool {
        let (w, _) = self.old_size();
        self.apply_image_operation(
            |img| Some(ops::flip_horizontal(img)),
            false,
            Some(&move |core: &mut EditorCore| core.flip_annotations(true, false, w, 0.0)),
        )
    }

    pub fn flip_vertical(&mut self) -> bool {
        let (_, h) = self.old_size();
        self.apply_image_operation(
            |img| Some(ops::flip_vertical(img)),
            false,
            Some(&move |core: &mut EditorCore| core.flip_annotations(false, true, 0.0, h)),
        )
    }

    /// `FlattenImage`: the composite the view rendered replaces the image; the
    /// annotations are cleared.
    pub fn flatten_image(&mut self, composite: RgbaImage) -> bool {
        if self.annotations.is_empty() {
            return false;
        }
        self.apply_image_operation(move |_| Some(composite), true, None)
    }

    /// `AutoCrop(tolerance)`.
    pub fn auto_crop(&mut self, tolerance: i32) -> bool {
        let Some(source) = self.source.clone() else {
            return false;
        };
        let top_left = *source.get_pixel(0, 0);
        let (min_x, min_y) = ops::content_bounds(&source, top_left, tolerance)
            .map(|b| (b.0 as f32, b.1 as f32))
            .unwrap_or((0.0, 0.0));
        self.apply_image_operation(
            move |img| Some(ops::auto_crop(img, top_left, tolerance)),
            false,
            Some(&move |core: &mut EditorCore| {
                core.transform_annotations(&|p| Point::new(p.x - min_x, p.y - min_y))
            }),
        )
    }

    fn transform_annotations(&mut self, f: &dyn Fn(Point) -> Point) {
        for a in &mut self.annotations {
            // `SetTailPoint(transform(GetEffectiveTailPoint()))`, with the tail taken
            // before the body moves: a default tail follows the body, so taking it after
            // (upstream's order) would move it twice.
            let balloon_tail =
                matches!(a.kind, Kind::SpeechBalloon(_)).then(|| balloon_effective_tail_point(a));
            a.start = f(a.start);
            a.end = f(a.end);
            match &mut a.kind {
                Kind::Freehand { points, .. } => {
                    for p in points.iter_mut() {
                        *p = f(*p);
                    }
                }
                Kind::Number(n) if n.tail_initialized => {
                    n.tail_point = f(n.tail_point);
                }
                // A bent line keeps its shape (upstream left the curve point behind).
                Kind::Line { curve, .. } | Kind::Arrow { curve, .. } if curve.activated => {
                    curve.point = f(curve.point);
                }
                _ => {}
            }
            if let Some(tail) = balloon_tail {
                set_balloon_tail(a, f(tail));
            }
        }
    }

    fn scale_annotations(&mut self, sx: f32, sy: f32) {
        self.transform_annotations(&|p| Point::new(p.x * sx, p.y * sy));
        for a in &mut self.annotations {
            a.stroke_width *= sx.min(sy);
            if let Kind::Text(t) = &mut a.kind {
                t.font_size *= sy;
            }
        }
    }

    fn rotate_annotations_orthogonal(&mut self, angle: i32, old_w: f32, old_h: f32) {
        self.transform_annotations(&|p| match angle {
            90 => Point::new(old_h - p.y, p.x),
            180 => Point::new(old_w - p.x, old_h - p.y),
            270 => Point::new(p.y, old_w - p.x),
            _ => p,
        });
        for a in &mut self.annotations {
            a.rotation = normalize_angle(a.rotation + angle as f32);
        }
    }

    fn rotate_annotations_arbitrary(
        &mut self,
        angle: f32,
        old_w: f32,
        old_h: f32,
        new_w: f32,
        new_h: f32,
        auto_resize: bool,
    ) {
        let rad = angle * std::f32::consts::PI / 180.0;
        let (sin, cos) = rad.sin_cos();
        let (cx, cy) = (old_w / 2.0, old_h / 2.0);
        let (tx, ty) = (new_w / 2.0, new_h / 2.0);
        self.transform_annotations(&|p| {
            let nx = cx + (p.x - cx) * cos - (p.y - cy) * sin;
            let ny = cy + (p.x - cx) * sin + (p.y - cy) * cos;
            if auto_resize {
                Point::new(nx - cx + tx, ny - cy + ty)
            } else {
                Point::new(nx, ny)
            }
        });
        for a in &mut self.annotations {
            a.rotation = normalize_angle(a.rotation + angle);
        }
    }

    fn flip_annotations(&mut self, flip_h: bool, flip_v: bool, width: f32, height: f32) {
        self.transform_annotations(&|p| {
            Point::new(
                if flip_h { width - p.x } else { p.x },
                if flip_v { height - p.y } else { p.y },
            )
        });
        for a in &mut self.annotations {
            if flip_h {
                a.rotation = 360.0 - a.rotation;
            }
            if flip_v {
                a.rotation = 360.0 - a.rotation;
            }
            a.rotation = normalize_angle(a.rotation);
        }
    }

    /// `CutOut(startPos, endPos, isVertical)`: removes a strip and joins the rest.
    pub fn cut_out(&mut self, start: i32, end: i32, vertical: bool) -> bool {
        let Some(source) = self.source.clone() else {
            return false;
        };
        let (start, end) = if start > end {
            (end, start)
        } else {
            (start, end)
        };
        let limit = if vertical {
            source.width() as i32
        } else {
            source.height() as i32
        };
        if start < 0 || end > limit || start >= end {
            return false;
        }
        let cut = end - start;
        if limit - cut <= 0 {
            return false;
        }
        self.create_canvas_memento();
        let (w, h) = (source.width(), source.height());
        let result = if vertical {
            let new_w = w - cut as u32;
            let mut out = RgbaImage::new(new_w, h);
            for y in 0..h {
                for x in 0..new_w {
                    let sx = if (x as i32) < start {
                        x
                    } else {
                        x + cut as u32
                    };
                    out.put_pixel(x, y, *source.get_pixel(sx, y));
                }
            }
            out
        } else {
            let new_h = h - cut as u32;
            let mut out = RgbaImage::new(w, new_h);
            for y in 0..new_h {
                let sy = if (y as i32) < start {
                    y
                } else {
                    y + cut as u32
                };
                for x in 0..w {
                    out.put_pixel(x, y, *source.get_pixel(x, sy));
                }
            }
            out
        };
        self.set_source(Arc::new(result));
        if vertical {
            self.adjust_for_cut(start, cut, true);
        } else {
            self.adjust_for_cut(start, cut, false);
        }
        self.refresh_spotlights();
        self.raise(CoreEvents {
            invalidate: true,
            image_changed: true,
            annotations_restored: true,
            history_changed: true,
            ..CoreEvents::default()
        });
        true
    }

    /// `AdjustAnnotationsForVerticalCut` / `...HorizontalCut`.
    fn adjust_for_cut(&mut self, cut_start: i32, cut_size: i32, vertical: bool) {
        let cut_end = (cut_start + cut_size) as f32;
        let cut_start = cut_start as f32;
        let cut_size = cut_size as f32;
        let axis = |p: Point| if vertical { p.x } else { p.y };
        let with_axis = |p: Point, v: f32| {
            if vertical {
                Point::new(v, p.y)
            } else {
                Point::new(p.x, v)
            }
        };
        let mut index = self.annotations.len();
        while index > 0 {
            index -= 1;
            let bounds = self.annotations[index].bounds();
            let (lo, hi) = if vertical {
                (bounds.left, bounds.right)
            } else {
                (bounds.top, bounds.bottom)
            };
            if lo >= cut_start && hi <= cut_end {
                self.annotations.remove(index);
                continue;
            }
            // Wholly after the strip, or across it; anything else stays where it is.
            let after = lo >= cut_end;
            if !after && hi <= cut_end {
                continue;
            }
            let a = &mut self.annotations[index];
            // Where a coordinate goes: past the strip back by its size, inside it to the
            // seam, before it unchanged.
            let shift = |v: f32| {
                if v >= cut_end {
                    v - cut_size
                } else if v > cut_start {
                    cut_start
                } else {
                    v
                }
            };
            // The balloon's tail as it is before the body moves (a default tail follows
            // the body and would otherwise move twice).
            let balloon_tail =
                matches!(a.kind, Kind::SpeechBalloon(_)).then(|| balloon_effective_tail_point(a));
            if after {
                a.start = with_axis(a.start, axis(a.start) - cut_size);
                a.end = with_axis(a.end, axis(a.end) - cut_size);
            } else {
                // Across the strip: the part before it stays, the rest closes up (upstream
                // moved both ends back, shifting the part before the strip as well).
                a.start = with_axis(a.start, shift(axis(a.start)));
                a.end = with_axis(a.end, shift(axis(a.end)));
            }
            match &mut a.kind {
                Kind::Freehand { points, .. } => {
                    for p in points.iter_mut() {
                        *p = with_axis(*p, shift(axis(*p)));
                    }
                }
                Kind::Number(n) if n.tail_initialized => {
                    let t = n.tail_point;
                    n.tail_point = with_axis(t, shift(axis(t)));
                }
                Kind::Line { curve, .. } | Kind::Arrow { curve, .. } if curve.activated => {
                    curve.point = with_axis(curve.point, shift(axis(curve.point)));
                }
                _ => {}
            }
            if let Some(t) = balloon_tail {
                set_balloon_tail(a, with_axis(t, shift(axis(t))));
            }
        }
    }

    /// `Crop(bounds)`.
    pub fn crop(&mut self, bounds: Rect) -> bool {
        let Some(source) = self.source.clone() else {
            return false;
        };
        let x = bounds.left.max(0.0) as i32;
        let y = bounds.top.max(0.0) as i32;
        // The part of the area left of or above the image is not in the picture.
        let visible_w = bounds.width() - (-bounds.left).max(0.0);
        let visible_h = bounds.height() - (-bounds.top).max(0.0);
        let width = ((source.width() as i32 - x) as f32).min(visible_w) as i32;
        let height = ((source.height() as i32 - y) as f32).min(visible_h) as i32;
        if width <= 0 || height <= 0 {
            return false;
        }
        if x == 0 && y == 0 && width == source.width() as i32 && height == source.height() as i32 {
            return false;
        }
        self.create_canvas_memento();
        let cropped =
            image::imageops::crop_imm(&*source, x as u32, y as u32, width as u32, height as u32)
                .to_image();
        let (xf, yf, wf, hf) = (x as f32, y as f32, width as f32, height as f32);
        let mut index = self.annotations.len();
        while index > 0 {
            index -= 1;
            if matches!(self.annotations[index].kind, Kind::Crop) {
                continue;
            }
            let b = self.annotations[index].bounds();
            if b.right < xf || b.left > xf + wf || b.bottom < yf || b.top > yf + hf {
                self.annotations.remove(index);
                continue;
            }
            let a = &mut self.annotations[index];
            // The balloon's tail before the body moves (a default tail follows the body).
            let balloon_tail =
                matches!(a.kind, Kind::SpeechBalloon(_)).then(|| balloon_effective_tail_point(a));
            a.start = a.start.offset(-xf, -yf);
            a.end = a.end.offset(-xf, -yf);
            match &mut a.kind {
                Kind::Freehand { points, .. } => {
                    for p in points.iter_mut() {
                        *p = p.offset(-xf, -yf);
                    }
                }
                Kind::Line { curve, .. } | Kind::Arrow { curve, .. } if curve.activated => {
                    curve.point = curve.point.offset(-xf, -yf);
                }
                _ => {}
            }
            if let Some(t) = balloon_tail {
                set_balloon_tail(a, t.offset(-xf, -yf));
            } else if let Kind::Number(n) = &a.kind
                && n.tail_initialized
            {
                let t = n.tail_point;
                set_number_tail(a, t.offset(-xf, -yf));
            }
        }
        self.set_source(Arc::new(cropped));
        self.refresh_spotlights();
        self.raise(CoreEvents {
            invalidate: true,
            image_changed: true,
            annotations_restored: true,
            history_changed: true,
            ..CoreEvents::default()
        });
        true
    }

    /// `ClearAll(resetHistory)`.
    pub fn clear_all(&mut self, reset_history: bool) {
        if !reset_history {
            self.create_annotations_memento(None, true);
        }
        self.annotations.clear();
        if reset_history {
            self.history.clear();
        }
        self.selected = None;
        self.raise(CoreEvents {
            invalidate: true,
            history_changed: true,
            ..CoreEvents::default()
        });
    }

    fn clear_annotations_for_image_mutation(&mut self) {
        if self.annotations.is_empty() {
            return;
        }
        self.annotations.clear();
        self.selected = None;
        self.raise(CoreEvents {
            annotations_restored: true,
            ..CoreEvents::default()
        });
    }

    /// Spotlights cover the whole canvas (`RefreshAnnotationsForCurrentImage`). Effect
    /// bitmaps are regenerated by the view, which owns the effect caches.
    fn refresh_spotlights(&mut self) {
        let size = self.canvas_size;
        for a in &mut self.annotations {
            if let Kind::Spotlight(s) = &mut a.kind {
                s.canvas_size = size;
            }
        }
    }

    pub fn select(&mut self, id: Option<AnnotationId>) {
        self.selected = id;
        self.raise(CoreEvents {
            invalidate: true,
            ..CoreEvents::default()
        });
    }

    /// `DeleteSelected`.
    pub fn delete_selected(&mut self) {
        if let Some(id) = self.selected {
            self.remove_annotation(id);
        }
    }

    fn reorder(&mut self, f: impl FnOnce(&mut Vec<Annotation>, usize) -> bool) {
        let Some(id) = self.selected else {
            return;
        };
        let Some(index) = self.index_of(id) else {
            return;
        };
        // The memento is only made when the order actually changes.
        let snapshot = self.memento(false, None);
        if f(&mut self.annotations, index) {
            self.history.add(snapshot);
            self.raise(CoreEvents {
                order_changed: true,
                history_changed: true,
                invalidate: true,
                ..CoreEvents::default()
            });
        }
    }

    pub fn bring_to_front(&mut self) {
        self.reorder(|list, i| {
            if i == list.len() - 1 {
                return false;
            }
            let a = list.remove(i);
            list.push(a);
            true
        });
    }

    pub fn send_to_back(&mut self) {
        self.reorder(|list, i| {
            if i == 0 {
                return false;
            }
            let a = list.remove(i);
            list.insert(0, a);
            true
        });
    }

    pub fn bring_forward(&mut self) {
        self.reorder(|list, i| {
            if i == list.len() - 1 {
                return false;
            }
            let a = list.remove(i);
            list.insert(i + 1, a);
            true
        });
    }

    pub fn send_backward(&mut self) {
        self.reorder(|list, i| {
            if i == 0 {
                return false;
            }
            let a = list.remove(i);
            list.insert(i - 1, a);
            true
        });
    }

    /// `RemoveAnnotation`: records the state first; removing a step renumbers the later
    /// steps.
    pub fn remove_annotation(&mut self, id: AnnotationId) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        self.create_annotations_memento(None, false);
        let mut renumbered = false;
        if let Kind::Number(n) = &self.annotations[index].kind {
            let deleted = n.number;
            for a in &mut self.annotations {
                if let Kind::Number(other) = &mut a.kind
                    && other.number > deleted
                {
                    other.number -= 1;
                }
            }
            renumbered = true;
        }
        self.annotations.remove(index);
        if self.selected == Some(id) {
            self.selected = None;
        }
        self.raise(CoreEvents {
            history_changed: true,
            invalidate: true,
            annotations_restored: renumbered,
            ..CoreEvents::default()
        });
    }

    /// `AddAnnotation`: records the state before adding.
    pub fn add_annotation(&mut self, annotation: Annotation) {
        let index = self.annotations.len();
        self.insert_annotation(index, annotation);
    }

    /// `AddAnnotation` at a position in the list. The view inserts highlights below the
    /// first non-effect annotation, where upstream's canvas puts their visuals.
    pub fn insert_annotation(&mut self, index: usize, annotation: Annotation) {
        self.create_annotations_memento(None, false);
        let index = index.min(self.annotations.len());
        self.annotations.insert(index, annotation);
        self.raise(CoreEvents {
            history_changed: true,
            invalidate: true,
            ..CoreEvents::default()
        });
    }

    pub fn undo(&mut self) {
        let Some(memento) = self.history.pop_undo() else {
            return;
        };
        let current = self.memento(memento.canvas.is_some(), None);
        self.history.push_redo(current);
        self.restore_state(memento);
        self.raise(CoreEvents {
            history_changed: true,
            ..CoreEvents::default()
        });
    }

    pub fn redo(&mut self) {
        let Some(memento) = self.history.pop_redo() else {
            return;
        };
        let current = self.memento(memento.canvas.is_some(), None);
        self.history.push_undo_raw(current);
        self.restore_state(memento);
        self.raise(CoreEvents {
            history_changed: true,
            ..CoreEvents::default()
        });
    }

    /// `RestoreState`.
    fn restore_state(&mut self, memento: Memento) {
        self.annotations = memento.annotations;
        if let Some(canvas) = memento.canvas {
            self.canvas_size = memento.canvas_size;
            self.source = Some(canvas);
            self.generation += 1;
            self.raise(CoreEvents {
                image_changed: true,
                ..CoreEvents::default()
            });
        }
        self.selected = memento
            .selected
            .filter(|id| self.annotations.iter().any(|a| a.id == *id));
        self.raise(CoreEvents {
            invalidate: true,
            annotations_restored: true,
            ..CoreEvents::default()
        });
    }

    /// `GetSnapshot` of the core: the source image only.
    pub fn snapshot(&self) -> Option<RgbaImage> {
        self.source.as_deref().cloned()
    }

    /// `SampleCanvasColor`: `#RRGGBB` of the source at a point (out of range samples 0,0).
    pub fn sample_canvas_color(&self, point: Point) -> Option<String> {
        let source = self.source.as_ref()?;
        let mut x = point.x.round() as i64;
        let mut y = point.y.round() as i64;
        if x < 0 || y < 0 || x >= source.width() as i64 || y >= source.height() as i64 {
            x = 0;
            y = 0;
        }
        let p = source.get_pixel(x as u32, y as u32).0;
        Some(Color::rgba(p[0], p[1], p[2], p[3]).to_hex_rgb())
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<Rgba<u8>> {
        self.source
            .as_ref()
            .filter(|s| x < s.width() && y < s.height())
            .map(|s| *s.get_pixel(x, y))
    }

    /// Mutating access used by the view for annotations it edits in place.
    pub fn with_annotation(&mut self, id: AnnotationId, f: impl FnOnce(&mut Annotation)) {
        if let Some(a) = self.annotation_mut(id) {
            f(a);
            self.raise(CoreEvents {
                invalidate: true,
                ..CoreEvents::default()
            });
        }
    }
}

fn normalize_angle(mut angle: f32) -> f32 {
    while angle >= 360.0 {
        angle -= 360.0;
    }
    while angle < 0.0 {
        angle += 360.0;
    }
    angle
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_with(width: u32, height: u32) -> EditorCore {
        let mut core = EditorCore::new();
        core.load_image(RgbaImage::from_pixel(
            width,
            height,
            Rgba([255, 255, 255, 255]),
        ));
        core.active_tool = EditorTool::Rectangle;
        core
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Annotation {
        let mut r = Annotation::rectangle();
        r.start = Point::new(x, y);
        r.end = Point::new(x + w, y + h);
        r
    }

    #[test]
    fn add_undo_redo_follow_the_memento_rules() {
        let mut core = core_with(100, 100);
        let a = rect(10.0, 10.0, 20.0, 20.0);
        let a_id = a.id;
        core.add_annotation(a);
        core.add_annotation(rect(50.0, 50.0, 10.0, 10.0));
        assert_eq!(core.annotations().len(), 2);
        // A move is not recorded on its own: undoing the second add goes back to the
        // copies taken when it was added, so the move is undone with it.
        core.with_annotation(a_id, |a| a.translate(5.0, 0.0));
        core.undo();
        assert_eq!(core.annotations().len(), 1);
        assert_eq!(core.annotations()[0].start.x, 10.0);
        core.undo();
        assert!(core.annotations().is_empty());
        assert!(!core.can_undo());
        core.redo();
        assert_eq!(core.annotations().len(), 1);
        core.redo();
        assert_eq!(core.annotations().len(), 2);
        assert_eq!(core.annotations()[0].start.x, 15.0);
        assert!(!core.can_redo());
    }

    #[test]
    fn removing_a_step_renumbers_the_later_ones() {
        let mut core = core_with(100, 100);
        let mut ids = Vec::new();
        for n in 1..=3 {
            let mut s = Annotation::number();
            if let Kind::Number(d) = &mut s.kind {
                d.number = n;
            }
            ids.push(s.id);
            core.add_annotation(s);
        }
        core.remove_annotation(ids[0]);
        let numbers: Vec<i32> = core
            .annotations()
            .iter()
            .map(|a| match &a.kind {
                Kind::Number(n) => n.number,
                _ => 0,
            })
            .collect();
        assert_eq!(numbers, vec![1, 2]);
        core.undo();
        assert_eq!(core.annotations().len(), 3);
    }

    #[test]
    fn crop_moves_and_drops_annotations_and_undo_restores_the_image() {
        let mut core = core_with(200, 100);
        core.add_annotation(rect(60.0, 20.0, 10.0, 10.0));
        core.add_annotation(rect(150.0, 80.0, 10.0, 10.0));
        assert!(core.crop(Rect::new(50.0, 10.0, 120.0, 60.0)));
        assert_eq!(core.source().unwrap().dimensions(), (70, 50));
        assert_eq!(core.annotations().len(), 1);
        assert_eq!(core.annotations()[0].start, Point::new(10.0, 10.0));
        core.undo();
        assert_eq!(core.source().unwrap().dimensions(), (200, 100));
        assert_eq!(core.annotations().len(), 2);
        core.redo();
        assert_eq!(core.source().unwrap().dimensions(), (70, 50));
    }

    /// A crop area that starts left of or above the image (a Shift-square quick crop
    /// dragged towards the corner) crops what it covers of the image; upstream kept the
    /// area's full size from the edge and took pixels the area never covered.
    #[test]
    fn a_crop_area_over_the_edge_crops_what_it_covers() {
        let mut core = core_with(200, 100);
        core.add_annotation(rect(60.0, 20.0, 10.0, 10.0));
        assert!(core.crop(Rect::new(-20.0, -10.0, 80.0, 60.0)));
        assert_eq!(core.source().unwrap().dimensions(), (80, 60));
        assert_eq!(core.annotations()[0].start, Point::new(60.0, 20.0));
    }

    #[test]
    fn full_image_crop_is_refused() {
        let mut core = core_with(50, 50);
        assert!(!core.crop(Rect::new(0.0, 0.0, 50.0, 50.0)));
        assert!(!core.can_undo());
    }

    #[test]
    fn vertical_cut_out_shifts_annotations_on_the_right() {
        let mut core = core_with(100, 40);
        core.add_annotation(rect(70.0, 5.0, 10.0, 10.0));
        core.add_annotation(rect(32.0, 5.0, 5.0, 5.0));
        assert!(core.cut_out(30, 50, true));
        assert_eq!(core.source().unwrap().dimensions(), (80, 40));
        assert_eq!(core.annotations().len(), 1);
        assert_eq!(core.annotations()[0].start.x, 50.0);
    }

    #[test]
    fn rotation_moves_annotations_with_the_image() {
        let mut core = core_with(100, 50);
        core.add_annotation(rect(10.0, 5.0, 20.0, 10.0));
        assert!(core.rotate_90_clockwise());
        assert_eq!(core.source().unwrap().dimensions(), (50, 100));
        let a = &core.annotations()[0];
        assert_eq!(a.start, Point::new(45.0, 10.0));
        assert_eq!(a.rotation, 90.0);
    }

    #[test]
    fn z_order_changes_are_undoable() {
        let mut core = core_with(100, 100);
        let a = rect(0.0, 0.0, 5.0, 5.0);
        let a_id = a.id;
        core.add_annotation(a);
        core.add_annotation(rect(10.0, 0.0, 5.0, 5.0));
        core.select(Some(a_id));
        core.bring_to_front();
        assert_eq!(core.annotations()[1].id, a_id);
        core.undo();
        assert_eq!(core.annotations()[0].id, a_id);
        assert_eq!(core.selected(), Some(a_id));
    }

    fn balloon(x: f32, y: f32, w: f32, h: f32) -> Annotation {
        let mut b = Annotation::speech_balloon();
        b.start = Point::new(x, y);
        b.end = Point::new(x + w, y + h);
        b
    }

    /// A balloon whose tail was never dragged has its default tail under the body (as
    /// every balloon drawn large enough has); the image operations move that tail once,
    /// with the body, instead of moving the body and then the new default again.
    #[test]
    fn a_default_balloon_tail_moves_once_with_the_image() {
        let tail = |core: &EditorCore| balloon_effective_tail_point(&core.annotations()[0]);
        // The default tail of a 200 x 100 balloon at (100, 100) is (200, 230).
        let mut core = core_with(400, 300);
        core.add_annotation(balloon(100.0, 100.0, 200.0, 100.0));
        assert!(core.crop(Rect::new(50.0, 50.0, 350.0, 250.0)));
        assert_eq!(tail(&core), Point::new(150.0, 180.0));

        let mut core = core_with(400, 300);
        core.add_annotation(balloon(100.0, 100.0, 200.0, 100.0));
        assert!(core.rotate_90_clockwise());
        assert_eq!(tail(&core), Point::new(70.0, 200.0));

        let mut core = core_with(400, 300);
        core.add_annotation(balloon(100.0, 100.0, 200.0, 100.0));
        assert!(core.flip_vertical());
        assert_eq!(tail(&core), Point::new(200.0, 70.0));

        let mut core = core_with(400, 300);
        core.add_annotation(balloon(100.0, 100.0, 200.0, 100.0));
        assert!(core.cut_out(20, 40, false));
        assert_eq!(tail(&core), Point::new(200.0, 210.0));
    }

    fn curved_line() -> Annotation {
        let mut l = Annotation::line();
        l.start = Point::new(10.0, 100.0);
        l.end = Point::new(200.0, 100.0);
        crate::model::set_curve_point(&mut l, Point::new(105.0, 150.0));
        l
    }

    /// A line or arrow bent by its middle handle keeps its shape: the curve point moves
    /// with the image like the end points.
    #[test]
    fn a_curved_line_keeps_its_shape_through_image_operations() {
        let curve = |core: &EditorCore| core.annotations()[0].curve().unwrap().point;
        let mut core = core_with(400, 300);
        core.add_annotation(curved_line());
        assert!(core.crop(Rect::new(5.0, 5.0, 305.0, 205.0)));
        assert_eq!(curve(&core), Point::new(100.0, 145.0));

        let mut core = core_with(400, 300);
        core.add_annotation(curved_line());
        assert!(core.rotate_90_clockwise());
        assert_eq!(curve(&core), Point::new(150.0, 105.0));

        let mut core = core_with(400, 300);
        core.add_annotation(curved_line());
        assert!(core.resize_image(200, 150));
        assert_eq!(curve(&core), Point::new(52.5, 75.0));

        let mut core = core_with(400, 300);
        core.add_annotation(curved_line());
        assert!(core.cut_out(50, 70, true));
        let a = &core.annotations()[0];
        assert_eq!((a.start.x, a.end.x), (10.0, 180.0));
        assert_eq!(curve(&core), Point::new(85.0, 150.0));
    }

    /// An annotation across the removed strip keeps the part before the strip where it
    /// was and loses the strip's width; upstream moved its whole body back by the width.
    #[test]
    fn a_cut_through_an_annotation_shortens_it() {
        let mut core = core_with(100, 100);
        core.add_annotation(rect(20.0, 5.0, 60.0, 10.0));
        assert!(core.cut_out(30, 50, true));
        let a = &core.annotations()[0];
        assert_eq!((a.start.x, a.end.x), (20.0, 60.0));

        let mut core = core_with(100, 100);
        core.add_annotation(rect(5.0, 20.0, 10.0, 60.0));
        assert!(core.cut_out(30, 50, false));
        let a = &core.annotations()[0];
        assert_eq!((a.start.y, a.end.y), (20.0, 60.0));

        // Drawn right to left, with an end inside the strip.
        let mut core = core_with(100, 100);
        let mut r = Annotation::rectangle();
        r.start = Point::new(80.0, 5.0);
        r.end = Point::new(40.0, 15.0);
        core.add_annotation(r);
        assert!(core.cut_out(30, 50, true));
        let a = &core.annotations()[0];
        assert_eq!((a.start.x, a.end.x), (60.0, 30.0));
    }

    #[test]
    fn additions_with_the_crop_tool_active_are_recorded() {
        let mut core = core_with(10, 10);
        core.active_tool = EditorTool::Crop;
        core.add_annotation(rect(0.0, 0.0, 2.0, 2.0));
        assert!(core.can_undo());
    }
}
