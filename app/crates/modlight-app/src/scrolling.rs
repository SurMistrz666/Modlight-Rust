//! Scrolling capture (`ScrollingCaptureWindow`, `ScrollingCaptureRegionWindow`): choose
//! the scrollable area, scroll and stitch in the background, show the result with
//! Capture, Upload/Save, Copy and Options, and send it on as a picture task.
//!
//! Scrolling another app needs synthetic input, which macOS allows only for apps trusted
//! under Privacy & Security, Accessibility. QA scrolls a fake page instead.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Color32, Rect, Stroke, StrokeKind, Ui, pos2, vec2};
use image::RgbaImage;
use modlight_core::capture::geometry::IRect;
use modlight_core::capture::options::{ScrollMethod, ScrollingCaptureOptions};
use modlight_core::capture::region::{RegionKind, RegionSession};
use modlight_core::capture::scrolling::{Scroller, ScrollingStatus};
use modlight_core::capture::source::WindowInfo;
use modlight_core::icons;
use modlight_core::l10n::{capture as c, t};
use modlight_core::tasks::host::NotificationSound;
use modlight_core::tasks::info::TaskMetadata;
use modlight_core::tasks::settings::TaskSettings;

use crate::capture_flow::PendingImage;
use crate::controls::number_box_at;
use crate::region_overlay::{OverlayPurpose, RegionOverlay};
use crate::state::{AppState, Effect};
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Look, button_at, checkbox_weighted, combo, layout, paint_icon, trimmed,
};

pub const WIDTH: f32 = 900.0;
pub const HEIGHT: f32 = 650.0;
pub const MIN_WIDTH: f32 = 620.0;
pub const MIN_HEIGHT: f32 = 440.0;

/// The window's state.
pub struct ScrollingWindow {
    pub settings: TaskSettings,
    pub options: ScrollingCaptureOptions,
    pub capturing: bool,
    pub stop: Arc<AtomicBool>,
    pub status: Option<ScrollingStatus>,
    pub status_text: String,
    pub result: Option<Arc<RgbaImage>>,
    texture: Option<egui::TextureHandle>,
    /// The options overlay, with its edits not yet saved.
    pub editing: Option<ScrollingCaptureOptions>,
    /// The area being captured, for the frame shown around it (`ShowRegion`).
    pub region: Option<IRect>,
    /// The window is out of the way while selecting and capturing (`Minimized`).
    pub minimized: bool,
}

/// What the window asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum ScrollingAction {
    Capture,
    Upload,
    Copy,
    Options,
    Help,
    CancelOptions,
    SaveOptions,
}

impl ScrollingWindow {
    pub fn new(settings: TaskSettings) -> ScrollingWindow {
        ScrollingWindow {
            options: settings.capture_settings.scrolling_capture_options.clone(),
            settings,
            capturing: false,
            stop: Arc::new(AtomicBool::new(false)),
            status: None,
            status_text: t(c::ScrollingCaptureWindow_Ready).to_string(),
            result: None,
            texture: None,
            editing: None,
            region: None,
            minimized: false,
        }
    }

