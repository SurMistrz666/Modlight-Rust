//! Pin to screen (`PinToScreenWindow`, `PinToScreenManager`, `PinToScreenStartupWindow`,
//! `PinToScreenOptionsWindow`): a picture in a frameless window above the others, with
//! its border and shadow; drag to move, the wheel scales, Cmd+wheel fades, a double click
//! folds it to its preview size, a right click closes it, the middle button resets it;
//! a toolbar on hover (copy, the scale, options, close).
//!
//! Sizes are in points. A picture pinned from the screen comes back at the size it had
//! there (its pixels divided by the display's scale); a file or the clipboard's picture
//! takes one point per pixel, as Avalonia's DIPs do.

use std::sync::Arc;

use egui::{
    Align2, Color32, Id, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle, Ui, Vec2,
    ViewportBuilder, ViewportCommand, ViewportId, pos2, vec2,
};
use image::RgbaImage;
use modlight_core::icons;
#[cfg(not(windows))]
use modlight_core::l10n::mac;
use modlight_core::l10n::{Key, s, t, tools as tl};
use modlight_core::tasks::host::{MessageIcon, NotificationSound};
use modlight_core::tasks::settings::TaskSettings;
use modlight_tools::color::Color;
use modlight_tools::pin::{self as geometry, PinState};
use modlight_tools::settings::{ContentAlignment, PinToScreenOptions, Size};

use super::jobs::OptionsHome;
use super::kit::{self, Track};
use super::region::RegionFor;
use super::{ToolWindow, WindowSpec};
use crate::state::{AppState, Effect};

/// The options window's wheel note: upstream's (Ctrl + wheel) on Windows, Cmd on a Mac.
#[cfg(windows)]
const WHEEL_HINT: Key =
    tl::PinToScreenOptionsWindow_Mouse_wheel_changes_scale_Ctrl_mouse_wheel_changes_opacity;
#[cfg(not(windows))]
const WHEEL_HINT: Key = mac::PinWheelHint;
use crate::theme::Weight;

/// `PinToScreenSource`: the picture, where it came from (desktop points) and how many
/// pixels make a point.
#[derive(Clone)]
pub struct PinSource {
    pub image: Arc<RgbaImage>,
    pub location: Option<Pos2>,
    pub pixels_per_point: f32,
}

/// One pinned picture.
pub struct Pin {
    pub key: u64,
    pub source: PinSource,
    pub options: PinToScreenOptions,
    pub home: OptionsHome,
    pub state: PinState,
    texture: Option<(bool, TextureHandle)>,
    /// The window's top left (desktop points) and size.
    pub position: Option<Pos2>,
    pub size: Vec2,
    last_press: Option<f64>,
    pub closed: bool,
    /// QA renders show the toolbar as if the pointer were over the picture.
    pub show_toolbar: bool,
}

impl Pin {
    pub fn new(key: u64, source: PinSource, options: PinToScreenOptions, home: OptionsHome) -> Pin {
        let state = PinState::new(&options);
        let mut pin = Pin {
            key,
            source,
            options,
            home,
            state,
            texture: None,
            position: None,
            size: Vec2::ZERO,
            last_press: None,
            closed: false,
            show_toolbar: false,
        };
        pin.size = pin.window_size();
        pin
    }

    fn image_points(&self) -> (f64, f64) {
        let ppp = self.source.pixels_per_point.max(0.01) as f64;
        (
            self.source.image.width() as f64 / ppp,
            self.source.image.height() as f64 / ppp,
        )
    }

    pub fn window_size(&self) -> Vec2 {
        let (w, h) = geometry::window_size(self.image_points(), &self.state, &self.options);
        vec2(w as f32, h as f32)
    }

    /// `OnOpened`: the requested place minus the inset, else the placement on the
    /// working area of the display under the pointer.
    pub fn place(&mut self, working_area: Rect) {
        let inset = geometry::outer_inset(&self.options) as f32;
        let at = match self.source.location {
            Some(location) => pos2(location.x - inset, location.y - inset),
            None => {
                let (x, y) = geometry::position(
                    self.options.placement,
                    self.options.placement_offset as f64,
                    (
                        working_area.left() as f64,
                        working_area.top() as f64,
                        working_area.width() as f64,
                        working_area.height() as f64,
                    ),
                    (self.size.x.ceil() as f64, self.size.y.ceil() as f64),
                );
                pos2(x as f32, y as f32)
            }
        };
        self.position = Some(at);
    }

    /// `ResizeWindow(keepCenter)`.
    fn resize(&mut self, keep_center: bool) {
        let old = self.size;
        let new = self.window_size();
        if keep_center
            && old.x > 0.0
            && old.y > 0.0
            && let Some(position) = self.position
        {
            let (x, y) = geometry::keep_center(
                (position.x as f64, position.y as f64),
                (old.x as f64, old.y as f64),
                (new.x as f64, new.y as f64),
            );
            self.position = Some(pos2(x as f32, y as f32));
        }
        self.size = new;
    }

    fn texture(&mut self, ctx: &egui::Context) -> TextureHandle {
        let smooth = self.options.high_quality_scale;
        if let Some((s, texture)) = &self.texture
            && *s == smooth
        {
            return texture.clone();
        }
        let texture = kit::texture(
            ctx,
            &format!("pin-{}", self.key),
            &self.source.image,
            !smooth,
        );
        self.texture = Some((smooth, texture.clone()));
        texture
    }

