//! Expiry, view limit and password of one file's link (`ModlightLinkSettingsWindow`).
//! Only what the person touched is sent: an untouched section is left out of the PATCH.

use chrono::{DateTime, Duration, Utc};
use egui::{Rect, Sense, Ui, pos2, vec2};
use modlight_api::UploadEdit;
use modlight_core::icons;
use modlight_core::l10n::{Arg, Key, format, s, t};
use modlight_core::library::CloudFile;

use super::{background, footer};
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Field, Look, button, button_at, button_width, layout, paint_icon, trimmed,
};

pub const WIDTH: f32 = 560.0;

#[derive(Clone, Debug, PartialEq)]
pub struct LinkSettings {
    pub file_id: String,
    pub file_name: String,
    pub had_password: bool,
    pub wants_expiry: bool,
    pub expiry: Option<DateTime<Utc>>,
    pub wants_view_limit: bool,
    pub view_limit: Option<i64>,
    pub view_limit_text: String,
    pub wants_password: bool,
    pub password: String,
    pub content_height: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LinkSettingsOutcome {
    Save(UploadEdit),
    Cancel,
}

/// The expiry presets, in hours (0 is "never").
const PRESETS: [(Key, i64); 5] = [
    (s::MySpace_LinkExpiry1h, 1),
    (s::MySpace_LinkExpiry24h, 24),
    (s::MySpace_LinkExpiry7d, 168),
    (s::MySpace_LinkExpiry30d, 720),
    (s::MySpace_LinkExpiryNever, 0),
];

impl LinkSettings {
    pub fn new(file: &CloudFile) -> LinkSettings {
        LinkSettings {
            file_id: file.id.clone(),
            file_name: file.name.clone(),
            had_password: file.has_password,
            wants_expiry: false,
            expiry: file.row.expires_at,
            wants_view_limit: false,
            view_limit: file.row.max_views,
            view_limit_text: file
                .row
                .max_views
                .map(|v| v.to_string())
                .unwrap_or_default(),
            wants_password: false,
            password: String::new(),
            content_height: 640.0,
        }
    }

    /// The edit to send, or `None` when nothing was touched.
    pub fn edit(&self) -> Option<UploadEdit> {
        let mut edit = UploadEdit::new();
        if self.wants_expiry {
            edit = edit.set_expiry(self.expiry);
        }
        if self.wants_view_limit {
            edit = edit.set_max_views(self.view_limit);
        }
        if self.wants_password {
            edit = edit.set_password((!self.password.is_empty()).then_some(self.password.as_str()));
        }
        (!edit.is_empty()).then_some(edit)
    }

    fn expiry_state(&self) -> String {
        match &self.expiry {
            None => t(s::MySpace_LinkNeverExpires).to_string(),
            Some(at) => format(
                t(s::MySpace_LinkExpiresOn),
                &[Arg::from(modlight_core::culture::local_short(at))],
            ),
        }
    }

    fn view_limit_state(&self) -> String {
        match self.view_limit {
            None => t(s::MySpace_LinkNoViewLimit).to_string(),
            Some(limit) => format(t(s::MySpace_LinkLimitedToViews), &[Arg::Int(limit)]),
        }
    }

