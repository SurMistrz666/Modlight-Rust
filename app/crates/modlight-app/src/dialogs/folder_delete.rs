//! Deleting a folder: two labelled paths rather than a checkbox (`ModlightFolderDeleteWindow`).
//! Emptying into the trash arms only after five seconds unless the folder is empty.

use egui::{Rect, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t};

use super::{background, footer_at};
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Look, button_at, button_width, layout, paint_icon, trimmed, wrapped,
};

pub const WIDTH: f32 = 620.0;
pub const ARMING_SECONDS: f64 = 5.0;

#[derive(Clone, Debug, PartialEq)]
pub struct FolderDelete {
    pub folder_id: String,
    pub name: String,
    pub file_count: i64,
    pub subfolder_count: i64,
    pub opened_at: f64,
    pub content_height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderDeleteChoice {
    KeepContents,
    WithContents,
    Cancel,
}

impl FolderDelete {
    pub fn new(
        folder_id: &str,
        name: &str,
        file_count: i64,
        subfolder_count: i64,
        now: f64,
    ) -> FolderDelete {
        FolderDelete {
            folder_id: folder_id.to_string(),
            name: name.to_string(),
            file_count,
            subfolder_count,
            opened_at: now,
            content_height: 330.0,
        }
    }

    /// Seconds until "delete with contents" arms, or 0 when it is armed.
    pub fn seconds_left(&self, now: f64) -> i64 {
        if self.file_count == 0 && self.subfolder_count == 0 {
            return 0;
        }
        (ARMING_SECONDS - (now - self.opened_at)).ceil().max(0.0) as i64
    }

    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors, now: f64) -> Option<FolderDeleteChoice> {
        background(ui, colors);
        let full = ui.max_rect();
        let left = full.left() + 22.0;
        let width = full.width() - 44.0;
        let painter = ui.painter().clone();
        let mut y = full.top() + 20.0;

        // Header: the warning tile and the title with its summary.
        let tile = Rect::from_min_size(pos2(left, y), vec2(44.0, 44.0));
        painter.rect_filled(tile, 10.0, colors.background_panel());
        paint_icon(
            ui,
            &painter,
            tile.center(),
            icons::TRIANGLE_ALERT,
            21.0,
            colors.text_secondary(),
        );
        let text_left = tile.right() + 13.0;
        let text_width = left + width - text_left;
        let title = layout(
            ui,
            trimmed(
                t(s::MySpace_DeleteFolderTitle),
                17.0,
                Weight::SemiBold,
                colors.text_secondary(),
                text_width,
                1,
            ),
        );
        let summary_text = format(
            t(s::MySpace_DeleteFolderSummary),
            &[
                Arg::from(self.name.as_str()),
                Arg::Int(self.file_count),
                Arg::Int(self.subfolder_count),
            ],
        );
        let summary = layout(
            ui,
            wrapped(
                &summary_text,
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                text_width,
            ),
        );
        let stack = title.size().y + 5.0 + summary.size().y;
        let header_h = stack.max(44.0);
        let mut ty = y + (header_h - stack) / 2.0;
        let title_h = title.size().y;
        painter.galley(pos2(text_left, ty), title, colors.text_secondary());
        ty += title_h + 5.0;
        painter.galley(pos2(text_left, ty), summary, colors.text_secondary());
        y += header_h + 14.0;

        let warning = layout(
            ui,
            wrapped(
                t(s::MySpace_DeleteFolderWarning),
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                width,
            ),
        );
        let warning_h = warning.size().y;
        painter.galley(pos2(left, y), warning, colors.text_secondary());
        y += warning_h + 14.0;

        // The two choices.
        let card_w = (width - 12.0) / 2.0;
        let card_inner = card_w - 28.0;
        let keep_title = layout(
            ui,
            wrapped(
                t(s::MySpace_DeleteFolderKeepTitle),
                13.0,
                Weight::SemiBold,
                colors.text_secondary(),
                card_inner,
            ),
        );
        let purge_title = layout(
            ui,
            wrapped(
                t(s::MySpace_DeleteFolderPurgeTitle),
                13.0,
                Weight::SemiBold,
                colors.text_secondary(),
                card_inner,
            ),
        );
        let title_h = keep_title.size().y.max(purge_title.size().y);
        let card_h = 14.0 + title_h + 10.0 + 76.0 + 10.0 + 40.0 + 14.0;
        let mut outcome = None;
        let seconds = self.seconds_left(now);
        for (index, (title, hint)) in [
            (keep_title, t(s::MySpace_DeleteFolderKeepHint)),
            (purge_title, t(s::MySpace_DeleteFolderPurgeHint)),
        ]
        .into_iter()
        .enumerate()
        {
            let card = Rect::from_min_size(
                pos2(left + index as f32 * (card_w + 12.0), y),
                vec2(card_w, card_h),
            );
            painter.rect_filled(card, 10.0, colors.background_panel());
            painter.rect_stroke(
                card,
                10.0,
                egui::Stroke::new(1.0, colors.border()),
                egui::StrokeKind::Inside,
            );
            let inner_left = card.left() + 14.0;
            let mut cy = card.top() + 14.0;
            painter.galley(pos2(inner_left, cy), title, colors.text_secondary());
            cy += title_h + 10.0;
            let hint = layout(
                ui,
                wrapped(
                    hint,
                    12.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    card_inner,
                ),
            );
            painter
                .with_clip_rect(Rect::from_min_size(
                    pos2(inner_left, cy),
                    vec2(card_inner, 76.0),
                ))
                .galley(pos2(inner_left, cy), hint, colors.text_secondary());
            cy += 76.0 + 10.0;
            let button_rect = Rect::from_min_size(pos2(inner_left, cy), vec2(card_inner, 40.0));
            let look = Look::compact(colors).padding(14.0);
            if index == 0 {
                let label = t(s::MySpace_DeleteFolderKeepAction);
                if button_at(ui, colors, &look, Content::label(label), true, button_rect).clicked()
                {
                    outcome = Some(FolderDeleteChoice::KeepContents);
                }
            } else {
                let armed = seconds == 0;
                let label = if armed {
                    t(s::MySpace_DeleteFolderPurgeAction).to_string()
                } else {
                    format(t(s::MySpace_DeleteFolderArming), &[Arg::Int(seconds)])
                };
                if button_at(
                    ui,
                    colors,
                    &look,
                    Content::label(&label),
                    armed,
                    button_rect,
                )
                .clicked()
                {
                    outcome = Some(FolderDeleteChoice::WithContents);
                }
                if !armed {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(200));
                }
            }
        }
        y += card_h + 18.0;

        let body_height = y - full.top();
        let bar = footer_at(
            ui,
            colors,
            Rect::from_min_size(pos2(full.left(), y), vec2(full.width(), 64.0)),
        );
        self.content_height = body_height + 64.0;
        let cancel = t(s::MySpace_Cancel);
        let look = Look::compact(colors);
        let w = button_width(ui, &look, Content::label(cancel));
        let rect = Rect::from_min_size(
            pos2(bar.right() - 14.0 - w, bar.top() + 12.0),
            vec2(w, 40.0),
        );
        if button_at(ui, colors, &look, Content::label(cancel), true, rect).clicked() {
            outcome = Some(FolderDeleteChoice::Cancel);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            outcome = Some(FolderDeleteChoice::Cancel);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            outcome = Some(FolderDeleteChoice::KeepContents);
        }
        outcome
    }
}
