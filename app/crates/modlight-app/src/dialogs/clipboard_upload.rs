//! "Clipboard upload" (`ClipboardUploadWindow`): 620 by 500 (down to 480 by 380). What
//! the clipboard holds, shown before it goes: a picture on a checkered surface, text in
//! a read-only box, or the list of files. "Don't show this again" when opened from the
//! main window; Cancel and Upload.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t};
use modlight_core::tasks::upload_manager::ClipboardContent;

use super::background;
use super::chrome::header_band;
use super::url_upload::compact_looks;
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, button_at, button_width, checkbox_weighted, layout, paint_checkered, trimmed, wrapped,
};

pub const WIDTH: f32 = 620.0;
pub const HEIGHT: f32 = 500.0;
pub const MIN_WIDTH: f32 = 480.0;
pub const MIN_HEIGHT: f32 = 380.0;

pub struct ClipboardUpload {
    pub content: Option<ClipboardContent>,
    pub show_dont_show_again: bool,
    pub dont_show_again: bool,
    image: Option<Arc<image::RgbaImage>>,
    texture: Option<egui::TextureHandle>,
    text_view: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardOutcome {
    Upload,
    Cancel,
}

impl ClipboardUpload {
    pub fn new(content: Option<ClipboardContent>, show_dont_show_again: bool) -> ClipboardUpload {
        let image = match &content {
            Some(ClipboardContent::Image(image)) => Some(Arc::new(image.clone())),
            _ => None,
        };
        let text_view = match &content {
            Some(ClipboardContent::Text(text)) => text.clone(),
            _ => String::new(),
        };
        ClipboardUpload {
            content,
            show_dont_show_again,
            dont_show_again: false,
            image,
            texture: None,
            text_view,
        }
    }

    pub fn title() -> String {
        format!("Modlight - {}", t(s::ClipboardUploadWindow_ClipboardUpload))
    }

