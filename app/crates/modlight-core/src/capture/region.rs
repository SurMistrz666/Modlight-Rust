//! The region capture's behaviour (`RegionCaptureWindow`, `RegionCaptureLightForm`)
//! without its window: pointer, wheel and keys in, a selection and an outcome out.
//!
//! Everything is in IMAGE pixels of the frozen screenshot, as upstream's overlay is; the
//! overlay converts its own coordinates once. Window rectangles and the result's screen
//! rectangle are in desktop points.

use std::sync::Arc;

use image::RgbaImage;

use super::geometry::{IRect, Point, Rect, ResizeHandle};
use super::options::{RegionCaptureAction, RegionCaptureOptions};
use super::source::{ScreenInfo, WindowInfo};

/// `RegionCaptureType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RegionKind {
    #[default]
    Default,
    /// `RegionCaptureLightForm` over a frozen picture.
    Light,
    /// `RegionCaptureLightForm` over the live desktop; the area is taken afterwards.
    Transparent,
}

/// What a region capture starts from (`AvaloniaRegionCaptureRequest`).
#[derive(Clone)]
pub struct RegionRequest {
    pub kind: RegionKind,
    /// The desktop area under the overlay, in points.
    pub screen_bounds: IRect,
    /// Image pixels per point.
    pub scale: f64,
    /// The frozen desktop (absent for a transparent capture).
    pub image: Option<Arc<RgbaImage>>,
    pub options: RegionCaptureOptions,
    pub enable_annotations: bool,
    pub windows: Vec<WindowInfo>,
    pub screens: Vec<ScreenInfo>,
    /// A capture that shows the pointer (`ShowCursor`): it is in the frozen picture, and
    /// the transparent kind takes the area with it.
    pub show_cursor: bool,
    /// `RegionCaptureIntegration.LastRegionRectangle`, desktop points.
    pub last_region: IRect,
    /// Where the pointer is when the overlay opens, desktop points.
    pub pointer: (i32, i32),
}

/// `RegionInteraction`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interaction {
    #[default]
    None,
    PendingHover,
    Creating,
    Moving,
    Resizing,
    Fixed,
}

/// The mouse buttons the overlay hears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
    X1,
    X2,
}

/// The keys the overlay hears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Enter,
    Space,
    /// The key left of 1 (`OemTilde`; ` on a US Mac keyboard, § on others).
    Tilde,
    Left,
    Right,
    Up,
    Down,
    Delete,
    Escape,
    /// Ctrl+H (Cmd+H on a Mac): hide or show the toolbar.
    ToggleToolbar,
}

/// How a capture ended.
#[derive(Clone, Debug, PartialEq)]
pub enum RegionOutcome {
    Cancelled,
    /// The chosen area: image pixels, desktop points, and the window it showed.
    Completed {
        image_rect: IRect,
        screen_rect: IRect,
        window: Option<WindowInfo>,
    },
}

/// The overlay's state (`RegionCaptureWindow`'s fields).
#[derive(Clone)]
pub struct RegionSession {
    pub request: RegionRequest,
    pub image_size: (f64, f64),
    pub selection: Rect,
    pub hover: Rect,
    hover_candidate: Option<usize>,
    selected_candidate: Option<usize>,
    pub interaction: Interaction,
    resize_handle: ResizeHandle,
    press_point: Point,
    pub last_point: Point,
    interaction_start: Rect,
    pub region_tool_active: bool,
    pub keyboard_enabled: bool,
    suppress_next_right_release: bool,
    pub show_handles: bool,
    pub toolbar_hidden: bool,
    pub show_magnifier: bool,
    pub magnifier_pixel_count: i64,
    /// Display pixels per overlay point (the window's `RenderScaling`).
    pub render_scale: f64,
    pub outcome: Option<RegionOutcome>,
    /// Light and transparent captures: the left button is down.
    light_mouse_down: bool,
}

impl RegionSession {
    pub fn new(request: RegionRequest) -> RegionSession {
        let scale = request.scale.max(0.1);
        let image_size = (
            (request.screen_bounds.width as f64 * scale)
                .round()
                .max(1.0),
            (request.screen_bounds.height as f64 * scale)
                .round()
                .max(1.0),
        );
        let pointer = Point::new(
            (request.pointer.0 - request.screen_bounds.x) as f64 * scale,
            (request.pointer.1 - request.screen_bounds.y) as f64 * scale,
        );
        let mut session = RegionSession {
            show_magnifier: request.options.show_magnifier,
            magnifier_pixel_count: request.options.magnifier_pixel_count,
            request,
            image_size,
            selection: Rect::default(),
            hover: Rect::default(),
            hover_candidate: None,
            selected_candidate: None,
            interaction: Interaction::None,
            resize_handle: ResizeHandle::None,
            press_point: Point::default(),
            last_point: Point::default(),
            interaction_start: Rect::default(),
            region_tool_active: true,
            keyboard_enabled: false,
            suppress_next_right_release: false,
            show_handles: false,
            toolbar_hidden: false,
            render_scale: scale,
            outcome: None,
            light_mouse_down: false,
        };
        session.last_point = session.clamp(pointer);
        session.magnifier_pixel_count =
            session.normalize_magnifier_pixel_count(session.magnifier_pixel_count);
        session.update_hover(session.last_point);
        session
    }