    /// The window's content: picture, border, shadow, toolbar. Returns what the toolbar
    /// or the keys asked of the app.
    pub fn ui(&mut self, ui: &mut Ui, colors: &crate::theme::Colors, now: f64) -> Vec<PinAsk> {
        let mut asks = Vec::new();
        let full = ui.max_rect();
        ui.multiply_opacity(self.state.window_opacity() as f32);
        let shadow = if self.options.shadow {
            geometry::SHADOW_MARGIN as f32
        } else {
            0.0
        };
        let border = geometry::border(&self.options) as f32;
        let frame = full.shrink(shadow);
        let painter = ui.painter().clone();
        if self.options.shadow {
            painter.add(
                egui::Shadow {
                    offset: [0, 3],
                    blur: 14,
                    spread: 0,
                    color: Color32::from_black_alpha(110),
                }
                .as_shape(frame, 0.0),
            );
        }
        let background = self.options.background_color;
        painter.rect_filled(frame, 0.0, to32(background));
        let image_rect = frame.shrink(border);
        let texture = self.texture(ui.ctx());
        painter.image(
            texture.id(),
            image_rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        if border > 0.0 {
            painter.rect_stroke(
                frame,
                0.0,
                Stroke::new(border, to32(self.options.border_color)),
                StrokeKind::Inside,
            );
        }

        // The pointer: drag, double click, right click, middle click, wheel.
        let surface = ui.interact(
            full,
            Id::new(("pin-surface", self.key)),
            Sense::click_and_drag(),
        );
        let inside = ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|p| full.contains(p));
        let toolbar_on = geometry::toolbar_visible(
            inside || self.show_toolbar,
            (self.size.x as f64, self.size.y as f64),
        );
        let mut over_toolbar = false;
        if toolbar_on {
            over_toolbar = self.toolbar(ui, colors, full, &mut asks);
        }
        let primary_pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        if primary_pressed && inside && !over_toolbar {
            let double = self.last_press.is_some_and(|t| now - t < 0.4);
            self.last_press = Some(now);
            if double {
                self.last_press = None;
                self.state.minimized = !self.state.minimized;
                self.resize(self.options.keep_center_location);
            } else {
                ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
            }
        }
        if surface.secondary_clicked() && !over_toolbar {
            self.closed = true;
        }
        if surface.middle_clicked() && !self.state.minimized {
            self.state.reset();
            self.resize(self.options.keep_center_location);
        }
        let (wheel, command, shift) = ui.input(|i| {
            // `Delta.Y > 0 ? 1 : Delta.Y < 0 ? -1 : 0`: an event without a vertical part
            // (a sideways swipe, a trackpad gesture starting or ending) is no step.
            let wheel: f32 = i
                .raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { delta, .. } if delta.y != 0.0 => {
                        Some(delta.y.signum())
                    }
                    _ => None,
                })
                .sum();
            (wheel, i.modifiers.command, i.modifiers.shift)
        });
        if inside && wheel != 0.0 && !self.state.minimized {
            let direction = wheel.signum() as i64;
            if command {
                self.state
                    .change_opacity(direction * self.options.opacity_step.max(1));
            } else if !shift
                && self
                    .state
                    .change_scale(direction * self.options.scale_step.max(1))
            {
                self.resize(self.options.keep_center_location);
            }
        }

        // Keys.
        let mut moved = Vec2::ZERO;
        ui.input(|i| {
            let distance = if i.modifiers.shift { 10.0 } else { 1.0 };
            if i.key_pressed(egui::Key::ArrowLeft) {
                moved.x -= distance;
            }
            if i.key_pressed(egui::Key::ArrowRight) {
                moved.x += distance;
            }
            if i.key_pressed(egui::Key::ArrowUp) {
                moved.y -= distance;
            }
            if i.key_pressed(egui::Key::ArrowDown) {
                moved.y += distance;
            }
            if i.modifiers.command && i.key_pressed(egui::Key::C) {
                asks.push(PinAsk::Copy);
            }
            let plus = i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals);
            let minus = i.key_pressed(egui::Key::Minus);
            if !self.state.minimized && (plus || minus) {
                let direction = if plus { 1 } else { -1 };
                if i.modifiers.command {
                    self.state
                        .change_opacity(direction * self.options.opacity_step.max(1));
                } else if self
                    .state
                    .change_scale(direction * self.options.scale_step.max(1))
                {
                    asks.push(PinAsk::Resized);
                }
            }
        });
        if asks.contains(&PinAsk::Resized) {
            self.resize(self.options.keep_center_location);
        }
        if moved != Vec2::ZERO
            && let Some(position) = self.position
        {
            self.position = Some(position + moved);
        }
        asks
    }

    /// The toolbar (`Border.tool-panel`, top centre); true while the pointer is on it.
    fn toolbar(
        &mut self,
        ui: &mut Ui,
        colors: &crate::theme::Colors,
        full: Rect,
        asks: &mut Vec<PinAsk>,
    ) -> bool {
        let scale_text = self.state.scale_text();
        let text_w = kit::text_width(ui, &scale_text, 13.0, Weight::Normal) + 16.0;
        let width = 4.0 + 34.0 + 2.0 + text_w + 2.0 + 34.0 + 2.0 + 34.0 + 4.0;
        let bar = Rect::from_min_size(
            pos2(full.center().x - width / 2.0, full.top()),
            vec2(width, 40.0),
        );
        kit::panel(ui, colors, bar);
        let mut x = bar.left() + 4.0;
        let mut button =
            |ui: &mut Ui, w: f32, icon: Option<char>, label: Option<&str>, tip: &str| {
                let r = Rect::from_min_size(pos2(x, bar.top() + 4.0), vec2(w, 32.0));
                x += w + 2.0;
                let look = crate::widgets::Look::icon_button(
                    colors,
                    32.0,
                    10.0,
                    colors.control_background_disabled(),
                )
                .width(w);
                let mut child = kit::child(ui, r);
                crate::widgets::button_at(
                    &mut child,
                    colors,
                    &look,
                    crate::widgets::Content { icon, label },
                    true,
                    r,
                )
                .on_hover_text(tip)
                .clicked()
            };
        if button(
            ui,
            34.0,
            Some(icons::COPY),
            None,
            t(tl::PinToScreenWindow_Copy_image),
        ) {
            asks.push(PinAsk::Copy);
        }
        if button(
            ui,
            text_w,
            None,
            Some(&scale_text),
            t(tl::PinToScreenWindow_Reset_scale_and_opacity),
        ) {
            self.state.reset();
            self.resize(self.options.keep_center_location);
        }
        if button(
            ui,
            34.0,
            Some(icons::SETTINGS),
            None,
            t(tl::PinToScreenWindow_Options),
        ) {
            asks.push(PinAsk::Options);
        }
        if button(
            ui,
            34.0,
            Some(icons::X),
            None,
            t(tl::PinToScreenWindow_Close),
        ) {
            self.closed = true;
        }
        ui.input(|i| i.pointer.hover_pos())
            .is_some_and(|p| bar.contains(p))
    }
}

