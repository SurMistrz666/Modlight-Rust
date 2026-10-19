//! Files stored more than once (`ModlightDuplicatesWindow`, `DuplicatesViewModel`).
//! Matched by checksum only; the default selection keeps the oldest copy of each set;
//! everything proposed goes to the trash, never further.

use egui::{Rect, ScrollArea, Sense, Ui, pos2, vec2};
use modlight_api::{DuplicateGroup, DuplicateScan};
use modlight_core::format::cloud_size;
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t};

use super::{background, footer};
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Look, button_at, button_width, checkbox, layout, paint_icon, paint_progress, trimmed,
    wrapped,
};

pub const WIDTH: f32 = 760.0;
pub const HEIGHT: f32 = 640.0;

#[derive(Clone, Debug, PartialEq)]
pub struct DuplicateFileRow {
    pub id: String,
    pub name: String,
    pub size: i64,
    pub created_at: Option<String>,
    pub folder_name: Option<String>,
    pub is_oldest: bool,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DuplicateSet {
    pub group: DuplicateGroup,
    pub files: Vec<DuplicateFileRow>,
    pub dismissed: bool,
    pub busy: bool,
}

impl DuplicateSet {
    pub fn new(group: DuplicateGroup) -> DuplicateSet {
        let files = group
            .files
            .clone()
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(i, f)| DuplicateFileRow {
                id: f.id.clone().unwrap_or_default(),
                name: f
                    .filename
                    .clone()
                    .filter(|n| !n.is_empty())
                    .or(f.code.clone())
                    .unwrap_or_default(),
                size: f.size,
                created_at: f.created_at.clone(),
                folder_name: f.folder_name.clone(),
                is_oldest: i == 0,
                selected: false,
            })
            .collect();
        DuplicateSet {
            dismissed: group.dismissed,
            group,
            files,
            busy: false,
        }
    }

    pub fn checksum(&self) -> &str {
        self.group.checksum.as_deref().unwrap_or("")
    }

    /// Select every copy but the oldest; a set marked as meant-to-exist selects nothing.
    pub fn select_extras(&mut self) {
        let dismissed = self.dismissed;
        for file in &mut self.files {
            file.selected = !dismissed && !file.is_oldest;
        }
    }

    pub fn clear_selection(&mut self) {
        for file in &mut self.files {
            file.selected = false;
        }
    }

    pub fn header(&self) -> String {
        format(
            t(s::MySpace_DuplicatesGroup),
            &[
                Arg::Int(self.group.copies),
                Arg::from(cloud_size(self.group.size)),
                Arg::from(cloud_size(self.group.reclaimable)),
            ],
        )
    }

