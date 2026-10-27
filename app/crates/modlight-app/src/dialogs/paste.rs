//! "New paste" (`PasteWindow`): 820 by 640, resizable down to 600 by 460. A header
//! band, the title and the language, a monospace editor that fills the window, the line
//! saying what it will become, and Cancel / Upload in the footer. Tab inserts a tab,
//! Ctrl+Enter (or Cmd+Enter) sends; the language is picked from a searchable list.

use egui::{Align2, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{s, t};
use modlight_core::tasks::paste::{Paste, PasteLanguage};

use super::url_upload::compact_looks;
use super::{background, footer_at};
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Field, Look, button_at, button_width, layout, paint_icon, paint_text, trimmed, wrapped,
};

pub const WIDTH: f32 = 820.0;
pub const HEIGHT: f32 = 640.0;
pub const MIN_WIDTH: f32 = 600.0;
pub const MIN_HEIGHT: f32 = 460.0;

#[derive(Clone, Debug, PartialEq)]
pub struct PasteWindow {
    pub model: Paste,
    /// The text came from the clipboard: it is selected when the window opens.
    pub prefilled: bool,
    pub language_open: bool,
    highlighted: Option<usize>,
    opened: bool,
}

/// What the window asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum PasteOutcome {
    Cancel,
    /// Upload these bytes under this name.
    Send(Vec<u8>, String),
}

impl PasteWindow {
    /// A paste, pre-filled with the clipboard's text when there is any.
    pub fn new(clipboard: Option<String>) -> PasteWindow {
        let mut model = Paste::default();
        let prefilled = match clipboard.filter(|c| !c.is_empty()) {
            Some(text) => {
                model.content = text;
                true
            }
            None => false,
        };
        PasteWindow {
            model,
            prefilled,
            language_open: false,
            highlighted: None,
            opened: false,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors) -> Option<PasteOutcome> {
        background(ui, colors);
        let full = ui.max_rect();
        let painter = ui.painter().clone();

        // Header: toolbar band with a rule under it.
        let text_width = full.width() - 18.0 - 40.0 - 12.0 - 18.0;
        let title = layout(
            ui,
            trimmed(
                t(s::Paste_WindowTitle),
                18.0,
                Weight::SemiBold,
                colors.heading(),
                text_width,
                1,
            ),
        );
        let subtitle = layout(
            ui,
            wrapped(
                t(s::Paste_Subtitle),
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                text_width,
            ),
        );
        let text_height = title.size().y + 2.0 + subtitle.size().y;
        let header = Rect::from_min_size(
            full.min,
            vec2(full.width(), 12.0 + text_height.max(40.0) + 12.0),
        );
        painter.rect_filled(header, 0.0, colors.background_toolbar());
        painter.hline(
            header.x_range(),
            header.bottom() - 0.5,
            Stroke::new(1.0, colors.border()),
        );
        let tile = Rect::from_min_size(
            pos2(header.left() + 18.0, header.center().y - 20.0),
            vec2(40.0, 40.0),
        );
        painter.rect_filled(tile, 8.0, colors.background_panel());
        paint_icon(
            ui,
            &painter,
            tile.center(),
            icons::CLIPBOARD_TYPE,
            20.0,
            colors.accent_start(),
        );
        let left = tile.right() + 12.0;
        let top = header.center().y - text_height / 2.0;
        let title_h = title.size().y;
        painter.galley(pos2(left, top), title, colors.heading());
        painter.galley(
            pos2(left, top + title_h + 2.0),
            subtitle,
            colors.text_secondary(),
        );

        // Footer bar first, so the editor knows how much room it has.
        let footer_height = 12.0 + 40.0 + 12.0;
        let bar = Rect::from_min_max(pos2(full.left(), full.bottom() - footer_height), full.max);
        footer_at(ui, colors, bar);

        // Body: margin 18,14.
        let body = Rect::from_min_max(
            pos2(full.left() + 18.0, header.bottom() + 14.0),
            pos2(full.right() - 18.0, bar.top() - 14.0),
        );
        let language_width = 210.0;
        let title_width = body.width() - 12.0 - language_width;
        paint_text(
            ui,
            &painter,
            Rect::from_min_size(body.min, vec2(title_width, 16.0)),
            Align2::LEFT_CENTER,
            t(s::Paste_TitleLabel),
            12.0,
            Weight::SemiBold,
            colors.text_secondary(),
        );
        paint_text(
            ui,
            &painter,
            Rect::from_min_size(
                pos2(body.left() + title_width + 12.0, body.top()),
                vec2(language_width, 16.0),
            ),
            Align2::LEFT_CENTER,
            t(s::Paste_LanguageLabel),
            12.0,
            Weight::SemiBold,
            colors.text_secondary(),
        );
        let row_top = body.top() + 16.0 + 5.0;
        let mut title_ui = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(
            pos2(body.left(), row_top),
            vec2(title_width, 40.0),
        )));
        Field {
            width: Some(title_width),
            hint: t(s::Paste_TitleWatermark),
            char_limit: 80,
            id: Some(Id::new("paste-title")),
            ..Field::new(&mut self.model.title)
        }
        .show(&mut title_ui, colors);