    pub fn title() -> &'static str {
        t(c::ScrollingCaptureWindow_Title)
    }

    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors) -> Option<ScrollingAction> {
        crate::dialogs::background(ui, colors);
        let full = ui.max_rect();
        let painter = ui.painter().clone();
        let mut action = None;
        let body = Rect::from_min_max(
            pos2(full.left() + 12.0, full.top()),
            pos2(full.right() - 12.0, full.bottom() - 12.0),
        );
        // `tool-panel docked-top-toolbar`: the toolbar along the top.
        let bar = Rect::from_min_size(body.min, vec2(body.width(), 12.0 + 34.0 + 12.0));
        painter.rect_filled(
            bar,
            egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: 4,
                se: 4,
            },
            colors.background_panel(),
        );
        painter.rect_stroke(
            bar,
            egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: 4,
                se: 4,
            },
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        let look = Look::button(colors).height(34.0).padding(10.0).spacing(7.0);
        let mut x = bar.left() + 12.0;
        let enabled = !self.capturing;
        let has_result = self.result.is_some() && enabled;
        for (glyph, label, on, which) in [
            (
                icons::SCAN_LINE,
                t(c::ScrollingCaptureWindow_Capture),
                enabled,
                ScrollingAction::Capture,
            ),
            (
                icons::UPLOAD,
                t(c::ScrollingCaptureWindow_Upload_Save),
                has_result,
                ScrollingAction::Upload,
            ),
            (
                icons::COPY,
                t(c::ScrollingCaptureWindow_Copy),
                has_result,
                ScrollingAction::Copy,
            ),
            (
                icons::SETTINGS_2,
                t(c::ScrollingCaptureWindow_Options),
                enabled,
                ScrollingAction::Options,
            ),
        ] {
            let w = crate::widgets::button_width(ui, &look, Content::both(glyph, label));
            let r = Rect::from_min_size(pos2(x, bar.top() + 12.0), vec2(w, 34.0));
            // The capture button also stops a capture in progress.
            let on = on || (which == ScrollingAction::Capture && self.capturing);
            if button_at(ui, colors, &look, Content::both(glyph, label), on, r).clicked() {
                action = Some(which);
            }
            x = r.right() + 6.0;
        }
        // The help button opened ShareX's documentation on getsharex.com, which the port
        // does not link to (PORT.md section 5); it is left out.

        // The status line at the bottom.
        let status_line = Rect::from_min_max(pos2(body.left(), body.bottom() - 20.0), body.max);
        let (glyph, color) = match (self.capturing, self.status) {
            (true, _) => (icons::LOADER_CIRCLE, Color32::from_rgb(0x1E, 0x90, 0xFF)),
            (_, Some(ScrollingStatus::Failed)) => {
                (icons::CIRCLE_X, Color32::from_rgb(0xCD, 0x5C, 0x5C))
            }
            (_, Some(ScrollingStatus::PartiallySuccessful)) => {
                (icons::TRIANGLE_ALERT, Color32::from_rgb(0xDA, 0xA5, 0x20))
            }
            (_, Some(ScrollingStatus::Successful)) => {
                (icons::CIRCLE_CHECK, Color32::from_rgb(0x3C, 0xB3, 0x71))
            }
            _ => (icons::CIRCLE_DASHED, colors.text_secondary()),
        };
        paint_icon(
            ui,
            &painter,
            pos2(status_line.left() + 8.5, status_line.center().y),
            glyph,
            17.0,
            color,
        );
        let text = layout(
            ui,
            trimmed(
                &self.status_text,
                13.0,
                Weight::Normal,
                colors.text_secondary(),
                status_line.width() - 200.0,
                1,
            ),
        );
        painter.galley(
            pos2(
                status_line.left() + 17.0 + 8.0,
                status_line.center().y - text.size().y / 2.0,
            ),
            text,
            colors.text_secondary(),
        );
        if let Some(result) = &self.result {
            let size = layout(
                ui,
                trimmed(
                    &format!("{}x{}", result.width(), result.height()),
                    13.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    200.0,
                    1,
                ),
            );
            painter.galley(
                pos2(
                    status_line.right() - size.size().x,
                    status_line.center().y - size.size().y / 2.0,
                ),
                size,
                colors.text_secondary(),
            );
        }

        // The preview.
        let preview = Rect::from_min_max(
            pos2(body.left(), bar.bottom() + 8.0),
            pos2(body.right(), status_line.top() - 8.0),
        );
        painter.rect_filled(preview, 4.0, colors.background_panel());
        painter.rect_stroke(
            preview,
            4.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        let inner = preview.shrink(1.0);
        if let Some(result) = &self.result {
            let texture = self.texture.get_or_insert_with(|| {
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [result.width() as usize, result.height() as usize],
                    result.as_raw(),
                );
                ui.ctx()
                    .load_texture("scrolling-result", color, egui::TextureOptions::NEAREST)
            });
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
            egui::ScrollArea::both()
                .id_salt("scrolling-preview")
                .auto_shrink([false, false])
                .show(&mut child, |ui| {
                    let size = vec2(result.width() as f32, result.height() as f32);
                    let (rect, _) =
                        ui.allocate_exact_size(size.max(inner.size()), egui::Sense::drag());
                    let image_rect = Rect::from_center_size(
                        pos2(
                            rect.center().x,
                            rect.top() + (rect.height().max(size.y)) / 2.0,
                        ),
                        size,
                    );
                    ui.painter().image(
                        texture.id(),
                        image_rect,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                });
        } else {
            paint_icon(
                ui,
                &painter,
                pos2(inner.center().x, inner.center().y - 18.0),
                icons::SCROLL_TEXT,
                48.0,
                colors.text_secondary(),
            );
            let text = layout(
                ui,
                trimmed(
                    t(c::ScrollingCaptureWindow_Select_scrollable_window),
                    13.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    inner.width(),
                    1,
                ),
            );
            painter.galley(
                pos2(
                    inner.center().x - text.size().x / 2.0,
                    inner.center().y + 14.0,
                ),
                text,
                colors.text_secondary(),
            );
        }

        if self.editing.is_some()
            && let Some(choice) = self.options_ui(ui, colors, full)
        {
            action = Some(choice);
        }
        action
    }

    /// The options overlay (`OptionsOverlay`).
    fn options_ui(&mut self, ui: &mut Ui, colors: &Colors, full: Rect) -> Option<ScrollingAction> {
        let options = self.editing.as_mut()?;
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, colors.overlay_modal());
        let panel = Rect::from_center_size(
            full.center(),
            vec2(
                500.0,
                (18.0_f32 * 2.0
                    + 24.0
                    + 14.0
                    + 4.0 * 32.0
                    + 3.0 * 9.0
                    + 14.0
                    + 4.0 * 32.0
                    + 3.0 * 8.0
                    + 14.0
                    + 40.0)
                    .min(620.0),
            ),
        );
        painter.rect_filled(panel, 4.0, colors.background_panel());
        painter.rect_stroke(
            panel,
            4.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        let inner = panel.shrink(18.0);
        paint_icon(
            ui,
            &painter,
            pos2(inner.left() + 10.0, inner.top() + 12.0),
            icons::SETTINGS_2,
            20.0,
            colors.accent_start(),
        );
        let heading = layout(
            ui,
            trimmed(
                t(c::ScrollingCaptureWindow_Options_heading),
                16.0,
                Weight::SemiBold,
                colors.text(),
                inner.width() - 30.0,
                1,
            ),
        );
        painter.galley(
            pos2(
                inner.left() + 30.0,
                inner.top() + 12.0 - heading.size().y / 2.0,
            ),
            heading,
            colors.text(),
        );
        let mut y = inner.top() + 24.0 + 14.0;
        let label_w = 170.0;
        let field_w = inner.width() - label_w - 8.0 - 8.0 - 90.0;
        let row = |ui: &mut Ui, y: f32, label: &str, hint: Option<&str>| -> Rect {
            let l = layout(
                ui,
                trimmed(label, 13.0, Weight::Normal, colors.text(), label_w, 1),
            );
            ui.painter().galley(
                pos2(inner.left(), y + 16.0 - l.size().y / 2.0),
                l,
                colors.text(),
            );
            if let Some(hint) = hint {
                let h = layout(
                    ui,
                    trimmed(hint, 13.0, Weight::Normal, colors.text_secondary(), 90.0, 1),
                );
                ui.painter().galley(
                    pos2(
                        inner.left() + label_w + 8.0 + field_w + 8.0,
                        y + 16.0 - h.size().y / 2.0,
                    ),
                    h,
                    colors.text_secondary(),
                );
            }
            Rect::from_min_size(pos2(inner.left() + label_w + 8.0, y), vec2(field_w, 32.0))
        };
        let ms = t(c::ScrollingCaptureWindow_Milliseconds);
        let r = row(ui, y, t(c::ScrollingCaptureWindow_Start_delay), Some(ms));
        let mut v = options.start_delay as f64;
        if number_box_at(
            ui,
            colors,
            "scroll-start-delay",
            r,
            &mut v,
            100.0,
            5000.0,
            100.0,
            0,
            true,
        ) {
            options.start_delay = v as i64;
        }
        y += 32.0 + 9.0;
        let r = row(ui, y, t(c::ScrollingCaptureWindow_Scroll_delay), Some(ms));
        let mut v = options.scroll_delay as f64;
        if number_box_at(
            ui,
            colors,
            "scroll-delay",
            r,
            &mut v,
            100.0,
            5000.0,
            100.0,
            0,
            true,
        ) {
            options.scroll_delay = v as i64;
        }
        y += 32.0 + 9.0;
        let r = row(ui, y, t(c::ScrollingCaptureWindow_Scroll_method), None);
        let methods: Vec<(ScrollMethod, String)> = ScrollMethod::ALL
            .iter()
            .map(|m| {
                (
                    *m,
                    modlight_core::l10n::enum_text("ScrollMethod", m.name(), None),
                )
            })
            .collect();
        let mut combo_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(r.min, vec2(field_w + 8.0 + 90.0, 32.0))),
        );
        if let Some(method) = combo(
            &mut combo_ui,
            colors,
            "scroll-method",
            field_w + 8.0 + 90.0,
            &options.scroll_method,
            &methods,
        ) {
            options.scroll_method = method;
        }
        y += 32.0 + 9.0;
        if options.scroll_method != ScrollMethod::PageDown {
            let r = row(
                ui,
                y,
                t(c::ScrollingCaptureWindow_Scroll_amount),
                Some(t(c::ScrollingCaptureWindow_Times)),
            );
            let mut v = options.scroll_amount as f64;
            if number_box_at(
                ui,
                colors,
                "scroll-amount",
                r,
                &mut v,
                1.0,
                6.0,
                1.0,
                0,
                true,
            ) {
                options.scroll_amount = v as i64;
            }
        }
        y += 32.0 + 14.0;
        let mut checks = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(
            pos2(inner.left(), y),
            vec2(inner.width(), 4.0 * 40.0),
        )));
        checks.spacing_mut().item_spacing.y = 8.0;
        checkbox_weighted(
            &mut checks,
            colors,
            &mut options.auto_scroll_top,
            Some(t(c::ScrollingCaptureWindow_Automatically_scroll_to_top)),
            true,
            Weight::SemiBold,
        );
        checkbox_weighted(
            &mut checks,
            colors,
            &mut options.auto_ignore_bottom_edge,
            Some(t(c::ScrollingCaptureWindow_Auto_ignore_bottom_edge)),
            true,
            Weight::SemiBold,
        );
        checkbox_weighted(
            &mut checks,
            colors,
            &mut options.show_region,
            Some(t(c::ScrollingCaptureWindow_Show_capture_region)),
            true,
            Weight::SemiBold,
        );
        checkbox_weighted(
            &mut checks,
            colors,
            &mut options.auto_upload,
            Some(t(c::ScrollingCaptureWindow_Automatically_upload_save)),
            true,
            Weight::SemiBold,
        );
        let look = Look::compact(colors);
        let (cancel, ok) = (
            t(c::ScrollingCaptureWindow_Cancel),
            t(c::ScrollingCaptureWindow_OK),
        );
        let ok_w = crate::widgets::button_width(ui, &look, Content::label(ok));
        let cancel_w = crate::widgets::button_width(ui, &look, Content::label(cancel));
        let ok_r = Rect::from_min_size(
            pos2(inner.right() - ok_w, inner.bottom() - 40.0),
            vec2(ok_w, 40.0),
        );
        let cancel_r = Rect::from_min_size(
            pos2(ok_r.left() - 8.0 - cancel_w, ok_r.top()),
            vec2(cancel_w, 40.0),
        );
        if button_at(ui, colors, &look, Content::label(cancel), true, cancel_r).clicked() {
            return Some(ScrollingAction::CancelOptions);
        }
        if button_at(ui, colors, &look, Content::label(ok), true, ok_r).clicked() {
            return Some(ScrollingAction::SaveOptions);
        }
        None
    }
}