/// What a pinned picture asks the app for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PinAsk {
    Copy,
    Options,
    Resized,
}

fn to32(c: Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

/// Every pinned picture, each its own frameless window.
pub fn viewports(ctx: &egui::Context, state: &mut AppState) {
    if state.tools.pins.is_empty() {
        return;
    }
    let colors = state.colors.clone();
    let now = state.time;
    let mut pins = std::mem::take(&mut state.tools.pins);
    let mut asks = Vec::new();
    for pin in pins.iter_mut() {
        let Some(position) = pin.position else {
            continue;
        };
        let mut builder = ViewportBuilder::default()
            .with_title(t(tl::PinToScreenWindow_Pinned_image))
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_taskbar(false)
            .with_position(position)
            .with_inner_size(pin.size);
        if pin.options.top_most {
            builder = builder.with_always_on_top();
        }
        let id = ViewportId::from_hash_of(("pin", pin.key));
        let key = pin.key;
        let result = ctx.show_viewport_immediate(id, builder, |ui, _| {
            // Where the system has the window now (after a drag).
            if let Some(outer) = ui.ctx().input(|i| i.viewport().outer_rect) {
                let wanted = pin.position.unwrap_or(outer.min);
                if (outer.min - wanted).length() > 0.5 && !pin_moved_by_us(ui.ctx(), key) {
                    pin.position = Some(outer.min);
                }
            }
            let before = (pin.position, pin.size);
            let asks = pin.ui(ui, &colors, now);
            if pin.size != before.1 {
                ui.ctx()
                    .send_viewport_cmd(ViewportCommand::InnerSize(pin.size));
            }
            if pin.position != before.0
                && let Some(p) = pin.position
            {
                ui.ctx()
                    .send_viewport_cmd(ViewportCommand::OuterPosition(p));
                mark_moved(ui.ctx(), key);
            }
            let closed = ui.ctx().input(|i| i.viewport().close_requested());
            (asks, closed)
        });
        if result.1 {
            pin.closed = true;
        }
        for ask in result.0 {
            asks.push((pin.key, ask));
        }
    }
    pins.retain(|p| !p.closed);
    pins.append(&mut state.tools.pins);
    state.tools.pins = pins;
    for (key, ask) in asks {
        match ask {
            PinAsk::Copy => {
                if let Some(pin) = state.tools.pins.iter().find(|p| p.key == key) {
                    let image = pin.source.image.clone();
                    state.effects.push(Effect::CopyImage(image));
                }
            }
            PinAsk::Options => state.open_pin_options(Some(key)),
            PinAsk::Resized => {}
        }
    }
}

/// The window was just moved by the app; its next reported place is ours, not a drag.
fn mark_moved(ctx: &egui::Context, key: u64) {
    ctx.data_mut(|d| d.insert_temp(Id::new(("pin-moved", key)), 2u8));
}

fn pin_moved_by_us(ctx: &egui::Context, key: u64) -> bool {
    let id = Id::new(("pin-moved", key));
    let left = ctx.data(|d| d.get_temp::<u8>(id).unwrap_or(0));
    if left > 0 {
        ctx.data_mut(|d| d.insert_temp(id, left - 1));
        true
    } else {
        false
    }
}

// ------------------------------------------------------------------ the startup window

/// `PinToScreenStartupWindow`: choose where the picture comes from.
pub struct PinStartupWindow {
    pub key: u64,
    pub settings: TaskSettings,
    pub busy: bool,
    pub error: String,
    height: f32,
    /// The region capture waits for the window to hide (`Task.Delay(200)`).
    pending_region: Option<f64>,
    pub done: bool,
}

impl PinStartupWindow {
    pub fn new(key: u64, settings: TaskSettings) -> PinStartupWindow {
        PinStartupWindow {
            key,
            settings,
            busy: false,
            error: String::new(),
            height: 230.0,
            pending_region: None,
            done: false,
        }
    }

    /// `SelectAsync`'s ending.
    pub fn selected(&mut self, state: &mut AppState, source: Option<PinSource>, empty: &str) {
        self.busy = false;
        match source {
            Some(source) => {
                state.pin(source, &self.settings);
                self.done = true;
            }
            None => self.error = empty.to_string(),
        }
    }
}

impl ToolWindow for PinStartupWindow {
    fn spec(&self) -> WindowSpec {
        WindowSpec {
            fit_height: true,
            ..WindowSpec::fixed(
                t(tl::PinToScreenStartupWindow_ShareX_Pin_to_Screen),
                vec2(560.0, self.height),
            )
        }
    }

    fn content_height(&self) -> Option<f32> {
        Some(self.height)
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        let now = state.time;
        kit::fill(ui, &colors);
        let full = kit::inset(ui.max_rect(), 16.0);
        // The heading panel.
        let description = t(
            tl::PinToScreenStartupWindow_Pin_an_image_above_your_other_windows_for_quick_reference,
        );
        let text_h = crate::widgets::layout(
            ui,
            crate::widgets::wrapped(
                description,
                13.0,
                Weight::Normal,
                colors.text_secondary(),
                full.width() - 24.0,
            ),
        )
        .size()
        .y;
        let head = Rect::from_min_size(
            full.min,
            vec2(
                full.width(),
                12.0 + kit::HEADING_HEIGHT + 6.0 + text_h + 12.0,
            ),
        );
        kit::panel(ui, &colors, head);
        let inner = head.shrink(12.0);
        kit::heading(
            ui,
            &colors,
            Rect::from_min_size(inner.min, vec2(inner.width(), kit::HEADING_HEIGHT)),
            t(tl::PinToScreenStartupWindow_Choose_an_image),
        );
        kit::paragraph(
            ui,
            Rect::from_min_size(
                pos2(inner.left(), inner.top() + kit::HEADING_HEIGHT + 6.0),
                vec2(inner.width(), text_h),
            ),
            description,
            13.0,
            Weight::Normal,
            colors.text_secondary(),
        );
        // The three choices.
        let row = Rect::from_min_size(
            pos2(full.left(), head.bottom() + 12.0),
            vec2(full.width(), 88.0),
        );
        let cols = kit::columns(row, &[Track::Star(1.0); 3], 10.0);
        let choices = [
            (icons::SCAN, t(tl::PinToScreenStartupWindow_Select_region)),
            (
                icons::CLIPBOARD_PASTE,
                t(tl::PinToScreenStartupWindow_From_clipboard),
            ),
            (
                icons::FOLDER_OPEN,
                t(tl::PinToScreenStartupWindow_Select_image),
            ),
        ];
        let mut clicked = None;
        for (index, (glyph, label)) in choices.iter().enumerate() {
            let r = cols[index];
            let look = crate::widgets::Look::button(&colors)
                .width(r.width())
                .height(88.0);
            let response = {
                let mut child = kit::child(ui, r);
                crate::widgets::button_at(
                    &mut child,
                    &colors,
                    &look,
                    crate::widgets::Content {
                        icon: None,
                        label: None,
                    },
                    !self.busy,
                    r,
                )
            };
            let dim = if self.busy { 0.35 } else { 1.0 };
            crate::widgets::paint_icon(
                ui,
                ui.painter(),
                pos2(r.center().x, r.center().y - 12.0),
                *glyph,
                24.0,
                colors.text_secondary().gamma_multiply(dim),
            );
            kit::text_at(
                ui,
                Rect::from_min_size(pos2(r.left(), r.center().y + 4.0), vec2(r.width(), 20.0)),
                Align2::CENTER_CENTER,
                label,
                13.0,
                Weight::SemiBold,
                colors.text_secondary().gamma_multiply(dim),
            );
            if response.clicked() {
                clicked = Some(index);
            }
        }
        let mut bottom = row.bottom();
        if !self.error.is_empty() {
            let r = Rect::from_min_size(
                pos2(full.left(), row.bottom() + 12.0),
                vec2(full.width(), 400.0),
            );
            let h = kit::paragraph(
                ui,
                r,
                &self.error,
                13.0,
                Weight::Normal,
                colors.cancel_border(),
            );
            bottom = r.top() + h;
        }
        self.height = (bottom + 16.0 - ui.max_rect().top()).round();

        match clicked {
            Some(0) => {
                // `RegionCaptureStarted = Hide`, then `Task.Delay(200)`.
                self.busy = true;
                self.error.clear();
                ui.ctx().send_viewport_cmd(ViewportCommand::Visible(false));
                self.pending_region = Some(now + 0.2);
            }
            Some(1) => {
                self.busy = true;
                self.error.clear();
                let source = state.clipboard_pin_source();
                self.selected(
                    state,
                    source,
                    t(tl::PinToScreenStartupViewModel_Clipboard_no_image),
                );
            }
            Some(2) => {
                self.busy = true;
                self.error.clear();
                let source = state.file_pin_source();
                self.selected(
                    state,
                    source,
                    t(tl::PinToScreenStartupViewModel_No_image_selected),
                );
            }
            _ => {}
        }
        if let Some(at) = self.pending_region {
            if now >= at {
                self.pending_region = None;
                let settings = self.settings.clone();
                state.tool_region(RegionFor::PinStartup(self.key), settings);
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
        ui.input(|i| i.key_released(egui::Key::Escape)) || self.done
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ------------------------------------------------------------------ the options window

/// `PinToScreenOptionsWindow`.
pub struct PinOptionsWindow {
    pub key: u64,
    /// The pinned picture that opened it (its options change on Save).
    pub pin: Option<u64>,
    pub home: OptionsHome,
    pub draft: PinToScreenOptions,
    pub close: bool,
}

impl ToolWindow for PinOptionsWindow {
    fn spec(&self) -> WindowSpec {
        WindowSpec::resizable(
            t(tl::PinToScreenOptionsWindow_ShareX_Pin_to_Screen_options),
            vec2(760.0, 560.0),
            vec2(700.0, 520.0),
        )
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        kit::fill(ui, &colors);
        let full = kit::inset(ui.max_rect(), 16.0);
        let rows = kit::rows(full, &[Track::Star(1.0), Track::Px(40.0)], 12.0);
        let cols = kit::columns(rows[0], &[Track::Star(1.0); 2], 12.0);
        let d = &mut self.draft;
        let line = 40.0;
        let toggle_line = 30.0;
        let key = self.key;
        let id = |name: &str| Id::new(("pin-options", key, name.to_string()));

        // Left: size and opacity.
        let p1_h = 12.0 * 2.0 + kit::HEADING_HEIGHT + 10.0 * 5.0 + line * 4.0 + toggle_line;
        let p1 = Rect::from_min_size(cols[0].min, vec2(cols[0].width(), p1_h));
        kit::panel(ui, &colors, p1);
        let mut y = p1.top() + 12.0;
        let inner_x = p1.left() + 12.0;
        let inner_w = p1.width() - 24.0;
        kit::heading(
            ui,
            &colors,
            Rect::from_min_size(pos2(inner_x, y), vec2(inner_w, kit::HEADING_HEIGHT)),
            t(tl::PinToScreenOptionsWindow_Size_and_opacity),
        );
        y += kit::HEADING_HEIGHT + 10.0;
        let number_row =
            |ui: &mut Ui, y: f32, label: &str, name: &str, value: &mut i64, min: i64, max: i64| {
                let r = Rect::from_min_size(pos2(inner_x, y), vec2(inner_w, line));
                let c = kit::columns(r, &[Track::Px(150.0), Track::Star(1.0)], 10.0);
                kit::label(ui, &colors, c[0], label);
                kit::int_at(
                    ui,
                    &colors,
                    c[1],
                    &format!("{:?}", id(name)),
                    value,
                    min,
                    max,
                    1,
                    true,
                );
            };
        number_row(
            ui,
            y,
            t(tl::PinToScreenOptionsWindow_Initial_scale),
            "scale",
            &mut d.initial_scale,
            20,
            500,
        );
        y += line + 10.0;
        number_row(
            ui,
            y,
            t(tl::PinToScreenOptionsWindow_Scale_step),
            "scale-step",
            &mut d.scale_step,
            1,
            100,
        );
        y += line + 10.0;
        number_row(
            ui,
            y,
            t(tl::PinToScreenOptionsWindow_Initial_opacity),
            "opacity",
            &mut d.initial_opacity,
            10,
            100,
        );
        y += line + 10.0;
        number_row(
            ui,
            y,
            t(tl::PinToScreenOptionsWindow_Opacity_step),
            "opacity-step",
            &mut d.opacity_step,
            1,
            90,
        );
        y += line + 10.0;
        let toggle_row = |ui: &mut Ui,
                          x: f32,
                          w: f32,
                          y: f32,
                          label: &str,
                          name: &str,
                          value: &mut bool,
                          enabled: bool| {
            let r = Rect::from_min_size(pos2(x, y), vec2(w, toggle_line));
            kit::text_at(
                ui,
                r,
                Align2::LEFT_CENTER,
                label,
                13.0,
                Weight::Normal,
                colors.text_secondary(),
            );
            kit::toggle_at(ui, &colors, r, id(name), value, enabled);
        };
        toggle_row(
            ui,
            inner_x,
            inner_w,
            y,
            t(tl::PinToScreenOptionsWindow_High_quality_scaling),
            "hq",
            &mut d.high_quality_scale,
            true,
        );

        // Left: placement.
        let p2_h = 12.0 * 2.0 + kit::HEADING_HEIGHT + 10.0 * 3.0 + line * 2.0 + toggle_line;
        let p2 = Rect::from_min_size(
            pos2(cols[0].left(), p1.bottom() + 12.0),
            vec2(cols[0].width(), p2_h),
        );
        kit::panel(ui, &colors, p2);
        let mut y = p2.top() + 12.0;
        kit::heading(
            ui,
            &colors,
            Rect::from_min_size(pos2(inner_x, y), vec2(inner_w, kit::HEADING_HEIGHT)),
            t(tl::PinToScreenOptionsWindow_Placement),
        );
        y += kit::HEADING_HEIGHT + 10.0;
        let r = Rect::from_min_size(pos2(inner_x, y), vec2(inner_w, line));
        let c = kit::columns(r, &[Track::Px(150.0), Track::Star(1.0)], 10.0);
        kit::label(
            ui,
            &colors,
            c[0],
            t(tl::PinToScreenOptionsWindow_Default_position),
        );
        let placements: Vec<(ContentAlignment, String)> = [
            (
                ContentAlignment::TopLeft,
                tl::PinToScreenOptionsViewModel_Top_left,
            ),
            (
                ContentAlignment::TopCenter,
                tl::PinToScreenOptionsViewModel_Top_center,
            ),
            (
                ContentAlignment::TopRight,
                tl::PinToScreenOptionsViewModel_Top_right,
            ),
            (
                ContentAlignment::MiddleLeft,
                tl::PinToScreenOptionsViewModel_Middle_left,
            ),
            (
                ContentAlignment::MiddleCenter,
                tl::PinToScreenOptionsViewModel_Center,
            ),
            (
                ContentAlignment::MiddleRight,
                tl::PinToScreenOptionsViewModel_Middle_right,
            ),
            (
                ContentAlignment::BottomLeft,
                tl::PinToScreenOptionsViewModel_Bottom_left,
            ),
            (
                ContentAlignment::BottomCenter,
                tl::PinToScreenOptionsViewModel_Bottom_center,
            ),
            (
                ContentAlignment::BottomRight,
                tl::PinToScreenOptionsViewModel_Bottom_right,
            ),
        ]
        .iter()
        .map(|(v, k)| (*v, t(*k).to_string()))
        .collect();
        if let Some(v) = kit::combo_at(
            ui,
            &colors,
            c[1],
            &format!("pin-placement-{key}"),
            &d.placement,
            &placements,
            true,
        ) {
            d.placement = v;
        }
        y += line + 10.0;
        let r = Rect::from_min_size(pos2(inner_x, y), vec2(inner_w, line));
        let c = kit::columns(r, &[Track::Px(150.0), Track::Star(1.0)], 10.0);
        kit::label(
            ui,
            &colors,
            c[0],
            t(tl::PinToScreenOptionsWindow_Screen_edge_offset),
        );
        kit::int_at(
            ui,
            &colors,
            c[1],
            &format!("pin-offset-{key}"),
            &mut d.placement_offset,
            0,
            300,
            1,
            true,
        );
        y += line + 10.0;
        toggle_row(
            ui,
            inner_x,
            inner_w,
            y,
            t(tl::PinToScreenOptionsWindow_Keep_center_while_resizing),
            "center",
            &mut d.keep_center_location,
            true,
        );

        // Right: appearance.
        let rx = cols[1].left() + 12.0;
        let rw = cols[1].width() - 24.0;
        let p3_h = 12.0 * 2.0 + kit::HEADING_HEIGHT + 10.0 * 5.0 + toggle_line * 3.0 + line * 2.0;
        let p3 = Rect::from_min_size(cols[1].min, vec2(cols[1].width(), p3_h));
        kit::panel(ui, &colors, p3);
        let mut y = p3.top() + 12.0;
        kit::heading(
            ui,
            &colors,
            Rect::from_min_size(pos2(rx, y), vec2(rw, kit::HEADING_HEIGHT)),
            t(tl::PinToScreenOptionsWindow_Appearance),
        );
        y += kit::HEADING_HEIGHT + 10.0;
        toggle_row(
            ui,
            rx,
            rw,
            y,
            t(tl::PinToScreenOptionsWindow_Always_on_top),
            "top",
            &mut d.top_most,
            true,
        );
        y += toggle_line + 10.0;
        toggle_row(
            ui,
            rx,
            rw,
            y,
            t(tl::PinToScreenOptionsWindow_Window_shadow),
            "shadow",
            &mut d.shadow,
            true,
        );
        y += toggle_line + 10.0;
        toggle_row(
            ui,
            rx,
            rw,
            y,
            t(tl::PinToScreenOptionsWindow_Image_border),
            "border",
            &mut d.border,
            true,
        );
        y += toggle_line + 10.0;
        let border_on = d.border;
        let r = Rect::from_min_size(pos2(rx, y), vec2(rw, line));
        let c = kit::columns(
            r,
            &[Track::Px(150.0), Track::Star(1.0), Track::Px(40.0)],
            10.0,
        );
        kit::text_at(
            ui,
            c[0],
            Align2::LEFT_CENTER,
            t(tl::PinToScreenOptionsWindow_Border_size),
            13.0,
            Weight::Normal,
            if border_on {
                colors.text_secondary()
            } else {
                colors.text_secondary().gamma_multiply(0.5)
            },
        );
        kit::int_at(
            ui,
            &colors,
            c[1],
            &format!("pin-border-{key}"),
            &mut d.border_size,
            1,
            30,
            1,
            border_on,
        );
        if let Some(c) = color_button(
            ui,
            &colors,
            c[2],
            id("border-color"),
            d.border_color,
            t(tl::PinToScreenOptionsWindow_Border_color),
            border_on,
        ) {
            d.border_color = c;
        }
        y += line + 10.0;
        let r = Rect::from_min_size(pos2(rx, y), vec2(rw, line));
        let c = kit::columns(
            r,
            &[Track::Px(150.0), Track::Star(1.0), Track::Px(40.0)],
            10.0,
        );
        kit::label(
            ui,
            &colors,
            c[0],
            t(tl::PinToScreenOptionsWindow_Background_color),
        );
        if let Some(c) = color_button(
            ui,
            &colors,
            c[2],
            id("background-color"),
            d.background_color,
            t(tl::PinToScreenOptionsWindow_Background_color),
            true,
        ) {
            d.background_color = c;
        }

        // Right: minimized preview.
        let p4_h = 12.0 * 2.0 + kit::HEADING_HEIGHT + 10.0 + line;
        let p4 = Rect::from_min_size(
            pos2(cols[1].left(), p3.bottom() + 12.0),
            vec2(cols[1].width(), p4_h),
        );
        kit::panel(ui, &colors, p4);
        let mut y = p4.top() + 12.0;
        kit::heading(
            ui,
            &colors,
            Rect::from_min_size(pos2(rx, y), vec2(rw, kit::HEADING_HEIGHT)),
            t(tl::PinToScreenOptionsWindow_Minimized_preview),
        );
        y += kit::HEADING_HEIGHT + 10.0;
        let times_w = kit::text_width(ui, "×", 13.0, Weight::Normal);
        let r = Rect::from_min_size(pos2(rx, y), vec2(rw, line));
        let c = kit::columns(
            r,
            &[
                Track::Px(150.0),
                Track::Star(1.0),
                Track::Px(times_w),
                Track::Star(1.0),
            ],
            8.0,
        );
        kit::label(
            ui,
            &colors,
            c[0],
            t(tl::PinToScreenOptionsWindow_Preview_size),
        );
        let Size { width, height } = d.minimize_size;
        if let Some(v) = kit::plain_number_at(ui, &colors, c[1], id("min-w"), width, 1, 1000, true)
        {
            d.minimize_size.width = v;
        }
        kit::text_at(
            ui,
            c[2],
            Align2::CENTER_CENTER,
            "×",
            13.0,
            Weight::Normal,
            colors.text_secondary(),
        );
        if let Some(v) = kit::plain_number_at(ui, &colors, c[3], id("min-h"), height, 1, 1000, true)
        {
            d.minimize_size.height = v;
        }

        // The footer: the hint, Save, Cancel.
        let items = [
            (None, Some(t(tl::PinToScreenOptionsWindow_Save))),
            (None, Some(t(tl::PinToScreenOptionsWindow_Cancel))),
        ];
        let rects = kit::right_aligned(ui, &colors, rows[1], &items, 8.0);
        let hint = Rect::from_min_max(
            pos2(rows[1].left(), rows[1].top() + 4.0),
            pos2(rects[0].left() - 8.0, rows[1].bottom()),
        );
        kit::text_at(
            ui,
            hint,
            Align2::LEFT_CENTER,
            t(WHEEL_HINT),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
        );
        let mut close = false;
        if kit::button_at(ui, &colors, rects[0], None, items[0].1, true).clicked() {
            let options = self.draft.clone();
            state.save_tool_options(&self.home, "PinToScreenOptions", &options);
            if let Some(pin) = self.pin
                && let Some(p) = state.tools.pins.iter_mut().find(|p| p.key == pin)
            {
                p.options = options;
                p.resize(p.options.keep_center_location);
            }
            close = true;
        }
        if kit::button_at(ui, &colors, rects[1], None, items[1].1, true).clicked() {
            close = true;
        }
        close
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// A colour button with Avalonia's `ColorView` in a flyout (the editor's approximation
/// of it).
fn color_button(
    ui: &mut Ui,
    colors: &crate::theme::Colors,
    rect: Rect,
    id: Id,
    value: Color,
    tip: &str,
    enabled: bool,
) -> Option<Color> {
    let response = kit::button_at(ui, colors, rect, None, None, enabled).on_hover_text(tip);
    let swatch = Rect::from_center_size(rect.center(), vec2(20.0, 20.0));
    ui.painter().rect_filled(swatch, 3.0, to32(value));
    ui.painter().rect_stroke(
        swatch,
        3.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let editor_colors = modlight_editor::ui::style::Colors {
        dark: colors.dark,
        palette: colors.palette,
    };
    let mut picked = None;
    egui::Popup::menu(&response)
        .id(id.with("flyout"))
        .frame(crate::menus::frame(colors))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let current = modlight_editor::color::Color::argb(value.a, value.r, value.g, value.b);
            if let Some(modlight_editor::ui::pickers::ColorEvent::Picked(c)) =
                modlight_editor::ui::pickers::color_panel(
                    ui,
                    &editor_colors,
                    id.with("panel"),
                    current,
                )
            {
                picked = Some(Color::argb(c.a, c.r, c.g, c.b));
            }
        });
    picked
}

// ------------------------------------------------------------------ the app's side

impl AppState {
    /// `PinToScreenManager.Pin` with the task's `PinToScreenOptions`, and the sound.
    pub fn pin(&mut self, source: PinSource, settings: &TaskSettings) {
        if source.image.width() == 0 || source.image.height() == 0 {
            return;
        }
        let options: PinToScreenOptions = super::jobs::tool_options(settings, "PinToScreenOptions");
        let home = OptionsHome::of(settings);
        let area = self.active_working_area();
        self.tools.next_key += 1;
        let mut pin = Pin::new(self.tools.next_key, source, options, home);
        pin.place(area);
        self.tools.pins.push(pin);
        self.play_task_sound(NotificationSound::ActionCompleted, settings);
    }

    /// `CaptureHelpers.GetActiveScreenWorkingArea`.
    pub fn active_working_area(&self) -> Rect {
        let area = self
            .captures
            .source
            .active_screen()
            .map(|s| s.working_area)
            .unwrap_or_default();
        if area.width <= 0 || area.height <= 0 {
            return Rect::from_min_size(pos2(0.0, 25.0), vec2(1440.0, 800.0));
        }
        Rect::from_min_size(
            pos2(area.x as f32, area.y as f32),
            vec2(area.width as f32, area.height as f32),
        )
    }

    /// `PinToScreen(image, location)` from a picture of the screen.
    pub fn pin_picture(&mut self, image: Arc<RgbaImage>, settings: &TaskSettings) {
        self.pin(
            PinSource {
                image,
                location: None,
                pixels_per_point: 1.0,
            },
            settings,
        );
    }

    /// `ClipboardHelpers.TryGetImage`: a picture, or a picture file, on the clipboard.
    pub fn clipboard_pin_source(&mut self) -> Option<PinSource> {
        use modlight_core::tasks::upload_manager::ClipboardContent;
        let image = match self.read_clipboard()? {
            ClipboardContent::Image(image) => image,
            ClipboardContent::Files(files) => files
                .iter()
                .find(|f| modlight_core::names::file_helpers::is_image_file(&f.to_string_lossy()))
                .and_then(|f| super::files::load_image(f))?,
            ClipboardContent::Text(_) => return None,
        };
        Some(PinSource {
            image: Arc::new(image),
            location: None,
            pixels_per_point: 1.0,
        })
    }

    /// `ImageHelpers.LoadImageWithFileDialog`.
    pub fn file_pin_source(&mut self) -> Option<PinSource> {
        let path = self.tool_open_file(
            t(tl::PinToScreenStartupWindow_Select_image),
            &[("Images", &super::files::IMAGE_EXTENSIONS)],
        )?;
        let image = super::files::load_image(&path)?;
        Some(PinSource {
            image: Arc::new(image),
            location: None,
            pixels_per_point: 1.0,
        })
    }

    /// `PinToScreen(taskSettings)`: the startup window.
    pub fn open_pin_startup(&mut self, settings: TaskSettings) {
        self.tools
            .open_keyed(|key| Box::new(PinStartupWindow::new(key, settings)));
    }

    /// `PinToScreen(filePath)`.
    pub fn pin_file(&mut self, path: &std::path::Path, settings: &TaskSettings) {
        if let Some(image) = super::files::load_image(path) {
            self.pin_picture(Arc::new(image), settings);
        }
    }

    /// `PinToScreenFromScreen`: a region, pinned where it was.
    pub fn pin_from_screen(&mut self, settings: TaskSettings) {
        self.tool_region(
            RegionFor::PinFromScreen(Box::new(settings.clone())),
            settings,
        );
    }

    /// `PinToScreenFromClipboard`.
    pub fn pin_from_clipboard(&mut self, settings: TaskSettings) {
        match self.clipboard_pin_source() {
            Some(source) => self.pin(source, &settings),
            None => {
                self.uploads
                    .windows
                    .messages
                    .push_back(crate::dialogs::message::MessageBox::new(
                        &format!("Modlight - {}", t(s::PinToScreen)),
                        t(s::ClipboardDoesNotContainAnImage),
                        MessageIcon::Information,
                    ))
            }
        }
    }

    /// `PinToScreenFromFile`.
    pub fn pin_from_file(&mut self, settings: TaskSettings) {
        if let Some(source) = self.file_pin_source() {
            self.pin(source, &settings);
        }
    }

    /// `PinToScreenCloseAll`.
    pub fn pin_close_all(&mut self, settings: TaskSettings) {
        self.tools.pins.clear();
        self.play_task_sound(NotificationSound::ActionCompleted, &settings);
    }

    /// A pinned picture's options button.
    pub fn open_pin_options(&mut self, pin: Option<u64>) {
        let (draft, home) = match pin.and_then(|k| self.tools.pins.iter().find(|p| p.key == k)) {
            Some(p) => (p.options.clone(), p.home.clone()),
            None => {
                let settings = self.default_task_settings();
                (
                    super::jobs::tool_options(&settings, "PinToScreenOptions"),
                    OptionsHome::Defaults,
                )
            }
        };
        self.tools.open_keyed(|key| {
            Box::new(PinOptionsWindow {
                key,
                pin,
                home,
                draft,
                close: false,
            })
        });
    }

    /// A region for pinning arrived (from the startup window or the hotkey).
    pub(super) fn pin_region_done(
        &mut self,
        startup: Option<u64>,
        settings: Option<TaskSettings>,
        picture: super::region::ScreenPicture,
    ) {
        let scale = self
            .captures
            .source
            .screens()
            .iter()
            .map(|s| s.scale)
            .fold(1.0f64, f64::max) as f32;
        let source = picture.map(|(image, rect)| {
            let ppp = if rect.width > 0 {
                image.width() as f32 / rect.width as f32
            } else {
                scale
            };
            PinSource {
                image,
                location: Some(pos2(rect.x as f32, rect.y as f32)),
                pixels_per_point: ppp.max(0.01),
            }
        });
        match (startup, settings) {
            (Some(key), _) => {
                self.tools.restore.push(key);
                super::with_window::<PinStartupWindow, _>(self, key, |window, state| {
                    window.selected(
                        state,
                        source,
                        t(tl::PinToScreenStartupViewModel_No_region_selected),
                    );
                });
            }
            (None, Some(settings)) => {
                if let Some(source) = source {
                    self.pin(source, &settings);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin() -> Pin {
        Pin::new(
            1,
            PinSource {
                image: Arc::new(RgbaImage::new(300, 200)),
                location: None,
                pixels_per_point: 1.0,
            },
            PinToScreenOptions::default(),
            OptionsHome::Defaults,
        )
    }

    /// One frame of the pinned picture with the pointer over it and these events.
    fn frame(pin: &mut Pin, events: Vec<egui::Event>, modifiers: egui::Modifiers) -> Vec<PinAsk> {
        let colors = crate::qa::state(&crate::qa::Look::default(), false).colors;
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::fonts());
        let screen = Rect::from_min_size(pos2(0.0, 0.0), pin.size);
        let mut asks = Vec::new();
        let mut events = events;
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        for events in [
            vec![egui::Event::PointerMoved(pos2(150.0, 150.0))],
            events,
        ] {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..egui::RawInput::default()
            };
            ctx.run_ui(input, |ui| {
                asks = pin.ui(ui, &colors, 10.0);
            })
            .drop_without_applying_deltas();
        }
        asks
    }

    fn wheel(x: f32, y: f32) -> egui::Event {
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(x, y),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// `OnPointerWheelChanged`: the direction is the sign of `Delta.Y`, and an event
    /// without a vertical part (a sideways swipe, a trackpad gesture starting or ending)
    /// changes nothing.
    #[test]
    fn only_the_vertical_wheel_scales_and_fades() {
        let mut pin = pin();
        frame(&mut pin, vec![wheel(6.0, 0.0)], egui::Modifiers::NONE);
        assert_eq!(pin.state.scale, 100);
        frame(&mut pin, vec![wheel(0.0, 0.0)], egui::Modifiers::NONE);
        assert_eq!(pin.state.scale, 100);
        frame(&mut pin, vec![wheel(0.0, 4.0)], egui::Modifiers::NONE);
        assert_eq!(pin.state.scale, 110);
        frame(&mut pin, vec![wheel(0.0, -4.0)], egui::Modifiers::NONE);
        assert_eq!(pin.state.scale, 100);
        // A gesture ending (zero) after a step down keeps the step.
        frame(
            &mut pin,
            vec![wheel(0.0, -4.0), wheel(0.0, 0.0), wheel(0.0, 0.0)],
            egui::Modifiers::NONE,
        );
        assert_eq!(pin.state.scale, 90);

        pin.state.opacity = 50;
        frame(&mut pin, vec![wheel(-3.0, 0.0)], egui::Modifiers::COMMAND);
        assert_eq!(pin.state.opacity, 50);
        frame(&mut pin, vec![wheel(0.0, 2.0)], egui::Modifiers::COMMAND);
        assert_eq!(pin.state.opacity, 60);
    }
}
