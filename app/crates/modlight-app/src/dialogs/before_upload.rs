//! "Before upload" (`BeforeUploadWindow`): 900 by 560 (down to 720 by 460), topmost.
//! The destinations this kind of upload can go to as radio buttons on the left, the
//! picture (or the file's name) on the right, Cancel and Upload under both. Picking a
//! destination rewrites the task's destination settings, as upstream's options do.

use std::path::Path;
use std::sync::Arc;

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t};
use modlight_core::tasks::info::{DataType, TaskInfo};
use modlight_core::tasks::settings::{
    FileDestination, ImageDestination, TaskSettings, TextDestination,
};

use super::background;
use super::chrome::header_band;
use super::url_upload::compact_looks;
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, button_at, button_width, layout, paint_checkered, paint_icon, paint_text, trimmed,
    wrapped,
};

pub const WIDTH: f32 = 900.0;
pub const HEIGHT: f32 = 560.0;
pub const MIN_WIDTH: f32 = 720.0;
pub const MIN_HEIGHT: f32 = 460.0;

/// One radio button (`BeforeUploadDestinationOption`) and what choosing it sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Destination {
    Image(ImageDestination),
    ImageFile(FileDestination),
    Text(TextDestination),
    TextFile(FileDestination),
    File(FileDestination),
    Shortener,
}

pub struct BeforeUpload {
    pub task: u64,
    pub info: TaskInfo,
    options: Vec<(Destination, String)>,
    selected: Option<usize>,
    pub image: Option<Arc<image::RgbaImage>>,
    texture: Option<egui::TextureHandle>,
}

/// What the person decided: upload (with the settings the destination choice left) or not.
#[derive(Clone, Debug, PartialEq)]
pub enum BeforeUploadOutcome {
    Upload(Box<TaskSettings>),
    Cancel,
    CopyImage,
}

impl BeforeUpload {
    pub fn new(task: u64, info: TaskInfo, signed_in: bool) -> BeforeUpload {
        let settings = &info.settings;
        let mut options = Vec::new();
        let mut selected = None;
        // Only destinations whose account is configured are offered
        // (`UploadersConfigValidator`): Modlight, when signed in.
        if signed_in {
            let mut add = |destination: Destination, is_selected: bool| {
                if is_selected && selected.is_none() {
                    selected = Some(options.len());
                }
                options.push((destination, "Modlight".to_string()));
            };
            match info.data_type {
                DataType::Image => {
                    add(
                        Destination::Image(ImageDestination::Modlight),
                        settings.image_destination == ImageDestination::Modlight,
                    );
                    add(
                        Destination::ImageFile(FileDestination::Modlight),
                        settings.image_destination == ImageDestination::FileUploader
                            && settings.image_file_destination == FileDestination::Modlight,
                    );
                }
                DataType::Text => {
                    add(
                        Destination::Text(TextDestination::Modlight),
                        settings.text_destination == TextDestination::Modlight,
                    );
                    add(
                        Destination::TextFile(FileDestination::Modlight),
                        settings.text_destination == TextDestination::FileUploader
                            && settings.text_file_destination == FileDestination::Modlight,
                    );
                }
                DataType::File => add(
                    Destination::File(FileDestination::Modlight),
                    settings.file_destination == FileDestination::Modlight,
                ),
                DataType::Url => add(Destination::Shortener, true),
                DataType::Default => {}
            }
        }
        let mut image = info.metadata.image.clone();
        if image.is_none() {
            let path = info.file_path();
            if !path.is_empty()
                && Path::new(path).is_file()
                && modlight_core::names::file_helpers::is_image_file(path)
            {
                image = image::open(path).ok().map(|i| Arc::new(i.to_rgba8()));
            }
        }
        BeforeUpload {
            task,
            info,
            options,
            selected,
            image,
            texture: None,
        }
    }

    fn prompt(&self) -> String {
        match self.selected.and_then(|i| self.options.get(i)) {
            Some((_, label)) => format(
                t(s::BeforeUploadWindow_UploadPrompt),
                &[Arg::from(&self.info.file_name), Arg::from(label)],
            ),
            None => t(s::BeforeUploadWindow_ChooseDestination).to_string(),
        }
    }

