//! `PrintWindow` and `TaskHelpers.PrintImage`: the picture's print settings (margin,
//! turn, scale, enlarge, centre), "Preview" and "Print" through the macOS print system
//! (`modlight_macos::print`): the print panel when the settings ask for it, the page as a
//! PDF in Preview for the preview. Without the settings window (`DontShowPrintSettingsDialog`)
//! the picture prints straight away. QA and headless runs never reach the print system.

use std::sync::Arc;

use egui::{Align2, Rect, Ui, pos2, vec2};
use image::RgbaImage;
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, helpers as hp, t};
use modlight_core::tasks::host::MessageIcon;
use modlight_tools::print::{self as layout, PrintSettings, UNITS_PER_POINT};

use super::kit;
use super::{ToolWindow, WindowSpec};
use crate::state::AppState;
use crate::theme::Weight;

pub struct PrintWindow {
    pub key: u64,
    pub image: Arc<RgbaImage>,
    pub settings: PrintSettings,
    pub preview_only: bool,
    close: bool,
    height: Option<f32>,
}

impl PrintWindow {
    pub fn new(
        key: u64,
        image: Arc<RgbaImage>,
        settings: PrintSettings,
        preview_only: bool,
    ) -> PrintWindow {
        PrintWindow {
            key,
            image,
            settings,
            preview_only,
            close: false,
            height: None,
        }
    }
}

/// Where the picture goes on the current paper, in points, and the picture as drawn.
fn page_layout(image: &RgbaImage, settings: &PrintSettings) -> (RgbaImage, (f64, f64, f64, f64)) {
    let (w, h) = modlight_macos::print::page_size();
    let placement = layout::place(
        (w * UNITS_PER_POINT, h * UNITS_PER_POINT),
        image.dimensions(),
        settings,
    );
    let to_points = |v: f64| v / UNITS_PER_POINT;
    (
        layout::prepared(image, &placement),
        (
            to_points(placement.x),
            to_points(placement.y),
            to_points(placement.width),
            to_points(placement.height),
        ),
    )
}

impl AppState {
    fn print_allowed(&self) -> bool {
        !self.env.qa && self.mode == crate::app::Mode::Normal
    }

    /// `PrintHelper.Print`: the print panel when the settings ask for it.
    pub fn print_now(&mut self, image: &RgbaImage, settings: &PrintSettings) {
        if !self.print_allowed() {
            log::info!("QA run: nothing is printed");
            return;
        }
        let (picture, rect) = page_layout(image, settings);
        let printer =
            Some(settings.default_printer_override.as_str()).filter(|p| !p.trim().is_empty());
        match modlight_macos::print::print(&picture, rect, settings.show_print_dialog, printer) {
            Ok(modlight_macos::print::Printed::UnknownPrinter) => {
                self.uploads
                    .windows
                    .messages
                    .push_back(crate::dialogs::message::MessageBox::new(
                        t(hp::PrintHelper_Invalid_printer_name),
                        &format(
                            t(hp::PrintHelper_Invalid_printer_message),
                            &[Arg::from(settings.default_printer_override.as_str())],
                        ),
                        MessageIcon::Information,
                    ));
            }
            Ok(_) => {}
            Err(error) => log::warn!("Printing failed: {error}"),
        }
    }

    /// `PrintHelper.ShowPreview`: the page as a PDF, opened in Preview.
    fn print_preview(&mut self, image: &RgbaImage, settings: &PrintSettings) {
        if !self.print_allowed() {
            return;
        }
        let (picture, rect) = page_layout(image, settings);
        let path = std::env::temp_dir().join("Modlight print preview.pdf");
        match modlight_macos::print::page_pdf(&picture, rect, &path) {
            Ok(()) => self.effects.push(crate::state::Effect::OpenFile(path)),
            Err(error) => log::warn!("The print preview failed: {error}"),
        }
    }

    fn print_settings(&self) -> PrintSettings {
        modlight_tools::settings::read(self.config.get("PrintSettings"))
    }