/// `ScrollingCaptureWindowIntegration.StartStopAsync`: open the window, or stop the
/// capture in progress, or choose an area again.
pub fn open(state: &mut AppState, settings: TaskSettings) {
    if let Some(window) = state.scrolling.as_mut() {
        if window.capturing {
            window.stop.store(true, Ordering::SeqCst);
            window.status_text = t(c::ScrollingCaptureWindow_Stopping_capture).to_string();
            return;
        }
    } else {
        state.scrolling = Some(ScrollingWindow::new(settings));
    }
    select_area(state);
}

/// `SelectWindowAsync`: the window gets out of the way and the overlay asks.
fn select_area(state: &mut AppState) {
    let Some(window) = state.scrolling.as_mut() else {
        return;
    };
    window.minimized = true;
    window.editing = None;
    let settings = window.settings.clone();
    let source = state.captures.source.clone();
    state.captures.busy = true;
    state.rt.spawn_blocking(move || {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let request =
            modlight_core::capture::region_request(RegionKind::Default, &settings, source.as_ref());
        move |state: &mut AppState, _ctx: &egui::Context| {
            state.captures.busy = false;
            match request {
                Ok(mut request) => {
                    request.enable_annotations = false;
                    // `GetRectangleRegion(new RegionCaptureOptions())`: default options.
                    request.options = Default::default();
                    let session = RegionSession::new(request);
                    state.captures.overlay = Some(RegionOverlay::new(
                        session,
                        OverlayPurpose::ScrollingRegion,
                        settings,
                        state.time,
                    ));
                }
                Err(error) => {
                    if let Some(window) = state.scrolling.as_mut() {
                        window.minimized = false;
                        window.status = Some(ScrollingStatus::Failed);
                        window.status_text = error.to_string();
                    }
                }
            }
        }
    });
}