    pub fn is_light(&self) -> bool {
        self.request.kind != RegionKind::Default
    }

    pub fn is_closed(&self) -> bool {
        self.outcome.is_some()
    }

    fn min_size(&self) -> f64 {
        self.request.options.minimum_size as f64
    }

    /// `HasValidSelection`.
    pub fn has_valid_selection(&self) -> bool {
        self.selection.width >= self.min_size() && self.selection.height >= self.min_size()
    }

    /// `ActiveRectangle`: the selection, or the hovered window.
    pub fn active_rect(&self) -> Rect {
        if self.selection.is_valid() {
            self.selection
        } else {
            self.hover
        }
    }

    fn clamp(&self, point: Point) -> Point {
        Point::new(
            point.x.clamp(0.0, self.image_size.0),
            point.y.clamp(0.0, self.image_size.1),
        )
    }

    fn image_bounds(&self) -> Rect {
        Rect::new(0.0, 0.0, self.image_size.0, self.image_size.1)
    }

    /// A desktop rectangle (points) in image pixels.
    pub fn to_image(&self, rect: IRect) -> Rect {
        let s = self.request.scale;
        Rect::new(
            (rect.x - self.request.screen_bounds.x) as f64 * s,
            (rect.y - self.request.screen_bounds.y) as f64 * s,
            rect.width as f64 * s,
            rect.height as f64 * s,
        )
    }

    /// An image point in desktop points.
    pub fn to_screen(&self, point: Point) -> (i32, i32) {
        let s = self.request.scale;
        (
            self.request.screen_bounds.x + (point.x / s).floor() as i32,
            self.request.screen_bounds.y + (point.y / s).floor() as i32,
        )
    }

    fn set_selection(&mut self, rect: Rect, candidate: Option<usize>) {
        self.selection = rect.intersect(&self.image_bounds());
        self.selected_candidate = candidate;
    }

    /// `ClearSelection`.
    pub fn clear_selection(&mut self) {
        self.interaction = Interaction::None;
        self.selected_candidate = None;
        self.selection = Rect::default();
        self.show_handles = false;
        self.update_hover(self.last_point);
    }

    /// `UpdateHover`: the window under the pointer, while nothing is selected.
    fn update_hover(&mut self, point: Point) {
        if self.is_light()
            || !self.request.options.detect_windows
            || self.has_valid_selection()
            || self.interaction != Interaction::None
        {
            self.hover = Rect::default();
            self.hover_candidate = None;
            return;
        }
        let (sx, sy) = self.to_screen(Point::new(point.x.round(), point.y.round()));
        let found = self
            .request
            .windows
            .iter()
            .position(|w| w.rect.has_area() && w.rect.contains(sx, sy));
        let Some(index) = found else {
            self.hover_candidate = None;
            self.hover = Rect::default();
            return;
        };
        let hover = self
            .to_image(self.request.windows[index].rect)
            .intersect(&self.image_bounds());
        self.hover_candidate = hover.is_valid().then_some(index);
        self.hover = hover;
    }

    // ----------------------------------------------------------------- pointer