    /// `TaskHelpers.PrintImage(image)`.
    pub fn print_picture(&mut self, image: RgbaImage) {
        let settings = self.print_settings();
        let skip = self
            .config
            .get("DontShowPrintSettingsDialog")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if skip {
            self.print_now(&image, &settings);
            return;
        }
        let image = Arc::new(image);
        self.tools
            .open_keyed(|key| Box::new(PrintWindow::new(key, image, settings, false)));
    }

    /// The settings page's "Image print settings": the window over a picture of the active
    /// screen, printing off (`previewOnly`).
    pub fn open_print_settings(&mut self) {
        let source = self.captures.source.clone();
        let settings = self.default_task_settings();
        self.rt.spawn_blocking(move || {
            let step = modlight_core::capture::capture(
                modlight_core::capture::CaptureKind::ActiveMonitor,
                &settings,
                source.as_ref(),
            );
            move |state: &mut AppState, _ctx: &egui::Context| {
                let image = match step {
                    modlight_core::capture::CaptureStep::Done(captured) => captured.metadata.image,
                    _ => None,
                };
                let image = image.unwrap_or_else(|| Arc::new(RgbaImage::new(1, 1)));
                let print = state.print_settings();
                state
                    .tools
                    .open_keyed(|key| Box::new(PrintWindow::new(key, image, print, true)));
            }
        });
    }
}

impl ToolWindow for PrintWindow {
    fn spec(&self) -> WindowSpec {
        let mut spec = WindowSpec::fixed(t(hp::PrintWindow_Title), vec2(470.0, 330.0));
        spec.fit_height = true;
        spec
    }

    fn content_height(&self) -> Option<f32> {
        self.height
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        kit::fill(ui, &colors);
        let full = ui.max_rect();
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.close = true;
        }
        // The heading band.
        let band = Rect::from_min_size(full.min, vec2(full.width(), 12.0 * 2.0 + 44.0));
        ui.painter()
            .rect_filled(band, 0.0, colors.background_toolbar());
        let icon_box = Rect::from_min_size(
            pos2(band.left() + 18.0, band.center().y - 20.0),
            vec2(40.0, 40.0),
        );
        ui.painter()
            .rect_filled(icon_box, 6.0, colors.background_panel());
        crate::widgets::paint_icon(
            ui,
            ui.painter(),
            icon_box.center(),
            icons::PRINTER,
            20.0,
            colors.accent_start(),
        );
        let text_x = icon_box.right() + 10.0;
        kit::text_at(
            ui,
            Rect::from_min_size(
                pos2(text_x, band.center().y - 23.0),
                vec2(full.width() - text_x - 18.0, 25.0),
            ),
            Align2::LEFT_CENTER,
            t(hp::PrintWindow_Heading),
            19.0,
            Weight::SemiBold,
            colors.accent_start(),
        );
        kit::text_at(
            ui,
            Rect::from_min_size(
                pos2(text_x, band.center().y + 3.0),
                vec2(full.width() - text_x - 18.0, 18.0),
            ),
            Align2::LEFT_CENTER,
            t(hp::PrintWindow_Description),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
        );

