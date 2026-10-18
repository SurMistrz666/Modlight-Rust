//! One line of text: a folder name, a vanity URL (`ModlightPromptWindow`).

use egui::{Rect, Ui, pos2, vec2};
use modlight_core::l10n::{s, t};

use super::{background, footer};
use crate::theme::{Colors, Weight};
use crate::widgets::{Content, Field, Look, button_at, button_width, layout, wrapped};

/// What the answer will be used for.
#[derive(Clone, Debug, PartialEq)]
pub enum PromptPurpose {
    NewFolder {
        parent: Option<String>,
    },
    RenameFolder {
        id: String,
        old: String,
    },
    CustomUrl {
        file: String,
        old: String,
    },
    MoveToNewFolder {
        files: Vec<String>,
        parent: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    pub heading: String,
    pub description: String,
    pub field_label: String,
    pub confirm_text: String,
    pub value: String,
    pub prefix: String,
    pub max_length: usize,
    pub problem: Option<String>,
    pub purpose: PromptPurpose,
    pub focus_requested: bool,
    pub content_height: f32,
}

pub const WIDTH: f32 = 460.0;

#[derive(Clone, Debug, PartialEq)]
pub enum PromptOutcome {
    Confirmed(String),
    Cancelled,
}

impl Prompt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        heading: &str,
        description: &str,
        field_label: &str,
        confirm_text: &str,
        initial: &str,
        prefix: &str,
        max_length: usize,
        purpose: PromptPurpose,
    ) -> Prompt {
        Prompt {
            heading: heading.to_string(),
            description: description.to_string(),
            field_label: field_label.to_string(),
            confirm_text: confirm_text.to_string(),
            value: initial.to_string(),
            prefix: prefix.to_string(),
            max_length,
            problem: None,
            purpose,
            focus_requested: false,
            content_height: 220.0,
        }
    }

    fn confirm(&mut self) -> Option<PromptOutcome> {
        let value = self.value.trim().to_string();
        if value.is_empty() {
            self.problem = Some(t(s::MySpace_NameRequired).to_string());
            self.focus_requested = false;
            return None;
        }
        Some(PromptOutcome::Confirmed(value))
    }

    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors) -> Option<PromptOutcome> {
        background(ui, colors);
        let full = ui.max_rect();
        let inner_left = full.left() + 22.0;
        let inner_width = full.width() - 44.0;
        let mut y = full.top() + 20.0;
        let painter = ui.painter().clone();

        let heading = layout(
            ui,
            wrapped(
                &self.heading,
                17.0,
                Weight::SemiBold,
                colors.text_secondary(),
                inner_width,
            ),
        );
        let h = heading.size().y;
        painter.galley(pos2(inner_left, y), heading, colors.text_secondary());
        y += h + 12.0;

        if !self.description.is_empty() {
            let description = layout(
                ui,
                wrapped(
                    &self.description,
                    12.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    inner_width,
                ),
            );
            let h = description.size().y;
            painter.galley(pos2(inner_left, y), description, colors.text_secondary());
            y += h + 12.0;
        }

        let label = layout(
            ui,
            wrapped(
                &self.field_label,
                12.0,
                Weight::SemiBold,
                colors.text_secondary(),
                inner_width,
            ),
        );
        let h = label.size().y;
        painter.galley(pos2(inner_left, y), label, colors.text_secondary());
        y += h + 6.0;

        let mut field_left = inner_left;
        if !self.prefix.is_empty() {
            let prefix = layout(
                ui,
                wrapped(
                    &self.prefix,
                    12.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    inner_width * 0.6,
                ),
            );
            let size = prefix.size();
            painter.galley(
                pos2(inner_left, y + 20.0 - size.y / 2.0),
                prefix,
                colors.text_secondary(),
            );
            field_left += size.x + 6.0;
        }
        let field_rect = Rect::from_min_size(
            pos2(field_left, y),
            vec2(inner_left + inner_width - field_left, 40.0),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field_rect));
        let field_id = child.id().with("prompt-field");
        let response = Field {
            width: Some(field_rect.width()),
            char_limit: self.max_length,
            id: Some(field_id),
            ..Field::new(&mut self.value)
        }
        .show(&mut child, colors);
        if !self.focus_requested {
            response.request_focus();
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), field_id) {
                let end = self.value.chars().count();
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(end),
                    )));
                state.store(ui.ctx(), field_id);
            }
            self.focus_requested = true;
        }
        y += 40.0;

        if let Some(problem) = &self.problem {
            y += 6.0;
            let galley = layout(
                ui,
                wrapped(
                    problem,
                    12.0,
                    Weight::Normal,
                    colors.accent_start(),
                    inner_width,
                ),
            );
            let h = galley.size().y;
            painter.galley(pos2(inner_left, y), galley, colors.accent_start());
            y += h;
        }
        y += 18.0;

        let body_height = y - full.top();
        ui.allocate_exact_size(vec2(full.width(), body_height), egui::Sense::hover());
        let bar = footer(ui, colors, 64.0);
        self.content_height = body_height + 64.0;

        let cancel = t(s::MySpace_Cancel);
        let plain = Look::compact(colors);
        let primary = Look::primary(colors);
        let confirm_w = button_width(ui, &primary, Content::label(&self.confirm_text));
        let cancel_w = button_width(ui, &plain, Content::label(cancel));
        let right = bar.right() - 14.0;
        let confirm_rect = Rect::from_min_size(
            pos2(right - confirm_w, bar.top() + 12.0),
            vec2(confirm_w, 40.0),
        );
        let cancel_rect = Rect::from_min_size(
            pos2(confirm_rect.left() - 8.0 - cancel_w, bar.top() + 12.0),
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
            outcome = Some(PromptOutcome::Cancelled);
        }
        let confirm_text = self.confirm_text.clone();
        if button_at(
            ui,
            colors,
            &primary,
            Content::label(&confirm_text),
            true,
            confirm_rect,
        )
        .clicked()
        {
            outcome = self.confirm();
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            outcome = self.confirm();
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            outcome = Some(PromptOutcome::Cancelled);
        }
        outcome
    }
}