    /// `OnRegionPointerPressed`.
    pub fn pointer_pressed(&mut self, button: Button, point: Point, click_count: u32) {
        if self.is_closed() || !self.region_tool_active {
            return;
        }
        let point = self.clamp(point);
        self.last_point = point;
        if self.is_light() {
            match button {
                Button::Left => {
                    self.press_point = point;
                    self.light_mouse_down = true;
                    self.selection = Rect::default();
                }
                Button::Right => {}
                _ => {}
            }
            return;
        }
        if self.try_cancel_creation(button, true) {
            return;
        }
        match button {
            Button::Right => return,
            Button::Middle => {
                self.run_action(self.request.options.region_capture_action_middle_click);
                return;
            }
            Button::X1 => {
                self.run_action(self.request.options.region_capture_action_x1_click);
                return;
            }
            Button::X2 => {
                self.run_action(self.request.options.region_capture_action_x2_click);
                return;
            }
            Button::Left => {}
        }

        if click_count >= 2 && self.has_valid_selection() && self.selection.contains(point) {
            self.complete(self.selection, true);
            return;
        }

        self.press_point = point;
        self.interaction_start = self.selection;
        self.resize_handle = ResizeHandle::hit_test(&self.selection, point);

        if self.resize_handle != ResizeHandle::None {
            self.interaction = Interaction::Resizing;
        } else if self.has_valid_selection() && self.selection.contains(point) {
            self.interaction = Interaction::Moving;
        } else if self.request.options.is_fixed_size {
            let size = self.request.options.fixed_size;
            let first = Point::new(
                point.x - size.width as f64 / 2.0,
                point.y - size.height as f64 / 2.0,
            );
            let second = Point::new(first.x + size.width as f64, first.y + size.height as f64);
            let rect = Rect::normalize_and_clamp(first, second, self.image_size);
            self.set_selection(rect, None);
            self.interaction = Interaction::Fixed;
        } else if self.hover.is_valid() {
            self.set_selection(self.hover, self.hover_candidate);
            self.interaction_start = self.selection;
            self.interaction = Interaction::PendingHover;
        } else {
            self.set_selection(Rect::default(), None);
            self.interaction = Interaction::Creating;
        }
        self.show_handles = false;
    }

    /// `OnRegionPointerMoved`.
    pub fn pointer_moved(&mut self, point: Point) {
        if self.is_closed() || !self.region_tool_active {
            return;
        }
        let point = self.clamp(point);
        self.last_point = point;
        if self.is_light() {
            if self.light_mouse_down {
                self.selection =
                    Rect::normalize_and_clamp(self.press_point, point, self.image_size);
            }
            return;
        }
        match self.interaction {
            Interaction::None => self.update_hover(point),
            Interaction::PendingHover => {
                if self.press_point.distance(point) >= 4.0 {
                    self.interaction = Interaction::Creating;
                    self.selected_candidate = None;
                    let rect = Rect::normalize_and_clamp(self.press_point, point, self.image_size);
                    self.set_selection(rect, None);
                }
            }
            Interaction::Creating => {
                let rect = Rect::normalize_and_clamp(self.press_point, point, self.image_size);
                self.set_selection(rect, None);
            }
            Interaction::Moving => {
                let start = self.interaction_start;
                self.move_selection_from(
                    point.x - self.press_point.x,
                    point.y - self.press_point.y,
                    start,
                );
            }
            Interaction::Resizing => self.resize_selection(point),
            Interaction::Fixed => {}
        }
    }

    /// `OnRegionPointerReleased`.
    pub fn pointer_released(&mut self, button: Button, point: Point) {
        if self.is_closed() || !self.region_tool_active {
            return;
        }
        let point = self.clamp(point);
        self.last_point = point;
        if self.is_light() {
            self.light_released(button, point);
            return;
        }
        if self.try_cancel_creation(button, false) {
            return;
        }
        if button == Button::Right {
            if self.suppress_next_right_release {
                self.suppress_next_right_release = false;
            } else {
                self.run_action(self.request.options.region_capture_action_right_click);
            }
            return;
        }
        if self.interaction == Interaction::None {
            if button == Button::Left {
                self.suppress_next_right_release = false;
            }
            return;
        }
        if button != Button::Left {
            return;
        }
        self.interaction = Interaction::None;
        self.resize_handle = ResizeHandle::None;
        let min = self.min_size();
        if self.selection.width < min || self.selection.height < min {
            if self.hover.is_valid() {
                self.set_selection(self.hover, self.hover_candidate);
            } else {
                self.clear_selection();
            }
        }
        if self.has_valid_selection() && self.request.options.quick_crop {
            self.complete(self.selection, true);
        } else {
            self.show_handles = self.has_valid_selection();
        }
    }

    /// `RegionCaptureLightForm_MouseUp`.
    fn light_released(&mut self, button: Button, point: Point) {
        const MINIMUM: f64 = 5.0;
        match button {
            Button::Left => {
                let rect = Rect::normalize_and_clamp(self.press_point, point, self.image_size);
                if self.light_mouse_down && rect.width > MINIMUM && rect.height > MINIMUM {
                    self.selection = rect;
                    self.complete(rect, false);
                } else {
                    self.light_mouse_down = false;
                }
            }
            Button::Right => {
                if self.light_mouse_down {
                    self.light_mouse_down = false;
                    self.selection = Rect::default();
                } else {
                    self.cancel();
                }
            }
            _ => {}
        }
    }

