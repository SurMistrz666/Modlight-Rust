//! `ScreenColorPickerWindow` (ShareX.Avalonia) on a Mac: the colour under the pointer,
//! a 15 x 15 magnifier and the info text beside the pointer; a click picks, Cmd+click
//! picks with the "Ctrl" format, a right click or Escape gives up.
//!
//! Upstream lays a transparent window over the live desktop and reads pixels with GDI as
//! the pointer moves. macOS gives pixels only through ScreenCaptureKit, so the desktop is
//! taken once when the picker opens (the same frozen picture the region capture uses)
//! and the picker reads that; one borderless window per display shows it.

use std::sync::Arc;

use egui::{
    Align2, Color32, Id, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle, Ui, ViewportBuilder,
    ViewportCommand, ViewportId, pos2, vec2,
};
use image::RgbaImage;
use modlight_core::capture::geometry::IRect;
use modlight_core::capture::source::{CaptureError, ScreenInfo};
use modlight_core::l10n::{Arg, format, s, t};
use modlight_core::tasks::host::{MessageIcon, NotificationSound};
use modlight_core::tasks::settings::TaskSettings;
use modlight_tools::color::Color;
use modlight_tools::color_format;
use modlight_tools::settings::ScreenColorPickerOptions;

use crate::state::AppState;
use crate::theme::{Colors, Weight};
use crate::widgets::{layout, wrapped};

/// `PreviewOffset` and `MagnifierPixelCount`.
const PREVIEW_OFFSET: f32 = 18.0;
pub const MAGNIFIER_PIXELS: i64 = 15;

/// Where the picked colour goes.
#[derive(Clone, Debug)]
pub enum PickTarget {
    /// `OpenScreenColorPicker`: formatted onto the clipboard.
    Copy(Box<TaskSettings>),
    /// The colour picker window's pipette.
    ColorPicker(u64),
    /// The image editor's colour pickers.
    Editor {
        key: u64,
        target: modlight_editor::editor::ColorTarget,
    },
}

/// A pick waiting for its delay (the asking window gets out of the way first).
pub struct PendingPick {
    pub at: f64,
    pub target: PickTarget,
    pub options: ScreenColorPickerOptions,
}

/// The frozen desktop the picker reads.
#[derive(Clone)]
pub struct Desktop {
    pub image: Arc<RgbaImage>,
    /// Every display together, in points.
    pub bounds: IRect,
    /// Image pixels per point.
    pub scale: f64,
    pub screens: Vec<ScreenInfo>,
    pub pointer: (i32, i32),
}

/// What a pick came to: the colour, where (desktop points), and whether Cmd was held.
pub type PickResult = Option<(Color, (i64, i64), bool)>;

pub struct ScreenPicker {
    pub target: PickTarget,
    pub options: ScreenColorPickerOptions,
    pub desktop: Desktop,
    /// The pointer, desktop points.
    pub pointer: (f64, f64),
    pub color: Color,
    pub position: (i64, i64),
    pressed: Option<(Color, (i64, i64))>,
    cancel_pressed: bool,
    /// One picture per display (a crop of the desktop, fitted to the GPU's limit).
    textures: Vec<(u32, TextureHandle)>,
    magnifier: Option<(TextureHandle, (i64, i64))>,
    pub result: Option<PickResult>,
}

impl ScreenPicker {
    pub fn new(
        target: PickTarget,
        options: ScreenColorPickerOptions,
        desktop: Desktop,
    ) -> ScreenPicker {
        let pointer = (desktop.pointer.0 as f64, desktop.pointer.1 as f64);
        let mut picker = ScreenPicker {
            target,
            options,
            desktop,
            pointer,
            color: Color::BLACK,
            position: (0, 0),
            pressed: None,
            cancel_pressed: false,
            textures: Vec::new(),
            magnifier: None,
            result: None,
        };
        picker.update(pointer);
        picker
    }

    /// The pixel of the frozen desktop under a desktop point (black outside it, as GDI
    /// gives for an area no display covers).
    pub fn pixel(&self, px: i64, py: i64) -> Color {
        let image = &self.desktop.image;
        if px < 0 || py < 0 || px >= image.width() as i64 || py >= image.height() as i64 {
            return Color::BLACK;
        }
        let p = image.get_pixel(px as u32, py as u32).0;
        Color::rgb(p[0], p[1], p[2])
    }

