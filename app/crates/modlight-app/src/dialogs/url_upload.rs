//! "Upload from URL" (`URLUploadWindow`): 620 wide, the header band, one address in an
//! input card, Cancel and Upload. Upload is offered only for an absolute http, https or
//! ftp address (`URLInputValidation.IsSupported`).

use egui::{Align2, Rect, Sense, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{s, t};

use super::background;
use super::chrome::{header_band, input_card};
use crate::theme::{Colors, Weight};
use crate::widgets::{Content, Field, Look, button_at, button_width, layout, paint_text, wrapped};

pub const WIDTH: f32 = 620.0;

#[derive(Clone, Debug, PartialEq)]
pub struct UrlUpload {
    pub url: String,
    select_initial: bool,
    focused: bool,
    pub content_height: f32,
}

/// `URLInputValidation.IsSupported`.
pub fn is_supported(url: &str) -> bool {
    match url::Url::parse(url) {
        Ok(parsed) => matches!(parsed.scheme(), "http" | "https" | "ftp") && parsed.has_host(),
        Err(_) => false,
    }
}

impl UrlUpload {
    pub fn new(initial: Option<&str>) -> UrlUpload {
        UrlUpload {
            url: initial.unwrap_or_default().to_string(),
            select_initial: initial.is_some_and(|i| !i.is_empty()),
            focused: false,
            content_height: 240.0,
        }
    }

    /// `Some(url)` on Upload, `Some(None)`... no: `Some(None)` is Cancel.
    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors) -> Option<Option<String>> {
        background(ui, colors);
        let full = ui.max_rect();
        let band = header_band(
            ui,
            colors,
            full.min,
            full.width(),
            icons::LINK,
            t(s::URLUploadWindow_Heading),
            19.0,
            Some(t(s::URLUploadWindow_Prompt)),
            6.0,
            false,
        );
        let trimmed_url = self.url.trim().to_string();
        let valid = is_supported(&trimmed_url);
        let show_error = !trimmed_url.is_empty() && !valid;
        let inner_left = full.left() + 18.0 + 12.0;
        let inner_width = full.width() - 36.0 - 24.0;
        let error = show_error.then(|| {
            layout(
                ui,
                wrapped(
                    t(s::URLUploadWindow_Validation),
                    12.0,
                    Weight::Normal,
                    super::ERROR_COLOR,
                    inner_width,
                ),
            )
        });
        let card_height = 12.0
            + 18.0
            + 6.0
            + 32.0
            + error.as_ref().map(|g| 6.0 + g.size().y).unwrap_or(0.0)
            + 12.0;
        let card = Rect::from_min_size(
            pos2(full.left() + 18.0, band.bottom() + 14.0),
            vec2(full.width() - 36.0, card_height),
        );
        input_card(ui, colors, card);
        paint_text(
            ui,
            ui.painter(),
            Rect::from_min_size(pos2(inner_left, card.top() + 12.0), vec2(inner_width, 18.0)),
            Align2::LEFT_CENTER,
            t(s::URLUploadWindow_URL),
            13.0,
            Weight::SemiBold,
            colors.text_secondary(),
        );
        let field_rect = Rect::from_min_size(
            pos2(inner_left, card.top() + 12.0 + 18.0 + 6.0),
            vec2(inner_width, 32.0),
        );
        let id = egui::Id::new("url-upload-field");
        let mut field_ui = ui.new_child(egui::UiBuilder::new().max_rect(field_rect));
        let response = Field {
            height: 32.0,
            width: Some(inner_width),
            hint: "https://example.com/file",
            id: Some(id),
            ..Field::new(&mut self.url)
        }
        .show(&mut field_ui, colors);
        if !self.focused {
            response.request_focus();
            if self.select_initial {
                select_all(ui, id, &self.url);
            }
            self.focused = true;
        }
        if let Some(error) = error {
            ui.painter().galley(
                pos2(inner_left, field_rect.bottom() + 6.0),
                error,
                super::ERROR_COLOR,
            );
        }

        // Cancel, then Upload; 7 px apart; margin 18,0,18,18.
        let top = card.bottom() + 14.0;
        let (cancel, upload) = (t(s::URLUploadWindow_Cancel), t(s::URLUploadWindow_Upload));
        let (plain, primary) = compact_looks(colors);
        let upload_w = button_width(ui, &primary, Content::label(upload));
        let cancel_w = button_width(ui, &plain, Content::label(cancel));
        let upload_rect = Rect::from_min_size(
            pos2(full.right() - 18.0 - upload_w, top),
            vec2(upload_w, 40.0),
        );
        let cancel_rect = Rect::from_min_size(
            pos2(upload_rect.left() - 7.0 - cancel_w, top),
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
        {
            outcome = Some(None);
        }
        if (button_at(
            ui,
            colors,
            &primary,
            Content::label(upload),
            valid,
            upload_rect,
        )
        .clicked()
            || (valid && ui.input(|i| i.key_pressed(egui::Key::Enter))))
            && valid
        {
            outcome = Some(Some(trimmed_url));
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            outcome = Some(None);
        }
        ui.allocate_rect(
            Rect::from_min_max(full.min, pos2(full.right(), top + 40.0 + 18.0)),
            Sense::hover(),
        );
        self.content_height = top + 40.0 + 18.0 - full.top();
        outcome
    }
}

/// `Button.compact` and `Button.compact.primary`: at least 96 wide, 14 px padding.
pub fn compact_looks(colors: &Colors) -> (Look, Look) {
    let mut plain = Look::compact(colors);
    plain.min_width = 96.0;
    plain.padding_x = 14.0;
    let mut primary = Look::primary(colors);
    primary.min_width = 96.0;
    primary.padding_x = 14.0;
    (plain, primary)
}

/// Select every character of the field `id`.
pub fn select_all(ui: &Ui, id: egui::Id, text: &str) {
    let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    let range = egui::text::CCursorRange::two(
        egui::text::CCursor::new(0),
        egui::text::CCursor::new(text.chars().count()),
    );
    state.cursor.set_char_range(Some(range));
    state.store(ui.ctx(), id);
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_web_and_ftp_addresses() {
        assert!(super::is_supported("https://example.com/a.png"));
        assert!(super::is_supported("ftp://example.com/a"));
        assert!(!super::is_supported("example.com/a.png"));
        assert!(!super::is_supported("file:///etc/hosts"));
        assert!(!super::is_supported("javascript:alert(1)"));
    }
}