/// The area is chosen: capture (`CaptureSelectedWindowAsync`); cancelled: say so.
pub fn region_chosen(state: &mut AppState, rect: IRect, window_info: Option<WindowInfo>) {
    let Some(window) = state.scrolling.as_mut() else {
        return;
    };
    window.capturing = true;
    window.result = None;
    window.texture = None;
    window.status = None;
    window.status_text = t(c::ScrollingCaptureWindow_Capturing).to_string();
    window.region = window.options.show_region.then_some(rect);
    window.stop.store(false, Ordering::SeqCst);
    let options = window.options.clone();
    let stop = window.stop.clone();
    let source = state.captures.source.clone();
    let scroller: Arc<dyn Scroller> = if state.env.qa {
        Arc::new(FakeScroller)
    } else {
        Arc::new(crate::platform::input::SystemScroller)
    };
    state.rt.spawn_blocking(move || {
        let (result, status) = modlight_core::capture::scrolling::run(
            source.as_ref(),
            scroller.as_ref(),
            rect,
            window_info.as_ref(),
            &options,
            &stop,
        );
        move |state: &mut AppState, _ctx: &egui::Context| finished(state, result, status)
    });
}

/// The selection was cancelled.
pub fn selection_cancelled(state: &mut AppState) {
    if let Some(window) = state.scrolling.as_mut() {
        window.minimized = false;
        window.status_text = t(c::ScrollingCaptureWindow_Selection_cancelled).to_string();
    }
}

