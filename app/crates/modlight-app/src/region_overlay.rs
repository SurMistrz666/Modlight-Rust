//! The region capture overlay (`RegionCaptureWindow`, `RegionSelectionOverlay`,
//! `MagnifierPixelGrid`, and `RegionCaptureLightForm` for the light and transparent
//! kinds): the frozen desktop, the dim outside the selection, the marching rectangle and
//! its handles, the magnifier with the pointer's coordinates, the selection's size, and
//! the toolbar with the region tool.
//!
//! macOS keeps a window on one display, so the overlay is one borderless window per
//! display it covers, all drawing the same session (`modlight_core::capture::region`).
//! Geometry that upstream draws in image pixels (handles, the rectangle) stays in image
//! pixels; the HUD is in points, as upstream's is.

use std::sync::Arc;

use egui::{
    Color32, ColorImage, Pos2, Rect, Shape, Stroke, StrokeKind, TextureHandle, Ui, pos2, vec2,
};
use image::RgbaImage;
use modlight_core::capture::geometry::{IRect, Point};
use modlight_core::capture::region::{Button, Key, RegionKind, RegionSession};
use modlight_core::capture::source::ScreenInfo;
use modlight_core::icons;
use modlight_core::recording::ScreenRecordOutput;
use modlight_core::tasks::settings::TaskSettings;

use crate::theme::{Colors, Weight};
use crate::widgets::{Content, Look, button_at, layout, trimmed};

/// What the chosen area is for.
#[derive(Clone, Debug)]
pub enum OverlayPurpose {
    /// A capture: the picture becomes a task.
    Capture,
    /// The area of a screen recording (`RegionCaptureTasks.GetRectangleRegion`).
    Recording(ScreenRecordOutput),
    /// The auto capture's area (`AutoCaptureWindow.SelectRegion`).
    AutoCaptureRegion,
    /// The scrolling capture's area.
    ScrollingRegion,
    /// The task settings' preconfigured region (`Select region...`).
    CustomRegion(crate::task_pages::TaskTarget),
}

/// The overlay and what it will do with its answer.
pub struct RegionOverlay {
    pub session: RegionSession,
    pub purpose: OverlayPurpose,
    pub settings: TaskSettings,
    /// UI seconds when it opened: keys count after `InputDelay`.
    pub opened_at: f64,
    texture: Option<TextureHandle>,
    cursor_texture: Option<TextureHandle>,
    magnifier: Option<(TextureHandle, (i64, i64, i64))>,
    /// The press that opened a double click (UI seconds and image point).
    last_click: Option<(f64, Point)>,
}

const MAGNIFIER_SIZE: f32 = 150.0;

impl RegionOverlay {
    pub fn new(
        session: RegionSession,
        purpose: OverlayPurpose,
        settings: TaskSettings,
        now: f64,
    ) -> RegionOverlay {
        RegionOverlay {
            session,
            purpose,
            settings,
            opened_at: now,
            texture: None,
            cursor_texture: None,
            magnifier: None,
            last_click: None,
        }
    }

    /// The displays the overlay covers, each getting its own window.
    pub fn screens(&self) -> Vec<ScreenInfo> {
        let bounds = self.session.request.screen_bounds;
        let mut screens: Vec<ScreenInfo> = self
            .session
            .request
            .screens
            .iter()
            .filter(|s| s.bounds.intersect(&bounds).has_area())
            .cloned()
            .collect();
        if screens.is_empty() {
            screens.push(ScreenInfo {
                id: 0,
                bounds,
                working_area: bounds,
                scale: self.session.request.scale,
                primary: true,
                name: String::new(),
            });
        }
        screens
    }

    /// `InputDelay`: keys are taken this long after the overlay opened.
    pub fn tick(&mut self, now: f64) {
        let delay = self.session.request.options.input_delay.max(0) as f64 / 1000.0;
        if !self.session.keyboard_enabled && now - self.opened_at >= delay {
            self.session.keyboard_enabled = true;
        }
    }

    /// A point of a display's window (points, from its top left) in image pixels.
    fn to_image(&self, screen: &ScreenInfo, local: Pos2) -> Point {
        let bounds = self.session.request.screen_bounds;
        let scale = self.session.request.scale;
        Point::new(
            ((screen.bounds.x - bounds.x) as f64 + local.x as f64) * scale,
            ((screen.bounds.y - bounds.y) as f64 + local.y as f64) * scale,
        )
    }

