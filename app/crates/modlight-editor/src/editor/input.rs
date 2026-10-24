//! `EditorInputController` and `EditorSelectionController`: drawing each tool, selecting,
//! moving, resizing and rotating, the handles, hover outlines, crop and cut-out, text
//! boxes, and the editor's keys (`EditorView.OnKeyDown/OnKeyUp`).
//!
//! Points are image pixels. `ctrl` is the Command key on a Mac (`MACOS.md`: Cmd stands
//! for Ctrl).

use std::sync::Arc;

use image::RgbaImage;

use crate::geom::{Point, Rect};
use crate::model::{
    self, Annotation, AnnotationId, EditorTool, EffectKind, Kind, balloon_effective_tail_point,
    set_balloon_tail, set_number_tail,
};

use super::{Editor, EmojiTarget, Modal, Request};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    /// Ctrl on Windows, Cmd here.
    pub ctrl: bool,
    pub alt: bool,
}

impl Modifiers {
    pub const NONE: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: false,
    };

    pub fn from_egui(m: egui::Modifiers) -> Modifiers {
        Modifiers {
            shift: m.shift,
            ctrl: m.command,
            alt: m.alt,
        }
    }

    pub fn is_none(&self) -> bool {
        !self.shift && !self.ctrl && !self.alt
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// The tags of the selection handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandleTag {
    TopLeft,
    TopCenter,
    TopRight,
    RightCenter,
    BottomRight,
    BottomCenter,
    BottomLeft,
    LeftCenter,
    Rotate,
    SegmentStart,
    SegmentEnd,
    SegmentCenter,
    BalloonTail,
    StepTail,
}

impl HandleTag {
    /// `TryGetHandleDirection`.
    pub fn direction(self) -> (i32, i32) {
        use HandleTag::*;
        let h = match self {
            TopLeft | BottomLeft | LeftCenter => -1,
            TopRight | BottomRight | RightCenter => 1,
            _ => 0,
        };
        let v = match self {
            TopLeft | TopCenter | TopRight => -1,
            BottomLeft | BottomCenter | BottomRight => 1,
            _ => 0,
        };
        (h, v)
    }

    fn contains_right(self) -> bool {
        self.direction().0 == 1
    }
    fn contains_left(self) -> bool {
        self.direction().0 == -1
    }
    fn contains_top(self) -> bool {
        self.direction().1 == -1
    }
    fn contains_bottom(self) -> bool {
        self.direction().1 == 1
    }
}

/// The 15-point handle (`CreateHandle`).
pub const HANDLE_SIZE: f32 = 15.0;
const MIN_SHAPE_SIZE: f32 = 5.0;
const MIN_CROP_SIZE: f32 = 16.0;

/// `EditorSelectionController`'s state.
#[derive(Clone, Debug, Default)]
pub struct SelectionState {
    pub selected: Option<AnnotationId>,
    pub dragging_handle: Option<HandleTag>,
    pub dragging_shape: bool,
    last_drag_point: Point,
    start_point: Point,
    pub hovered: Option<AnnotationId>,
    pending_emoji_exact: bool,
    pub modifiers: Modifiers,
    last_pointer: Option<Point>,
}

/// The crop handles (`Crop_*` tags).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropHandle {
    TopLeft,
    TopCenter,
    TopRight,
    RightCenter,
    BottomRight,
    BottomCenter,
    BottomLeft,
    LeftCenter,
    Move,
}

impl CropHandle {
    pub const HANDLES: [CropHandle; 8] = [
        CropHandle::TopLeft,
        CropHandle::TopCenter,
        CropHandle::TopRight,
        CropHandle::RightCenter,
        CropHandle::BottomRight,
        CropHandle::BottomCenter,
        CropHandle::BottomLeft,
        CropHandle::LeftCenter,
    ];

    pub fn is_corner(self) -> bool {
        matches!(
            self,
            CropHandle::TopLeft
                | CropHandle::TopRight
                | CropHandle::BottomRight
                | CropHandle::BottomLeft
        )
    }

    pub fn is_horizontal_edge(self) -> bool {
        matches!(
            self,
            CropHandle::TopLeft
                | CropHandle::TopCenter
                | CropHandle::TopRight
                | CropHandle::BottomRight
                | CropHandle::BottomCenter
                | CropHandle::BottomLeft
        )
    }

    /// Where the handle sits on the crop rectangle.
    pub fn position(self, r: &Rect) -> Point {
        let cx = r.left + r.width() / 2.0;
        let cy = r.top + r.height() / 2.0;
        match self {
            CropHandle::TopLeft => Point::new(r.left, r.top),
            CropHandle::TopCenter => Point::new(cx, r.top),
            CropHandle::TopRight => Point::new(r.right, r.top),
            CropHandle::RightCenter => Point::new(r.right, cy),
            CropHandle::BottomRight => Point::new(r.right, r.bottom),
            CropHandle::BottomCenter => Point::new(cx, r.bottom),
            CropHandle::BottomLeft => Point::new(r.left, r.bottom),
            CropHandle::LeftCenter => Point::new(r.left, cy),
            CropHandle::Move => r.center(),
        }
    }

    /// The hit area (`CropHandleCornerHitSize` 40, edges 40 by 28).
    pub fn hit_size(self) -> (f32, f32) {
        if self.is_corner() {
            (40.0, 40.0)
        } else if self.is_horizontal_edge() {
            (40.0, 28.0)
        } else {
            (28.0, 40.0)
        }
    }
}

/// The crop overlay and its chrome.
#[derive(Clone, Debug, Default)]
pub struct CropState {
    /// `_cropActive`: a rectangle with handles waiting for confirmation.
    pub active: bool,
    /// The visible overlay rectangle (image pixels).
    pub rect: Option<Rect>,
    /// Quick crop draws a dashed 2-point outline while dragging.
    pub quick: bool,
    dragging: Option<(CropHandle, Point, Rect)>,
}

impl CropState {
    pub fn is_dragging(&self) -> bool {
        self.dragging.is_some()
    }
}

/// The cut-out preview (`CutOutOverlay`).
#[derive(Clone, Debug, Default)]
pub struct CutOutState {
    pub rect: Option<Rect>,
    pub direction: Option<bool>,
}

/// `EditorInputController`'s state.
#[derive(Default)]
pub struct InputState {
    pub sel: SelectionState,
    pub crop: CropState,
    pub cut_out: Option<CutOutState>,
    drawing: bool,
    start_point: Point,
    last_draw_point: Point,
    /// The annotation being drawn (`_currentShape`), not yet in the core.
    pub pending: Option<Annotation>,
    /// Where it will go in the list (highlights go under vector shapes).
    pending_index: Option<usize>,
    is_creating_effect: bool,
    moving_during_creation: bool,
    ctrl_held_during_draw: bool,
    /// The context menu opened by a right click (over an annotation or the canvas).
    pub context_menu: Option<Point>,
    pub(crate) suppress_dirty: bool,
    pub(crate) visible_center: Option<Point>,
    pub(crate) pending_insert: Option<Arc<RgbaImage>>,
    pub file_menu_requested: bool,
    pub center_on_zoom: bool,
}

impl InputState {
    pub fn is_drawing(&self) -> bool {
        self.drawing
    }

    /// Where the annotation being drawn goes in the list.
    pub fn pending_index(&self) -> Option<usize> {
        self.pending_index
    }

    pub fn is_interacting(&self) -> bool {
        self.sel.dragging_handle.is_some() || self.sel.dragging_shape || self.crop.is_dragging()
    }
}

// ---------------------------------------------------------------------------- selection

/// `ClearSelection`.
pub fn clear_selection(e: &mut Editor) {
    let was = e.input.sel.selected.is_some();
    e.input.sel.selected = None;
    e.input.sel.dragging_handle = None;
    e.input.sel.dragging_shape = false;
    e.input.sel.pending_emoji_exact = false;
    e.input.sel.hovered = None;
    if was {
        selection_changed(e, false);
    }
    e.invalidate();
}

/// `SetSelectedShape`: also opens a balloon's text box.
pub fn set_selected_shape(e: &mut Editor, id: AnnotationId) {
    let was_none = e.input.sel.selected.is_none();
    e.input.sel.selected = Some(id);
    e.input.sel.hovered = Some(id);
    if was_none {
        selection_changed(e, true);
    }
    if matches!(
        e.annotation(id).map(|a| &a.kind),
        Some(Kind::SpeechBalloon(_))
    ) {
        show_balloon_text_editor(e, id);
    }
    e.invalidate();
}

/// `OnSelectionChanged` of the view.
fn selection_changed(e: &mut Editor, has: bool) {
    let selected = if has { e.input.sel.selected } else { None };
    e.vm_selected = selected;
    e.core.select(selected);
    if selected.is_some() {
        e.sync_options_from_selection();
    }
}

/// `ShouldIgnoreSelection`: with a drawing tool, Cmd draws over existing shapes.
fn should_ignore_selection(e: &Editor, m: Modifiers) -> bool {
    e.tools.active_tool != EditorTool::Select && m.ctrl
}

/// The bounds `GetLogicalRect` gives a shape.
pub fn logical_rect(a: &Annotation) -> Rect {
    match &a.kind {
        Kind::SpeechBalloon(_) | Kind::Spotlight(_) => a.bounds(),
        _ => a.bounds(),
    }
}

/// Avalonia's visual hit test of stroked paths, which reaches half the pen width.
fn stroke_hit(a: &Annotation, p: Point) -> bool {
    let reach = a.stroke_width / 2.0;
    match &a.kind {
        Kind::Line { .. } | Kind::Arrow { .. } => model::distance_to_path(a, p) <= reach.max(0.0),
        Kind::Freehand { points, .. } => points
            .windows(2)
            .any(|w| crate::geom::distance_to_segment(p, w[0], w[1]) <= reach),
        _ => false,
    }
}