        let language_rect = Rect::from_min_size(
            pos2(body.left() + title_width + 12.0, row_top),
            vec2(language_width, 40.0),
        );
        let language_response = self.language_button(ui, colors, language_rect);
        if language_response.clicked() {
            self.language_open = !self.language_open;
            self.model.language_filter.clear();
            self.highlighted = None;
        }

        // The footer line and the key hint, under the editor.
        let footer_text = layout(
            ui,
            trimmed(
                &self.model.footer_text(),
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                body.width() * 0.6,
                1,
            ),
        );
        let key_hint = layout(
            ui,
            trimmed(
                t(s::Paste_KeyHint),
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                body.width() * 0.4,
                1,
            ),
        );
        let problem = (!self.model.problem.is_empty()).then(|| {
            layout(
                ui,
                wrapped(
                    &self.model.problem,
                    12.0,
                    Weight::Normal,
                    colors.accent_start(),
                    body.width() * 0.6,
                ),
            )
        });
        let lines_height =
            footer_text.size().y + problem.as_ref().map(|p| 3.0 + p.size().y).unwrap_or(0.0);
        let lines_top = body.bottom() - lines_height;
        let footer_h = footer_text.size().y;
        painter.galley(
            pos2(body.left(), lines_top),
            footer_text,
            colors.text_secondary(),
        );
        if let Some(problem) = problem {
            painter.galley(
                pos2(body.left(), lines_top + footer_h + 3.0),
                problem,
                colors.accent_start(),
            );
        }
        painter.galley(
            pos2(body.right() - key_hint.size().x, lines_top),
            key_hint,
            colors.text_secondary(),
        );