    fn password_state(&self) -> &'static str {
        if self.wants_password && self.password.is_empty() {
            t(s::MySpace_LinkPasswordWillBeRemoved)
        } else if self.wants_password {
            t(s::MySpace_LinkPasswordWillBeSet)
        } else if self.had_password {
            t(s::MySpace_LinkPasswordOn)
        } else {
            t(s::MySpace_LinkPasswordOff)
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        colors: &Colors,
        now: DateTime<Utc>,
    ) -> Option<LinkSettingsOutcome> {
        background(ui, colors);
        let full = ui.max_rect();
        let left = full.left() + 22.0;
        let width = full.width() - 44.0;
        let content =
            Rect::from_min_size(pos2(left, full.top() + 20.0), vec2(width, full.height()));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(content)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let mut outcome = None;

        let painter = child.painter().clone();
        // Title and the file it is about.
        crate::widgets::text(
            &mut child,
            t(s::MySpace_LinkSettings),
            17.0,
            Weight::SemiBold,
            colors.text_secondary(),
        );
        child.add_space(3.0);
        let name = layout(
            &child,
            trimmed(
                &self.file_name,
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                width,
                1,
            ),
        );
        let (rect, _) = child.allocate_exact_size(vec2(width, name.size().y), Sense::hover());
        painter.galley(rect.min, name, colors.text_secondary());
        child.add_space(14.0);
        crate::widgets::rule(&mut child, colors.border(), 0.0, 0.0);
        child.add_space(14.0);

        // Expiry.
        section_header(&mut child, colors, icons::CLOCK, t(s::MySpace_LinkExpiry));
        child.add_space(8.0);
        crate::widgets::text_wrapped(
            &mut child,
            t(s::MySpace_LinkExpiryHint),
            12.0,
            Weight::Normal,
            colors.text_secondary(),
        );
        child.add_space(8.0);
        let chip = Look::chip(colors);
        let mut x = 0.0;
        let row_top = child.cursor().top();
        let mut row_y = row_top;
        for (key, hours) in PRESETS {
            let label = t(key);
            let w = button_width(&child, &chip, Content::label(label));
            if x > 0.0 && x + w > width {
                x = 0.0;
                row_y += 40.0 + 7.0;
            }
            let rect = Rect::from_min_size(pos2(left + x, row_y), vec2(w, 40.0));
            if button_at(&mut child, colors, &chip, Content::label(label), true, rect).clicked() {
                self.wants_expiry = true;
                self.expiry = (hours != 0).then(|| now + Duration::hours(hours));
            }
            x += w + 7.0;
        }
        // The chips were allocated where they stand, which already moved the cursor
        // below them; only the wrap panel's bottom margin (7) may still be missing.
        let bottom = row_y + 40.0 + 7.0;
        let cursor = child.cursor().top();
        if bottom > cursor {
            child.add_space(bottom - cursor);
        }
        let _ = row_top;
        state_text(&mut child, colors, &self.expiry_state());
        child.add_space(14.0);
        crate::widgets::rule(&mut child, colors.border(), 0.0, 0.0);
        child.add_space(14.0);

        // View limit.
        section_header(&mut child, colors, icons::EYE, t(s::MySpace_LinkViewLimit));
        child.add_space(8.0);
        crate::widgets::text_wrapped(
            &mut child,
            t(s::MySpace_LinkViewLimitHint),
            12.0,
            Weight::Normal,
            colors.text_secondary(),
        );
        child.add_space(8.0);
        child.horizontal(|ui| {
            if numeric(ui, colors, &mut self.view_limit_text) {
                self.wants_view_limit = true;
                self.view_limit = self
                    .view_limit_text
                    .trim()
                    .parse::<i64>()
                    .ok()
                    .map(|v| v.max(1));
                if let Some(v) = self.view_limit {
                    self.view_limit_text = v.to_string();
                }
            }
            ui.add_space(8.0);
            if button(
                ui,
                colors,
                &chip,
                Content::label(t(s::MySpace_LinkOneTime)),
                true,
            )
            .clicked()
            {
                self.wants_view_limit = true;
                self.view_limit = Some(1);
                self.view_limit_text = "1".into();
            }
            ui.add_space(8.0);
            if button(
                ui,
                colors,
                &chip,
                Content::label(t(s::MySpace_LinkNoLimit)),
                true,
            )
            .clicked()
            {
                self.wants_view_limit = true;
                self.view_limit = None;
                self.view_limit_text.clear();
            }
        });
        child.add_space(8.0);
        state_text(&mut child, colors, &self.view_limit_state());
        child.add_space(14.0);
        crate::widgets::rule(&mut child, colors.border(), 0.0, 0.0);
        child.add_space(14.0);

        // Password.
        section_header(
            &mut child,
            colors,
            icons::LOCK_KEYHOLE,
            t(s::MySpace_LinkPassword),
        );
        child.add_space(8.0);
        crate::widgets::text_wrapped(
            &mut child,
            t(s::MySpace_LinkPasswordHint),
            12.0,
            Weight::Normal,
            colors.text_secondary(),
        );
        child.add_space(8.0);
        let remove_label = t(s::MySpace_LinkRemovePassword);
        let remove_w = if self.had_password {
            button_width(&child, &chip, Content::label(remove_label)) + 8.0
        } else {
            0.0
        };
        child.horizontal(|ui| {
            let before = self.password.clone();
            Field {
                width: Some(width - remove_w),
                password: true,
                hint: "••••••••",
                ..Field::new(&mut self.password)
            }
            .show(ui, colors);
            if self.password != before {
                self.wants_password = !self.password.is_empty();
            }
            if self.had_password {
                ui.add_space(8.0);
                if button(ui, colors, &chip, Content::label(remove_label), true).clicked() {
                    self.password.clear();
                    self.wants_password = true;
                    self.had_password = false;
                }
            }
        });
        child.add_space(8.0);
        state_text(&mut child, colors, self.password_state());
        child.add_space(18.0);

        let body_height = child.min_rect().height() + 20.0;
        ui.allocate_exact_size(vec2(full.width(), body_height), Sense::hover());
        let bar = footer(ui, colors, 64.0);
        self.content_height = body_height + 64.0;
        let cancel = t(s::MySpace_Cancel);
        let save = t(s::MySpace_Save);
        let plain = Look::compact(colors);
        let primary = Look::primary(colors);
        let save_w = button_width(ui, &primary, Content::label(save));
        let cancel_w = button_width(ui, &plain, Content::label(cancel));
        let save_rect = Rect::from_min_size(
            pos2(bar.right() - 14.0 - save_w, bar.top() + 12.0),
            vec2(save_w, 40.0),
        );
        let cancel_rect = Rect::from_min_size(
            pos2(save_rect.left() - 8.0 - cancel_w, bar.top() + 12.0),
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
        {
            outcome = Some(LinkSettingsOutcome::Cancel);
        }
        // Save is the default button (`IsDefault`): Enter presses it.
        if button_at(ui, colors, &primary, Content::label(save), true, save_rect).clicked()
            || ui.input(|i| i.key_pressed(egui::Key::Enter))
        {
            outcome = Some(match self.edit() {
                Some(edit) => LinkSettingsOutcome::Save(edit),
                None => LinkSettingsOutcome::Cancel,
            });
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            outcome = Some(LinkSettingsOutcome::Cancel);
        }
        let _ = painter;
        outcome
    }
}

fn section_header(ui: &mut Ui, colors: &Colors, glyph: char, title: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(16.0, 20.0), Sense::hover());
        paint_icon(
            ui,
            ui.painter(),
            rect.center(),
            glyph,
            15.0,
            colors.accent_start(),
        );
        ui.add_space(8.0);
        crate::widgets::text(ui, title, 14.0, Weight::SemiBold, colors.text_secondary());
    });
}