    fn summary(&self) -> String {
        match &self.content {
            Some(ClipboardContent::Image(image)) => format(
                t(s::ClipboardUploadWindow_ImageSummary),
                &[
                    Arg::from(image.width() as i64),
                    Arg::from(image.height() as i64),
                ],
            ),
            Some(ClipboardContent::Text(text)) => format(
                t(s::ClipboardUploadWindow_TextSummary),
                &[Arg::from(text.encode_utf16().count())],
            ),
            Some(ClipboardContent::Files(files)) => format(
                t(s::ClipboardUploadWindow_FileSummary),
                &[Arg::from(files.len())],
            ),
            None => t(s::ClipboardUploadWindow_Empty).to_string(),
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors) -> Option<ClipboardOutcome> {
        background(ui, colors);
        let full = ui.max_rect();
        let summary = self.summary();
        let band = header_band(
            ui,
            colors,
            full.min,
            full.width(),
            icons::CLIPBOARD,
            t(s::ClipboardUploadWindow_ClipboardUpload),
            19.0,
            Some(&summary),
            6.0,
        );
        let footer_top = full.bottom() - 18.0 - 40.0;
        let card = Rect::from_min_max(
            pos2(full.left() + 18.0, band.bottom() + 14.0),
            pos2(full.right() - 18.0, footer_top - 14.0),
        );
        let painter = ui.painter().clone();
        painter.rect_filled(card, 6.0, colors.background_panel());
        painter.rect_stroke(
            card,
            6.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        let inner = card.shrink(1.0);
        match &self.content {
            Some(ClipboardContent::Image(_)) => {
                paint_checkered(ui, colors, inner, 5.0);
                if let Some(image) = &self.image {
                    let texture = self.texture.get_or_insert_with(|| {
                        let color = egui::ColorImage::from_rgba_unmultiplied(
                            [image.width() as usize, image.height() as usize],
                            image.as_raw(),
                        );
                        ui.ctx().load_texture(
                            "clipboard-preview",
                            color,
                            egui::TextureOptions::LINEAR,
                        )
                    });
                    let area = inner.shrink(10.0);
                    let (w, h) = (image.width() as f32, image.height() as f32);
                    let scale = (area.width() / w).min(area.height() / h);
                    let rect = Rect::from_center_size(area.center(), vec2(w * scale, h * scale));
                    painter.image(
                        texture.id(),
                        rect,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
            }
            Some(ClipboardContent::Text(_)) => {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
                egui::ScrollArea::both()
                    .id_salt("clipboard-text")
                    .auto_shrink([false, false])
                    .show(&mut child, |ui| {
                        egui::TextEdit::multiline(&mut self.text_view.as_str())
                            .frame(egui::Frame::NONE)
                            .margin(egui::Margin::same(12))
                            .font(egui::FontId::proportional(13.0))
                            .text_color(colors.text())
                            .desired_width(f32::INFINITY)
                            .show(ui);
                    });
            }
            Some(ClipboardContent::Files(files)) => {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner.shrink(6.0)));
                let files: Vec<PathBuf> = files.clone();
                egui::ScrollArea::vertical()
                    .id_salt("clipboard-files")
                    .auto_shrink([false, false])
                    .show(&mut child, |ui| {
                        for file in &files {
                            let text = file.to_string_lossy();
                            let galley = layout(
                                ui,
                                trimmed(
                                    &text,
                                    13.0,
                                    Weight::Normal,
                                    colors.text(),
                                    inner.width() - 32.0,
                                    1,
                                ),
                            );
                            let (row, _) = ui.allocate_exact_size(
                                vec2(inner.width() - 12.0, 32.0),
                                Sense::hover(),
                            );
                            crate::widgets::paint_text_left(
                                ui.painter(),
                                row.left() + 8.0,
                                row.center().y,
                                galley,
                                colors.text(),
                            );
                        }
                    });
            }
            None => {
                let galley = layout(ui, {
                    let mut job = wrapped(
                        &summary,
                        13.0,
                        Weight::Normal,
                        colors.text_secondary(),
                        inner.width() - 48.0,
                    );
                    job.halign = egui::Align::Center;
                    job
                });
                painter.galley(
                    pos2(inner.center().x, inner.center().y - galley.size().y / 2.0),
                    galley,
                    colors.text_secondary(),
                );
            }
        }

        // Footer: the checkbox on the left, Cancel and Upload on the right.
        if self.show_dont_show_again {
            let mut check_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_size(
                        pos2(full.left() + 18.0, footer_top),
                        vec2(full.width() * 0.5, 40.0),
                    ))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            checkbox_weighted(
                &mut check_ui,
                colors,
                &mut self.dont_show_again,
                Some(t(s::ClipboardUploadWindow_DontShowAgain)),
                true,
                Weight::Normal,
            );
        }
        let (mut plain, mut primary) = compact_looks(colors);
        plain.min_width = 92.0;
        primary.min_width = 92.0;
        let (cancel, upload) = (
            t(s::ClipboardUploadWindow_Cancel),
            t(s::ClipboardUploadWindow_Upload),
        );
        let can_upload = self.content.is_some();
        let upload_w = button_width(ui, &primary, Content::label(upload));
        let cancel_w = button_width(ui, &plain, Content::label(cancel));
        let upload_rect = Rect::from_min_size(
            pos2(full.right() - 18.0 - upload_w, footer_top),
            vec2(upload_w, 40.0),
        );
        let cancel_rect = Rect::from_min_size(
            pos2(upload_rect.left() - 7.0 - cancel_w, footer_top),
            vec2(cancel_w, 40.0),
        );
        let mut outcome = None;
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
            outcome = Some(ClipboardOutcome::Cancel);
        }
        if (button_at(
            ui,
            colors,
            &primary,
            Content::label(upload),
            can_upload,
            upload_rect,
        )
        .clicked()
            || (can_upload && ui.input(|i| i.key_pressed(egui::Key::Enter))))
            && can_upload
        {
            outcome = Some(ClipboardOutcome::Upload);
        }
        outcome
    }
}