    /// Whether the light capture is dragging (the rectangle shows only then).
    pub fn light_dragging(&self) -> bool {
        self.light_mouse_down
    }

    /// `TryCancelRegionCreation`: a right button during creation drops the selection.
    fn try_cancel_creation(&mut self, button: Button, pressed: bool) -> bool {
        if button != Button::Right
            || !matches!(
                self.interaction,
                Interaction::Creating | Interaction::PendingHover | Interaction::Fixed
            )
        {
            return false;
        }
        self.suppress_next_right_release = pressed;
        self.clear_selection();
        true
    }

    /// `OnRegionPointerWheelChanged`: the magnifier's zoom.
    pub fn wheel(&mut self, delta_y: f64) {
        if self.is_light() || !self.region_tool_active || delta_y == 0.0 {
            return;
        }
        let delta = if delta_y > 0.0 { -2 } else { 2 };
        self.magnifier_pixel_count =
            self.normalize_magnifier_pixel_count(self.magnifier_pixel_count + delta);
        self.show_magnifier = true;
    }

    /// `NormalizeMagnifierPixelCount`: odd, and at least 6 display pixels per cell.
    pub fn normalize_magnifier_pixel_count(&self, requested: i64) -> i64 {
        const MAGNIFIER_SIZE: f64 = 150.0;
        const MINIMUM_PIXEL_SIZE: f64 = 6.0;
        let scale = if self.render_scale.is_finite() && self.render_scale > 0.0 {
            self.render_scale
        } else {
            1.0
        };
        let size_limited = (MAGNIFIER_SIZE * scale / MINIMUM_PIXEL_SIZE).floor() as i64;
        let mut maximum = size_limited.clamp(
            RegionCaptureOptions::MAGNIFIER_PIXEL_COUNT_MINIMUM,
            RegionCaptureOptions::MAGNIFIER_PIXEL_COUNT_MAXIMUM,
        );
        if maximum % 2 == 0 {
            maximum -= 1;
        }
        let mut count =
            requested.clamp(RegionCaptureOptions::MAGNIFIER_PIXEL_COUNT_MINIMUM, maximum);
        if count % 2 == 0 {
            count = if count < maximum {
                count + 1
            } else {
                count - 1
            };
        }
        count
    }

    // ----------------------------------------------------------------- keys

    /// `OnKeyDown` / `OnKeyUp` (Escape is a key-up upstream; here every key arrives once).
    pub fn key(&mut self, key: Key, shift: bool) {
        if self.is_closed() {
            return;
        }
        if key == Key::Escape {
            self.cancel();
            return;
        }
        if self.is_light() {
            return;
        }
        if key == Key::ToggleToolbar {
            if self.keyboard_enabled {
                self.toolbar_hidden = !self.toolbar_hidden;
            }
            return;
        }
        if !self.keyboard_enabled || !self.region_tool_active {
            return;
        }
        match key {
            Key::Enter if self.has_valid_selection() => self.complete(self.selection, true),
            Key::Space => {
                let all = self.image_bounds();
                self.complete(all, false);
            }
            Key::Tilde => self.complete_active_monitor(),
            Key::Left | Key::Right | Key::Up | Key::Down if self.has_valid_selection() => {
                let distance = if shift {
                    RegionCaptureOptions::MOVE_SPEED_MAXIMUM
                } else {
                    RegionCaptureOptions::MOVE_SPEED_MINIMUM
                };
                let dx = match key {
                    Key::Left => -distance,
                    Key::Right => distance,
                    _ => 0.0,
                };
                let dy = match key {
                    Key::Up => -distance,
                    Key::Down => distance,
                    _ => 0.0,
                };
                let start = self.selection;
                self.move_selection_from(dx, dy, start);
            }
            Key::Delete if self.has_valid_selection() => self.clear_selection(),
            _ => {}
        }
    }

    fn move_selection_from(&mut self, dx: f64, dy: f64, source: Rect) {
        let x = (source.x + dx).clamp(0.0, (self.image_size.0 - source.width).max(0.0));
        let y = (source.y + dy).clamp(0.0, (self.image_size.1 - source.height).max(0.0));
        self.set_selection(Rect::new(x, y, source.width, source.height), None);
    }