fn state_text(ui: &mut Ui, colors: &Colors, text: &str) {
    crate::widgets::text_wrapped(ui, text, 12.0, Weight::Normal, colors.accent_start());
}

/// A `NumericUpDown` (140 wide, minimum 1): a centred field with a spinner. Returns
/// whether the value changed.
fn numeric(ui: &mut Ui, colors: &Colors, text: &mut String) -> bool {
    let before = text.clone();
    let (rect, _) = ui.allocate_exact_size(vec2(140.0, 40.0), Sense::hover());
    let mut child = ui.new_child(
        egui::UiBuilder::new().max_rect(Rect::from_min_size(rect.min, vec2(140.0 - 18.0, 40.0))),
    );
    Field {
        width: Some(140.0 - 18.0),
        ..Field::new(text)
    }
    .show(&mut child, colors);
    let spinner = Rect::from_min_size(pos2(rect.right() - 18.0, rect.top()), vec2(18.0, 40.0));
    let painter = ui.painter();
    painter.rect_filled(
        spinner,
        egui::CornerRadius {
            nw: 0,
            sw: 0,
            ne: 8,
            se: 8,
        },
        colors.control_background_disabled(),
    );
    let up = Rect::from_min_size(spinner.min, vec2(18.0, 20.0));
    let down = Rect::from_min_size(pos2(spinner.left(), spinner.top() + 20.0), vec2(18.0, 20.0));
    let up_response = ui.interact(up, ui.id().with("numeric-up"), Sense::click());
    let down_response = ui.interact(down, ui.id().with("numeric-down"), Sense::click());
    paint_icon(
        ui,
        ui.painter(),
        up.center(),
        icons::CHEVRON_UP,
        12.0,
        colors.text_secondary(),
    );
    paint_icon(
        ui,
        ui.painter(),
        down.center(),
        icons::CHEVRON_DOWN,
        12.0,
        colors.text_secondary(),
    );
    let current = text.trim().parse::<i64>().unwrap_or(0);
    if up_response.clicked() {
        *text = (current + 1).max(1).to_string();
    }
    if down_response.clicked() {
        *text = (current - 1).max(1).to_string();
    }
    let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits != *text {
        *text = digits;
    }
    *text != before
}