        // The editor: a panel with the text box filling it.
        let editor = Rect::from_min_max(
            pos2(body.left(), row_top + 40.0 + 10.0),
            pos2(body.right(), lines_top - 10.0),
        );
        painter.rect_filled(editor, 8.0, colors.background_panel());
        painter.rect_stroke(
            editor,
            8.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        let mut send = false;
        let editor_id = Id::new("paste-editor");
        let mut editor_ui = ui.new_child(egui::UiBuilder::new().max_rect(editor.shrink(1.0)));
        let before = self.model.content.clone();
        egui::ScrollArea::both()
            .id_salt("paste-editor-scroll")
            .auto_shrink([false, false])
            .show(&mut editor_ui, |ui| {
                let hint = egui::RichText::new(t(s::Paste_EditorWatermark))
                    .font(egui::FontId::monospace(13.0))
                    .color(colors.text_secondary().gamma_multiply(0.75));
                let output = egui::TextEdit::multiline(&mut self.model.content)
                    .id(editor_id)
                    .code_editor()
                    .font(egui::FontId::monospace(13.0))
                    .text_color(colors.text())
                    .frame(egui::Frame::NONE.inner_margin(11))
                    .desired_width(f32::INFINITY)
                    .desired_rows(8)
                    .min_size(vec2(editor.width() - 2.0, editor.height() - 2.0))
                    .hint_text(hint)
                    .show(ui);
                if !self.opened {
                    output.response.request_focus();
                    if self.prefilled {
                        let mut state = output.state.clone();
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(self.model.content.chars().count()),
                            )));
                        state.store(ui.ctx(), editor_id);
                    }
                    self.opened = true;
                }
                if output.response.has_focus()
                    && ui.input(|i| {
                        i.key_pressed(egui::Key::Enter) && (i.modifiers.ctrl || i.modifiers.command)
                    })
                {
                    send = true;
                }
            });
        if send {
            // The Enter that sent must not stay in the text.
            self.model.content = before;
        } else if self.model.content != before {
            self.model.problem.clear();
        }

        // The language list, over everything.
        if self.language_open
            && let Some(language) = self.language_popup(ui, colors, language_rect)
        {
            self.model.language = language;
            self.language_open = false;
        }

        // Footer: Cancel, Upload.
        let (plain, primary) = compact_looks(colors);
        let (cancel, upload) = (t(s::MySpace_Cancel), t(s::Paste_Send));
        let upload_w = button_width(ui, &primary, Content::label(upload));
        let cancel_w = button_width(ui, &plain, Content::label(cancel));
        let upload_rect = Rect::from_min_size(
            pos2(bar.right() - 14.0 - upload_w, bar.top() + 12.0),
            vec2(upload_w, 40.0),
        );
        let cancel_rect = Rect::from_min_size(
            pos2(upload_rect.left() - 8.0 - cancel_w, bar.top() + 12.0),
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
            outcome = Some(PasteOutcome::Cancel);
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
            send = true;
        }
        if !self.language_open && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            outcome = Some(PasteOutcome::Cancel);
        }
        if send {
            match self.model.try_send() {
                Some((bytes, name)) => outcome = Some(PasteOutcome::Send(bytes, name)),
                None => ui.ctx().memory_mut(|m| m.request_focus(editor_id)),
            }
        }
        outcome
    }

    fn language_button(&self, ui: &mut Ui, colors: &Colors, rect: Rect) -> egui::Response {
        let response = ui.allocate_rect(rect, Sense::click());
        let look = Look::button(colors);
        crate::widgets::paint_button(ui, colors, &look, Content::label(""), true, rect, &response);
        let painter = ui.painter();
        let name = layout(
            ui,
            trimmed(
                self.model.language.name(),
                13.0,
                Weight::Normal,
                colors.text_secondary(),
                rect.width() - 20.0 - 30.0,
                1,
            ),
        );
        crate::widgets::paint_text_left(
            painter,
            rect.left() + 10.0,
            rect.center().y,
            name,
            colors.text_secondary(),
        );
        paint_icon(
            ui,
            painter,
            pos2(rect.right() - 10.0 - 7.0, rect.center().y),
            icons::CHEVRON_DOWN,
            14.0,
            colors.text_secondary(),
        );
        response
    }

    /// The search box and the list, under the language button. Returns a picked language.
    fn language_popup(
        &mut self,
        ui: &mut Ui,
        colors: &Colors,
        anchor: Rect,
    ) -> Option<PasteLanguage> {
        let mut picked = None;
        let languages = self.model.languages();
        let area = egui::Area::new(Id::new("paste-language-popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(anchor.left(), anchor.bottom() + 6.0));
        let inner = area.show(ui.ctx(), |ui| {
            crate::widgets::popup_frame(colors).show(ui, |ui| {
                ui.set_width(300.0 - 12.0);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(18.0, 40.0), Sense::hover());
                    paint_icon(
                        ui,
                        ui.painter(),
                        pos2(r.left() + 4.0 + 7.0, r.center().y),
                        icons::SEARCH,
                        14.0,
                        colors.text_secondary(),
                    );
                    ui.add_space(8.0);
                    let search = Field {
                        width: Some(300.0 - 12.0 - 18.0 - 8.0),
                        hint: t(s::Paste_LanguageSearch),
                        id: Some(Id::new("paste-language-search")),
                        ..Field::new(&mut self.model.language_filter)
                    }
                    .show(ui, colors);
                    search.request_focus();
                    if search.changed() {
                        self.highlighted = None;
                    }
                });
                ui.add_space(8.0);
                if languages.is_empty() {
                    crate::widgets::text_wrapped(
                        ui,
                        t(s::Paste_LanguageNoMatch),
                        12.0,
                        Weight::Normal,
                        colors.text_secondary(),
                    );
                    ui.add_space(4.0);
                }
                egui::ScrollArea::vertical()
                    .id_salt("paste-language-list")
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for (index, language) in languages.iter().enumerate() {
                            let (row, response) =
                                ui.allocate_exact_size(vec2(300.0 - 12.0, 32.0), Sense::click());
                            let current = *language == self.model.language;
                            if current || self.highlighted == Some(index) || response.hovered() {
                                ui.painter().rect_filled(
                                    row,
                                    6.0,
                                    colors.control_background_hover(),
                                );
                            }
                            let name = layout(
                                ui,
                                trimmed(
                                    language.name(),
                                    13.0,
                                    Weight::SemiBold,
                                    colors.text_secondary(),
                                    row.width() - 90.0,
                                    1,
                                ),
                            );
                            crate::widgets::paint_text_left(
                                ui.painter(),
                                row.left() + 8.0,
                                row.center().y,
                                name,
                                colors.text_secondary(),
                            );
                            let ext = ui.fonts_mut(|f| {
                                f.layout_no_wrap(
                                    language.extension.to_string(),
                                    egui::FontId::monospace(11.0),
                                    colors.text_secondary(),
                                )
                            });
                            let ext_w = ext.size().x;
                            crate::widgets::paint_text_left(
                                ui.painter(),
                                row.right() - 8.0 - ext_w,
                                row.center().y,
                                ext,
                                colors.text_secondary(),
                            );
                            if response.clicked() {
                                picked = Some(*language);
                            }
                        }
                    });
            });
        });
        let (enter, escape, down, up) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowUp),
            )
        });
        if down && !languages.is_empty() {
            self.highlighted = Some(
                self.highlighted
                    .map(|h| (h + 1).min(languages.len() - 1))
                    .unwrap_or(0),
            );
        }
        if up {
            self.highlighted = self.highlighted.map(|h| h.saturating_sub(1));
        }
        if enter && !languages.is_empty() {
            picked = Some(languages[self.highlighted.unwrap_or(0).min(languages.len() - 1)]);
        }
        if escape {
            self.language_open = false;
        }
        // A click outside the list closes it (light dismiss).
        if ui.input(|i| i.pointer.any_pressed())
            && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
            && !inner.response.rect.contains(pointer)
            && !anchor.contains(pointer)
        {
            self.language_open = false;
        }
        picked
    }
}