    fn resize_selection(&mut self, point: Point) {
        let r = self.interaction_start;
        let (mut left, mut top, mut right, mut bottom) = (r.left(), r.top(), r.right(), r.bottom());
        match self.resize_handle {
            ResizeHandle::TopLeft => {
                left = point.x;
                top = point.y;
            }
            ResizeHandle::Top => top = point.y,
            ResizeHandle::TopRight => {
                right = point.x;
                top = point.y;
            }
            ResizeHandle::Right => right = point.x,
            ResizeHandle::BottomRight => {
                right = point.x;
                bottom = point.y;
            }
            ResizeHandle::Bottom => bottom = point.y,
            ResizeHandle::BottomLeft => {
                left = point.x;
                bottom = point.y;
            }
            ResizeHandle::Left => left = point.x,
            ResizeHandle::None => {}
        }
        let rect = Rect::normalize_and_clamp(
            Point::new(left, top),
            Point::new(right, bottom),
            self.image_size,
        );
        self.set_selection(rect, None);
    }

    // ----------------------------------------------------------------- actions

    /// `RunCaptureAction`.
    pub fn run_action(&mut self, action: RegionCaptureAction) {
        match action {
            RegionCaptureAction::None => {}
            RegionCaptureAction::CancelCapture => self.cancel(),
            RegionCaptureAction::RemoveShapeCancelCapture => {
                if self.has_valid_selection() && self.selection.contains(self.last_point) {
                    self.clear_selection();
                } else {
                    self.cancel();
                }
            }
            RegionCaptureAction::RemoveShape => {
                if self.has_valid_selection() && self.selection.contains(self.last_point) {
                    self.clear_selection();
                }
            }
            RegionCaptureAction::SwapToolType => {
                // Between the region tool and the editor's annotation tools; a host without
                // the editor keeps the region tool.
                if self.request.enable_annotations && crate::capture::ANNOTATION_TOOLS_AVAILABLE {
                    self.region_tool_active = !self.region_tool_active;
                }
            }
            RegionCaptureAction::CaptureFullscreen => {
                let all = self.image_bounds();
                self.complete(all, false);
            }
            RegionCaptureAction::CaptureActiveMonitor => self.complete_active_monitor(),
            RegionCaptureAction::CaptureLastRegion => self.complete_last_region(),
        }
    }

    fn complete_last_region(&mut self) {
        let last = self.request.last_region;
        if last.is_empty() {
            return;
        }
        let rect = last.intersect(&self.request.screen_bounds);
        if !rect.has_area() {
            return;
        }
        let image = self.to_image(rect);
        self.complete(image, false);
    }

    fn complete_active_monitor(&mut self) {
        let (x, y) = self.to_screen(Point::new(
            self.last_point.x.round(),
            self.last_point.y.round(),
        ));
        let Some(screen) = self
            .request
            .screens
            .iter()
            .find(|s| s.bounds.contains(x, y))
        else {
            return;
        };
        let rect = self.to_image(screen.bounds).intersect(&self.image_bounds());
        self.complete(rect, false);
    }

    /// `Complete`: whole image pixels, the desktop rectangle, the window it shows.
    pub fn complete(&mut self, selection: Rect, include_window_info: bool) {
        if self.is_closed() || !selection.is_valid() {
            return;
        }
        let (iw, ih) = (self.image_size.0 as i32, self.image_size.1 as i32);
        let left = (selection.left().floor() as i32).clamp(0, iw - 1);
        let top = (selection.top().floor() as i32).clamp(0, ih - 1);
        let right = (selection.right().ceil() as i32).clamp(left + 1, iw);
        let bottom = (selection.bottom().ceil() as i32).clamp(top + 1, ih);
        let image_rect = IRect::new(left, top, right - left, bottom - top);
        let s = self.request.scale;
        let bounds = self.request.screen_bounds;
        let screen_rect = IRect::new(
            bounds.x + (left as f64 / s).floor() as i32,
            bounds.y + (top as f64 / s).floor() as i32,
            ((right - left) as f64 / s).round().max(1.0) as i32,
            ((bottom - top) as f64 / s).round().max(1.0) as i32,
        );
        let window = if include_window_info && !self.is_light() {
            self.find_window(screen_rect)
        } else {
            None
        };
        self.outcome = Some(RegionOutcome::Completed {
            image_rect,
            screen_rect,
            window,
        });
    }

    /// `FindTopLevelWindowInfo`.
    fn find_window(&self, rect: IRect) -> Option<WindowInfo> {
        if let Some(index) = self.selected_candidate {
            return self.request.windows.get(index).cloned();
        }
        let cx = rect.x + rect.width / 2;
        let cy = rect.y + rect.height / 2;
        self.request
            .windows
            .iter()
            .find(|w| w.rect.contains(cx, cy))
            .cloned()
    }

    /// `CancelCapture`.
    pub fn cancel(&mut self) {
        if self.is_closed() {
            return;
        }
        self.outcome = Some(RegionOutcome::Cancelled);
    }

