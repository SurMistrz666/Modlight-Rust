//! The OK message box (`ShareX.AvaloniaUI.MessageBox` with `MessageBoxButtons.OK`): the
//! same 480-wide window as the Yes/No one, the icon in a 48 px tile, one button.

use egui::{Rect, Sense, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{avalonia, t};
use modlight_core::tasks::host::MessageIcon;

use super::{ERROR_COLOR, WARNING_COLOR, background, footer};
use crate::theme::{Colors, Weight};
use crate::widgets::{Content, Look, button_at, button_width, layout, paint_icon, wrapped};

pub const WIDTH: f32 = 480.0;

#[derive(Clone, Debug, PartialEq)]
pub struct MessageBox {
    pub title: String,
    pub text: String,
    pub icon: MessageIcon,
    pub content_height: f32,
}

impl MessageBox {
    pub fn new(title: &str, text: &str, icon: MessageIcon) -> MessageBox {
        MessageBox {
            title: title.to_string(),
            text: text.to_string(),
            icon,
            content_height: 160.0,
        }
    }

    /// True when the box was dismissed.
    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors) -> bool {
        background(ui, colors);
        let width = ui.max_rect().width();
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
        let (glyph, color) = match self.icon {
            MessageIcon::Error => (icons::CIRCLE_X, ERROR_COLOR),
            MessageIcon::Warning => (icons::TRIANGLE_ALERT, WARNING_COLOR),
            MessageIcon::Information => (icons::INFO, colors.accent_start()),
        };
        paint_icon(ui, painter, tile.center(), glyph, 24.0, color);
        let text_top = body.top() + 22.0 + (48.0 - galley.size().y).max(0.0) / 2.0;
        painter.galley(pos2(tile.right() + 16.0, text_top), galley, colors.text());

        let bar = footer(ui, colors, 60.0);
        let ok = t(avalonia::MessageBox_OK);
        let mut look = Look::primary(colors).height(36.0);
        look.min_width = 88.0;
        let w = button_width(ui, &look, Content::label(ok));
        let rect = Rect::from_min_size(
            pos2(bar.right() - 14.0 - w, bar.top() + 12.0),
            vec2(w, 36.0),
        );
        let mut done = button_at(ui, colors, &look, Content::label(ok), true, rect).clicked();
        if ui.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter)) {
            done = true;
        }
        self.content_height = body_height + 60.0;
        done
    }
}