/// `HitTestShape`: the topmost annotation under the point.
pub fn hit_test_shape(e: &Editor, p: Point) -> Option<AnnotationId> {
    for a in e.core.annotations().iter().rev() {
        if matches!(a.kind, Kind::Crop | Kind::CutOut { .. }) {
            continue;
        }
        if let Kind::Spotlight(_) = a.kind {
            if a.hit_test(p) {
                return Some(a.id);
            }
            continue;
        }
        let bounds = match &a.kind {
            Kind::Line { .. } | Kind::Arrow { .. } => a.bounds().inflate(10.0, 10.0),
            Kind::Freehand { .. } => a.bounds().inflate(5.0, 5.0),
            Kind::SpeechBalloon(_) | Kind::Number(_) => model::interaction_bounds(a, 5.0),
            _ => logical_rect(a),
        };
        if (a.rotation != 0.0 || bounds.contains_inclusive(p) || stroke_hit(a, p))
            && (a.hit_test(p) || stroke_hit(a, p))
        {
            return Some(a.id);
        }
    }
    None
}

/// `TryGetRotatableAnnotation`.
pub fn is_rotatable(a: &Annotation) -> bool {
    matches!(
        &a.kind,
        Kind::SpeechBalloon(_)
            | Kind::Effect(_)
            | Kind::Ellipse { .. }
            | Kind::Text(_)
            | Kind::Rectangle { .. }
            | Kind::Emoji { .. }
            | Kind::Image { .. }
    )
}

/// `UpdateSelectionHandles`: the handles of the selected annotation.
pub fn selection_handles(e: &Editor) -> Vec<(HandleTag, Point)> {
    let Some(a) = e.input.sel.selected.and_then(|id| e.annotation(id)) else {
        return Vec::new();
    };
    if suppress_handles(e) {
        return Vec::new();
    }
    handles_for(a)
}

pub fn handles_for(a: &Annotation) -> Vec<(HandleTag, Point)> {
    use HandleTag::*;
    let mut out = Vec::new();
    match &a.kind {
        Kind::Line { .. } | Kind::Arrow { .. } => {
            out.push((SegmentStart, a.start));
            out.push((SegmentEnd, a.end));
            if model::supports_curve(a) {
                out.push((SegmentCenter, model::effective_curve_point(a)));
            }
            return out;
        }
        Kind::SpeechBalloon(b) => {
            out.extend(rotatable_handles(a));
            if b.tail_enabled {
                let tail = balloon_effective_tail_point(a);
                let p = if a.rotation == 0.0 {
                    tail
                } else {
                    tail.rotate_around(a.bounds().center(), a.rotation)
                };
                out.push((BalloonTail, p));
            }
            return out;
        }
        Kind::Number(n) => {
            if n.tail_enabled {
                out.push((StepTail, model::number_tail_handle(a)));
            }
            return out;
        }
        Kind::Cursor { .. } | Kind::Freehand { .. } => return out,
        _ => {}
    }
    if is_rotatable(a) && !a.is_smart_eraser() {
        return rotatable_handles(a);
    }
    let b = if let Kind::Spotlight(_) = a.kind {
        a.bounds()
    } else {
        logical_rect(a)
    };
    if b.width() <= 0.0 || b.height() <= 0.0 {
        return out;
    }
    let (l, t, w, h) = (b.left, b.top, b.width(), b.height());
    out.push((TopLeft, Point::new(l, t)));
    out.push((TopCenter, Point::new(l + w / 2.0, t)));
    out.push((TopRight, Point::new(l + w, t)));
    out.push((RightCenter, Point::new(l + w, t + h / 2.0)));
    out.push((BottomRight, Point::new(l + w, t + h)));
    out.push((BottomCenter, Point::new(l + w / 2.0, t + h)));
    out.push((BottomLeft, Point::new(l, t + h)));
    out.push((LeftCenter, Point::new(l, t + h / 2.0)));
    out
}

/// `TryCreateRotatableSelectionHandles`: the eight handles turned with the annotation
/// and the rotation handle 30 points above the top centre.
fn rotatable_handles(a: &Annotation) -> Vec<(HandleTag, Point)> {
    use HandleTag::*;
    let b = logical_rect(a);
    let (l, t) = (b.left, b.top);
    let w = if b.width() > 0.0 { b.width() } else { 20.0 };
    let h = if b.height() > 0.0 { b.height() } else { 20.0 };
    let c = Point::new(l + w / 2.0, t + h / 2.0);
    let r = |p: Point| p.rotate_around(c, a.rotation);
    vec![
        (TopLeft, r(Point::new(l, t))),
        (TopCenter, r(Point::new(l + w / 2.0, t))),
        (TopRight, r(Point::new(l + w, t))),
        (RightCenter, r(Point::new(l + w, t + h / 2.0))),
        (BottomRight, r(Point::new(l + w, t + h))),
        (BottomCenter, r(Point::new(l + w / 2.0, t + h))),
        (BottomLeft, r(Point::new(l, t + h))),
        (LeftCenter, r(Point::new(l, t + h / 2.0))),
        (Rotate, r(Point::new(l + w / 2.0, t - 30.0))),
    ]
}

/// `ShouldSuppressSelectionHandles`.
fn suppress_handles(e: &Editor) -> bool {
    !e.input.is_interacting()
        && e.input.sel.selected.is_some()
        && should_ignore_selection(e, e.input.sel.modifiers)
}

fn hit_handle(e: &Editor, p: Point) -> Option<HandleTag> {
    // Handles are drawn in image space, scaled with the zoom.
    let half = HANDLE_SIZE / 2.0;
    selection_handles(e)
        .into_iter()
        .rev()
        .find(|(_, h)| (p.x - h.x).abs() <= half && (p.y - h.y).abs() <= half)
        .map(|(tag, _)| tag)
}

/// The tool that made an annotation (`GetControlToolType`).
fn tool_of(e: &Editor, id: AnnotationId) -> Option<EditorTool> {
    e.annotation(id).map(|a| a.tool)
}

/// `EditorSelectionController.OnPointerPressed`: true when it took the click.
fn selection_pressed(e: &mut Editor, p: Point, m: Modifiers, clicks: u32) -> bool {
    e.input.sel.modifiers = m;
    if should_ignore_selection(e, m) {
        return false;
    }
    if let Some(tag) = hit_handle(e, p) {
        if tag == HandleTag::SegmentCenter
            && let Some(id) = e.input.sel.selected
            && let Some(a) = e.core.annotation_mut(id)
            && model::supports_curve(a)
        {
            model::ensure_curve_activated(a);
        }
        e.input.sel.dragging_handle = Some(tag);
        e.input.sel.start_point = p;
        return true;
    }
    let tool = e.tools.active_tool;
    if tool != EditorTool::Select
        && tool != EditorTool::Spotlight
        && tool != EditorTool::Freehand
        && let Some(hit) = hit_test_shape(e, p)
        && tool_of(e, hit) == Some(tool)
    {
        if clicks == 2 && double_click_edit(e, hit) {
            return true;
        }
        begin_drag(e, hit, p);
        return true;
    }
    if tool == EditorTool::Select || tool == EditorTool::Spotlight {
        let mut hit = hit_test_shape(e, p);
        if tool == EditorTool::Spotlight
            && !hit.is_some_and(|id| {
                matches!(e.annotation(id).map(|a| &a.kind), Some(Kind::Spotlight(_)))
            })
        {
            hit = None;
        }
        match hit {
            Some(id) => {
                if clicks == 2 && double_click_edit(e, id) {
                    return true;
                }
                begin_drag(e, id, p);
                return true;
            }
            None => {
                clear_selection(e);
                return false;
            }
        }
    }
    false
}

fn begin_drag(e: &mut Editor, id: AnnotationId, p: Point) {
    e.input.sel.selected = Some(id);
    e.input.sel.dragging_shape = true;
    e.input.sel.last_drag_point = p;
    selection_changed(e, true);
    e.invalidate();
}

/// `TryHandleDoubleClickEdit`.
fn double_click_edit(e: &mut Editor, id: AnnotationId) -> bool {
    let Some(kind) = e.annotation(id).map(|a| a.kind.clone()) else {
        return false;
    };
    match kind {
        Kind::Text(_) => {
            show_text_editor(e, id);
            true
        }
        Kind::SpeechBalloon(_) => {
            show_balloon_text_editor(e, id);
            true
        }
        Kind::Emoji { .. } => {
            select_quietly(e, id);
            e.show_emoji_picker(EmojiTarget::Replace(id));
            true
        }
        Kind::Image { .. } => {
            select_quietly(e, id);
            e.request(Request::ReplaceImage(id));
            true
        }
        _ => false,
    }
}

fn select_quietly(e: &mut Editor, id: AnnotationId) {
    e.input.sel.selected = Some(id);
    selection_changed(e, true);
}

/// `EditorSelectionController.OnPointerMoved`: true when it used the move.
fn selection_moved(e: &mut Editor, p: Point, m: Modifiers) -> bool {
    e.input.sel.modifiers = m;
    e.input.sel.last_pointer = Some(p);
    if let Some(tag) = e.input.sel.dragging_handle
        && e.input.sel.selected.is_some()
    {
        handle_resize(e, tag, p, m.shift);
        return true;
    }
    if e.input.sel.dragging_shape && e.input.sel.selected.is_some() {
        handle_move(e, p);
        return true;
    }
    if should_ignore_selection(e, m) {
        e.input.sel.hovered = None;
        return false;
    }
    update_hover(e, p);
    false
}

/// `EditorSelectionController.OnPointerReleased`.
fn selection_released(e: &mut Editor) -> bool {
    if e.input.sel.dragging_handle.is_some() {
        finalize_emoji_render(e);
        e.input.sel.dragging_handle = None;
        e.refresh_selected_effect();
        return true;
    }
    if e.input.sel.dragging_shape {
        e.input.sel.pending_emoji_exact = false;
        e.input.sel.dragging_shape = false;
        e.refresh_selected_effect();
        return true;
    }
    false
}

fn finalize_emoji_render(e: &mut Editor) {
    if !e.input.sel.pending_emoji_exact {
        return;
    }
    e.input.sel.pending_emoji_exact = false;
    e.invalidate();
}

/// `UpdateHoverState`: the ant lines follow the shape under the pointer.
fn update_hover(e: &mut Editor, p: Point) {
    let tool = e.tools.active_tool;
    if matches!(
        tool,
        EditorTool::Crop | EditorTool::CutOut | EditorTool::Freehand
    ) {
        e.input.sel.hovered = None;
        return;
    }
    let mut hit = hit_test_shape(e, p);
    if tool == EditorTool::Spotlight
        && !hit
            .is_some_and(|id| matches!(e.annotation(id).map(|a| &a.kind), Some(Kind::Spotlight(_))))
    {
        hit = None;
    }
    if tool != EditorTool::Select
        && tool != EditorTool::Spotlight
        && hit.is_some_and(|id| tool_of(e, id) != Some(tool))
    {
        hit = None;
    }
    if hit.is_some() && hit == e.input.sel.selected {
        e.input.sel.hovered = e.input.sel.selected;
    } else {
        e.input.sel.hovered = hit;
    }
}