        // The options.
        let panel_top = band.bottom() + 14.0;
        let rows_h = 34.0 + 3.0 + 4.0 * 24.0 + 6.0 * 4.0;
        let panel = Rect::from_min_size(
            pos2(full.left() + 18.0, panel_top),
            vec2(full.width() - 36.0, rows_h + 24.0),
        );
        ui.painter()
            .rect_filled(panel, 6.0, colors.background_panel());
        ui.painter().rect_stroke(
            panel,
            6.0,
            egui::Stroke::new(1.0, colors.border()),
            egui::StrokeKind::Inside,
        );
        let inner = Rect::from_min_max(
            pos2(panel.left() + 14.0, panel.top() + 12.0),
            pos2(panel.right() - 14.0, panel.bottom() - 12.0),
        );
        let mut changed = false;
        let label = t(hp::PrintWindow_Margin);
        let label_w = kit::text_width(ui, label, 13.0, Weight::Normal);
        let row = Rect::from_min_size(inner.min, vec2(inner.width(), 34.0));
        kit::text_at(
            ui,
            row,
            Align2::LEFT_CENTER,
            label,
            13.0,
            Weight::Normal,
            colors.text(),
        );
        let number = Rect::from_min_size(
            pos2(row.left() + label_w + 8.0, row.top()),
            vec2(130.0, 34.0),
        );
        changed |= kit::int_at(
            ui,
            &colors,
            number,
            &format!("print-margin-{}", self.key),
            &mut self.settings.margin,
            0,
            1000,
            1,
            true,
        );
        kit::text_at(
            ui,
            Rect::from_min_max(pos2(number.right() + 8.0, row.top()), row.max),
            Align2::LEFT_CENTER,
            t(hp::PrintWindow_Pixels),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
        );
        let mut y = row.bottom() + 3.0 + 6.0;
        let scale = self.settings.auto_scale_image;
        let s = &mut self.settings;
        for (indent, label, value, enabled) in [
            (
                0.0,
                t(hp::PrintWindow_Auto_rotate_image),
                &mut s.auto_rotate_image,
                true,
            ),
            (
                0.0,
                t(hp::PrintWindow_Auto_scale_image),
                &mut s.auto_scale_image,
                true,
            ),
            (
                24.0,
                t(hp::PrintWindow_Allow_image_to_enlarge),
                &mut s.allow_enlarge_image,
                scale,
            ),
            (
                24.0,
                t(hp::PrintWindow_Center_image_position),
                &mut s.center_image,
                scale,
            ),
        ] {
            let r = Rect::from_min_size(
                pos2(inner.left() + indent, y),
                vec2(inner.width() - indent, 24.0),
            );
            changed |= kit::check_normal_at(ui, &colors, r, value, label, enabled);
            y += 24.0 + 6.0;
        }
        if changed {
            state.save_app_options("PrintSettings", &self.settings);
        }

        // Preview, Print, Cancel.
        let buttons = Rect::from_min_size(
            pos2(full.left() + 18.0, panel.bottom() + 12.0),
            vec2(full.width() - 36.0, 36.0),
        );
        let compact =
            |ui: &Ui, label: &str| kit::button_width(ui, &colors, None, Some(label)).max(92.0);
        let preview = t(hp::PrintWindow_Preview);
        let print_label = format!(
            "{}{}",
            t(hp::PrintForm_LoadSettings_Print),
            if self.settings.show_print_dialog {
                "..."
            } else {
                ""
            }
        );
        let cancel = t(hp::MyMessageBox_MyMessageBox_Cancel);
        let r = Rect::from_min_size(buttons.min, vec2(compact(ui, preview), 36.0));
        if kit::button_at(ui, &colors, r, None, Some(preview), true).clicked() {
            let image = self.image.clone();
            let settings = self.settings.clone();
            state.print_preview(&image, &settings);
        }
        let cancel_r = Rect::from_min_size(
            pos2(buttons.right() - compact(ui, cancel), buttons.top()),
            vec2(compact(ui, cancel), 36.0),
        );
        let print_w = compact(ui, &print_label);
        let print_r = Rect::from_min_size(
            pos2(cancel_r.left() - 7.0 - print_w, buttons.top()),
            vec2(print_w, 36.0),
        );
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter)) && !super::history::typing(ui);
        if (kit::primary_at(
            ui,
            &colors,
            print_r,
            None,
            Some(&print_label),
            !self.preview_only,
        )
        .clicked()
            || (enter && !self.preview_only))
            && !self.close
        {
            let image = self.image.clone();
            let settings = self.settings.clone();
            state.print_now(&image, &settings);
            self.close = true;
        }
        if kit::button_at(ui, &colors, cancel_r, None, Some(cancel), true).clicked() {
            self.close = true;
        }
        self.height = Some((buttons.bottom() + 18.0 - full.top()).ceil());
        self.close
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