    /// An image point in a display window's points.
    fn to_local(&self, screen: &ScreenInfo, origin: Pos2, point: Point) -> Pos2 {
        let bounds = self.session.request.screen_bounds;
        let scale = self.session.request.scale;
        pos2(
            origin.x + (point.x / scale - (screen.bounds.x - bounds.x) as f64) as f32,
            origin.y + (point.y / scale - (screen.bounds.y - bounds.y) as f64) as f32,
        )
    }

    fn texture(&mut self, ctx: &egui::Context) -> Option<TextureHandle> {
        if self.texture.is_none()
            && let Some(image) = &self.session.request.image
        {
            let color = ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                image.as_raw(),
            );
            self.texture =
                Some(ctx.load_texture("region-overlay", color, egui::TextureOptions::NEAREST));
        }
        self.texture.clone()
    }

    /// Hear the window's pointer, wheel and keys.
    pub fn input(&mut self, ui: &Ui, screen: &ScreenInfo, now: f64) {
        let origin = ui.max_rect().min;
        let events = ui.input(|i| i.events.clone());
        let modifiers = ui.input(|i| i.modifiers);
        for event in events {
            match event {
                egui::Event::PointerMoved(pos) => {
                    let point = self.to_image(screen, pos - origin.to_vec2());
                    self.session.pointer_moved(point);
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } => {
                    let point = self.to_image(screen, pos - origin.to_vec2());
                    let button = match button {
                        egui::PointerButton::Primary => Button::Left,
                        egui::PointerButton::Secondary => Button::Right,
                        egui::PointerButton::Middle => Button::Middle,
                        egui::PointerButton::Extra1 => Button::X1,
                        egui::PointerButton::Extra2 => Button::X2,
                    };
                    if pressed {
                        let mut clicks = 1;
                        if button == Button::Left {
                            if let Some((at, p)) = self.last_click
                                && now - at < 0.5
                                && p.distance(point) < 4.0
                            {
                                clicks = 2;
                            }
                            self.last_click = Some((now, point));
                        }
                        self.session.pointer_pressed(button, point, clicks);
                    } else {
                        self.session.pointer_released(button, point);
                    }
                }
                egui::Event::MouseWheel { delta, .. } => self.session.wheel(delta.y as f64),
                egui::Event::Key {
                    key, pressed: true, ..
                } => {
                    let mapped = match key {
                        egui::Key::Enter => Some(Key::Enter),
                        egui::Key::Space => Some(Key::Space),
                        egui::Key::Backtick => Some(Key::Tilde),
                        egui::Key::ArrowLeft => Some(Key::Left),
                        egui::Key::ArrowRight => Some(Key::Right),
                        egui::Key::ArrowUp => Some(Key::Up),
                        egui::Key::ArrowDown => Some(Key::Down),
                        egui::Key::Delete | egui::Key::Backspace => Some(Key::Delete),
                        egui::Key::Escape => Some(Key::Escape),
                        // Ctrl+H upstream; Cmd+H hides the app on a Mac, so Control+H.
                        egui::Key::H if modifiers.ctrl && !modifiers.command => {
                            Some(Key::ToggleToolbar)
                        }
                        _ => None,
                    };
                    if let Some(key) = mapped {
                        self.session.key(key, modifiers.shift);
                    }
                }
                _ => {}
            }
        }
    }

    /// Draw this display's part of the overlay, and the toolbar on the display under
    /// the pointer. Returns whether the region tool button was clicked.
    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors, screen: &ScreenInfo) {
        let ctx = ui.ctx().clone();
        let full = ui.max_rect();
        let painter = ui.painter().clone();
        let scale = self.session.request.scale as f32;
        let kind = self.session.request.kind;
        let texture = if kind == RegionKind::Transparent {
            None
        } else {
            self.texture(&ctx)
        };
        let session = &self.session;

        // The frozen desktop (or, for the transparent kind, almost nothing: a window
        // with alpha 1/255, which still receives the pointer as upstream's layered one).
        if kind == RegionKind::Transparent {
            painter.rect_filled(full, 0.0, Color32::from_black_alpha(1));
        } else if let Some(texture) = texture {
            let (iw, ih) = (session.image_size.0 as f32, session.image_size.1 as f32);
            let top_left = self.to_image(screen, pos2(0.0, 0.0));
            let bottom_right = self.to_image(screen, full.size().to_pos2());
            let uv = Rect::from_min_max(
                pos2(top_left.x as f32 / iw, top_left.y as f32 / ih),
                pos2(bottom_right.x as f32 / iw, bottom_right.y as f32 / ih),
            );
            painter.image(texture.id(), full, uv, Color32::WHITE);
        } else {
            painter.rect_filled(full, 0.0, Color32::BLACK);
        }
        let session = &self.session;

        if session.is_light() {
            // `RegionCaptureLightForm`: only the dotted rectangle while dragging.
            if session.light_dragging()
                && session.selection.width > 5.0
                && session.selection.height > 5.0
            {
                let r = self.image_rect_local(screen, full.min, session.selection);
                dotted_rect(&painter, r, Color32::WHITE);
            }
            return;
        }

        // The pointer, when the capture shows it (inserted as a picture upstream).
        if let Some((cursor, at)) = session.request.cursor.clone() {
            let min = self.to_local(screen, full.min, at);
            let texture = self.cursor_texture.get_or_insert_with(|| {
                let color = ColorImage::from_rgba_unmultiplied(
                    [cursor.width() as usize, cursor.height() as usize],
                    cursor.as_raw(),
                );
                ctx.load_texture("region-cursor", color, egui::TextureOptions::LINEAR)
            });
            let size = vec2(
                cursor.width() as f32 / scale,
                cursor.height() as f32 / scale,
            );
            painter.image(
                texture.id(),
                Rect::from_min_size(min, size),
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        let session = &self.session;

        // `RegionSelectionOverlay.Render`.
        let dim = Color32::from_black_alpha(session.request.options.dim_alpha());
        let active = session
            .active_rect()
            .intersect(&modlight_core::capture::geometry::Rect::new(
                0.0,
                0.0,
                session.image_size.0,
                session.image_size.1,
            ));
        if session.region_tool_active {
            if dim.a() > 0 {
                if active.is_valid() {
                    let clear = self.image_rect_local(screen, full.min, active);
                    for part in outside(full, clear) {
                        painter.rect_filled(part, 0.0, dim);
                    }
                } else {
                    painter.rect_filled(full, 0.0, dim);
                }
            }
            if active.is_valid() {
                let r = self.image_rect_local(screen, full.min, active);
                // Half a pixel in, one pixel narrower: the accent line, then the ants.
                let px = 1.0 / scale;
                let ant = Rect::from_min_max(
                    r.min + vec2(px * 0.5, px * 0.5),
                    r.max - vec2(px * 0.5, px * 0.5),
                );
                painter.rect_stroke(
                    ant,
                    0.0,
                    Stroke::new(px, colors.accent_start()),
                    StrokeKind::Middle,
                );
                ants(&painter, ant, px);
                if session.show_handles && session.selection.is_valid() {
                    for p in session.selection.handle_points() {
                        let c = self.to_local(screen, full.min, p);
                        let size = 8.0 / scale;
                        let handle = Rect::from_center_size(c, vec2(size, size));
                        painter.rect_filled(handle, 0.0, Color32::WHITE);
                        painter.rect_stroke(
                            handle,
                            0.0,
                            Stroke::new(px, Color32::BLACK),
                            StrokeKind::Inside,
                        );
                    }
                }
            }
        }

        // The HUD, in points.
        let pointer_local = self.to_local(screen, full.min, session.last_point);
        let pointer_here = full.contains(pointer_local);
        let show_magnifier = session.show_magnifier && session.region_tool_active;
        let show_info = session.request.options.show_info && session.region_tool_active;
        if (show_magnifier || show_info) && pointer_here {
            let info_text = session.pointer_info();
            let info = layout(
                ui,
                trimmed(&info_text, 12.0, Weight::SemiBold, colors.text(), 400.0, 1),
            );
            let info_size = vec2(info.size().x + 14.0 + 2.0, info.size().y + 8.0 + 2.0);
            let width = if show_magnifier { 152.0 } else { info_size.x };
            let height = if show_magnifier {
                152.0 + if show_info { 10.0 + info_size.y } else { 0.0 }
            } else {
                info_size.y
            };
            let panel = position_near(full, pointer_local, vec2(width, height), 18.0);
            let mut y = panel.top();
            if show_magnifier {
                let center = pos2(panel.left() + 76.0, y + 76.0);
                painter.circle_filled(center, 76.0, Color32::WHITE);
                if let Some(texture) = self.magnifier_texture(&ctx, scale) {
                    painter.image(
                        texture.id(),
                        Rect::from_center_size(center, vec2(MAGNIFIER_SIZE, MAGNIFIER_SIZE)),
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                painter.circle_stroke(center, 74.5, Stroke::new(1.0, Color32::BLACK));
                y += 152.0 + 10.0;
            }
            if show_info {
                let x = if show_magnifier {
                    panel.center().x - info_size.x / 2.0
                } else {
                    panel.left()
                };
                let r = Rect::from_min_size(pos2(x, y), info_size);
                painter.rect_filled(r, 5.0, colors.background_main());
                painter.rect_stroke(
                    r,
                    5.0,
                    Stroke::new(1.0, colors.border()),
                    StrokeKind::Inside,
                );
                painter.galley(
                    pos2(r.center().x - info.size().x / 2.0, r.top() + 1.0 + 4.0),
                    info,
                    colors.text(),
                );
            }
        }
        let session = &self.session;
        if let Some(text) = session.selection_info() {
            // `UpdateSelectionInfo`: under the selection, or above it near the bottom.
            let galley = layout(
                ui,
                trimmed(&text, 12.0, Weight::SemiBold, colors.text(), 400.0, 1),
            );
            let size = vec2(galley.size().x + 16.0, galley.size().y + 10.0);
            let sel = self.image_rect_local(screen, full.min, session.selection);
            let x = sel
                .left()
                .clamp(full.left(), (full.right() - 100.0).max(full.left()));
            let mut y = sel.bottom() + 8.0;
            if y + 36.0 > full.bottom() {
                y = (sel.top() - 36.0).max(full.top());
            }
            if full.contains(pos2(x, y)) || full.contains(pos2(x + size.x, y + size.y)) {
                let r = Rect::from_min_size(pos2(x, y), size);
                painter.rect_filled(r, 5.0, colors.background_main());
                painter.rect_stroke(
                    r,
                    5.0,
                    Stroke::new(1.0, colors.border()),
                    StrokeKind::Inside,
                );
                painter.galley(r.min + vec2(8.0, 5.0), galley, colors.text());
            }
        }
    }

    /// The toolbar, top centre of the display under the pointer (`UpdateToolbarPosition`).
    /// Returns true when the region tool was clicked.
    pub fn toolbar(
        &mut self,
        ui: &mut Ui,
        colors: &Colors,
        screen: &ScreenInfo,
        pointer_screen: bool,
    ) -> bool {
        if !pointer_screen || self.session.toolbar_hidden || self.session.is_light() {
            return false;
        }
        let full = ui.max_rect();
        let painter = ui.painter().clone();
        // `Border Padding="4,4,0,4" BorderThickness="1,1,0,1" CornerRadius="0,0,0,4"`, a
        // 40 by 40 toolbar button inside. The annotation toolbar beside it arrives with
        // the image editor (phase 4).
        let size = vec2(1.0 + 4.0 + 40.0, 1.0 + 4.0 + 40.0 + 4.0 + 1.0);
        let r = Rect::from_min_size(pos2(full.center().x - size.x / 2.0, full.top()), size);
        painter.rect_filled(
            r,
            egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: 4,
                se: 0,
            },
            colors.background_toolbar(),
        );
        let stroke = Stroke::new(1.0, colors.border());
        painter.line_segment(
            [
                r.left_top() + vec2(0.5, 0.0),
                r.left_bottom() + vec2(0.5, -4.0),
            ],
            stroke,
        );
        painter.line_segment(
            [
                r.left_top() + vec2(0.0, 0.5),
                r.right_top() + vec2(0.0, 0.5),
            ],
            stroke,
        );
        painter.line_segment(
            [
                r.left_bottom() + vec2(4.0, -0.5),
                r.right_bottom() + vec2(0.0, -0.5),
            ],
            stroke,
        );
        let look = Look::primary(colors)
            .width(40.0)
            .padding(0.0)
            .icon(18.0, None);
        let button = Rect::from_min_size(r.min + vec2(1.0 + 4.0, 1.0 + 4.0), vec2(40.0, 40.0));
        let _ = screen;
        button_at(ui, colors, &look, Content::icon(icons::SCAN), true, button)
            .on_hover_text("Rectangle region")
            .clicked()
    }

    fn image_rect_local(
        &self,
        screen: &ScreenInfo,
        origin: Pos2,
        rect: modlight_core::capture::geometry::Rect,
    ) -> Rect {
        let min = self.to_local(screen, origin, Point::new(rect.left(), rect.top()));
        let max = self.to_local(screen, origin, Point::new(rect.right(), rect.bottom()));
        Rect::from_min_max(min, max)
    }

    /// The magnifier's picture: the pixels around the pointer, nearest neighbour, the
    /// grid, the centre cell's outline, round (`UpdateMagnifier`, `MagnifierPixelGrid`).
    fn magnifier_texture(
        &mut self,
        ctx: &egui::Context,
        render_scale: f32,
    ) -> Option<TextureHandle> {
        let image = self.session.request.image.clone()?;
        let count = self.session.magnifier_pixel_count.max(1);
        let cx = self.session.last_point.x.round() as i64;
        let cy = self.session.last_point.y.round() as i64;
        let key = (cx, cy, count);
        if let Some((texture, at)) = &self.magnifier
            && *at == key
        {
            return Some(texture.clone());
        }
        let picture = magnifier_picture(&image, cx, cy, count, render_scale.max(1.0));
        let color = ColorImage::from_rgba_unmultiplied(
            [picture.width() as usize, picture.height() as usize],
            picture.as_raw(),
        );
        let texture = ctx.load_texture("region-magnifier", color, egui::TextureOptions::NEAREST);
        self.magnifier = Some((texture.clone(), key));
        Some(texture)
    }

    /// The area chosen, in desktop points, when the overlay completed.
    pub fn completed_screen_rect(&self) -> Option<IRect> {
        match &self.session.outcome {
            Some(modlight_core::capture::region::RegionOutcome::Completed {
                screen_rect, ..
            }) => Some(*screen_rect),
            _ => None,
        }
    }
}

/// The magnifier's pixels at `scale` display pixels per point.
pub fn magnifier_picture(image: &RgbaImage, cx: i64, cy: i64, count: i64, scale: f32) -> RgbaImage {
    let size = (MAGNIFIER_SIZE * scale).round() as u32;
    let radius = count / 2;
    let cell = size as f32 / count as f32;
    let (w, h) = (image.width() as i64, image.height() as i64);
    let mut out = RgbaImage::new(size, size);
    let snap = |v: f32| v.round() as i64;
    for py in 0..size {
        for px in 0..size {
            let ix = ((px as f32 / cell) as i64).min(count - 1);
            let iy = ((py as f32 / cell) as i64).min(count - 1);
            let sx = (cx + ix - radius).clamp(0, w - 1);
            let sy = (cy + iy - radius).clamp(0, h - 1);
            let mut p = *image.get_pixel(sx as u32, sy as u32);
            p.0[3] = 255;
            out.put_pixel(px, py, p);
        }
    }
    // Grid lines: the physical pixel before each cell boundary, black at 75/255.
    let blend = |out: &mut RgbaImage, x: u32, y: u32, color: [u8; 3], alpha: f32| {
        if x < size && y < size {
            let p = out.get_pixel_mut(x, y);
            for (channel, target) in p.0.iter_mut().zip(color) {
                *channel = (*channel as f32 * (1.0 - alpha) + target as f32 * alpha).round() as u8;
            }
        }
    };
    for index in 1..count {
        let x = snap(index as f32 * cell) - 1;
        for y in 0..size {
            blend(&mut out, x.max(0) as u32, y, [0, 0, 0], 75.0 / 255.0);
        }
        let y = snap(index as f32 * cell) - 1;
        for x in 0..size {
            blend(&mut out, x, y.max(0) as u32, [0, 0, 0], 75.0 / 255.0);
        }
    }
    // The centre cell: a black outline one pixel early, a white one inside it.
    let c = radius;
    let left = snap(c as f32 * cell) - 1;
    let top = snap(c as f32 * cell) - 1;
    let right = snap((c + 1) as f32 * cell);
    let bottom = snap((c + 1) as f32 * cell);
    let mut outline = |l: i64, t: i64, r: i64, b: i64, color: [u8; 3]| {
        for x in l..r {
            blend(&mut out, x.max(0) as u32, t.max(0) as u32, color, 1.0);
            blend(&mut out, x.max(0) as u32, (b - 1).max(0) as u32, color, 1.0);
        }
        for y in t..b {
            blend(&mut out, l.max(0) as u32, y.max(0) as u32, color, 1.0);
            blend(&mut out, (r - 1).max(0) as u32, y.max(0) as u32, color, 1.0);
        }
    };
    outline(left, top, right, bottom, [0, 0, 0]);
    outline(left + 1, top + 1, right - 1, bottom - 1, [255, 255, 255]);
    // Round: transparent outside the circle, with a soft edge.
    let r = size as f32 / 2.0;
    for py in 0..size {
        for px in 0..size {
            let dx = px as f32 + 0.5 - r;
            let dy = py as f32 + 0.5 - r;
            let d = (dx * dx + dy * dy).sqrt();
            let alpha = (r - d + 0.5).clamp(0.0, 1.0);
            let p = out.get_pixel_mut(px, py);
            p.0[3] = (alpha * 255.0) as u8;
        }
    }
    out
}

/// The four rectangles around `clear` (`DrawDimmedOutside`).
fn outside(surface: Rect, clear: Rect) -> Vec<Rect> {
    let mut parts = Vec::new();
    let clear = clear.intersect(surface);
    let push = |parts: &mut Vec<Rect>, r: Rect| {
        if r.width() > 0.0 && r.height() > 0.0 {
            parts.push(r);
        }
    };
    push(
        &mut parts,
        Rect::from_min_max(surface.min, pos2(surface.right(), clear.top())),
    );
    push(
        &mut parts,
        Rect::from_min_max(
            pos2(surface.left(), clear.top()),
            pos2(clear.left(), clear.bottom()),
        ),
    );
    push(
        &mut parts,
        Rect::from_min_max(
            pos2(clear.right(), clear.top()),
            pos2(surface.right(), clear.bottom()),
        ),
    );
    push(
        &mut parts,
        Rect::from_min_max(pos2(surface.left(), clear.bottom()), surface.max),
    );
    parts
}

/// The dashed black pen of the marching rectangle: 5 on, 5 off, alpha 230.
fn ants(painter: &egui::Painter, r: Rect, px: f32) {
    let stroke = Stroke::new(px, Color32::from_rgba_unmultiplied(0, 0, 0, 230));
    let dash = 5.0 * px;
    for (a, b) in [
        (r.left_top(), r.right_top()),
        (r.left_top(), r.left_bottom()),
        (r.right_top(), r.right_bottom()),
        (r.left_bottom(), r.right_bottom()),
    ] {
        painter.extend(Shape::dashed_line(&[a, b], stroke, dash, dash));
    }
}

/// `DrawDottedRectangle`: a white line, black dashes over it.
fn dotted_rect(painter: &egui::Painter, r: Rect, color: Color32) {
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, color), StrokeKind::Inside);
    ants(painter, r.shrink(0.5), 1.0);
}

