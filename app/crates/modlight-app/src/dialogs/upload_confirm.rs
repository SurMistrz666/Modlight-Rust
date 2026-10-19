//! The two questions asked before an upload starts (`UploadConfirmation`): the large-file
//! warning and the many-files confirmation. Same window: 440 wide, 18 px margin, the
//! message in a panel with a 32 px icon and a "don't show again" box, the buttons under
//! it with the primary one first.

use egui::{Rect, Sense, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t};

use super::background;
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Look, button_at, button_width, checkbox_weighted, layout, paint_icon, wrapped,
};

pub const WIDTH: f32 = 440.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadQuestion {
    /// `LargeFileUploadWarningWindow`.
    LargeFile,
    /// `MultiUploadConfirmationWindow` for this many files.
    ManyFiles(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct UploadConfirm {
    pub question: UploadQuestion,
    pub dont_show_again: bool,
    pub content_height: f32,
}

/// What the person answered: go on or not, and whether to stop asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UploadConfirmOutcome {
    pub go_on: bool,
    pub dont_show_again: bool,
}

impl UploadConfirm {
    pub fn new(question: UploadQuestion) -> UploadConfirm {
        UploadConfirm {
            question,
            dont_show_again: false,
            content_height: 180.0,
        }
    }

    pub fn title(&self) -> &'static str {
        match self.question {
            UploadQuestion::LargeFile => t(s::LargeFileUploadWarningWindow_Title),
            UploadQuestion::ManyFiles(_) => t(s::MultiUploadConfirmationWindow_Title),
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors) -> Option<UploadConfirmOutcome> {
        background(ui, colors);
        let full = ui.max_rect();
        let (message, glyph, glyph_color, go, cancel, dont_show) = match self.question {
            UploadQuestion::LargeFile => (
                t(s::LargeFileUploadWarningWindow_Message).to_string(),
                icons::TRIANGLE_ALERT,
                colors.cancel_border(),
                t(s::LargeFileUploadWarningWindow_Continue),
                t(s::LargeFileUploadWarningWindow_Cancel),
                t(s::LargeFileUploadWarningWindow_DontShowAgain),
            ),
            UploadQuestion::ManyFiles(count) => (
                format(
                    t(s::MultiUploadConfirmationWindow_Message),
                    &[Arg::from(count)],
                ),
                icons::FILES,
                colors.accent_start(),
                t(s::MultiUploadConfirmationWindow_Upload),
                t(s::MultiUploadConfirmationWindow_Cancel),
                t(s::MultiUploadConfirmationWindow_DontShowAgain),
            ),
        };

        // The panel: 14 px padding, a 32 px icon column, 16 px gap, text then checkbox.
        let panel_left = full.left() + 18.0;
        let panel_width = full.width() - 36.0;
        let icon_width = 32.0;
        let text_left = panel_left + 14.0 + icon_width + 16.0;
        let text_width = (panel_left + panel_width - 14.0 - text_left).max(60.0);
        let galley = layout(
            ui,
            wrapped(&message, 15.0, Weight::Normal, colors.text(), text_width),
        );
        let checkbox_height = 32.0;
        let inner_height = galley.size().y.max(icon_width) + 14.0 + checkbox_height;
        let panel = Rect::from_min_size(
            pos2(panel_left, full.top() + 18.0),
            vec2(panel_width, inner_height + 28.0),
        );
        ui.allocate_rect(
            Rect::from_min_size(full.min, vec2(full.width(), panel.bottom() - full.top())),
            Sense::hover(),
        );
        let painter = ui.painter();
        painter.rect_filled(panel, 8.0, colors.background_panel());
        painter.rect_stroke(
            panel,
            8.0,
            egui::Stroke::new(1.0, colors.border()),
            egui::StrokeKind::Inside,
        );
        paint_icon(
            ui,
            painter,
            pos2(
                panel.left() + 14.0 + icon_width / 2.0,
                panel.top() + 14.0 + 16.0,
            ),
            glyph,
            32.0,
            glyph_color,
        );
        let text_top = panel.top() + 14.0;
        let text_height = galley.size().y;
        let text_y = text_top + (icon_width - text_height).max(0.0) / 2.0;
        painter.galley(pos2(text_left, text_y), galley, colors.text());
        let check_top = text_top + text_height.max(icon_width) + 14.0;
        let mut check_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    pos2(text_left, check_top),
                    vec2(text_width, checkbox_height),
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        checkbox_weighted(
            &mut check_ui,
            colors,
            &mut self.dont_show_again,
            Some(dont_show),
            true,
            Weight::Normal,
        );

        // The buttons: primary then cancel, right-aligned, 7 px apart.
        let mut primary = Look::primary(colors);
        primary.min_width = 96.0;
        primary.padding_x = 14.0;
        let mut plain = Look::button(colors);
        plain.min_width = 96.0;
        plain.padding_x = 14.0;
        let top = panel.bottom() + 12.0;
        let cancel_w = button_width(ui, &plain, Content::label(cancel));
        let go_w = button_width(ui, &primary, Content::label(go));
        let cancel_rect = Rect::from_min_size(
            pos2(full.right() - 18.0 - cancel_w, top),
            vec2(cancel_w, 40.0),
        );
        let go_rect =
            Rect::from_min_size(pos2(cancel_rect.left() - 7.0 - go_w, top), vec2(go_w, 40.0));
        let mut outcome = None;
        if button_at(ui, colors, &primary, Content::label(go), true, go_rect).clicked()
            || ui.input(|i| i.key_pressed(egui::Key::Enter))
        {
            outcome = Some(true);
        }
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
            outcome = Some(false);
        }
        self.content_height = (top + 40.0 + 18.0) - full.top();
        outcome.map(|go_on| UploadConfirmOutcome {
            go_on,
            dont_show_again: self.dont_show_again,
        })
    }
}