    /// The image pixel under a desktop point.
    fn image_point(&self, point: (f64, f64)) -> (i64, i64) {
        (
            ((point.0 - self.desktop.bounds.x as f64) * self.desktop.scale).floor() as i64,
            ((point.1 - self.desktop.bounds.y as f64) * self.desktop.scale).floor() as i64,
        )
    }

    /// `UpdatePicker`.
    pub fn update(&mut self, point: (f64, f64)) {
        self.pointer = point;
        let (px, py) = self.image_point(point);
        self.color = self.pixel(px, py);
        self.position = (point.0.floor() as i64, point.1.floor() as i64);
    }

    /// The magnifier's 15 x 15 pixels around the pointer.
    pub fn magnifier_image(&self) -> RgbaImage {
        let (cx, cy) = self.image_point(self.pointer);
        let radius = MAGNIFIER_PIXELS / 2;
        let mut out = RgbaImage::new(MAGNIFIER_PIXELS as u32, MAGNIFIER_PIXELS as u32);
        for y in 0..MAGNIFIER_PIXELS {
            for x in 0..MAGNIFIER_PIXELS {
                let c = self.pixel(cx - radius + x, cy - radius + y);
                out.put_pixel(x as u32, y as u32, image::Rgba([c.r, c.g, c.b, 255]));
            }
        }
        out
    }

    pub fn info_text(&self) -> String {
        color_format::screen_picker_text(
            &self.options.info_text,
            self.color,
            self.position.0,
            self.position.1,
        )
    }

    /// The part of the desktop a display shows.
    fn screen_texture(&mut self, ctx: &egui::Context, screen: &ScreenInfo) -> TextureHandle {
        if let Some((_, texture)) = self.textures.iter().find(|(id, _)| *id == screen.id) {
            return texture.clone();
        }
        let b = self.desktop.bounds;
        let scale = self.desktop.scale;
        let image = &self.desktop.image;
        let x = (((screen.bounds.x - b.x) as f64 * scale).round() as i64)
            .clamp(0, image.width() as i64) as u32;
        let y = (((screen.bounds.y - b.y) as f64 * scale).round() as i64)
            .clamp(0, image.height() as i64) as u32;
        let w = ((screen.bounds.width as f64 * scale).round() as u32)
            .min(image.width() - x)
            .max(1);
        let h = ((screen.bounds.height as f64 * scale).round() as u32)
            .min(image.height() - y)
            .max(1);
        let mut crop = image::imageops::crop_imm(image.as_ref(), x, y, w, h).to_image();
        let max = ctx.input(|i| i.max_texture_side).max(256) as u32;
        if crop.width() > max || crop.height() > max {
            let f = (max as f64 / crop.width() as f64).min(max as f64 / crop.height() as f64);
            crop = image::imageops::resize(
                &crop,
                ((crop.width() as f64 * f) as u32).max(1),
                ((crop.height() as f64 * f) as u32).max(1),
                image::imageops::FilterType::Triangle,
            );
        }
        let texture = super::kit::texture(
            ctx,
            &format!("screen-color-picker-{}", screen.id),
            &crop,
            true,
        );
        self.textures.push((screen.id, texture.clone()));
        texture
    }

    fn magnifier_texture(&mut self, ctx: &egui::Context) -> TextureHandle {
        let key = self.image_point(self.pointer);
        if let Some((texture, at)) = &self.magnifier
            && *at == key
        {
            return texture.clone();
        }
        let image = self.magnifier_image();
        let texture = super::kit::texture(ctx, "screen-color-picker-magnifier", &image, true);
        self.magnifier = Some((texture.clone(), key));
        texture
    }