/// `PositionPanelNearPointer`.
fn position_near(bounds: Rect, pointer: Pos2, size: egui::Vec2, offset: f32) -> Rect {
    let mut x = pointer.x + offset;
    let mut y = pointer.y + offset;
    if x + size.x > bounds.right() {
        x = pointer.x - size.x - offset;
    }
    if y + size.y > bounds.bottom() {
        y = pointer.y - size.y - offset;
    }
    let x = x.clamp(bounds.left(), (bounds.right() - size.x).max(bounds.left()));
    let y = y.clamp(bounds.top(), (bounds.bottom() - size.y).max(bounds.top()));
    Rect::from_min_size(pos2(x.round(), y.round()), size)
}

/// A frozen picture for the overlay from an image the size of a display.
pub fn shared(image: RgbaImage) -> Arc<RgbaImage> {
    Arc::new(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_magnifier_is_round_with_a_marked_centre() {
        let image = RgbaImage::from_fn(40, 40, |x, y| {
            image::Rgba([(x * 6) as u8, (y * 6) as u8, 0, 255])
        });
        let picture = magnifier_picture(&image, 20, 20, 15, 1.0);
        assert_eq!(picture.dimensions(), (150, 150));
        assert_eq!(picture.get_pixel(0, 0).0[3], 0, "outside the circle");
        assert_eq!(picture.get_pixel(75, 75).0[3], 255);
        // The centre cell (index 7 of 15, 10 px each) has a black outline at 69.
        assert_eq!(picture.get_pixel(69, 75).0[..3], [0, 0, 0]);
        assert_eq!(picture.get_pixel(70, 75).0[..3], [255, 255, 255]);
    }

    #[test]
    fn panels_flip_away_from_the_edges() {
        let bounds = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let r = position_near(bounds, pos2(100.0, 100.0), vec2(152.0, 188.0), 18.0);
        assert_eq!(r.min, pos2(118.0, 118.0));
        let r = position_near(bounds, pos2(780.0, 590.0), vec2(152.0, 188.0), 18.0);
        assert_eq!(r.min, pos2(610.0, 384.0));
        assert_eq!(
            outside(
                bounds,
                Rect::from_min_size(pos2(100.0, 100.0), vec2(50.0, 50.0))
            )
            .len(),
            4
        );
        assert_eq!(outside(bounds, bounds).len(), 0);
    }
}