fn finished(state: &mut AppState, result: Option<RgbaImage>, status: ScrollingStatus) {
    let Some(window) = state.scrolling.as_mut() else {
        return;
    };
    window.capturing = false;
    window.minimized = false;
    window.region = None;
    window.status = Some(status);
    window.status_text = match status {
        ScrollingStatus::Failed => t(c::ScrollingCaptureWindow_Capture_failed),
        ScrollingStatus::PartiallySuccessful => {
            t(c::ScrollingCaptureWindow_Capture_partially_successful)
        }
        ScrollingStatus::Successful => t(c::ScrollingCaptureWindow_Capture_successful),
    }
    .to_string();
    window.result = result.map(Arc::new);
    window.texture = None;
    let settings = window.settings.clone();
    let auto_upload = window.options.auto_upload;
    state.play_task_sound(NotificationSound::ActionCompleted, &settings);
    if auto_upload {
        upload(state);
    }
}

/// `UploadResult`: the result as a picture task (`RunImageTask(img, taskSettings)`).
pub fn upload(state: &mut AppState) {
    let Some(window) = state.scrolling.as_ref() else {
        return;
    };
    let Some(result) = window.result.clone() else {
        return;
    };
    let settings = window.settings.clone();
    state.run_image_task(PendingImage {
        metadata: TaskMetadata {
            image: Some(result),
            ..TaskMetadata::default()
        },
        settings,
        file: None,
        skip_quick_menu: false,
    });
}