    /// One display's window: the frozen picture, the pointer's input, the preview.
    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors, screen: &ScreenInfo) {
        let rect = ui.max_rect();
        let ctx = ui.ctx().clone();
        // The picture of this display.
        let texture = self.screen_texture(&ctx, screen);
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        ui.painter().image(texture.id(), rect, uv, Color32::WHITE);

        // Input.
        let response = ui.interact(
            rect,
            Id::new(("screen-picker", screen.id)),
            Sense::click_and_drag(),
        );
        let hover = ui.input(|i| i.pointer.hover_pos());
        let here = hover.is_some();
        if let Some(local) = hover {
            let point = (
                screen.bounds.x as f64 + (local.x - rect.left()) as f64,
                screen.bounds.y as f64 + (local.y - rect.top()) as f64,
            );
            if point != self.pointer {
                self.update(point);
            }
        }
        let (primary_down, secondary_down, primary_up, secondary_up, command) = ui.input(|i| {
            (
                i.pointer.button_pressed(egui::PointerButton::Primary),
                i.pointer.button_pressed(egui::PointerButton::Secondary),
                i.pointer.button_released(egui::PointerButton::Primary),
                i.pointer.button_released(egui::PointerButton::Secondary),
                i.modifiers.command,
            )
        });
        if here {
            if primary_down {
                self.pressed = Some((self.color, self.position));
            }
            if secondary_down {
                self.cancel_pressed = true;
            }
            if primary_up && let Some((color, position)) = self.pressed.take() {
                self.result = Some(Some((color, position, command)));
            }
            if secondary_up && self.cancel_pressed {
                self.result = Some(None);
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.result = Some(None);
        }
        let _ = response;

        // The preview beside the pointer, on the display it is on.
        let pointer = self.pointer;
        if !screen
            .bounds
            .contains(pointer.0.floor() as i32, pointer.1.floor() as i32)
        {
            return;
        }
        let local = pos2(
            rect.left() + (pointer.0 - screen.bounds.x as f64) as f32,
            rect.top() + (pointer.1 - screen.bounds.y as f64) as f32,
        );
        self.paint_preview(ui, colors, rect, local);
    }

    /// The preview panel (`PickerPreview`) next to `cursor`, kept inside `bounds`.
    pub fn paint_preview(&mut self, ui: &Ui, colors: &Colors, bounds: Rect, cursor: Pos2) {
        let info = self.info_text();
        let galley = layout(
            ui,
            wrapped(&info, 14.0, Weight::SemiBold, colors.text(), 240.0),
        );
        let text_w = galley.size().x;
        let row_h = galley.size().y.max(28.0);
        let magnifier = self.options.show_magnifier;
        let inner_w = (28.0 + 8.0 + text_w).max(if magnifier { 136.0 } else { 0.0 });
        let inner_h = row_h + if magnifier { 136.0 + 8.0 } else { 0.0 };
        let size = vec2(inner_w + 16.0 + 2.0, inner_h + 16.0 + 2.0);
        let mut x = cursor.x + PREVIEW_OFFSET;
        let mut y = cursor.y + PREVIEW_OFFSET;
        if x + size.x > bounds.right() {
            x = cursor.x - size.x - PREVIEW_OFFSET;
        }
        if y + size.y > bounds.bottom() {
            y = cursor.y - size.y - PREVIEW_OFFSET;
        }
        x = x.clamp(bounds.left(), (bounds.right() - size.x).max(bounds.left()));
        y = y.clamp(bounds.top(), (bounds.bottom() - size.y).max(bounds.top()));
        let panel = Rect::from_min_size(pos2(x, y), size);
        let painter = ui.painter();
        // `BoxShadow="0 8 24 #66000000"`.
        painter.add(
            egui::Shadow {
                offset: [0, 8],
                blur: 24,
                spread: 0,
                color: Color32::from_black_alpha(0x66),
            }
            .as_shape(panel, 8.0),
        );
        painter.rect_filled(panel, 8.0, colors.background_main());
        painter.rect_stroke(
            panel,
            8.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        let inner = panel.shrink(9.0);
        let mut top = inner.top();
        if magnifier {
            let ctx = ui.ctx().clone();
            let texture = self.magnifier_texture(&ctx);
            let grid = Rect::from_min_size(pos2(inner.left(), top), vec2(136.0, 136.0));
            let image_rect = Rect::from_center_size(grid.center(), vec2(135.0, 135.0));
            painter.image(
                texture.id(),
                image_rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            let center = Rect::from_center_size(grid.center(), vec2(11.0, 11.0));
            painter.rect_stroke(
                center.expand(1.0),
                0.0,
                Stroke::new(1.0, Color32::from_black_alpha(0xCC)),
                StrokeKind::Inside,
            );
            painter.rect_stroke(
                center,
                0.0,
                Stroke::new(1.0, Color32::WHITE),
                StrokeKind::Inside,
            );
            top += 136.0 + 8.0;
        }
        let swatch = Rect::from_min_size(
            pos2(inner.left(), top + (row_h - 28.0) / 2.0),
            vec2(28.0, 28.0),
        );
        painter.rect_filled(
            swatch,
            4.0,
            Color32::from_rgb(self.color.r, self.color.g, self.color.b),
        );
        painter.rect_stroke(
            swatch,
            4.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        let text_pos = pos2(swatch.right() + 8.0, top + (row_h - galley.size().y) / 2.0);
        painter.galley(text_pos, galley, colors.text());
        let _ = Align2::LEFT_CENTER;
    }
}

impl AppState {
    /// Open the screen colour picker after `delay` seconds (`Task.Delay(250)` for a
    /// window that steps aside first).
    pub fn start_screen_pick(
        &mut self,
        target: PickTarget,
        options: ScreenColorPickerOptions,
        delay: f64,
    ) {
        if self.tools.screen_picker.is_some() || self.tools.pending_pick.is_some() {
            return;
        }
        self.tools.pending_pick = Some(PendingPick {
            at: self.time + delay,
            target,
            options,
        });
    }

    /// `OpenScreenColorPicker`.
    pub fn open_screen_color_picker(&mut self, settings: TaskSettings) {
        let options: ScreenColorPickerOptions =
            super::jobs::tool_options(&settings, "ScreenColorPickerOptions");
        self.start_screen_pick(PickTarget::Copy(Box::new(settings)), options, 0.0);
    }

    /// The desktop is taken off the UI thread; then the picker opens.
    pub(super) fn tick_screen_pick(&mut self) {
        let due = self
            .tools
            .pending_pick
            .as_ref()
            .is_some_and(|p| self.time >= p.at);
        if !due {
            return;
        }
        let Some(pending) = self.tools.pending_pick.take() else {
            return;
        };
        let source = self.captures.source.clone();
        self.tools.screen_pick_busy = true;
        self.rt.spawn_blocking(move || {
            let bounds = source.screen_bounds();
            let screens = source.screens();
            let pointer = source.cursor_position();
            let taken = source.capture_rect(bounds, false);
            move |state: &mut AppState, _ctx: &egui::Context| {
                state.tools.screen_pick_busy = false;
                match taken {
                    Ok(image) => {
                        let scale = if bounds.width > 0 {
                            image.width() as f64 / bounds.width as f64
                        } else {
                            1.0
                        };
                        let desktop = Desktop {
                            image: Arc::new(image),
                            bounds,
                            scale,
                            screens,
                            pointer,
                        };
                        state.tools.screen_picker =
                            Some(ScreenPicker::new(pending.target, pending.options, desktop));
                    }
                    Err(error) => {
                        if error == CaptureError::NotPermitted {
                            state.uploads.windows.messages.push_back(
                                crate::dialogs::message::MessageBox::new(
                                    &format!("Modlight - {}", t(s::Error)),
                                    &CaptureError::NotPermitted.to_string(),
                                    MessageIcon::Warning,
                                ),
                            );
                        } else {
                            log::warn!(
                                "The screen colour picker could not read the screen: {error}"
                            );
                        }
                        state.screen_pick_finished(pending.target, None);
                    }
                }
            }
        });
    }

    /// The picker closed with `result`.
    pub fn screen_pick_finished(&mut self, target: PickTarget, result: PickResult) {
        match target {
            PickTarget::Copy(settings) => {
                let Some((color, (x, y), command)) = result else {
                    return;
                };
                let options: ScreenColorPickerOptions =
                    super::jobs::tool_options(&settings, "ScreenColorPickerOptions");
                let input = if command {
                    options.format_ctrl
                } else {
                    options.format
                };
                if input.is_empty() {
                    return;
                }
                let text = color_format::pixel_info(&input, color, x, y);
                self.copy_text(&text);
                self.play_task_sound(NotificationSound::ActionCompleted, &settings);
                if settings
                    .general_settings
                    .show_toast_notification_after_task_completed
                {
                    self.show_notification_tip(
                        &format(
                            t(s::TaskHelpers_OpenQuickScreenColorPicker_Copied_to_clipboard___0_),
                            &[Arg::from(text.as_str())],
                        ),
                        &format!("Modlight - {}", t(s::ScreenColorPicker)),
                        None,
                    );
                }
            }
            PickTarget::ColorPicker(key) => {
                self.tools.restore.push(key);
                if let Some(window) = self
                    .tools
                    .window_mut::<super::color_picker::ColorPickerWindow>(key)
                {
                    window.minimized = false;
                    if let Some((color, _, _)) = result {
                        window.screen_picked(color);
                        let recent = window.recent.clone();
                        self.set_recent_colors(&recent);
                    }
                }
            }
            PickTarget::Editor { key, target } => {
                if let Some((color, _, _)) = result {
                    self.editor_screen_color_picked(key, target, color);
                }
            }
        }
    }
}

/// The picker's windows, one per display; called every frame.
pub fn viewports(ctx: &egui::Context, state: &mut AppState) {
    let Some(picker) = state.tools.screen_picker.as_mut() else {
        return;
    };
    let colors = state.colors.clone();
    let screens = picker.desktop.screens.clone();
    for screen in &screens {
        let title = format!("Modlight - {} {}", t(s::ScreenColorPicker), screen.id);
        let builder = ViewportBuilder::default()
            .with_title(title.clone())
            .with_decorations(false)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_position(pos2(screen.bounds.x as f32, screen.bounds.y as f32))
            .with_inner_size(vec2(
                screen.bounds.width as f32,
                screen.bounds.height as f32,
            ));
        let id = ViewportId::from_hash_of(("screen-color-picker", screen.id));
        let raised_key = Id::new(("screen-color-picker-raised", screen.id));
        ctx.show_viewport_immediate(id, builder, |ui, _| {
            if !ui
                .ctx()
                .data(|d| d.get_temp::<bool>(raised_key).unwrap_or(false))
                && modlight_macos::windows::raise_overlay(&title)
            {
                ui.ctx().data_mut(|d| d.insert_temp(raised_key, true));
                ui.ctx().send_viewport_cmd(ViewportCommand::Focus);
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            picker.ui(ui, &colors, screen);
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                picker.result = Some(None);
            }
        });
    }
    ctx.request_repaint();
    if let Some(result) = picker.result.take()
        && let Some(picker) = state.tools.screen_picker.take()
    {
        for screen in &picker.desktop.screens {
            ctx.data_mut(|d| d.remove::<bool>(Id::new(("screen-color-picker-raised", screen.id))));
        }
        state.screen_pick_finished(picker.target, result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desktop() -> Desktop {
        // Two points wide per pixel pair: a 2x display 4 x 2 points = 8 x 4 pixels.
        let mut image = RgbaImage::new(8, 4);
        for (x, y, p) in image.enumerate_pixels_mut() {
            *p = image::Rgba([x as u8 * 10, y as u8 * 10, 7, 255]);
        }
        Desktop {
            image: Arc::new(image),
            bounds: IRect::new(-4, 0, 4, 2),
            scale: 2.0,
            screens: Vec::new(),
            pointer: (-3, 1),
        }
    }

    #[test]
    fn samples_the_pixel_under_the_pointer() {
        let picker = ScreenPicker::new(
            PickTarget::ColorPicker(1),
            ScreenColorPickerOptions::default(),
            desktop(),
        );
        // (-3, 1) points is pixel (2, 2).
        assert_eq!(picker.color, Color::rgb(20, 20, 7));
        assert_eq!(picker.position, (-3, 1));
        assert_eq!(picker.info_text(), "#141407");
    }

    #[test]
    fn the_magnifier_is_black_off_the_desktop() {
        let picker = ScreenPicker::new(
            PickTarget::ColorPicker(1),
            ScreenColorPickerOptions::default(),
            desktop(),
        );
        let image = picker.magnifier_image();
        assert_eq!(image.dimensions(), (15, 15));
        // The centre is the sampled pixel; the corner lies off the desktop.
        assert_eq!(image.get_pixel(7, 7).0, [20, 20, 7, 255]);
        assert_eq!(image.get_pixel(0, 0).0, [0, 0, 0, 255]);
    }
}