    /// The hovered or selected window's index, for tests and the overlay.
    pub fn hovered_window(&self) -> Option<&WindowInfo> {
        self.hover_candidate
            .and_then(|i| self.request.windows.get(i))
    }

    /// `"{width} × {height}"` of the selection (`UpdateSelectionInfo`).
    pub fn selection_info(&self) -> Option<String> {
        if !self.request.options.show_info || !self.selection.is_valid() || self.is_light() {
            return None;
        }
        let w = (self.selection.width.round() as i64).max(1);
        let h = (self.selection.height.round() as i64).max(1);
        Some(format!("{w} \u{00d7} {h}"))
    }

    /// `"X: {x} Y: {y}"` of the pointer, desktop coordinates (`UpdateHud`).
    pub fn pointer_info(&self) -> String {
        let (x, y) = self.to_screen(self.last_point);
        format!("X: {x} Y: {y}")
    }
}

/// Crop the frozen picture to a completed capture.
pub fn crop(image: &RgbaImage, rect: IRect) -> RgbaImage {
    image::imageops::crop_imm(
        image,
        rect.x as u32,
        rect.y as u32,
        rect.width as u32,
        rect.height as u32,
    )
    .to_image()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::source::{CaptureSource, FakeSource};

    fn request(kind: RegionKind) -> RegionRequest {
        let source = FakeSource::sample();
        let bounds = source.screen_bounds();
        RegionRequest {
            kind,
            screen_bounds: bounds,
            scale: 1.0,
            image: Some(Arc::new(source.capture_rect(bounds, false).unwrap())),
            options: RegionCaptureOptions::default(),
            enable_annotations: true,
            windows: source.windows(),
            screens: source.screens(),
            show_cursor: false,
            last_region: IRect::default(),
            pointer: (10, 10),
        }
    }

    fn completed(session: &RegionSession) -> (IRect, IRect, Option<String>) {
        match session.outcome.clone() {
            Some(RegionOutcome::Completed {
                image_rect,
                screen_rect,
                window,
            }) => (image_rect, screen_rect, window.map(|w| w.title)),
            other => panic!("not completed: {other:?}"),
        }
    }

    #[test]
    fn a_drag_selects_and_quick_crop_completes() {
        let mut s = RegionSession::new(request(RegionKind::Default));
        s.pointer_pressed(Button::Left, Point::new(1300.0, 780.0), 1);
        assert_eq!(s.interaction, Interaction::Creating);
        s.pointer_moved(Point::new(1400.5, 840.25));
        assert_eq!(s.selection, Rect::new(1300.0, 780.0, 100.5, 60.25));
        s.pointer_released(Button::Left, Point::new(1400.5, 840.25));
        let (image, screen, window) = completed(&s);
        assert_eq!(image, IRect::new(1300, 780, 101, 61));
        assert_eq!(screen, IRect::new(1300, 780, 101, 61));
        assert_eq!(window, None, "no window under the centre of that area");
    }

    #[test]
    fn hovering_a_window_and_clicking_takes_it() {
        let mut s = RegionSession::new(request(RegionKind::Default));
        s.pointer_moved(Point::new(300.0, 200.0));
        assert_eq!(s.hovered_window().unwrap().title, "Notes");
        assert_eq!(s.hover, Rect::new(160.0, 120.0, 640.0, 420.0));
        s.pointer_pressed(Button::Left, Point::new(300.0, 200.0), 1);
        assert_eq!(s.interaction, Interaction::PendingHover);
        s.pointer_released(Button::Left, Point::new(301.0, 201.0));
        let (image, _, window) = completed(&s);
        assert_eq!(image, IRect::new(160, 120, 640, 420));
        assert_eq!(window.as_deref(), Some("Notes"));
    }

    #[test]
    fn dragging_away_from_a_hovered_window_draws_a_rectangle() {
        let mut s = RegionSession::new(request(RegionKind::Default));
        s.pointer_moved(Point::new(300.0, 200.0));
        s.pointer_pressed(Button::Left, Point::new(300.0, 200.0), 1);
        s.pointer_moved(Point::new(302.0, 202.0));
        assert_eq!(s.interaction, Interaction::PendingHover);
        s.pointer_moved(Point::new(350.0, 260.0));
        assert_eq!(s.interaction, Interaction::Creating);
        assert_eq!(s.selection, Rect::new(300.0, 200.0, 50.0, 60.0));
    }

    #[test]
    fn without_quick_crop_the_selection_can_be_moved_resized_and_confirmed() {
        let mut req = request(RegionKind::Default);
        req.options.quick_crop = false;
        req.options.detect_windows = false;
        let mut s = RegionSession::new(req);
        s.keyboard_enabled = true;
        s.pointer_pressed(Button::Left, Point::new(100.0, 100.0), 1);
        s.pointer_moved(Point::new(300.0, 200.0));
        s.pointer_released(Button::Left, Point::new(300.0, 200.0));
        assert!(s.outcome.is_none());
        assert!(s.show_handles);
        // Move by dragging the inside.
        s.pointer_pressed(Button::Left, Point::new(200.0, 150.0), 1);
        assert_eq!(s.interaction, Interaction::Moving);
        s.pointer_moved(Point::new(210.0, 160.0));
        s.pointer_released(Button::Left, Point::new(210.0, 160.0));
        assert_eq!(s.selection, Rect::new(110.0, 110.0, 200.0, 100.0));
        // Resize from the bottom-right handle.
        s.pointer_pressed(Button::Left, Point::new(310.0, 210.0), 1);
        assert_eq!(s.interaction, Interaction::Resizing);
        s.pointer_moved(Point::new(410.0, 260.0));
        s.pointer_released(Button::Left, Point::new(410.0, 260.0));
        assert_eq!(s.selection, Rect::new(110.0, 110.0, 300.0, 150.0));
        // Arrows move by 1, Shift by 10.
        s.key(Key::Right, false);
        s.key(Key::Down, true);
        assert_eq!(s.selection, Rect::new(111.0, 120.0, 300.0, 150.0));
        s.key(Key::Enter, false);
        assert_eq!(completed(&s).0, IRect::new(111, 120, 300, 150));
    }

    #[test]
    fn keys_before_the_input_delay_are_ignored_and_escape_always_cancels() {
        let mut s = RegionSession::new(request(RegionKind::Default));
        s.key(Key::Space, false);
        assert!(s.outcome.is_none());
        s.keyboard_enabled = true;
        s.key(Key::Space, false);
        assert_eq!(completed(&s).1, IRect::new(0, 0, 2720, 900));
        let mut s = RegionSession::new(request(RegionKind::Default));
        s.key(Key::Escape, false);
        assert_eq!(s.outcome, Some(RegionOutcome::Cancelled));
    }

    #[test]
    fn tilde_takes_the_monitor_under_the_pointer() {
        let mut s = RegionSession::new(request(RegionKind::Default));
        s.keyboard_enabled = true;
        s.pointer_moved(Point::new(2000.0, 650.0));
        s.key(Key::Tilde, false);
        assert_eq!(completed(&s).1, IRect::new(1440, 0, 1280, 720));
    }

    #[test]
    fn right_click_removes_the_selection_then_cancels() {
        let mut req = request(RegionKind::Default);
        req.options.quick_crop = false;
        req.options.detect_windows = false;
        let mut s = RegionSession::new(req);
        s.pointer_pressed(Button::Left, Point::new(10.0, 10.0), 1);
        s.pointer_moved(Point::new(90.0, 90.0));
        s.pointer_released(Button::Left, Point::new(90.0, 90.0));
        s.pointer_moved(Point::new(50.0, 50.0));
        s.pointer_pressed(Button::Right, Point::new(50.0, 50.0), 1);
        s.pointer_released(Button::Right, Point::new(50.0, 50.0));
        assert!(!s.selection.is_valid());
        assert!(s.outcome.is_none());
        s.pointer_pressed(Button::Right, Point::new(50.0, 50.0), 1);
        s.pointer_released(Button::Right, Point::new(50.0, 50.0));
        assert_eq!(s.outcome, Some(RegionOutcome::Cancelled));
    }

    #[test]
    fn a_right_click_while_drawing_only_drops_the_rectangle() {
        let mut s = RegionSession::new(request(RegionKind::Default));
        s.pointer_pressed(Button::Left, Point::new(1000.0, 700.0), 1);
        s.pointer_moved(Point::new(1100.0, 800.0));
        s.pointer_pressed(Button::Right, Point::new(1100.0, 800.0), 1);
        assert!(!s.selection.is_valid());
        s.pointer_released(Button::Right, Point::new(1100.0, 800.0));
        assert!(
            s.outcome.is_none(),
            "the release that ended creation runs no action"
        );
    }

    #[test]
    fn small_selections_fall_back_to_the_hovered_window_or_nothing() {
        let mut req = request(RegionKind::Default);
        req.options.quick_crop = false;
        let mut s = RegionSession::new(req);
        s.pointer_pressed(Button::Left, Point::new(1000.0, 700.0), 1);
        s.pointer_moved(Point::new(1003.0, 702.0));
        s.pointer_released(Button::Left, Point::new(1003.0, 702.0));
        assert!(!s.selection.is_valid());
        assert!(!s.show_handles);
    }

    #[test]
    fn mouse_side_buttons_run_their_actions() {
        let mut req = request(RegionKind::Default);
        req.last_region = IRect::new(5, 6, 70, 80);
        req.options.region_capture_action_x1_click = RegionCaptureAction::CaptureLastRegion;
        let mut s = RegionSession::new(req);
        s.pointer_pressed(Button::X1, Point::new(1.0, 1.0), 1);
        assert_eq!(completed(&s).1, IRect::new(5, 6, 70, 80));
        let mut s = RegionSession::new(request(RegionKind::Default));
        s.pointer_pressed(Button::X1, Point::new(1.0, 1.0), 1);
        assert_eq!(completed(&s).1, IRect::new(0, 0, 2720, 900));
    }

    #[test]
    fn fixed_size_centres_on_the_pointer() {
        let mut req = request(RegionKind::Default);
        req.options.is_fixed_size = true;
        let mut s = RegionSession::new(req);
        s.pointer_pressed(Button::Left, Point::new(500.0, 500.0), 1);
        assert_eq!(s.selection, Rect::new(375.0, 375.0, 250.0, 250.0));
        s.pointer_released(Button::Left, Point::new(500.0, 500.0));
        assert_eq!(completed(&s).0, IRect::new(375, 375, 250, 250));
    }

    #[test]
    fn the_magnifier_stays_odd_and_within_its_size() {
        let s = RegionSession::new(request(RegionKind::Default));
        assert_eq!(s.normalize_magnifier_pixel_count(15), 15);
        assert_eq!(s.normalize_magnifier_pixel_count(16), 17);
        assert_eq!(s.normalize_magnifier_pixel_count(100), 25);
        assert_eq!(s.normalize_magnifier_pixel_count(0), 3);
        let mut s = s;
        s.render_scale = 2.0;
        assert_eq!(s.normalize_magnifier_pixel_count(100), 35);
        s.wheel(-1.0);
        assert_eq!(s.magnifier_pixel_count, 17);
    }

    #[test]
    fn retina_pixels_map_back_to_points() {
        let mut req = request(RegionKind::Default);
        req.scale = 2.0;
        req.screen_bounds = IRect::new(0, 0, 1440, 900);
        let mut s = RegionSession::new(req);
        assert_eq!(s.image_size, (2880.0, 1800.0));
        s.pointer_pressed(Button::Left, Point::new(2000.0, 1000.0), 1);
        s.pointer_moved(Point::new(2400.0, 1400.0));
        s.pointer_released(Button::Left, Point::new(2400.0, 1400.0));
        let (image, screen, _) = completed(&s);
        assert_eq!(image, IRect::new(2000, 1000, 400, 400));
        assert_eq!(screen, IRect::new(1000, 500, 200, 200));
        assert_eq!(s.pointer_info(), "X: 1200 Y: 700");
    }

    #[test]
    fn the_light_capture_completes_on_release_and_right_click_cancels() {
        let mut s = RegionSession::new(request(RegionKind::Light));
        s.pointer_moved(Point::new(300.0, 200.0));
        assert!(
            !s.hover.is_valid(),
            "no window detection in the light capture"
        );
        s.pointer_pressed(Button::Left, Point::new(10.0, 10.0), 1);
        s.pointer_moved(Point::new(14.0, 14.0));
        s.pointer_released(Button::Left, Point::new(14.0, 14.0));
        assert!(s.outcome.is_none(), "5 pixels or less is no selection");
        s.pointer_pressed(Button::Left, Point::new(10.0, 10.0), 1);
        s.pointer_moved(Point::new(110.0, 60.0));
        assert!(s.light_dragging());
        s.pointer_released(Button::Left, Point::new(110.0, 60.0));
        assert_eq!(completed(&s).1, IRect::new(10, 10, 100, 50));
        let mut s = RegionSession::new(request(RegionKind::Transparent));
        s.pointer_released(Button::Right, Point::new(1.0, 1.0));
        assert_eq!(s.outcome, Some(RegionOutcome::Cancelled));
    }

    #[test]
    fn info_texts() {
        let mut s = RegionSession::new(request(RegionKind::Default));
        s.pointer_moved(Point::new(12.0, 34.0));
        assert_eq!(s.pointer_info(), "X: 12 Y: 34");
        s.pointer_pressed(Button::Left, Point::new(1000.0, 700.0), 1);
        s.pointer_moved(Point::new(1200.0, 800.0));
        assert_eq!(s.selection_info().unwrap(), "200 \u{00d7} 100");
    }
}