#[cfg(test)]
mod tests {
    use super::*;
    use modlight_api::Upload;

    fn file(has_password: bool) -> CloudFile {
        CloudFile::new(Upload {
            id: Some("f".into()),
            filename: Some("a.png".into()),
            has_password,
            max_views: Some(5),
            ..Default::default()
        })
    }

    #[test]
    fn only_touched_sections_are_sent() {
        let mut dialog = LinkSettings::new(&file(true));
        assert_eq!(dialog.edit(), None, "nothing touched, nothing sent");
        dialog.wants_view_limit = true;
        dialog.view_limit = None;
        let edit = dialog.edit().unwrap();
        assert_eq!(edit.fields().len(), 1);
        assert_eq!(edit.fields()[0].0, "maxViews");
        assert_eq!(
            edit.fields()[0].1,
            serde_json::Value::Null,
            "cleared, not left alone"
        );

        dialog.wants_password = true;
        dialog.password.clear();
        let edit = dialog.edit().unwrap();
        assert_eq!(
            edit.fields()[1],
            ("password".to_string(), serde_json::Value::Null)
        );
    }

    /// Save is the window's default button (`IsDefault="True"`): Enter saves what was
    /// changed, as Escape cancels (`IsCancel`).
    #[test]
    fn enter_saves_as_the_default_button_does() {
        let colors = crate::qa::state(&crate::qa::Look::default(), true)
            .colors
            .clone();
        let now = crate::qa::frozen_now();
        let mut dialog = LinkSettings::new(&file(false));
        dialog.wants_view_limit = true;
        dialog.view_limit = Some(3);
        let outcome: Option<LinkSettingsOutcome> = None;
        // The first pass runs while the harness is built, before the app's fonts are in.
        let fonts_in = std::rc::Rc::new(std::cell::Cell::new(false));
        let ready = fonts_in.clone();
        let mut harness = egui_kittest::Harness::new_ui_state(
            move |ui, (dialog, outcome): &mut (LinkSettings, Option<LinkSettingsOutcome>)| {
                if ready.get()
                    && let Some(answer) = dialog.ui(ui, &colors, now)
                {
                    *outcome = Some(answer);
                }
            },
            (dialog, outcome),
        );
        harness.ctx.set_fonts(crate::theme::fonts());
        harness.step();
        fonts_in.set(true);
        harness.step();
        assert_eq!(harness.state().1, None, "nothing yet");
        harness.key_press(egui::Key::Enter);
        harness.step();
        let Some(LinkSettingsOutcome::Save(edit)) = harness.state().1.clone() else {
            panic!("Enter did not save: {:?}", harness.state().1);
        };
        assert_eq!(
            edit.fields(),
            &[("maxViews".to_string(), serde_json::json!(3))]
        );
    }
}