    pub fn partial(&self) -> Option<String> {
        (self.group.copies > self.files.len() as i64).then(|| {
            format(
                t(s::MySpace_DuplicatesGroupPartial),
                &[Arg::from(self.files.len()), Arg::Int(self.group.copies)],
            )
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Duplicates {
    pub sets: Vec<DuplicateSet>,
    pub scan: Option<DuplicateScan>,
    pub busy: bool,
    pub status: String,
    pub failed: bool,
    /// Files trashed while the window was open (the window reports it on close).
    pub trashed: i64,
}

/// What the person asked for.
#[derive(Clone, Debug, PartialEq)]
pub enum DuplicatesAction {
    ToggleKeep(usize),
    Trash,
    Close,
}

impl Duplicates {
    /// A fresh scan arrived.
    pub fn apply_scan(&mut self, scan: DuplicateScan, keep_status: bool) {
        self.sets = scan
            .groups
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|g| {
                let mut set = DuplicateSet::new(g);
                set.select_extras();
                set
            })
            .collect();
        self.scan = Some(scan);
        if !keep_status {
            self.status.clear();
        }
        self.failed = false;
    }

    pub fn fail(&mut self, message: &str) {
        self.failed = true;
        self.status = message.to_string();
    }

    pub fn selected_count(&self) -> usize {
        self.sets
            .iter()
            .flat_map(|s| &s.files)
            .filter(|f| f.selected)
            .count()
    }

    pub fn selected_bytes(&self) -> i64 {
        self.sets
            .iter()
            .flat_map(|s| &s.files)
            .filter(|f| f.selected)
            .map(|f| f.size)
            .sum()
    }

    /// The ids to trash, each once.
    pub fn selected_ids(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for file in self.sets.iter().flat_map(|s| &s.files) {
            if file.selected && !file.id.is_empty() && !out.contains(&file.id) {
                out.push(file.id.clone());
            }
        }
        out
    }

    pub fn can_trash(&self) -> bool {
        !self.busy && self.selected_count() > 0
    }

    pub fn trash_label(&self) -> String {
        if self.selected_count() == 0 {
            t(s::MySpace_DuplicatesTrashNone).to_string()
        } else {
            format(
                t(s::MySpace_DuplicatesTrash),
                &[
                    Arg::from(self.selected_count()),
                    Arg::from(cloud_size(self.selected_bytes())),
                ],
            )
        }
    }

    pub fn summary(&self) -> String {
        let Some(scan) = &self.scan else {
            return String::new();
        };
        let totals = scan.active.clone().unwrap_or_default();
        format(
            t(s::MySpace_DuplicatesSummary),
            &[
                Arg::Int(totals.groups),
                Arg::Int(totals.redundant),
                Arg::from(cloud_size(totals.reclaimable)),
            ],
        )
    }

    pub fn due_text(&self) -> Option<String> {
        let scan = self.scan.as_ref().filter(|s| s.due)?;
        let limits = scan.thresholds.clone().unwrap_or_default();
        Some(format(
            t(s::MySpace_DuplicatesDue),
            &[
                Arg::Int(limits.redundant),
                Arg::from(cloud_size(limits.bytes)),
            ],
        ))
    }

    pub fn blind_text(&self) -> Option<String> {
        let scan = self.scan.as_ref().filter(|s| s.unchecksummed > 0)?;
        Some(format(
            t(s::MySpace_DuplicatesBlind),
            &[Arg::Int(scan.unchecksummed)],
        ))
    }

    pub fn truncated_text(&self) -> Option<String> {
        let scan = self.scan.as_ref().filter(|s| s.truncated)?;
        Some(format(
            t(s::MySpace_DuplicatesTruncated),
            &[Arg::from(self.sets.len()), Arg::Int(scan.total_groups)],
        ))
    }

    pub fn show_empty(&self) -> bool {
        !self.busy && self.scan.is_some() && self.sets.is_empty()
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        colors: &Colors,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Option<DuplicatesAction> {
        background(ui, colors);
        let full = ui.max_rect();
        let left = full.left() + 22.0;
        let width = full.width() - 44.0;
        let mut action = None;

        // The header: what the scan found, before anything is proposed.
        let mut y = full.top() + 18.0;
        let painter = ui.painter().clone();
        let line = |text: &str, size: f32, weight: Weight, color: egui::Color32, y: &mut f32| {
            let galley = layout(ui, wrapped(text, size, weight, color, width));
            let h = galley.size().y;
            painter.galley(pos2(left, *y), galley, color);
            *y += h + 6.0;
        };
        line(
            t(s::MySpace_DuplicatesTitle),
            17.0,
            Weight::SemiBold,
            colors.text_secondary(),
            &mut y,
        );
        line(
            &self.summary(),
            12.0,
            Weight::Normal,
            colors.text_secondary(),
            &mut y,
        );
        if let Some(due) = self.due_text() {
            line(&due, 12.0, Weight::Normal, colors.accent_start(), &mut y);
        }
        if let Some(blind) = self.blind_text() {
            line(
                &blind,
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                &mut y,
            );
        }
        if let Some(truncated) = self.truncated_text() {
            line(
                &truncated,
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                &mut y,
            );
        }
        if !self.status.is_empty() {
            let color = if self.failed {
                colors.cancel_border()
            } else {
                colors.accent_start()
            };
            line(&self.status.clone(), 12.0, Weight::Normal, color, &mut y);
        }
        y += 4.0;
        if self.busy {
            paint_progress(
                ui,
                colors,
                Rect::from_min_size(pos2(full.left(), y), vec2(full.width(), 2.0)),
                None,
            );
        }
        y += 2.0;

        let footer_h = 64.0;
        let list_rect = Rect::from_min_max(
            pos2(full.left(), y),
            pos2(full.right(), full.bottom() - footer_h),
        );
        let mut list = ui.new_child(egui::UiBuilder::new().max_rect(list_rect));
        ScrollArea::vertical()
            .id_salt("duplicates")
            .auto_shrink([false, false])
            .show(&mut list, |ui| {
                ui.add_space(4.0);
                let inner_width = list_rect.width() - 44.0;
                ui.horizontal(|ui| {
                    ui.add_space(22.0);
                    ui.vertical(|ui| {
                        ui.set_width(inner_width);
                        if self.show_empty() {
                            crate::widgets::text_wrapped(
                                ui,
                                t(s::MySpace_DuplicatesNone),
                                12.0,
                                Weight::Normal,
                                colors.text_secondary(),
                            );
                            ui.add_space(10.0);
                        }
                        let idle = !self.busy;
                        for (index, set) in self.sets.iter_mut().enumerate() {
                            if let Some(a) =
                                group_card(ui, colors, set, index, idle, inner_width, now)
                            {
                                action = Some(a);
                            }
                            ui.add_space(10.0);
                        }
                        ui.add_space(14.0);
                    });
                });
            });

        // The footer.
        let bar_rect = Rect::from_min_size(
            pos2(full.left(), full.bottom() - footer_h),
            vec2(full.width(), footer_h),
        );
        let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar_rect));
        let bar = footer(&mut bar_ui, colors, footer_h);
        let chip = Look::chip(colors);
        let mut x = bar.left() + 14.0;
        let has_groups = !self.sets.is_empty();
        if has_groups {
            for (index, label) in [
                t(s::MySpace_DuplicatesSelectExtras),
                t(s::MySpace_DuplicatesClear),
            ]
            .into_iter()
            .enumerate()
            {
                let w = button_width(&bar_ui, &chip, Content::label(label));
                let rect = Rect::from_min_size(pos2(x, bar.top() + 12.0), vec2(w, 40.0));
                if button_at(
                    &mut bar_ui,
                    colors,
                    &chip,
                    Content::label(label),
                    true,
                    rect,
                )
                .clicked()
                {
                    for set in &mut self.sets {
                        if index == 0 {
                            set.select_extras();
                        } else {
                            set.clear_selection();
                        }
                    }
                }
                x += w + 8.0;
            }
        }
        let plain = Look::compact(colors);
        let primary = Look::primary(colors);
        let trash_label = self.trash_label();
        let close = t(s::MySpace_Close);
        let trash_w = button_width(&bar_ui, &primary, Content::label(&trash_label));
        let close_w = button_width(&bar_ui, &plain, Content::label(close));
        let trash_rect = Rect::from_min_size(
            pos2(bar.right() - 14.0 - trash_w, bar.top() + 12.0),
            vec2(trash_w, 40.0),
        );
        let close_rect = Rect::from_min_size(
            pos2(trash_rect.left() - 8.0 - close_w, bar.top() + 12.0),
            vec2(close_w, 40.0),
        );
        if button_at(
            &mut bar_ui,
            colors,
            &plain,
            Content::label(close),
            true,
            close_rect,
        )
        .clicked()
        {
            action = Some(DuplicatesAction::Close);
        }
        if button_at(
            &mut bar_ui,
            colors,
            &primary,
            Content::label(&trash_label),
            self.can_trash(),
            trash_rect,
        )
        .clicked()
        {
            action = Some(DuplicatesAction::Trash);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            action = Some(DuplicatesAction::Close);
        }
        action
    }
}

#[allow(clippy::too_many_arguments)]
fn group_card(
    ui: &mut Ui,
    colors: &Colors,
    set: &mut DuplicateSet,
    index: usize,
    idle: bool,
    width: f32,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<DuplicatesAction> {
    let mut action = None;
    crate::widgets::card(
        colors.background_panel(),
        colors.border(),
        10,
        egui::Margin::symmetric(14, 12),
    )
    .show(ui, |ui| {
        ui.set_width(width - 28.0);
        let inner = width - 28.0;
        // Header: icon, "N copies of X, Y reclaimable", kept badge, keep button.
        let chip = Look::chip(colors);
        let keep_label = if set.dismissed {
            t(s::MySpace_DuplicatesCountAgain)
        } else {
            t(s::MySpace_DuplicatesKeep)
        };
        let keep_w = button_width(ui, &chip, Content::label(keep_label));
        let badge_text = t(s::MySpace_DuplicatesKeptBadge);
        let badge = layout(
            ui,
            crate::widgets::job(badge_text, 11.0, Weight::SemiBold, colors.text_secondary()),
        );
        let badge_w = if set.dismissed {
            badge.size().x + 12.0 + 9.0
        } else {
            0.0
        };
        let (row, _) = ui.allocate_exact_size(vec2(inner, 40.0), Sense::hover());
        let painter = ui.painter().clone();
        paint_icon(
            ui,
            &painter,
            pos2(row.left() + 8.0, row.center().y),
            icons::COPY,
            15.0,
            colors.accent_start(),
        );
        let header_width = (inner - 16.0 - 9.0 - badge_w - keep_w - 9.0).max(10.0);
        let header = layout(
            ui,
            trimmed(
                &set.header(),
                13.0,
                Weight::SemiBold,
                colors.text_secondary(),
                header_width,
                1,
            ),
        );
        crate::widgets::paint_text_left(
            &painter,
            row.left() + 16.0 + 9.0,
            row.center().y,
            header,
            colors.text_secondary(),
        );
        if set.dismissed {
            let badge_rect = Rect::from_min_size(
                pos2(
                    row.right() - keep_w - 9.0 - badge_w + 9.0 - 9.0,
                    row.center().y - 10.0,
                ),
                vec2(badge.size().x + 12.0, 20.0),
            );
            painter.rect_filled(badge_rect, 6.0, colors.background_toolbar());
            painter.rect_stroke(
                badge_rect,
                6.0,
                egui::Stroke::new(1.0, colors.border()),
                egui::StrokeKind::Inside,
            );
            painter.galley(
                badge_rect.center() - badge.size() / 2.0,
                badge,
                colors.text_secondary(),
            );
        }
        let keep_rect =
            Rect::from_min_size(pos2(row.right() - keep_w, row.top()), vec2(keep_w, 40.0));
        if button_at(
            ui,
            colors,
            &chip,
            Content::label(keep_label),
            idle && !set.busy,
            keep_rect,
        )
        .clicked()
        {
            action = Some(DuplicatesAction::ToggleKeep(index));
        }
        ui.add_space(9.0);
        if let Some(partial) = set.partial() {
            crate::widgets::text_wrapped(
                ui,
                &partial,
                12.0,
                Weight::Normal,
                colors.text_secondary(),
            );
            ui.add_space(9.0);
        }
        for file in &mut set.files {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                checkbox(ui, colors, &mut file.selected, None, true);
                ui.add_space(9.0);
                let age = modlight_core::format::age_of(
                    file.created_at
                        .as_deref()
                        .and_then(modlight_api::models::parse_time),
                    now,
                );
                let folder = file
                    .folder_name
                    .clone()
                    .filter(|f| !f.is_empty())
                    .unwrap_or_else(|| t(s::MySpace_DuplicatesNoFolder).to_string());
                let age_galley = layout(
                    ui,
                    crate::widgets::job(&age, 12.0, Weight::Normal, colors.text_secondary()),
                );
                let folder_galley = layout(
                    ui,
                    trimmed(
                        &folder,
                        12.0,
                        Weight::Normal,
                        colors.text_secondary(),
                        180.0,
                        1,
                    ),
                );
                let oldest = file.is_oldest.then(|| {
                    layout(
                        ui,
                        crate::widgets::job(
                            t(s::MySpace_DuplicatesOldest),
                            11.0,
                            Weight::SemiBold,
                            colors.text_secondary(),
                        ),
                    )
                });
                let fixed = age_galley.size().x
                    + 9.0
                    + folder_galley.size().x
                    + 9.0
                    + oldest
                        .as_ref()
                        .map(|g| g.size().x + 12.0 + 9.0)
                        .unwrap_or(0.0);
                let name_width = (ui.available_width() - fixed).max(20.0);
                let name = layout(
                    ui,
                    trimmed(
                        &file.name,
                        13.0,
                        Weight::SemiBold,
                        colors.text_secondary(),
                        name_width,
                        1,
                    ),
                );
                let (rect, _) = ui.allocate_exact_size(vec2(name_width, 32.0), Sense::hover());
                crate::widgets::paint_text_left(
                    ui.painter(),
                    rect.left(),
                    rect.center().y,
                    name,
                    colors.text_secondary(),
                );
                if let Some(oldest) = oldest {
                    let (badge_rect, _) =
                        ui.allocate_exact_size(vec2(oldest.size().x + 12.0, 32.0), Sense::hover());
                    let badge_rect = Rect::from_center_size(
                        badge_rect.center(),
                        vec2(oldest.size().x + 12.0, 20.0),
                    );
                    ui.painter()
                        .rect_filled(badge_rect, 6.0, colors.background_toolbar());
                    ui.painter().rect_stroke(
                        badge_rect,
                        6.0,
                        egui::Stroke::new(1.0, colors.border()),
                        egui::StrokeKind::Inside,
                    );
                    ui.painter().galley(
                        badge_rect.center() - oldest.size() / 2.0,
                        oldest,
                        colors.text_secondary(),
                    );
                    ui.add_space(9.0);
                }
                let (folder_rect, _) =
                    ui.allocate_exact_size(vec2(folder_galley.size().x, 32.0), Sense::hover());
                crate::widgets::paint_text_left(
                    ui.painter(),
                    folder_rect.left(),
                    folder_rect.center().y,
                    folder_galley,
                    colors.text_secondary(),
                );
                ui.add_space(9.0);
                let (age_rect, _) =
                    ui.allocate_exact_size(vec2(age_galley.size().x, 32.0), Sense::hover());
                crate::widgets::paint_text_left(
                    ui.painter(),
                    age_rect.left(),
                    age_rect.center().y,
                    age_galley,
                    colors.text_secondary(),
                );
            });
            ui.add_space(2.0);
        }
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use modlight_api::{DuplicateFile, DuplicateTotals};

    fn scan() -> DuplicateScan {
        let file = |id: &str| DuplicateFile {
            id: Some(id.into()),
            filename: Some(format!("{id}.png")),
            size: 100,
            ..Default::default()
        };
        DuplicateScan {
            groups: Some(vec![
                DuplicateGroup {
                    checksum: Some("aa".into()),
                    copies: 3,
                    size: 100,
                    bytes: 300,
                    reclaimable: 200,
                    files: Some(vec![file("a1"), file("a2"), file("a3")]),
                    ..Default::default()
                },
                DuplicateGroup {
                    checksum: Some("bb".into()),
                    copies: 2,
                    dismissed: true,
                    files: Some(vec![file("b1"), file("b2")]),
                    ..Default::default()
                },
            ]),
            active: Some(DuplicateTotals {
                groups: 1,
                redundant: 2,
                reclaimable: 200,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn the_default_selection_keeps_the_oldest_copy() {
        let mut model = Duplicates::default();
        model.apply_scan(scan(), false);
        assert_eq!(
            model.selected_ids(),
            vec!["a2", "a3"],
            "the oldest stays, a kept set selects nothing"
        );
        assert_eq!(model.selected_bytes(), 200);
        assert!(model.can_trash());
        for set in &mut model.sets {
            set.clear_selection();
        }
        assert!(!model.can_trash());
        assert_eq!(model.trash_label(), t(s::MySpace_DuplicatesTrashNone));
    }
}
