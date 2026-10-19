//! The Yes/No message box (`ShareX.AvaloniaUI.MessageBox` with `MessageBoxIcon.Warning`
//! and the second button as default): 480 wide, the icon in a 48 px tile, the buttons
//! in a toolbar-coloured footer.

use egui::{Align2, Rect, Sense, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{avalonia, t};

use super::{WARNING_COLOR, background, footer};
use crate::theme::{Colors, Weight};
use crate::widgets::{Content, Look, button_at, button_width, layout, paint_icon, wrapped};

/// What happens on Yes.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfirmAction {
    Trash(Vec<String>),
    DeleteForever(Vec<String>),
    EmptyTrash {
        ids: Vec<String>,
        bytes: i64,
    },
    DuplicatesTrash,
    /// A file on this Mac, to the Trash (the desktop notice's "Delete file").
    DeleteLocalFile(std::path::PathBuf),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Confirm {
    /// The window title: "Modlight - `<title>`".
    pub title: String,
    pub text: String,
    pub action: ConfirmAction,
    /// The height the content asked for last frame.
    pub content_height: f32,
}

pub const WIDTH: f32 = 480.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmOutcome {
    Yes,
    No,
}

impl Confirm {
    pub fn new(title: &str, text: &str, action: ConfirmAction) -> Confirm {
        Confirm {
            title: format!("{} - {}", modlight_core::APP_NAME, title),
            text: text.to_string(),
            action,
            content_height: 160.0,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors) -> Option<ConfirmOutcome> {
        background(ui, colors);
        let full = ui.max_rect();
        let width = full.width();

        // Message area: 22,22,22,20 margins, a 48 px icon tile, 16 px gap, wrapped text.
        let text_width = (width - 22.0 - 22.0 - 48.0 - 16.0).max(80.0);
        let galley = layout(
            ui,
            wrapped(&self.text, 13.0, Weight::Normal, colors.text(), text_width),
        );
        let body_height = galley.size().y.max(48.0) + 22.0 + 20.0;
        let (body, _) = ui.allocate_exact_size(vec2(width, body_height), Sense::hover());
        let painter = ui.painter();
        let tile = Rect::from_min_size(
            pos2(body.left() + 22.0, body.top() + 22.0),
            vec2(48.0, 48.0),
        );
        painter.rect_filled(tile, 8.0, colors.background_panel());
        paint_icon(
            ui,
            painter,
            tile.center(),
            icons::TRIANGLE_ALERT,
            24.0,
            WARNING_COLOR,
        );
        let text_top = body.top() + 22.0 + (48.0 - galley.size().y).max(0.0) / 2.0;
        painter.galley(pos2(tile.right() + 16.0, text_top), galley, colors.text());

        // Footer: Yes, No (No is the default, drawn as the active one).
        let bar = footer(ui, colors, 36.0 + 24.0);
        let yes = t(avalonia::MessageBox_Yes);
        let no = t(avalonia::MessageBox_No);
        let plain = Look::button(colors).height(36.0);
        let mut active = Look::primary(colors).height(36.0);
        active.min_width = 88.0;
        active.padding_x = 10.0;
        let mut plain_sized = plain.clone();
        plain_sized.min_width = 88.0;
        let no_w = button_width(ui, &active, Content::label(no));
        let yes_w = button_width(ui, &plain_sized, Content::label(yes));
        let right = bar.right() - 14.0;
        let no_rect = Rect::from_min_size(pos2(right - no_w, bar.top() + 12.0), vec2(no_w, 36.0));
        let yes_rect = Rect::from_min_size(
            pos2(no_rect.left() - 8.0 - yes_w, bar.top() + 12.0),
            vec2(yes_w, 36.0),
        );
        let mut outcome = None;
        if button_at(
            ui,
            colors,
            &plain_sized,
            Content::label(yes),
            true,
            yes_rect,
        )
        .clicked()
        {
            outcome = Some(ConfirmOutcome::Yes);
        }
        if button_at(ui, colors, &active, Content::label(no), true, no_rect).clicked() {
            outcome = Some(ConfirmOutcome::No);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter)) {
            outcome = Some(ConfirmOutcome::No);
        }
        self.content_height = body_height + 60.0;
        let _ = Align2::CENTER_CENTER;
        outcome
    }
}