pub fn action(state: &mut AppState, action: ScrollingAction) {
    match action {
        ScrollingAction::Capture => {
            let settings = state
                .scrolling
                .as_ref()
                .map(|w| w.settings.clone())
                .unwrap_or_default();
            open(state, settings);
        }
        ScrollingAction::Upload => upload(state),
        ScrollingAction::Copy => {
            if let Some(result) = state.scrolling.as_ref().and_then(|w| w.result.clone()) {
                state.effects.push(Effect::CopyImage(result));
            }
        }
        ScrollingAction::Options => {
            if let Some(window) = state.scrolling.as_mut() {
                window.editing = Some(window.options.clone());
            }
        }
        ScrollingAction::CancelOptions => {
            if let Some(window) = state.scrolling.as_mut() {
                window.editing = None;
            }
        }
        ScrollingAction::SaveOptions => {
            if let Some(window) = state.scrolling.as_mut()
                && let Some(options) = window.editing.take()
            {
                window.options = options.clone();
                // `OnSaveOptionsClick` writes `CaptureSettingsReference
                // .ScrollingCaptureOptions`: the defaults, or the hotkey's own capture
                // settings.
                let home = crate::tools::jobs::OptionsHome::of_capture(&window.settings);
                window.settings.capture_settings.scrolling_capture_options = options.clone();
                state.save_task_member(
                    &home,
                    &["CaptureSettings", "ScrollingCaptureOptions"],
                    &options,
                );
            }
        }
        ScrollingAction::Help => {}
    }
}

/// QA scrolls nothing: the fake desktop does not move, so the capture ends after the
/// second picture, as it does on a page that cannot scroll.
struct FakeScroller;

impl Scroller for FakeScroller {
    fn activate(&self, _window: Option<&WindowInfo>) {}
    fn scroll_to_top(&self) {}
    fn scroll(&self, _method: ScrollMethod, _amount: i64) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use modlight_core::hotkeys::{FakeBackend, HotkeyManager, HotkeySettings, HotkeyType};

    fn edit_and_save(state: &mut AppState, delay: i64, method: ScrollMethod) {
        action(state, ScrollingAction::Options);
        let editing = state.scrolling.as_mut().unwrap().editing.as_mut().unwrap();
        editing.scroll_delay = delay;
        editing.scroll_method = method;
        action(state, ScrollingAction::SaveOptions);
        let window = state.scrolling.as_ref().unwrap();
        assert!(window.editing.is_none());
        assert_eq!(window.options.scroll_delay, delay);
    }

    /// "OK" in the options writes upstream's `CaptureSettingsReference
    /// .ScrollingCaptureOptions`: the settings the window was opened with keep them, and
    /// the next scrolling capture starts with them.
    #[test]
    fn saved_options_are_kept_in_the_settings() {
        let mut state = crate::qa::state(&crate::qa::Look::default(), false);
        state.scrolling = Some(ScrollingWindow::new(state.default_task_settings()));
        edit_and_save(&mut state, 900, ScrollMethod::PageDown);
        let saved = state
            .default_task_settings()
            .capture_settings
            .scrolling_capture_options;
        assert_eq!(saved.scroll_delay, 900);
        assert_eq!(saved.scroll_method, ScrollMethod::PageDown);
        assert_eq!(
            saved.start_delay,
            ScrollingCaptureOptions::default().start_delay
        );
        // Cancel keeps what was saved.
        action(&mut state, ScrollingAction::Options);
        state
            .scrolling
            .as_mut()
            .unwrap()
            .editing
            .as_mut()
            .unwrap()
            .scroll_delay = 100;
        action(&mut state, ScrollingAction::CancelOptions);
        assert_eq!(
            state
                .default_task_settings()
                .capture_settings
                .scrolling_capture_options
                .scroll_delay,
            900
        );

        // A hotkey with capture settings of its own keeps them in its own.
        let task = serde_json::json!({
            "Job": "ScrollingCapture",
            "UseDefaultCaptureSettings": false,
            "CaptureSettings": { "ShowCursor": false }
        });
        let hotkey = HotkeySettings {
            task,
            ..HotkeySettings::new(HotkeyType::ScrollingCapture, 0)
        };
        state.captures.hotkeys =
            HotkeyManager::new(Box::new(FakeBackend::default()), vec![hotkey], false);
        let defaults = state.config.get("DefaultTaskSettings").cloned().unwrap();
        let settings = state.captures.hotkeys.hotkeys[0].task_settings(&defaults);
        state.scrolling = Some(ScrollingWindow::new(settings));
        edit_and_save(&mut state, 1500, ScrollMethod::DownArrow);
        let own =
            &state.captures.hotkeys.hotkeys[0].task["CaptureSettings"]["ScrollingCaptureOptions"];
        assert_eq!(own["ScrollDelay"], 1500);
        assert_eq!(own["ScrollMethod"], "DownArrow");
        assert_eq!(
            state
                .default_task_settings()
                .capture_settings
                .scrolling_capture_options
                .scroll_delay,
            900,
            "the defaults stay as they were"
        );
    }
}
