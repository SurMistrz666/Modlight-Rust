//! "Text upload" (`ShareX.UploadersLib.TextUploadWindow`): 680 by 560 (down to 480 by
//! 380). A plain editor pre-filled with the clipboard's text, the character count, Cancel
//! and Upload. What it returns goes through `UploadManager.UploadText`.

use egui::{Rect, Stroke, StrokeKind, Ui, pos2};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, t, uploaders};

use super::background;
use super::chrome::header_band;
use super::url_upload::compact_looks;
use crate::theme::{Colors, Weight};
use crate::widgets::{Content, button_at, button_width, layout, trimmed};

pub const WIDTH: f32 = 680.0;
pub const HEIGHT: f32 = 560.0;
pub const MIN_WIDTH: f32 = 480.0;
pub const MIN_HEIGHT: f32 = 380.0;

#[derive(Clone, Debug, PartialEq)]
pub struct TextUpload {
    pub content: String,
    select_initial: bool,
    opened: bool,
}

impl TextUpload {
    pub fn new(clipboard: Option<String>) -> TextUpload {
        let content = clipboard.unwrap_or_default();
        TextUpload {
            select_initial: !content.is_empty(),
            content,
            opened: false,
        }
    }

    pub fn title() -> &'static str {
        t(uploaders::TextUploadWindow_ShareX_Text_upload)
    }

    /// `Some(Some(text))` on Upload, `Some(None)` on Cancel.
    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors) -> Option<Option<String>> {
        background(ui, colors);
        let full = ui.max_rect();
        let band = header_band(
            ui,
            colors,
            full.min,
            full.width(),
            icons::FILE_TEXT,
            t(uploaders::TextUploadWindow_Text_upload),
            19.0,
            Some(t(
                uploaders::TextUploadWindow_Enter_or_paste_the_text_you_want_to_upload,
            )),
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
        let id = egui::Id::new("text-upload-editor");
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(card.shrink(1.0)));
        egui::ScrollArea::both()
            .id_salt("text-upload-scroll")
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                let output = egui::TextEdit::multiline(&mut self.content)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .margin(egui::Margin::same(12))
                    .font(egui::FontId::proportional(13.0))
                    .text_color(colors.text())
                    .desired_width(f32::INFINITY)
                    .min_size(card.size() - egui::vec2(2.0, 2.0))
                    .show(ui);
                if !self.opened {
                    output.response.request_focus();
                    if self.select_initial {
                        let mut state = output.state.clone();
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(self.content.chars().count()),
                            )));
                        state.store(ui.ctx(), id);
                    }
                    self.opened = true;
                }
            });

        // The count: UTF-16 units, as `Text.Length`.
        let length = self.content.encode_utf16().count();
        let template = if length == 1 {
            t(uploaders::TextUploadWindow_Character_count_singular)
        } else {
            t(uploaders::TextUploadWindow_Character_count_plural)
        };
        let count = layout(
            ui,
            trimmed(
                &format(template, &[Arg::from(length)]),
                13.0,
                Weight::Normal,
                colors.text_secondary(),
                full.width() * 0.5,
                1,
            ),
        );
        crate::widgets::paint_text_left(
            &painter,
            full.left() + 18.0,
            footer_top + 20.0,
            count,
            colors.text_secondary(),
        );

        let (plain, primary) = compact_looks(colors);
        let (cancel, upload) = (
            t(uploaders::TextUploadWindow_Cancel),
            t(uploaders::TextUploadWindow_Upload),
        );
        let upload_w = button_width(ui, &primary, Content::label(upload));
        let cancel_w = button_width(ui, &plain, Content::label(cancel));
        let upload_rect = Rect::from_min_size(
            pos2(full.right() - 18.0 - upload_w, footer_top),
            egui::vec2(upload_w, 40.0),
        );
        let cancel_rect = Rect::from_min_size(
            pos2(upload_rect.left() - 7.0 - cancel_w, footer_top),
            egui::vec2(cancel_w, 40.0),
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
            outcome = Some(None);
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
        {
            outcome = Some(Some(self.content.clone()));
        }
        outcome
    }
}