    fn choose(&mut self, index: usize) {
        self.selected = Some(index);
        let settings = &mut self.info.settings;
        match self.options[index].0 {
            Destination::Image(destination) => settings.image_destination = destination,
            Destination::ImageFile(destination) => {
                settings.image_destination = ImageDestination::FileUploader;
                settings.image_file_destination = destination;
            }
            Destination::Text(destination) => settings.text_destination = destination,
            Destination::TextFile(destination) => {
                settings.text_destination = TextDestination::FileUploader;
                settings.text_file_destination = destination;
            }
            Destination::File(destination) => {
                settings.image_destination = ImageDestination::FileUploader;
                settings.text_destination = TextDestination::FileUploader;
                settings.image_file_destination = destination;
                settings.text_file_destination = destination;
                settings.file_destination = destination;
            }
            Destination::Shortener => {}
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors) -> Option<BeforeUploadOutcome> {
        background(ui, colors);
        let full = ui.max_rect();
        let prompt = self.prompt();
        let band = header_band(
            ui,
            colors,
            full.min,
            full.width(),
            icons::UPLOAD,
            t(s::BeforeUploadWindow_Heading),
            19.0,
            Some(&prompt),
            6.0,
        );
        let body = Rect::from_min_max(
            pos2(full.left() + 18.0, band.bottom() + 14.0),
            pos2(full.right() - 18.0, full.bottom() - 18.0),
        );
        let buttons_top = body.bottom() - 40.0;
        let list = Rect::from_min_max(body.min, pos2(body.left() + 360.0, buttons_top - 10.0));
        let preview = Rect::from_min_max(
            pos2(list.right() + 14.0, body.top()),
            pos2(body.right(), buttons_top - 10.0),
        );
        let painter = ui.painter().clone();
        let mut outcome = None;

        // Destinations.
        painter.rect_filled(list, 6.0, colors.background_panel());
        painter.rect_stroke(
            list,
            6.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        let inner = list.shrink(8.0);
        let head = Rect::from_min_size(inner.min, vec2(inner.width(), 7.0 + 18.0 + 7.0));
        painter.rect_filled(head, 6.0, colors.background_toolbar());
        paint_icon(
            ui,
            &painter,
            pos2(head.left() + 10.0 + 8.0, head.center().y),
            icons::UPLOAD,
            16.0,
            colors.accent_start(),
        );
        paint_text(
            ui,
            &painter,
            Rect::from_min_max(pos2(head.left() + 10.0 + 16.0 + 7.0, head.top()), head.max),
            Align2::LEFT_CENTER,
            t(s::BeforeUploadWindow_Destination),
            13.0,
            Weight::SemiBold,
            colors.text_secondary(),
        );
        let mut y = head.bottom() + 6.0 + 2.0;
        for index in 0..self.options.len() {
            let row = Rect::from_min_size(
                pos2(inner.left() + 4.0 + 2.0, y),
                vec2(inner.width() - 12.0, 28.0),
            );
            let response = ui.interact(
                row,
                egui::Id::new(("before-upload-option", index)),
                Sense::click(),
            );
            let checked = self.selected == Some(index);
            let circle = pos2(row.left() + 7.0 + 10.0, row.center().y);
            painter.circle_filled(
                circle,
                10.0,
                if checked {
                    colors.accent_start()
                } else {
                    colors.control_background()
                },
            );
            if checked {
                painter.circle_filled(circle, 4.0, colors.accent_foreground());
            } else {
                painter.circle_stroke(
                    circle,
                    9.5,
                    Stroke::new(
                        1.0,
                        if response.hovered() {
                            colors.accent_start()
                        } else {
                            colors.border()
                        },
                    ),
                );
            }
            let label = layout(
                ui,
                trimmed(
                    &self.options[index].1,
                    13.0,
                    Weight::Normal,
                    colors.text(),
                    row.width() - 40.0,
                    1,
                ),
            );
            crate::widgets::paint_text_left(
                &painter,
                circle.x + 10.0 + 8.0,
                row.center().y,
                label,
                colors.text(),
            );
            if response.clicked() {
                self.choose(index);
            }
            y += 28.0;
        }

        // The preview.
        paint_checkered(ui, colors, preview, 6.0);
        painter.rect_stroke(
            preview,
            6.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        if let Some(image) = &self.image {
            let texture = self.texture.get_or_insert_with(|| {
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                );
                ui.ctx().load_texture(
                    format!("before-upload-{}", self.task),
                    color,
                    egui::TextureOptions::LINEAR,
                )
            });
            let area = preview.shrink(10.0);
            let (w, h) = (image.width() as f32, image.height() as f32);
            let scale = (area.width() / w).min(area.height() / h);
            let rect = Rect::from_center_size(area.center(), vec2(w * scale, h * scale));
            painter.image(
                texture.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            let badge_text = format!("{} × {}", image.width(), image.height());
            let galley = layout(
                ui,
                trimmed(&badge_text, 12.0, Weight::Normal, Color32::WHITE, 200.0, 1),
            );
            let badge = Rect::from_min_size(
                pos2(
                    preview.right() - 8.0 - galley.size().x - 14.0,
                    preview.bottom() - 8.0 - galley.size().y - 6.0,
                ),
                vec2(galley.size().x + 14.0, galley.size().y + 6.0),
            );
            painter.rect_filled(
                badge,
                4.0,
                Color32::from_rgba_unmultiplied(0x20, 0x20, 0x20, 0xCC),
            );
            painter.galley(badge.min + vec2(7.0, 3.0), galley, Color32::WHITE);
            let surface = ui.interact(
                preview,
                egui::Id::new(("before-upload-preview", self.task)),
                Sense::click(),
            );
            egui::Popup::context_menu(&surface)
                .frame(crate::menus::frame(colors))
                .show(|ui| {
                    let entries = [crate::menus::Entry::item(
                        t(s::BeforeUploadWindow_CopyPreviewImage),
                        icons::IMAGE,
                        crate::commands::Command::NotYet,
                    )];
                    if crate::menus::entries(ui, colors, &entries).is_some() {
                        outcome = Some(BeforeUploadOutcome::CopyImage);
                    }
                });
        } else {
            let text = if self.info.file_name.is_empty() {
                t(s::BeforeUploadWindow_NoPreview).to_string()
            } else {
                self.info.file_name.clone()
            };
            let note = layout(ui, {
                let mut job = wrapped(
                    &text,
                    13.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    320.0_f32.min(preview.width() - 20.0),
                );
                job.halign = egui::Align::Center;
                job
            });
            let total = 48.0 + 8.0 + note.size().y;
            let top = preview.center().y - total / 2.0;
            let mut faded = painter.clone();
            faded.set_opacity(0.45);
            paint_icon(
                ui,
                &faded,
                pos2(preview.center().x, top + 24.0),
                icons::FILE_TEXT,
                48.0,
                colors.text_secondary(),
            );
            painter.galley(
                pos2(preview.center().x, top + 48.0 + 8.0),
                note,
                colors.text_secondary(),
            );
        }

        // Cancel, Upload.
        let (plain, primary) = compact_looks(colors);
        let (cancel, upload) = (
            t(s::BeforeUploadWindow_Cancel),
            t(s::BeforeUploadWindow_Upload),
        );
        let upload_w = button_width(ui, &primary, Content::label(upload));
        let cancel_w = button_width(ui, &plain, Content::label(cancel));
        let upload_rect = Rect::from_min_size(
            pos2(body.right() - upload_w, buttons_top),
            vec2(upload_w, 40.0),
        );
        let cancel_rect = Rect::from_min_size(
            pos2(upload_rect.left() - 7.0 - cancel_w, buttons_top),
            vec2(cancel_w, 40.0),
        );
        if button_at(
            ui,
            colors,
            &plain,
            Content::label(cancel),
            true,
            cancel_rect,
        )
        .clicked()
            || ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            outcome = Some(BeforeUploadOutcome::Cancel);
        }
        if button_at(
            ui,
            colors,
            &primary,
            Content::label(upload),
            true,
            upload_rect,
        )
        .clicked()
            || ui.input(|i| i.key_pressed(egui::Key::Enter))
        {
            outcome = Some(BeforeUploadOutcome::Upload(Box::new(
                self.info.settings.clone(),
            )));
        }
        outcome
    }
}