/// `RefreshHoverFeedback` after a modifier change.
pub fn refresh_hover(e: &mut Editor, m: Modifiers) {
    e.input.sel.modifiers = m;
    if e.input.is_interacting() {
        return;
    }
    if should_ignore_selection(e, m) {
        e.input.sel.hovered = None;
        return;
    }
    if let Some(p) = e.input.sel.last_pointer {
        update_hover(e, p);
    }
    e.invalidate();
}

/// `UnrotatePoint`/`RotatePoint` about the origin, in doubles as the controller does.
fn rotate_d(p: (f64, f64), angle: f64) -> (f64, f64) {
    if angle == 0.0 {
        return p;
    }
    let rad = angle.to_radians();
    let (s, c) = rad.sin_cos();
    (p.0 * c - p.1 * s, p.0 * s + p.1 * c)
}

/// `HandleResize`.
fn handle_resize(e: &mut Editor, tag: HandleTag, current: Point, shift: bool) {
    let Some(id) = e.input.sel.selected else {
        return;
    };
    let dx = current.x - e.input.sel.start_point.x;
    let dy = current.y - e.input.sel.start_point.y;
    let Some(a) = e.core.annotation(id).cloned() else {
        return;
    };
    let mut a = a;
    let mut refresh_effect = false;
    match tag {
        HandleTag::SegmentStart | HandleTag::SegmentEnd | HandleTag::SegmentCenter
            if a.is_curved_segment() =>
        {
            let start = a.start;
            let end = a.end;
            match tag {
                HandleTag::SegmentStart => {
                    let s = if shift {
                        snap_45(end, current)
                    } else {
                        current
                    };
                    model::set_endpoints(&mut a, s, end);
                }
                HandleTag::SegmentEnd => {
                    let en = if shift {
                        snap_45(start, current)
                    } else {
                        current
                    };
                    model::set_endpoints(&mut a, start, en);
                }
                _ => model::set_curve_point(&mut a, current),
            }
        }
        HandleTag::BalloonTail => {
            if let Kind::SpeechBalloon(b) = &a.kind
                && b.tail_enabled
            {
                let point = if a.rotation != 0.0 {
                    current.rotate_around(a.bounds().center(), -a.rotation)
                } else {
                    current
                };
                set_balloon_tail(&mut a, point);
            }
        }
        HandleTag::StepTail => {
            if let Kind::Number(n) = &a.kind
                && n.tail_enabled
            {
                set_number_tail(&mut a, current);
            }
        }
        HandleTag::Rotate if is_rotatable(&a) => {
            let b = logical_rect(&a);
            let cx = (b.left + b.width() / 2.0) as f64;
            let cy = (b.top + b.height() / 2.0) as f64;
            let mut angle = (current.x as f64 - cx)
                .atan2(-(current.y as f64 - cy))
                .to_degrees();
            if shift {
                angle = (angle / 45.0).round() * 45.0;
            }
            a.rotation = angle as f32;
            refresh_effect = a.is_effect();
        }
        _ => {
            let rotated_resize =
                is_rotatable(&a) && !matches!(a.kind, Kind::Emoji { .. }) && a.rotation != 0.0;
            if rotated_resize {
                resize_rotated(&mut a, tag, current);
                refresh_effect = a.is_effect();
            } else if let Kind::Emoji { .. } = a.kind {
                if let Some(r) = rotated_square_resize(&a, tag, current, 16.0) {
                    a.start = Point::new(r.left, r.top);
                    a.end = Point::new(r.right, r.bottom);
                    e.input.sel.pending_emoji_exact = true;
                }
            } else {
                let rect = logical_rect(&a);
                let (left, top, width, height) = (rect.left, rect.top, rect.width(), rect.height());
                match &a.kind {
                    Kind::SpeechBalloon(_) => {
                        let (mut nl, mut nt, mut nw, mut nh) = (left, top, width, height);
                        if tag.contains_right() {
                            nw = (width + dx).max(20.0);
                        } else if tag.contains_left() {
                            let change = (width - 20.0).min(dx);
                            nl += change;
                            nw -= change;
                        }
                        if tag.contains_bottom() {
                            nh = (height + dy).max(20.0);
                        } else if tag.contains_top() {
                            let change = (height - 20.0).min(dy);
                            nt += change;
                            nh -= change;
                        }
                        apply_resized_bounds(&mut a, nl, nt, nw, nh);
                    }
                    Kind::Spotlight(_) => {
                        let b = a.bounds();
                        let (mut nl, mut nt, mut nw, mut nh) =
                            (b.left, b.top, b.width(), b.height());
                        if tag.contains_right() {
                            nw = (nw + dx).max(10.0);
                        } else if tag.contains_left() {
                            let change = (nw - 10.0).min(dx);
                            nl += change;
                            nw -= change;
                        }
                        if tag.contains_bottom() {
                            nh = (nh + dy).max(10.0);
                        } else if tag.contains_top() {
                            let change = (nh - 10.0).min(dy);
                            nt += change;
                            nh -= change;
                        }
                        a.start = Point::new(nl, nt);
                        a.end = Point::new(nl + nw, nt + nh);
                    }
                    Kind::Rectangle { .. }
                    | Kind::SmartEraser { .. }
                    | Kind::Ellipse { .. }
                    | Kind::Effect(_)
                    | Kind::Image { .. }
                    | Kind::Cursor { .. }
                    | Kind::Text(_) => {
                        let (mut nl, mut nt, mut nw, mut nh) = (left, top, width, height);
                        if tag.contains_right() {
                            nw = (width + dx).max(1.0);
                        } else if tag.contains_left() {
                            let change = (width - 1.0).min(dx);
                            nl += change;
                            nw -= change;
                        }
                        if tag.contains_bottom() {
                            nh = (height + dy).max(1.0);
                        } else if tag.contains_top() {
                            let change = (height - 1.0).min(dy);
                            nt += change;
                            nh -= change;
                        }
                        a.start = Point::new(nl, nt);
                        a.end = Point::new(nl + nw, nt + nh);
                        refresh_effect = a.is_effect();
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some(slot) = e.core.annotation_mut(id) {
        *slot = a;
    }
    e.input.sel.start_point = current;
    if refresh_effect {
        e.refresh_effect(id);
    }
    e.invalidate();
}

/// `ApplyResizedBounds` (the balloon's tail scales with its body).
fn apply_resized_bounds(a: &mut Annotation, left: f32, top: f32, width: f32, height: f32) {
    let previous = a.bounds();
    a.start = Point::new(left, top);
    a.end = Point::new(left + width, top + height);
    if matches!(a.kind, Kind::SpeechBalloon(_))
        && model::balloon_has_tail_point(a)
        && previous.width() > 0.0
        && previous.height() > 0.0
    {
        let new = a.bounds();
        let tail = balloon_effective_tail_point(a);
        let sx = new.width() / previous.width();
        let sy = new.height() / previous.height();
        set_balloon_tail(
            a,
            Point::new(
                new.left + (tail.x - previous.left) * sx,
                new.top + (tail.y - previous.top) * sy,
            ),
        );
    }
}

/// `TryResizeRotatedAnnotation`: resize along the annotation's own axes.
fn resize_rotated(a: &mut Annotation, tag: HandleTag, current: Point) {
    let (h_dir, v_dir) = tag.direction();
    if h_dir == 0 && v_dir == 0 {
        return;
    }
    let b = logical_rect(a);
    if b.width() <= 0.0 || b.height() <= 0.0 {
        return;
    }
    let min = if matches!(a.kind, Kind::Text(_)) {
        10.0
    } else {
        1.0
    };
    let angle = a.rotation as f64;
    let center = (b.mid_x() as f64, b.mid_y() as f64);
    let local_center = rotate_d(center, -angle);
    let local_pointer = rotate_d((current.x as f64, current.y as f64), -angle);
    let half_w = b.width() as f64 / 2.0;
    let half_h = b.height() as f64 / 2.0;
    let anchor_x = local_center.0 - h_dir as f64 * half_w;
    let anchor_y = local_center.1 - v_dir as f64 * half_h;
    let (mut cx, mut cy) = local_center;
    let (mut w, mut h) = (b.width() as f64, b.height() as f64);
    if h_dir != 0 {
        let px = if h_dir < 0 {
            local_pointer.0.min(anchor_x - min)
        } else {
            local_pointer.0.max(anchor_x + min)
        };
        w = (anchor_x - px).abs();
        cx = (anchor_x + px) / 2.0;
    }
    if v_dir != 0 {
        let py = if v_dir < 0 {
            local_pointer.1.min(anchor_y - min)
        } else {
            local_pointer.1.max(anchor_y + min)
        };
        h = (anchor_y - py).abs();
        cy = (anchor_y + py) / 2.0;
    }
    let world = rotate_d((cx, cy), angle);
    apply_resized_bounds(
        a,
        (world.0 - w / 2.0) as f32,
        (world.1 - h / 2.0) as f32,
        w as f32,
        h as f32,
    );
}

/// `TryGetRotatedSquareResizeBounds`: emoji stay square.
fn rotated_square_resize(a: &Annotation, tag: HandleTag, current: Point, min: f64) -> Option<Rect> {
    let (h_dir, v_dir) = tag.direction();
    let b = logical_rect(a);
    let size = b.width().max(b.height()) as f64;
    if size <= 0.0 || (h_dir == 0 && v_dir == 0) {
        return None;
    }
    let angle = a.rotation as f64;
    let lc = rotate_d((b.mid_x() as f64, b.mid_y() as f64), -angle);
    let lp = rotate_d((current.x as f64, current.y as f64), -angle);
    let half = size / 2.0;
    let (mut ncx, mut ncy) = lc;
    let new_size;
    if h_dir != 0 && v_dir != 0 {
        let ax = lc.0 - h_dir as f64 * half;
        let ay = lc.1 - v_dir as f64 * half;
        let sx = if h_dir < 0 { ax - lp.0 } else { lp.0 - ax };
        let sy = if v_dir < 0 { ay - lp.1 } else { lp.1 - ay };
        new_size = min.max(sx.max(sy));
        ncx = (ax + ax + h_dir as f64 * new_size) / 2.0;
        ncy = (ay + ay + v_dir as f64 * new_size) / 2.0;
    } else if h_dir != 0 {
        let ax = lc.0 - h_dir as f64 * half;
        let sx = if h_dir < 0 { ax - lp.0 } else { lp.0 - ax };
        new_size = min.max(sx);
        ncx = (ax + ax + h_dir as f64 * new_size) / 2.0;
    } else {
        let ay = lc.1 - v_dir as f64 * half;
        let sy = if v_dir < 0 { ay - lp.1 } else { lp.1 - ay };
        new_size = min.max(sy);
        ncy = (ay + ay + v_dir as f64 * new_size) / 2.0;
    }
    let world = rotate_d((ncx, ncy), angle);
    Some(Rect::from_xywh(
        (world.0 - new_size / 2.0) as f32,
        (world.1 - new_size / 2.0) as f32,
        new_size as f32,
        new_size as f32,
    ))
}

/// `HandleMove`.
fn handle_move(e: &mut Editor, current: Point) {
    let Some(id) = e.input.sel.selected else {
        return;
    };
    let dx = current.x - e.input.sel.last_drag_point.x;
    let dy = current.y - e.input.sel.last_drag_point.y;
    let mut is_effect = false;
    if let Some(a) = e.core.annotation_mut(id) {
        a.translate(dx, dy);
        is_effect = a.is_effect();
    }
    e.input.sel.last_drag_point = current;
    if is_effect {
        e.refresh_effect(id);
    }
    e.invalidate();
}

/// `MoveSelectedShape` (arrow keys).
pub fn move_selected(e: &mut Editor, dx: f32, dy: f32) {
    if e.input.sel.selected.is_none() {
        return;
    }
    e.input.sel.last_drag_point = Point::default();
    handle_move(e, Point::new(dx, dy));
    e.refresh_selected_effect();
}

/// `SnapTo45Degrees`.
pub fn snap_45(start: Point, end: Point) -> Point {
    let dx = (end.x - start.x) as f64;
    let dy = (end.y - start.y) as f64;
    let distance = (dx * dx + dy * dy).sqrt();
    if distance < 0.001 {
        return end;
    }
    let angle = dy.atan2(dx);
    let snapped = (angle / (std::f64::consts::PI / 4.0)).round() * (std::f64::consts::PI / 4.0);
    Point::new(
        (start.x as f64 + distance * snapped.cos()) as f32,
        (start.y as f64 + distance * snapped.sin()) as f32,
    )
}

// ---------------------------------------------------------------------------- text boxes

/// `ShowTextEditor`: edit an existing text annotation in place (it is hidden meanwhile).
pub fn show_text_editor(e: &mut Editor, id: AnnotationId) {
    let Some(text) = e
        .annotation(id)
        .and_then(|a| a.text_data())
        .map(|t| t.text.clone())
    else {
        return;
    };
    e.text_editing = Some(super::TextEditing {
        id,
        text,
        creating: false,
        balloon: false,
    });
}

/// `ShowSpeechBalloonTextEditor`: a balloon too small to write in grows to 200 by 100.
pub fn show_balloon_text_editor(e: &mut Editor, id: AnnotationId) {
    let Some(a) = e.core.annotation_mut(id) else {
        return;
    };
    let b = a.bounds();
    if b.width() < 50.0 || b.height() < 30.0 {
        let w = b.width().max(200.0);
        let h = b.height().max(100.0);
        a.end = Point::new(a.start.x + w, a.start.y + h);
        let tail_enabled = matches!(&a.kind, Kind::SpeechBalloon(d) if d.tail_enabled);
        let near_start = matches!(&a.kind, Kind::SpeechBalloon(d)
            if (d.tail_point.x - a.start.x).abs() < 1.0 && (d.tail_point.y - a.start.y).abs() < 1.0);
        if tail_enabled && (!model::balloon_has_tail_point(a) || near_start) {
            let p = model::balloon_default_tail_point(a);
            set_balloon_tail(a, p);
        }
    }
    let text = a.text_data().map(|t| t.text.clone()).unwrap_or_default();
    e.text_editing = Some(super::TextEditing {
        id,
        text,
        creating: false,
        balloon: true,
    });
    e.invalidate();
}

/// The text box lost focus (or Enter/Escape): `OnCreationLostFocus`, `CompleteEditing`,
/// the balloon editor's `LostFocus`.
pub fn commit_text_edit(e: &mut Editor) {
    let Some(edit) = e.text_editing.take() else {
        return;
    };
    if edit.creating {
        let Some(mut a) = e.input.pending.take().filter(|p| p.id == edit.id) else {
            return;
        };
        if edit.text.trim().is_empty() {
            e.invalidate();
            return;
        }
        if let Some(t) = a.text_data_mut() {
            t.text = edit.text.clone();
        }
        // `MeasureNaturalSize`, then the control's desired size (at least the bounds).
        let (w, h) = natural_text_size(&a);
        a.end = Point::new(a.start.x + w, a.start.y + h);
        let id = a.id;
        e.core.add_annotation(a);
        set_selected_shape(e, id);
        e.pump();
        return;
    }
    if let Some(a) = e.core.annotation_mut(edit.id)
        && let Some(t) = a.text_data_mut()
    {
        t.text = edit.text;
    }
    e.invalidate();
}

/// `OutlinedTextControl.MeasureNaturalSize`: the text plus 8 points of padding and the
/// outline width on each side.
pub fn natural_text_size(a: &Annotation) -> (f32, f32) {
    let Some(t) = a.text_data() else {
        return (0.0, 0.0);
    };
    if t.text.is_empty() {
        return (0.0, 0.0);
    }
    let laid = crate::text::layout(
        &t.text,
        &crate::text::FontRequest {
            family: t.font_family.clone(),
            bold: t.bold,
            italic: t.italic,
        },
        t.font_size,
        None,
        t.align,
    );
    let pad = 16.0 + a.stroke_width * 2.0;
    (laid.width + pad, laid.height + pad)
}

// ---------------------------------------------------------------------------- crop

/// `ComputeAutoCropRect`: the content bounds when they are smaller than the image.
fn auto_crop_rect(e: &Editor) -> Rect {
    let full = e.canvas_rect();
    let Some(source) = e.core.source() else {
        return full;
    };
    let corner = *source.get_pixel(0, 0);
    match crate::ops::content_bounds(source, corner, 10) {
        Some((min_x, min_y, max_x, max_y)) => {
            let w = max_x - min_x + 1;
            let h = max_y - min_y + 1;
            if w >= source.width() && h >= source.height() {
                full
            } else {
                Rect::from_xywh(min_x as f32, min_y as f32, w as f32, h as f32)
            }
        }
        None => full,
    }
}

/// `ActivateCropToFullImage`.
pub fn activate_crop_to_full_image(e: &mut Editor) {
    if !e.has_image() {
        return;
    }
    let rect = auto_crop_rect(e);
    e.input.crop = CropState {
        active: true,
        rect: Some(rect),
        quick: false,
        dragging: None,
    };
    e.invalidate();
}

/// `CancelCrop`: true when a crop was showing.
pub fn cancel_crop(e: &mut Editor) -> bool {
    let showing = e.input.crop.active || e.input.crop.rect.is_some();
    e.input.crop = CropState::default();
    if showing {
        e.invalidate();
    }
    showing
}

/// `TryConfirmCrop`.
pub fn confirm_crop(e: &mut Editor) -> bool {
    if !e.input.crop.active {
        return false;
    }
    perform_crop(e);
    true
}

fn perform_crop(e: &mut Editor) {
    let Some(r) = e.input.crop.rect else {
        return;
    };
    e.input.crop = CropState::default();
    if r.width() <= 0.0 || r.height() <= 0.0 {
        return;
    }
    let x = r.left.round();
    let y = r.top.round();
    let w = r.width().round();
    let h = r.height().round();
    if w > 0.0 && h > 0.0 && e.core.crop(Rect::from_xywh(x, y, w, h)) {
        e.pump();
        e.image_cropped_notification();
    }
    e.invalidate();
}

/// `ComputeCropHandleResizedRect`.
fn crop_resized(
    handle: CropHandle,
    start: Point,
    current: Point,
    original: Rect,
    w: f32,
    h: f32,
) -> Rect {
    let clamp = |v: f32, lo: f32, hi: f32| if hi < lo { lo } else { v.clamp(lo, hi) };
    let (mut left, mut top, mut right, mut bottom) =
        (original.left, original.top, original.right, original.bottom);
    let cx = clamp(current.x, 0.0, w);
    let cy = clamp(current.y, 0.0, h);
    match handle {
        CropHandle::TopLeft => {
            left = clamp(cx, 0.0, right - MIN_CROP_SIZE);
            top = clamp(cy, 0.0, bottom - MIN_CROP_SIZE);
        }
        CropHandle::TopCenter => top = clamp(cy, 0.0, bottom - MIN_CROP_SIZE),
        CropHandle::TopRight => {
            right = clamp(cx, left + MIN_CROP_SIZE, w);
            top = clamp(cy, 0.0, bottom - MIN_CROP_SIZE);
        }
        CropHandle::RightCenter => right = clamp(cx, left + MIN_CROP_SIZE, w),
        CropHandle::BottomRight => {
            right = clamp(cx, left + MIN_CROP_SIZE, w);
            bottom = clamp(cy, top + MIN_CROP_SIZE, h);
        }
        CropHandle::BottomCenter => bottom = clamp(cy, top + MIN_CROP_SIZE, h),
        CropHandle::BottomLeft => {
            left = clamp(cx, 0.0, right - MIN_CROP_SIZE);
            bottom = clamp(cy, top + MIN_CROP_SIZE, h);
        }
        CropHandle::LeftCenter => left = clamp(cx, 0.0, right - MIN_CROP_SIZE),
        CropHandle::Move => {
            let dx = current.x - start.x;
            let dy = current.y - start.y;
            let nl = clamp(original.left + dx, 0.0, (w - original.width()).max(0.0));
            let nt = clamp(original.top + dy, 0.0, (h - original.height()).max(0.0));
            return Rect::from_xywh(nl, nt, original.width(), original.height());
        }
    }
    left = clamp(left, 0.0, w - MIN_CROP_SIZE);
    top = clamp(top, 0.0, h - MIN_CROP_SIZE);
    right = clamp(right, left + MIN_CROP_SIZE, w);
    bottom = clamp(bottom, top + MIN_CROP_SIZE, h);
    Rect::from_xywh(
        left,
        top,
        MIN_CROP_SIZE.max(right - left),
        MIN_CROP_SIZE.max(bottom - top),
    )
}

/// The crop handle under the point (`Crop_*` hit areas, scaled to `scale` points per
/// image pixel since they keep their size on screen... they are drawn in image space).
pub fn hit_crop_handle(e: &Editor, p: Point) -> Option<CropHandle> {
    let r = e.input.crop.rect?;
    CropHandle::HANDLES.into_iter().rev().find(|h| {
        let c = h.position(&r);
        let (w, hh) = h.hit_size();
        (p.x - c.x).abs() <= w / 2.0 && (p.y - c.y).abs() <= hh / 2.0
    })
}

/// The "Crop" button's rectangle under the crop area (above it when there is no room).
pub fn crop_button_rect(e: &Editor, size: (f32, f32)) -> Option<Rect> {
    let r = e.input.crop.rect?;
    if !e.input.crop.active {
        return None;
    }
    let (bw, bh) = size;
    let x = r.left + (r.width() - bw) / 2.0;
    let canvas_h = e.core.canvas_size().1;
    let below = r.bottom + 10.0;
    let above = r.top - 10.0 - bh;
    let y = if below + bh <= canvas_h { below } else { above };
    Some(Rect::from_xywh(x, y, bw, bh))
}

// ---------------------------------------------------------------------------- input

fn clamp_to_canvas(e: &Editor, p: Point) -> Point {
    let (w, h) = e.core.canvas_size();
    Point::new(p.x.clamp(0.0, w), p.y.clamp(0.0, h))
}

/// `ShouldClearSelectionOnMouseDrawStart`.
fn clears_selection_on_draw(tool: EditorTool) -> bool {
    !matches!(
        tool,
        EditorTool::Select | EditorTool::Image | EditorTool::Emoji
    )
}

/// `ApplyShadowOptions`.
fn apply_shadow(e: &Editor, a: &mut Annotation) {
    a.shadow.enabled = e.tools.shadow_enabled;
    a.shadow.color = e.options.shadow_color_hex.clone();
    a.shadow.blur_radius = e.options.shadow_blur_radius;
    a.shadow.opacity = e.options.shadow_opacity;
    a.shadow.offset_x = e.options.shadow_offset_x;
    a.shadow.offset_y = e.options.shadow_offset_y;
}

/// `OnCanvasPointerPressed`.
pub fn pointer_pressed(e: &mut Editor, p: Point, button: Button, m: Modifiers, clicks: u32) {
    if !e.has_image() || e.modal.is_some() {
        return;
    }
    if button == Button::Middle {
        return; // panning is the view's
    }
    let tool = e.tools.active_tool;
    if e.input.drawing
        && button == Button::Right
        && matches!(tool, EditorTool::Crop | EditorTool::CutOut)
    {
        cancel_region_drawing(e);
        return;
    }
    if e.input.crop.active && button == Button::Right && tool == EditorTool::Crop {
        cancel_crop(e);
        return;
    }
    if button == Button::Right {
        if let Some(hit) = hit_test_shape(e, p)
            && e.input.sel.selected != Some(hit)
        {
            set_selected_shape(e, hit);
        }
        e.input.context_menu = Some(p);
        return;
    }
    if e.input.crop.active {
        if let Some(handle) = hit_crop_handle(e, p) {
            let rect = e.input.crop.rect.unwrap_or_default();
            e.input.crop.dragging = Some((handle, p, rect));
            return;
        }
        if let Some(rect) = e.input.crop.rect {
            if clicks == 2 && rect.contains_inclusive(p) {
                confirm_crop(e);
                return;
            }
            if rect.contains_inclusive(p) {
                e.input.crop.dragging = Some((CropHandle::Move, p, rect));
                return;
            }
        }
        cancel_crop(e);
    }
    if selection_pressed(e, p, m, clicks) {
        return;
    }
    if clears_selection_on_draw(tool) {
        clear_selection(e);
    } else {
        e.input.sel.hovered = None;
    }
    let point = if matches!(
        tool,
        EditorTool::Crop | EditorTool::CutOut | EditorTool::SmartEraser
    ) {
        clamp_to_canvas(e, p)
    } else {
        p
    };
    e.input.start_point = point;
    e.input.last_draw_point = point;
    e.input.moving_during_creation = false;
    e.input.ctrl_held_during_draw = m.ctrl;
    e.input.drawing = true;
    e.input.pending_index = None;
    let start = point;
    let t = e.tools.clone();
    match tool {
        EditorTool::Crop => {
            if e.options.quick_crop {
                e.input.crop = CropState {
                    active: false,
                    rect: Some(Rect::from_xywh(start.x, start.y, 0.0, 0.0)),
                    quick: true,
                    dragging: None,
                };
            } else if !e.input.crop.active {
                activate_crop_to_full_image(e);
            }
            return;
        }
        EditorTool::CutOut => {
            e.input.cut_out = Some(CutOutState::default());
            return;
        }
        EditorTool::Image => {
            e.input.drawing = false;
            e.request(Request::InsertImage);
            return;
        }
        _ => {}
    }
    let pending = match tool {
        EditorTool::Rectangle => {
            let mut a = Annotation::rectangle();
            a.stroke_color = t.selected_color.clone();
            a.stroke_width = t.stroke_width as f32;
            a.set_border_style(t.border_style);
            a.fill_color = t.fill_color.clone();
            if let Kind::Rectangle { corner_radius, .. } = &mut a.kind {
                *corner_radius = t.corner_radius;
            }
            a.start = start;
            a.end = start;
            apply_shadow(e, &mut a);
            Some(a)
        }
        EditorTool::Ellipse => {
            let mut a = Annotation::ellipse();
            a.stroke_color = t.selected_color.clone();
            a.stroke_width = t.stroke_width as f32;
            a.set_border_style(t.border_style);
            a.fill_color = t.fill_color.clone();
            a.start = start;
            a.end = start;
            apply_shadow(e, &mut a);
            Some(a)
        }
        EditorTool::Line => {
            let mut a = Annotation::line();
            a.stroke_color = t.selected_color.clone();
            a.stroke_width = t.stroke_width as f32;
            a.set_border_style(t.border_style);
            a.start = start;
            a.end = start;
            apply_shadow(e, &mut a);
            Some(a)
        }
        EditorTool::Arrow => {
            let mut a = Annotation::arrow();
            a.stroke_color = t.selected_color.clone();
            a.stroke_width = t.stroke_width as f32;
            model::set_arrow_style(&mut a, t.arrow_style);
            a.start = start;
            a.end = start;
            apply_shadow(e, &mut a);
            Some(a)
        }
        EditorTool::Cursor => {
            let Some(bitmap) = crate::cursor::annotation_bitmap(t.cursor_type) else {
                e.input.drawing = false;
                return;
            };
            let mut a = Annotation::cursor(t.cursor_type);
            a.start = start;
            a.end = Point::new(
                start.x + bitmap.width() as f32,
                start.y + bitmap.height() as f32,
            );
            a.set_image(Some(bitmap));
            Some(a)
        }
        EditorTool::Text => {
            begin_text(e, start);
            return;
        }
        EditorTool::Spotlight => {
            let mut a = Annotation::spotlight();
            let opacity = (t.effect_strength / tools_max(EditorTool::Spotlight) * 255.0)
                .clamp(0.0, 255.0) as u8;
            a.start = start;
            a.end = start;
            if let Kind::Spotlight(s) = &mut a.kind {
                s.canvas_size = e.core.canvas_size();
                s.darken_opacity = opacity;
                s.blur_amount = t.spotlight_blur;
                s.is_ellipse = t.effect_ellipse;
            }
            Some(a)
        }
        EditorTool::Blur | EditorTool::Pixelate | EditorTool::Magnify | EditorTool::Highlight => {
            let kind = match tool {
                EditorTool::Blur => EffectKind::Blur,
                EditorTool::Pixelate => EffectKind::Pixelate,
                EditorTool::Magnify => EffectKind::Magnify,
                _ => EffectKind::Highlight,
            };
            let mut a = Annotation::effect(kind);
            a.start = start;
            a.end = start;
            if let Some(d) = a.effect_data_mut() {
                if kind != EffectKind::Highlight {
                    d.amount = t.effect_strength;
                }
                if kind == EffectKind::Magnify {
                    d.is_ellipse = t.effect_ellipse;
                }
            }
            if kind == EffectKind::Highlight {
                a.fill_color = t.fill_color.clone();
                // Highlights go under the first annotation that is not an effect.
                let index = e
                    .core
                    .annotations()
                    .iter()
                    .position(|x| !x.is_effect())
                    .unwrap_or(e.core.annotations().len());
                e.input.pending_index = Some(index);
            }
            e.input.is_creating_effect = true;
            Some(a)
        }
        EditorTool::SpeechBalloon => {
            let mut fill = t.fill_color.clone();
            if fill.is_empty() || fill == "#00000000" || fill == "Transparent" {
                let stroke = crate::color::Color::parse(&t.selected_color);
                let light = stroke.map(|c| c.luminance() > 0.5).unwrap_or(true);
                fill = if light { "#FF000000" } else { "#FFFFFFFF" }.to_string();
            }
            let mut a = Annotation::speech_balloon();
            a.stroke_color = t.selected_color.clone();
            a.stroke_width = t.stroke_width as f32;
            a.fill_color = fill;
            if let Kind::SpeechBalloon(b) = &mut a.kind {
                b.text.text_color = t.text_color.clone();
                b.text.font_size = t.font_size;
                b.text.font_family = t.font_family.clone();
                b.text.bold = t.text_bold;
                b.text.italic = t.text_italic;
                b.text.align = t.text_align;
                b.corner_radius = t.corner_radius;
                b.tail_enabled = t.speech_balloon_tail;
            }
            a.start = start;
            a.end = start;
            apply_shadow(e, &mut a);
            Some(a)
        }
        EditorTool::Step => {
            let mut a = Annotation::number();
            a.stroke_color = t.selected_color.clone();
            a.stroke_width = t.stroke_width as f32;
            a.fill_color = t.fill_color.clone();
            if let Kind::Number(n) = &mut a.kind {
                n.text_color = t.text_color.clone();
                n.font_size = t.font_size;
                n.bold = t.text_bold;
                n.step_type = t.step_type;
                n.tail_enabled = t.speech_balloon_tail;
                n.number = t.number_counter;
            }
            a.start = start;
            apply_shadow(e, &mut a);
            e.tools.number_counter += 1;
            Some(a)
        }
        EditorTool::SmartEraser => {
            let mut a = Annotation::smart_eraser();
            a.start = start;
            a.end = start;
            if let Some(source) = e.core.source() {
                model::configure_smart_eraser(&mut a, source);
            }
            Some(a)
        }
        EditorTool::Freehand => {
            let mut a = Annotation::freehand();
            a.stroke_color = t.selected_color.clone();
            a.stroke_width = t.stroke_width as f32;
            a.set_border_style(t.border_style);
            if let Some(points) = a.freehand_points_mut() {
                points.push(start);
            }
            apply_shadow(e, &mut a);
            Some(a)
        }
        _ => None,
    };
    e.input.pending = pending;
    e.invalidate();
}

fn tools_max(tool: EditorTool) -> f32 {
    super::tools::max_effect_strength(tool)
}

/// `HandleTextTool`: a text box at the click; the annotation is made when it closes.
fn begin_text(e: &mut Editor, start: Point) {
    let t = e.tools.clone();
    let mut text_color = t.text_color.clone();
    if crate::color::Color::parse(&text_color).is_some_and(|c| c.a == 0) {
        let fallback = e.options.text_text_color();
        text_color = fallback.to_hex_argb();
        e.set_text_color(&text_color.clone());
    }
    let mut a = Annotation::text();
    a.stroke_color = t.selected_color.clone();
    a.fill_color = t.fill_color.clone();
    a.stroke_width = t.stroke_width as f32;
    if let Some(d) = a.text_data_mut() {
        d.text_color = text_color;
        d.font_size = t.font_size;
        d.font_family = t.font_family.clone();
        d.align = t.text_align;
        d.bold = t.text_bold;
        d.italic = t.text_italic;
    }
    a.start = start;
    a.end = start;
    apply_shadow(e, &mut a);
    let id = a.id;
    e.input.pending = Some(a);
    e.input.drawing = false;
    e.text_editing = Some(super::TextEditing {
        id,
        text: String::new(),
        creating: true,
        balloon: false,
    });
    e.invalidate();
}

fn cancel_region_drawing(e: &mut Editor) {
    if e.input.crop.active {
        cancel_crop(e);
    }
    if e.input.crop.quick {
        e.input.crop = CropState::default();
    }
    e.input.cut_out = None;
    e.input.pending = None;
    e.input.drawing = false;
    e.input.moving_during_creation = false;
    e.input.ctrl_held_during_draw = false;
    e.invalidate();
}

/// `CanMoveShapeDuringCreation`.
fn can_move_during_creation(a: &Annotation) -> bool {
    matches!(
        a.kind,
        Kind::Rectangle { .. }
            | Kind::SmartEraser { .. }
            | Kind::Ellipse { .. }
            | Kind::Effect(_)
            | Kind::SpeechBalloon(_)
            | Kind::Spotlight(_)
            | Kind::Freehand { .. }
            | Kind::Line { .. }
            | Kind::Arrow { .. }
    )
}

/// `OnCanvasPointerMoved`. `left_down`/`right_down` are the buttons held.
pub fn pointer_moved(e: &mut Editor, p: Point, right_down: bool, m: Modifiers) {
    if !e.has_image() {
        return;
    }
    if !e.input.drawing && selection_moved(e, p, m) {
        return;
    }
    if let Some((handle, start, rect)) = e.input.crop.dragging {
        let (w, h) = e.core.canvas_size();
        e.input.crop.rect = Some(crop_resized(handle, start, p, rect, w, h));
        e.invalidate();
        return;
    }
    if !e.input.drawing {
        return;
    }
    let tool = e.tools.active_tool;
    let mut current = p;
    if matches!(
        tool,
        EditorTool::Crop | EditorTool::CutOut | EditorTool::SmartEraser
    ) {
        if matches!(tool, EditorTool::Crop | EditorTool::CutOut) && right_down {
            cancel_region_drawing(e);
            return;
        }
        current = clamp_to_canvas(e, current);
    }
    if tool == EditorTool::CutOut {
        move_cut_out(e, current);
        return;
    }
    if tool == EditorTool::Crop {
        if e.input.crop.quick {
            let start = e.input.start_point;
            let mut left = start.x.min(current.x);
            let mut top = start.y.min(current.y);
            let mut w = (current.x - start.x).abs();
            let mut h = (current.y - start.y).abs();
            if m.shift {
                w = w.max(h);
                h = w;
                if current.x < start.x {
                    left = start.x - w;
                }
                if current.y < start.y {
                    top = start.y - h;
                }
            }
            e.input.crop.rect = Some(Rect::from_xywh(left, top, w, h));
            e.invalidate();
        }
        return;
    }
    if e.input.pending.is_none() {
        return;
    }
    if !m.ctrl {
        e.input.moving_during_creation = false;
    } else if e
        .input
        .pending
        .as_ref()
        .is_some_and(can_move_during_creation)
        && (e.input.moving_during_creation || !e.input.ctrl_held_during_draw)
    {
        move_during_creation(e, current);
        e.input.moving_during_creation = true;
        e.input.last_draw_point = current;
        e.input.ctrl_held_during_draw = true;
        return;
    }
    let start = e.input.start_point;
    let mut left = start.x.min(current.x);
    let mut top = start.y.min(current.y);
    let mut width = (current.x - start.x).abs();
    let mut height = (current.y - start.y).abs();
    if m.shift {
        width = width.max(height);
        height = width;
        if current.x < start.x {
            left = start.x - width;
        }
        if current.y < start.y {
            top = start.y - height;
        }
    }
    let Some(mut a) = e.input.pending.take() else {
        return;
    };
    let mut refresh = false;
    match &mut a.kind {
        Kind::Freehand { points, .. } => points.push(current),
        Kind::Rectangle { .. }
        | Kind::Ellipse { .. }
        | Kind::Effect(_)
        | Kind::SmartEraser { .. }
        | Kind::Spotlight(_)
        | Kind::SpeechBalloon(_) => {
            a.start = Point::new(left, top);
            a.end = Point::new(left + width, top + height);
            refresh = a.is_effect() && e.input.is_creating_effect;
            if a.is_smart_eraser() {
                // `UpdateSmartEraserFill`: clamped to the image, then sampled.
                let b = a.bounds();
                let tl = clamp_to_canvas(e, b.top_left());
                let br = clamp_to_canvas(e, b.bottom_right());
                a.start = tl;
                a.end = br;
                if let Some(source) = e.core.source() {
                    model::configure_smart_eraser(&mut a, source);
                }
            }
        }
        Kind::Line { .. } | Kind::Arrow { .. } => {
            a.end = if m.shift {
                snap_45(start, current)
            } else {
                current
            };
        }
        Kind::Cursor { .. } => {
            let w = a.end.x - a.start.x;
            let h = a.end.y - a.start.y;
            a.start = current;
            a.end = Point::new(current.x + w.max(1.0), current.y + h.max(1.0));
        }
        Kind::Number(_) => a.start = current,
        _ => {}
    }
    let id = a.id;
    e.input.pending = Some(a);
    if refresh {
        e.refresh_effect(id);
    }
    e.input.last_draw_point = current;
    e.input.ctrl_held_during_draw = m.ctrl;
    e.invalidate();
}

/// `MoveShapeDuringCreation`.
fn move_during_creation(e: &mut Editor, current: Point) {
    let dx = current.x - e.input.last_draw_point.x;
    let dy = current.y - e.input.last_draw_point.y;
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    e.input.start_point = e.input.start_point.offset(dx, dy);
    let Some(a) = e.input.pending.as_mut() else {
        return;
    };
    a.start = a.start.offset(dx, dy);
    a.end = a.end.offset(dx, dy);
    match &mut a.kind {
        Kind::Freehand { points, .. } => {
            for p in points.iter_mut() {
                *p = p.offset(dx, dy);
            }
        }
        Kind::SpeechBalloon(b) if b.tail_initialized || b.tail_point != Point::default() => {
            b.tail_point = b.tail_point.offset(dx, dy);
            b.tail_initialized = true;
        }
        _ => {}
    }
    let id = a.id;
    if a.is_effect() {
        e.refresh_effect(id);
    }
    e.invalidate();
}

/// The cut-out preview while dragging (direction fixed after 15 pixels).
fn move_cut_out(e: &mut Editor, current: Point) {
    const THRESHOLD: f32 = 15.0;
    const PREVIEW: f32 = 30.0;
    let start = e.input.start_point;
    let (w, h) = e.core.canvas_size();
    let dx = (current.x - start.x).abs();
    let dy = (current.y - start.y).abs();
    let Some(cut) = e.input.cut_out.as_mut() else {
        return;
    };
    if dx < THRESHOLD && dy < THRESHOLD {
        cut.direction = None;
        cut.rect = Some(Rect::from_xywh(
            start.x - PREVIEW / 2.0,
            start.y - PREVIEW / 2.0,
            PREVIEW,
            PREVIEW,
        ));
        e.invalidate();
        return;
    }
    if dx > THRESHOLD || dy > THRESHOLD {
        cut.direction = Some(dx > dy);
    }
    if let Some(vertical) = cut.direction {
        cut.rect = Some(if vertical {
            Rect::from_xywh(start.x.min(current.x), 0.0, dx, h)
        } else {
            Rect::from_xywh(0.0, start.y.min(current.y), w, dy)
        });
    }
    e.invalidate();
}

/// `OnCanvasPointerReleased`.
pub fn pointer_released(e: &mut Editor, p: Point, button: Button, _m: Modifiers) {
    if button == Button::Middle || !e.has_image() {
        return;
    }
    if selection_released(e) {
        return;
    }
    if e.input.crop.dragging.take().is_some() {
        e.invalidate();
        return;
    }
    if !e.input.drawing {
        return;
    }
    e.input.drawing = false;
    e.input.moving_during_creation = false;
    e.input.ctrl_held_during_draw = false;
    let tool = e.tools.active_tool;
    if tool == EditorTool::Crop {
        let rect = e.input.crop.rect;
        match rect {
            Some(r) if r.width() >= MIN_SHAPE_SIZE && r.height() >= MIN_SHAPE_SIZE => {
                if e.options.quick_crop && e.input.crop.quick {
                    perform_crop(e);
                } else {
                    e.input.crop.active = true;
                    e.input.crop.quick = false;
                }
            }
            Some(_) => {
                cancel_crop(e);
            }
            None => {}
        }
        e.invalidate();
        return;
    }
    if tool == EditorTool::CutOut {
        perform_cut_out(e);
        return;
    }
    let Some(mut a) = e.input.pending.take() else {
        e.input.is_creating_effect = false;
        return;
    };
    if a.is_smart_eraser() {
        let b = a.bounds();
        a.start = clamp_to_canvas(e, b.top_left());
        a.end = clamp_to_canvas(e, b.bottom_right());
        if let Some(source) = e.core.source() {
            model::configure_smart_eraser(&mut a, source);
        }
    }
    let size_based = !matches!(
        tool,
        EditorTool::Step | EditorTool::Freehand | EditorTool::Text | EditorTool::Cursor
    );
    if size_based {
        let (w, h) = if a.is_smart_eraser() {
            let b = a.bounds();
            (b.width(), b.height())
        } else {
            (
                (p.x - e.input.start_point.x).abs(),
                (p.y - e.input.start_point.y).abs(),
            )
        };
        if w < MIN_SHAPE_SIZE && h < MIN_SHAPE_SIZE {
            e.input.is_creating_effect = false;
            e.invalidate();
            return;
        }
    }
    let id = a.id;
    let freehand = matches!(a.kind, Kind::Freehand { .. });
    let index = e.input.pending_index.take();
    e.input.is_creating_effect = false;
    // The shape is selected first, then added to the core (and its history).
    e.input.pending = Some(a);
    if !freehand {
        if e.input.pending.as_ref().is_some_and(|a| a.is_effect()) {
            e.refresh_effect(id);
        }
        let was_none = e.input.sel.selected.is_none();
        e.input.sel.selected = Some(id);
        e.input.sel.hovered = Some(id);
        if was_none {
            selection_changed(e, true);
        }
    }
    let a = e.input.pending.take().unwrap();
    let is_balloon = matches!(a.kind, Kind::SpeechBalloon(_));
    match index {
        Some(i) => e.core.insert_annotation(i, a),
        None => e.core.add_annotation(a),
    }
    if is_balloon && !freehand {
        show_balloon_text_editor(e, id);
    }
    e.pump();
}

fn perform_cut_out(e: &mut Editor) {
    let Some(cut) = e.input.cut_out.take() else {
        return;
    };
    let (Some(rect), Some(vertical)) = (cut.rect, cut.direction) else {
        e.invalidate();
        return;
    };
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let ok = if vertical {
        e.core.cut_out(
            rect.left.round() as i32,
            (rect.left + rect.width()).round() as i32,
            true,
        )
    } else {
        e.core.cut_out(
            rect.top.round() as i32,
            (rect.top + rect.height()).round() as i32,
            false,
        )
    };
    e.pump();
    if ok {
        e.image_cut_out_notification();
    }
}

/// `OnKeyUp` of the input controller: releasing Cmd stops moving the shape being drawn.
pub fn ctrl_released(e: &mut Editor) {
    if e.input.drawing {
        e.input.moving_during_creation = false;
        e.input.ctrl_held_during_draw = false;
    }
}

// ---------------------------------------------------------------------------- keys

/// `EditorView.OnKeyDown`: true when the key was used.
pub fn key_down(e: &mut Editor, key: egui::Key, m: egui::Modifiers) -> bool {
    use egui::Key;
    if e.text_editing.is_some() || e.modal.is_some() {
        return false;
    }
    let mods = Modifiers::from_egui(m);
    if mods.ctrl && !e.input.drawing {
        refresh_hover(e, mods);
    }
    let ctrl = m.command;
    if key == Key::Delete || (key == Key::Backspace && !ctrl && !m.shift) {
        if m.shift {
            if e.has_annotations() {
                e.clear_annotations();
                return true;
            }
            return false;
        }
        e.delete_selected();
        return true;
    }
    if key == Key::Backspace && m.shift && e.has_annotations() {
        e.clear_annotations();
        return true;
    }
    if ctrl && m.shift {
        if e.try_toolbar_hotkey(key, m) {
            return true;
        }
        match key {
            Key::Z => e.redo(),
            Key::C => {
                if !e.has_selection() {
                    return false;
                }
                e.copy_annotation();
            }
            Key::F => e.flatten(),
            Key::P => {
                if !e.has_image() {
                    return false;
                }
                e.request(Request::Print);
            }
            Key::S => {
                if !e.has_image() {
                    return false;
                }
                e.request(Request::SaveAs);
            }
            _ => return false,
        }
        return true;
    }
    if ctrl && !m.shift {
        if e.try_toolbar_hotkey(key, m) {
            return true;
        }
        match key {
            Key::Z => e.undo(),
            Key::Y => e.redo(),
            Key::H => e.show_toolbars = !e.show_toolbars,
            Key::X => e.cut_annotation(),
            Key::C => {
                if !e.has_image() {
                    return false;
                }
                e.request(Request::CopyImage);
            }
            Key::V => e.request(Request::Paste),
            Key::D => e.duplicate_selected(),
            Key::S => {
                if !e.has_image() {
                    return false;
                }
                e.request(Request::Save);
            }
            Key::N => e.modal_new_image(),
            Key::O => e.request(Request::OpenImage),
            Key::P => {
                if !e.has_image() {
                    return false;
                }
                e.request(Request::PinToScreen);
            }
            Key::U => {
                if !e.has_image() {
                    return false;
                }
                e.request(Request::Upload);
            }
            _ => return false,
        }
        return true;
    }
    if !m.alt && !ctrl {
        let step = if m.shift { 10.0 } else { 1.0 };
        let arrow = match key {
            Key::ArrowLeft => Some((-step, 0.0)),
            Key::ArrowRight => Some((step, 0.0)),
            Key::ArrowUp => Some((0.0, -step)),
            Key::ArrowDown => Some((0.0, step)),
            _ => None,
        };
        if let Some((dx, dy)) = arrow
            && e.has_selection()
        {
            move_selected(e, dx, dy);
            return true;
        }
        if m.shift {
            return e.try_toolbar_hotkey(key, m);
        }
        if e.try_toolbar_hotkey(key, m) {
            return true;
        }
        match key {
            Key::Home => e.bring_to_front(),
            Key::End => e.send_to_back(),
            Key::PageUp => e.bring_forward(),
            Key::PageDown => e.send_backward(),
            Key::Enter => {
                if confirm_crop(e) {
                    return true;
                }
                if e.config.show_task_buttons {
                    e.continue_task();
                    return true;
                }
                return false;
            }
            _ => return false,
        }
        return true;
    }
    false
}

/// `EditorView.OnKeyUp`: Escape closes a dialog, the effects panel, a crop, the
/// selection, or the editor, in that order.
pub fn key_up(e: &mut Editor, key: egui::Key, m: egui::Modifiers) -> bool {
    if key == egui::Key::Escape && !m.command && !m.shift && !m.alt {
        if e.modal.is_some() {
            e.close_modal();
            return true;
        }
        if e.is_effects_panel_open {
            e.close_effects_panel();
            return true;
        }
        if e.text_editing.is_some() {
            commit_text_edit(e);
            return true;
        }
        if cancel_crop(e) {
            return true;
        }
        if e.has_selection() {
            clear_selection(e);
            return true;
        }
        // `CancelCommand` in the task workflow, `ExitEditorCommand` otherwise: both close.
        e.request_close(false);
        return true;
    }
    false
}

impl Editor {
    /// `CloseModalCommand` (and the insert dialog's implicit cancel).
    pub fn close_modal(&mut self) {
        if matches!(self.modal, Some(Modal::InsertImage { .. })) {
            self.input.pending_insert = None;
        }
        self.modal = None;
    }

    /// `NewImageCommand`.
    pub fn modal_new_image(&mut self) {
        self.modal = Some(Modal::NewImage(super::NewImageState::default()));
    }

    /// `RequestStartScreen`.
    pub fn show_start_screen(&mut self) {
        if matches!(self.modal, Some(Modal::StartScreen(_))) {
            return;
        }
        self.modal = Some(Modal::StartScreen(super::StartScreenState {
            recent_files: self.options.recent_image_files.clone(),
            ..super::StartScreenState::default()
        }));
    }

    /// Shows a status line on the start screen (opening it when needed).
    pub fn start_screen_status(&mut self, message: &str) {
        self.show_start_screen();
        if let Some(Modal::StartScreen(s)) = &mut self.modal {
            s.status = Some(message.to_string());
            s.url_loading = false;
        }
    }

    /// The point where the pending text box sits, for the view.
    pub fn text_box_anchor(&self) -> Option<Point> {
        let edit = self.text_editing.as_ref()?;
        self.annotation(edit.id).map(|a| a.bounds().top_left())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::EditorConfig;
    use crate::options::ImageEditorOptions;

    fn editor() -> Editor {
        crate::text::set_system_fonts_enabled(false);
        let mut e = Editor::new(ImageEditorOptions::default(), EditorConfig::default());
        e.load_image(
            RgbaImage::from_pixel(400, 300, image::Rgba([255, 255, 255, 255])),
            None,
        );
        e
    }

    fn drag(e: &mut Editor, from: Point, to: Point, m: Modifiers) {
        pointer_pressed(e, from, Button::Left, m, 1);
        let steps = 5;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            pointer_moved(
                e,
                Point::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t),
                false,
                m,
            );
        }
        pointer_released(e, to, Button::Left, m);
    }

    #[test]
    fn drawing_a_rectangle_adds_selects_and_records_it() {
        let mut e = editor();
        e.select_tool(EditorTool::Rectangle);
        drag(
            &mut e,
            Point::new(10.0, 10.0),
            Point::new(110.0, 60.0),
            Modifiers::NONE,
        );
        assert_eq!(e.core.annotations().len(), 1);
        let a = &e.core.annotations()[0];
        assert_eq!(a.bounds(), Rect::new(10.0, 10.0, 110.0, 60.0));
        assert_eq!(e.selected(), Some(a.id));
        assert!(e.core.can_undo());
        assert!(e.is_dirty);
        e.undo();
        assert!(e.core.annotations().is_empty());
    }

    #[test]
    fn tiny_drags_are_discarded() {
        let mut e = editor();
        e.select_tool(EditorTool::Ellipse);
        drag(
            &mut e,
            Point::new(10.0, 10.0),
            Point::new(12.0, 13.0),
            Modifiers::NONE,
        );
        assert!(e.core.annotations().is_empty());
    }

    #[test]
    fn shift_makes_squares_and_snaps_lines() {
        let mut e = editor();
        e.select_tool(EditorTool::Rectangle);
        let shift = Modifiers {
            shift: true,
            ..Modifiers::NONE
        };
        drag(
            &mut e,
            Point::new(10.0, 10.0),
            Point::new(110.0, 40.0),
            shift,
        );
        assert_eq!(
            e.core.annotations()[0].bounds(),
            Rect::new(10.0, 10.0, 110.0, 110.0)
        );
        e.select_tool(EditorTool::Line);
        drag(
            &mut e,
            Point::new(200.0, 200.0),
            Point::new(300.0, 210.0),
            shift,
        );
        let line = e.core.annotations().last().unwrap();
        assert!((line.end.y - 200.0).abs() < 0.01);
    }

    #[test]
    fn select_drag_and_resize_with_handles() {
        let mut e = editor();
        e.select_tool(EditorTool::Rectangle);
        drag(
            &mut e,
            Point::new(10.0, 10.0),
            Point::new(110.0, 60.0),
            Modifiers::NONE,
        );
        e.select_tool(EditorTool::Select);
        drag(
            &mut e,
            Point::new(50.0, 30.0),
            Point::new(70.0, 40.0),
            Modifiers::NONE,
        );
        let id = e.core.annotations()[0].id;
        assert_eq!(
            e.core.annotations()[0].bounds(),
            Rect::new(30.0, 20.0, 130.0, 70.0)
        );
        assert_eq!(e.selected(), Some(id));
        // The bottom-right handle stretches it.
        drag(
            &mut e,
            Point::new(130.0, 70.0),
            Point::new(150.0, 100.0),
            Modifiers::NONE,
        );
        assert_eq!(
            e.core.annotations()[0].bounds(),
            Rect::new(30.0, 20.0, 150.0, 100.0)
        );
        // A rotation handle 30 points above the top centre.
        let handles = selection_handles(&e);
        let rotate = handles
            .iter()
            .find(|(t, _)| *t == HandleTag::Rotate)
            .unwrap()
            .1;
        assert_eq!(rotate, Point::new(90.0, -10.0));
    }

    #[test]
    fn quick_crop_crops_on_release() {
        let mut e = editor();
        e.select_tool(EditorTool::Crop);
        drag(
            &mut e,
            Point::new(50.0, 40.0),
            Point::new(250.0, 140.0),
            Modifiers::NONE,
        );
        assert_eq!(e.image_size(), (200, 100));
        assert!(e.notification.is_some());
        e.undo();
        assert_eq!(e.image_size(), (400, 300));
    }

    #[test]
    fn cut_out_removes_a_strip() {
        let mut e = editor();
        e.select_tool(EditorTool::CutOut);
        drag(
            &mut e,
            Point::new(100.0, 50.0),
            Point::new(160.0, 55.0),
            Modifiers::NONE,
        );
        assert_eq!(e.image_size(), (340, 300));
    }

    #[test]
    fn steps_count_up_and_renumber_after_delete() {
        let mut e = editor();
        e.select_tool(EditorTool::Step);
        for x in [50.0, 120.0, 190.0] {
            pointer_pressed(
                &mut e,
                Point::new(x, 50.0),
                Button::Left,
                Modifiers::NONE,
                1,
            );
            pointer_released(&mut e, Point::new(x, 50.0), Button::Left, Modifiers::NONE);
        }
        let numbers: Vec<i32> = e
            .core
            .annotations()
            .iter()
            .map(|a| match &a.kind {
                Kind::Number(n) => n.number,
                _ => 0,
            })
            .collect();
        assert_eq!(numbers, vec![1, 2, 3]);
        assert_eq!(e.tools.number_counter, 4);
        let first = e.core.annotations()[0].id;
        set_selected_shape(&mut e, first);
        e.delete_selected();
        assert_eq!(e.tools.number_counter, 3);
    }

    #[test]
    fn text_is_created_when_its_box_closes() {
        let mut e = editor();
        e.select_tool(EditorTool::Text);
        pointer_pressed(
            &mut e,
            Point::new(30.0, 30.0),
            Button::Left,
            Modifiers::NONE,
            1,
        );
        assert!(e.text_editing.is_some());
        assert!(e.core.annotations().is_empty());
        e.text_editing.as_mut().unwrap().text = "Hello".into();
        commit_text_edit(&mut e);
        assert_eq!(e.core.annotations().len(), 1);
        let a = &e.core.annotations()[0];
        assert!(a.bounds().width() > 100.0);
        assert_eq!(e.selected(), Some(a.id));
    }

    /// A duplicate or a pasted copy is the same shape 20 pixels away: a bent line's
    /// curve point and a step's or balloon's tail move with it.
    #[test]
    fn duplicates_and_pastes_keep_curves_and_tails() {
        let mut e = editor();
        let mut line = Annotation::line();
        line.start = Point::new(10.0, 100.0);
        line.end = Point::new(200.0, 100.0);
        model::set_curve_point(&mut line, Point::new(105.0, 150.0));
        let id = line.id;
        e.core.add_annotation(line);
        set_selected_shape(&mut e, id);
        e.duplicate_selected();
        let copy = e.core.annotations().last().unwrap();
        assert_ne!(copy.id, id);
        assert_eq!(copy.curve().unwrap().point, Point::new(125.0, 170.0));

        let mut step = Annotation::number();
        step.start = Point::new(50.0, 50.0);
        set_number_tail(&mut step, Point::new(90.0, 120.0));
        let id = step.id;
        e.core.add_annotation(step);
        set_selected_shape(&mut e, id);
        e.copy_annotation();
        e.paste_annotation();
        let pasted = e.core.annotations().last().unwrap();
        assert_eq!(pasted.start, Point::new(70.0, 70.0));
        assert_eq!(model::number_tail_handle(pasted), Point::new(110.0, 140.0));

        let mut balloon = Annotation::speech_balloon();
        balloon.start = Point::new(100.0, 100.0);
        balloon.end = Point::new(200.0, 150.0);
        set_balloon_tail(&mut balloon, Point::new(60.0, 220.0));
        let id = balloon.id;
        e.core.add_annotation(balloon);
        set_selected_shape(&mut e, id);
        commit_text_edit(&mut e);
        e.duplicate_selected();
        let copy = e.core.annotations().last().unwrap();
        assert_eq!(balloon_effective_tail_point(copy), Point::new(80.0, 240.0));
    }

    /// Home, End, Page Up and Page Down move the selected annotation only. After an
    /// undo the core still remembered the annotation selected when its memento was
    /// made, while nothing showed as selected; the keys reordered that one.
    #[test]
    fn z_order_keys_need_a_selection() {
        let mut e = editor();
        e.select_tool(EditorTool::Rectangle);
        drag(
            &mut e,
            Point::new(10.0, 10.0),
            Point::new(60.0, 60.0),
            Modifiers::NONE,
        );
        drag(
            &mut e,
            Point::new(100.0, 10.0),
            Point::new(160.0, 60.0),
            Modifiers::NONE,
        );
        let order: Vec<AnnotationId> = e.core.annotations().iter().map(|a| a.id).collect();
        set_selected_shape(&mut e, order[0]);
        e.delete_selected();
        e.undo();
        assert_eq!(e.selected(), None);
        for key in [
            egui::Key::Home,
            egui::Key::End,
            egui::Key::PageUp,
            egui::Key::PageDown,
        ] {
            key_down(&mut e, key, egui::Modifiers::NONE);
            let now: Vec<AnnotationId> = e.core.annotations().iter().map(|a| a.id).collect();
            assert_eq!(now, order, "{key:?} reordered an unselected annotation");
        }
        assert!(e.core.can_redo(), "the undo was replaced by a reorder");
    }

    #[test]
    fn auto_copy_asks_for_one_plain_copy_per_change() {
        let mut e = editor();
        e.options.auto_copy_image_to_clipboard = true;
        e.load_image(
            RgbaImage::from_pixel(50, 40, image::Rgba([9, 9, 9, 255])),
            None,
        );
        assert_eq!(e.take_requests(), vec![Request::AutoCopyImage]);
        e.select_tool(EditorTool::Rectangle);
        drag(
            &mut e,
            Point::new(5.0, 5.0),
            Point::new(30.0, 30.0),
            Modifiers::NONE,
        );
        assert_eq!(e.take_requests(), vec![Request::AutoCopyImage]);
    }

    #[test]
    fn keys_undo_redo_and_delete() {
        let mut e = editor();
        e.select_tool(EditorTool::Rectangle);
        drag(
            &mut e,
            Point::new(10.0, 10.0),
            Point::new(110.0, 60.0),
            Modifiers::NONE,
        );
        let cmd = egui::Modifiers::COMMAND;
        assert!(key_down(&mut e, egui::Key::Z, cmd));
        assert!(e.core.annotations().is_empty());
        assert!(key_down(&mut e, egui::Key::Z, cmd | egui::Modifiers::SHIFT));
        assert_eq!(e.core.annotations().len(), 1);
        let id = e.core.annotations()[0].id;
        set_selected_shape(&mut e, id);
        assert!(key_down(&mut e, egui::Key::Delete, egui::Modifiers::NONE));
        assert!(e.core.annotations().is_empty());
        // Tool hotkeys.
        assert!(key_down(&mut e, egui::Key::E, egui::Modifiers::NONE));
        assert_eq!(e.tools.active_tool, EditorTool::Ellipse);
    }
}
