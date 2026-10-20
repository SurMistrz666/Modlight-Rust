//! "Quick task menu editor" (`QuickTaskMenuEditorWindow`): 760 by 640 (down to 620 by
//! 500). A toolbar (Add, Edit, Remove, up, down, Reset), the presets list, the hint and
//! Close; an overlay to add or edit one preset, and one to confirm the reset.

use egui::{Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t};
use modlight_core::tasks::settings::{AfterCaptureTasks, AfterUploadTasks, FlagNames};

use super::background;
use super::quick_task_menu::QuickTaskInfo;
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Field, Look, button_at, checkbox_weighted, layout, paint_icon, trimmed, wrapped,
};

pub const WIDTH: f32 = 760.0;
pub const HEIGHT: f32 = 640.0;
pub const MIN_WIDTH: f32 = 620.0;
pub const MIN_HEIGHT: f32 = 500.0;

/// The preset being added or edited.
pub struct ItemEditor {
    /// `None`: a new preset.
    pub index: Option<usize>,
    pub name: String,
    pub after_capture: Vec<(AfterCaptureTasks, &'static str, bool)>,
    pub after_upload: Vec<(AfterUploadTasks, &'static str, bool)>,
}

pub struct QuickTaskEditor {
    pub presets: Vec<QuickTaskInfo>,
    pub selected: Option<usize>,
    pub editor: Option<ItemEditor>,
    pub confirm_reset: bool,
    /// The list changed and should be saved.
    pub dirty: bool,
}

/// What the window asks the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickTaskEditorOutcome {
    Close,
}

impl ItemEditor {
    fn new(index: Option<usize>, preset: &QuickTaskInfo) -> ItemEditor {
        ItemEditor {
            index,
            name: preset.name.clone().unwrap_or_default(),
            after_capture: super::after_capture::after_capture_options()
                .into_iter()
                .map(|(task, label)| (task, label, preset.after_capture.has(task)))
                .collect(),
            after_upload: super::after_capture::after_upload_options()
                .into_iter()
                .map(|(task, label)| (task, label, preset.after_upload.has(task)))
                .collect(),
        }
    }

    fn flags(&self) -> (AfterCaptureTasks, AfterUploadTasks) {
        let capture = self
            .after_capture
            .iter()
            .filter(|(_, _, c)| *c)
            .fold(AfterCaptureTasks::None, |acc, (task, _, _)| acc.with(*task));
        let upload = self
            .after_upload
            .iter()
            .filter(|(_, _, c)| *c)
            .fold(AfterUploadTasks::None, |acc, (task, _, _)| acc.with(*task));
        (capture, upload)
    }
}

impl QuickTaskEditor {
    pub fn new(presets: Vec<QuickTaskInfo>) -> QuickTaskEditor {
        let selected = (!presets.is_empty()).then_some(0);
        QuickTaskEditor {
            presets,
            selected,
            editor: None,
            confirm_reset: false,
            dirty: false,
        }
    }

    pub fn title() -> &'static str {
        t(s::QuickTaskMenuEditorWindow_Title)
    }

    /// The presets as the settings file keeps them.
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::Value::Array(
            self.presets
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "Name": p.name.clone().unwrap_or_default(),
                        "AfterCaptureTasks": p.after_capture.names(),
                        "AfterUploadTasks": p.after_upload.names(),
                    })
                })
                .collect(),
        )
    }

    /// `QuickTaskPresetItem.Summary`.
    fn summary(preset: &QuickTaskInfo) -> String {
        let capture: Vec<&str> = super::after_capture::after_capture_options()
            .into_iter()
            .filter(|(task, _)| preset.after_capture.has(*task))
            .map(|(_, l)| l)
            .collect();
        let upload: Vec<&str> = super::after_capture::after_upload_options()
            .into_iter()
            .filter(|(task, _)| preset.after_upload.has(*task))
            .map(|(_, l)| l)
            .collect();
        if upload.is_empty() {
            capture.join(", ")
        } else {
            format(
                t(s::QuickTaskMenuEditorWindow_AfterUploadSummary),
                &[Arg::from(capture.join(", ")), Arg::from(upload.join(", "))],
            )
        }
    }

    fn remove_selected(&mut self) {
        if let Some(index) = self.selected.filter(|i| *i < self.presets.len()) {
            self.presets.remove(index);
            self.selected = if self.presets.is_empty() {
                None
            } else {
                Some(index.min(self.presets.len() - 1))
            };
            self.dirty = true;
        }
    }

    fn move_selected(&mut self, offset: i64) {
        let Some(index) = self.selected else { return };
        let target = index as i64 + offset;
        if target < 0 || target as usize >= self.presets.len() {
            return;
        }
        self.presets.swap(index, target as usize);
        self.selected = Some(target as usize);
        self.dirty = true;
    }

    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors) -> Option<QuickTaskEditorOutcome> {
        background(ui, colors);
        let full = ui.max_rect();
        let painter = ui.painter().clone();
        let mut outcome = None;
        let body = Rect::from_min_max(
            pos2(full.left() + 12.0, full.top()),
            pos2(full.right() - 12.0, full.bottom() - 12.0),
        );
        let blocked = self.editor.is_some() || self.confirm_reset;

        // The toolbar.
        let bar = Rect::from_min_size(body.min, vec2(body.width(), 12.0 + 34.0 + 12.0));
        let radius = egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: 4,
            se: 4,
        };
        painter.rect_filled(bar, radius, colors.background_panel());
        painter.rect_stroke(
            bar,
            radius,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        let look = Look::button(colors).height(34.0).padding(9.0).spacing(7.0);
        let square = Look::button(colors)
            .height(34.0)
            .width(34.0)
            .padding(0.0)
            .icon(16.0, None);
        let index = self.selected;
        let has = index.is_some();
        let mut x = bar.left() + 12.0;
        let mut clicked = None;
        for (glyph, label, enabled, which) in [
            (icons::PLUS, t(s::QuickTaskMenuEditorWindow_Add), true, 0),
            (
                icons::SETTINGS_2,
                t(s::QuickTaskMenuEditorWindow_Edit),
                has,
                1,
            ),
            (
                icons::TRASH_2,
                t(s::QuickTaskMenuEditorWindow_Remove),
                has,
                2,
            ),
        ] {
            let w = crate::widgets::button_width(ui, &look, Content::both(glyph, label));
            let r = Rect::from_min_size(pos2(x, bar.top() + 12.0), vec2(w, 34.0));
            if button_at(
                ui,
                colors,
                &look,
                Content::both(glyph, label),
                enabled && !blocked,
                r,
            )
            .clicked()
            {
                clicked = Some(which);
            }
            x = r.right();
        }
        for (glyph, tip, enabled, which) in [
            (
                icons::CHEVRON_UP,
                t(s::QuickTaskMenuEditorWindow_MoveUp),
                index.is_some_and(|i| i > 0),
                3,
            ),
            (
                icons::CHEVRON_DOWN,
                t(s::QuickTaskMenuEditorWindow_MoveDown),
                index.is_some_and(|i| i + 1 < self.presets.len()),
                4,
            ),
        ] {
            let r = Rect::from_min_size(pos2(x, bar.top() + 12.0), vec2(34.0, 34.0));
            if button_at(
                ui,
                colors,
                &square,
                Content::icon(glyph),
                enabled && !blocked,
                r,
            )
            .on_hover_text(tip)
            .clicked()
            {
                clicked = Some(which);
            }
            x = r.right();
        }
        let reset = t(s::QuickTaskMenuEditorWindow_Reset);
        let w = crate::widgets::button_width(ui, &look, Content::both(icons::ROTATE_CCW, reset));
        if button_at(
            ui,
            colors,
            &look,
            Content::both(icons::ROTATE_CCW, reset),
            !blocked,
            Rect::from_min_size(
                pos2(bar.right() - 12.0 - w, bar.top() + 12.0),
                vec2(w, 34.0),
            ),
        )
        .clicked()
        {
            clicked = Some(5);
        }
        match clicked {
            Some(0) => {
                self.editor = Some(ItemEditor::new(
                    None,
                    &QuickTaskInfo {
                        name: None,
                        after_capture: AfterCaptureTasks::None,
                        after_upload: AfterUploadTasks::None,
                    },
                ))
            }
            Some(1) => {
                if let Some(i) = self.selected {
                    self.editor = Some(ItemEditor::new(Some(i), &self.presets[i]));
                }
            }
            Some(2) => self.remove_selected(),
            Some(3) => self.move_selected(-1),
            Some(4) => self.move_selected(1),
            Some(5) => self.confirm_reset = true,
            _ => {}
        }

        // The footer: the hint and Close.
        let footer = Rect::from_min_max(pos2(body.left(), body.bottom() - 34.0), body.max);
        let close_rect =
            Rect::from_min_size(pos2(footer.right() - 88.0, footer.top()), vec2(88.0, 34.0));
        if button_at(
            ui,
            colors,
            &Look::compact(colors).height(34.0).width(88.0),
            Content::label(t(s::QuickTaskMenuEditorWindow_Close)),
            !blocked,
            close_rect,
        )
        .clicked()
        {
            outcome = Some(QuickTaskEditorOutcome::Close);
        }
        let hint = layout(
            ui,
            wrapped(
                t(s::QuickTaskMenuEditorWindow_SeparatorHint),
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                close_rect.left() - 10.0 - footer.left(),
            ),
        );
        painter.galley(
            pos2(footer.left(), footer.center().y - hint.size().y / 2.0),
            hint,
            colors.text_secondary(),
        );

        // The list.
        let list = Rect::from_min_max(
            pos2(body.left(), bar.bottom() + 8.0),
            pos2(body.right(), footer.top() - 8.0),
        );
        painter.rect_filled(list, 0.0, colors.background_main());
        painter.rect_stroke(
            list,
            0.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list.shrink(1.0)));
        let mut double = None;
        egui::ScrollArea::vertical()
            .id_salt("quick-task-list")
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, preset) in self.presets.iter().enumerate() {
                    let (outer, response) = ui.allocate_exact_size(
                        vec2(ui.available_width(), 42.0 + 4.0),
                        Sense::click(),
                    );
                    let row = outer.shrink(2.0);
                    if self.selected == Some(i) || response.hovered() {
                        ui.painter()
                            .rect_filled(row, 4.0, colors.control_background_hover());
                    }
                    if preset.is_valid() {
                        let title = layout(
                            ui,
                            trimmed(
                                &preset.text(),
                                13.0,
                                Weight::Normal,
                                colors.text(),
                                row.width() - 16.0,
                                1,
                            ),
                        );
                        let summary = layout(
                            ui,
                            trimmed(
                                &Self::summary(preset),
                                12.0,
                                Weight::Normal,
                                colors.text_secondary(),
                                row.width() - 16.0,
                                1,
                            ),
                        );
                        let total = title.size().y + 1.0 + summary.size().y;
                        let top = row.center().y - total / 2.0;
                        let title_h = title.size().y;
                        ui.painter()
                            .galley(pos2(row.left() + 8.0, top), title, colors.text());
                        ui.painter().galley(
                            pos2(row.left() + 8.0, top + title_h + 1.0),
                            summary,
                            colors.text_secondary(),
                        );
                    } else {
                        let label = layout(
                            ui,
                            trimmed(
                                t(s::QuickTaskMenuEditorWindow_Separator),
                                12.0,
                                Weight::Normal,
                                colors.text_secondary(),
                                300.0,
                                1,
                            ),
                        );
                        let cx = row.center().x;
                        let half = label.size().x / 2.0 + 10.0;
                        let y = row.center().y;
                        ui.painter().hline(
                            row.left() + 8.0..=cx - half,
                            y,
                            Stroke::new(1.0, colors.border()),
                        );
                        ui.painter().hline(
                            cx + half..=row.right() - 8.0,
                            y,
                            Stroke::new(1.0, colors.border()),
                        );
                        ui.painter().galley(
                            pos2(cx - label.size().x / 2.0, y - label.size().y / 2.0),
                            label,
                            colors.text_secondary(),
                        );
                    }
                    if response.clicked() && !blocked {
                        self.selected = Some(i);
                    }
                    if response.double_clicked() && !blocked {
                        double = Some(i);
                    }
                }
            });
        if let Some(i) = double {
            self.editor = Some(ItemEditor::new(Some(i), &self.presets[i]));
        }
        if !blocked && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
            self.remove_selected();
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.confirm_reset {
                self.confirm_reset = false;
            } else if self.editor.is_some() {
                self.editor = None;
            } else {
                outcome = Some(QuickTaskEditorOutcome::Close);
            }
        }

        if self.editor.is_some() {
            self.item_editor_ui(ui, colors, full);
        }
        if self.confirm_reset {
            painter.rect_filled(full, 0.0, colors.overlay_modal());
            let text = layout(
                ui,
                wrapped(
                    t(s::QuickTaskMenuEditorWindow_ResetConfirmation),
                    13.0,
                    Weight::Normal,
                    colors.text(),
                    420.0 - 36.0,
                ),
            );
            let h = 18.0 + 24.0 + 12.0 + text.size().y + 12.0 + 40.0 + 18.0;
            let panel = Rect::from_center_size(full.center(), vec2(420.0, h));
            painter.rect_filled(panel, 8.0, colors.background_panel());
            painter.rect_stroke(
                panel,
                8.0,
                Stroke::new(1.0, colors.border()),
                StrokeKind::Inside,
            );
            let inner = panel.shrink(18.0);
            paint_icon(
                ui,
                &painter,
                pos2(inner.left() + 10.0, inner.top() + 12.0),
                icons::ROTATE_CCW,
                20.0,
                colors.accent_start(),
            );
            let heading = layout(
                ui,
                trimmed(
                    t(s::QuickTaskMenuEditorWindow_ResetPrompt),
                    18.0,
                    Weight::SemiBold,
                    colors.text_secondary(),
                    inner.width() - 30.0,
                    1,
                ),
            );
            painter.galley(
                pos2(
                    inner.left() + 29.0,
                    inner.top() + 12.0 - heading.size().y / 2.0,
                ),
                heading,
                colors.text_secondary(),
            );
            let text_h = text.size().y;
            painter.galley(
                pos2(inner.left(), inner.top() + 24.0 + 12.0),
                text,
                colors.text(),
            );
            let look = Look::compact(colors).height(40.0);
            let (go, cancel) = (
                t(s::QuickTaskMenuEditorWindow_ResetAction),
                t(s::QuickTaskMenuEditorWindow_Cancel),
            );
            let cancel_w =
                crate::widgets::button_width(ui, &look, Content::label(cancel)).max(84.0);
            let go_w = crate::widgets::button_width(ui, &look, Content::label(go)).max(84.0);
            let y = inner.top() + 24.0 + 12.0 + text_h + 12.0;
            let cancel_r =
                Rect::from_min_size(pos2(inner.right() - cancel_w, y), vec2(cancel_w, 40.0));
            let go_r = Rect::from_min_size(pos2(cancel_r.left() - 8.0 - go_w, y), vec2(go_w, 40.0));
            if button_at(ui, colors, &look, Content::label(go), true, go_r).clicked() {
                self.presets = QuickTaskInfo::defaults();
                self.selected = Some(0);
                self.confirm_reset = false;
                self.dirty = true;
            }
            if button_at(ui, colors, &look, Content::label(cancel), true, cancel_r).clicked() {
                self.confirm_reset = false;
            }
        }
        outcome
    }

    fn item_editor_ui(&mut self, ui: &mut Ui, colors: &Colors, full: Rect) {
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, colors.overlay_modal());
        let panel = Rect::from_center_size(
            full.center(),
            vec2(
                700.0_f32.min(full.width() - 24.0),
                600.0_f32.min(full.height() - 24.0),
            ),
        );
        painter.rect_filled(panel, 8.0, colors.background_panel());
        painter.rect_stroke(
            panel,
            8.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        let inner = panel.shrink(16.0);
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let title = if editor.index.is_none() {
            t(s::QuickTaskMenuEditorWindow_AddQuickTask)
        } else {
            t(s::QuickTaskMenuEditorWindow_EditQuickTask)
        };
        paint_icon(
            ui,
            &painter,
            pos2(inner.left() + 10.5, inner.top() + 20.0),
            icons::LIST_CHECKS,
            21.0,
            colors.accent_start(),
        );
        let heading = layout(
            ui,
            trimmed(
                title,
                19.0,
                Weight::SemiBold,
                colors.text_secondary(),
                inner.width() - 30.0,
                1,
            ),
        );
        let hint = layout(
            ui,
            trimmed(
                t(s::QuickTaskMenuEditorWindow_ChooseActions),
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                inner.width() - 30.0,
                1,
            ),
        );
        let heading_h = heading.size().y;
        painter.galley(
            pos2(inner.left() + 30.0, inner.top()),
            heading,
            colors.text_secondary(),
        );
        painter.galley(
            pos2(inner.left() + 30.0, inner.top() + heading_h + 1.0),
            hint,
            colors.text_secondary(),
        );
        let content = Rect::from_min_max(
            pos2(inner.left(), inner.top() + 44.0 + 12.0),
            pos2(inner.right(), inner.bottom() - 40.0 - 12.0),
        );
        let (capture, upload) = editor.flags();
        let generated = QuickTaskInfo {
            name: None,
            after_capture: capture,
            after_upload: upload,
        }
        .text();
        let placeholder = if generated.is_empty() {
            t(s::QuickTaskMenuEditorWindow_Separator).to_string()
        } else {
            generated
        };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content));
        egui::ScrollArea::vertical()
            .id_salt("quick-task-editor")
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                crate::widgets::text(
                    ui,
                    t(s::QuickTaskMenuEditorWindow_MenuText),
                    13.0,
                    Weight::Normal,
                    colors.text_secondary(),
                );
                Field {
                    height: 32.0,
                    hint: &placeholder,
                    ..Field::new(&mut editor.name)
                }
                .show(ui, colors);
                crate::widgets::text_wrapped(
                    ui,
                    t(s::QuickTaskMenuEditorWindow_MenuTextHint),
                    12.0,
                    Weight::Normal,
                    colors.text_secondary(),
                );
                ui.add_space(10.0);
                ui.columns(2, |columns| {
                    for (column, title) in [
                        (0usize, t(s::QuickTaskMenuEditorWindow_AfterCaptureTasks)),
                        (1, t(s::QuickTaskMenuEditorWindow_AfterUploadTasks)),
                    ] {
                        let ui = &mut columns[column];
                        egui::Frame::new()
                            .fill(colors.control_background())
                            .stroke(Stroke::new(1.0, colors.border()))
                            .corner_radius(5)
                            .inner_margin(10)
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                crate::widgets::text(
                                    ui,
                                    title,
                                    14.0,
                                    Weight::SemiBold,
                                    colors.text_secondary(),
                                );
                                if column == 0 {
                                    for (_, label, checked) in editor.after_capture.iter_mut() {
                                        checkbox_weighted(
                                            ui,
                                            colors,
                                            checked,
                                            Some(label),
                                            true,
                                            Weight::Normal,
                                        );
                                    }
                                } else {
                                    for (_, label, checked) in editor.after_upload.iter_mut() {
                                        checkbox_weighted(
                                            ui,
                                            colors,
                                            checked,
                                            Some(label),
                                            true,
                                            Weight::Normal,
                                        );
                                    }
                                }
                            });
                    }
                });
            });
        let look = Look::compact(colors).height(40.0);
        let (save, cancel) = (
            t(s::QuickTaskMenuEditorWindow_Save),
            t(s::QuickTaskMenuEditorWindow_Cancel),
        );
        let cancel_w = crate::widgets::button_width(ui, &look, Content::label(cancel)).max(84.0);
        let save_w = crate::widgets::button_width(ui, &look, Content::label(save)).max(84.0);
        let cancel_r = Rect::from_min_size(
            pos2(inner.right() - cancel_w, inner.bottom() - 40.0),
            vec2(cancel_w, 40.0),
        );
        let save_r = Rect::from_min_size(
            pos2(cancel_r.left() - 8.0 - save_w, cancel_r.top()),
            vec2(save_w, 40.0),
        );
        if button_at(ui, colors, &look, Content::label(save), true, save_r).clicked() {
            let editor = self.editor.take().unwrap();
            let (capture, upload) = editor.flags();
            let preset = QuickTaskInfo {
                name: Some(editor.name.clone()),
                after_capture: capture,
                after_upload: upload,
            };
            match editor.index {
                Some(i) if i < self.presets.len() => {
                    self.presets[i] = preset;
                    self.selected = Some(i);
                }
                _ => {
                    self.presets.push(preset);
                    self.selected = Some(self.presets.len() - 1);
                }
            }
            self.dirty = true;
            return;
        }
        if button_at(ui, colors, &look, Content::label(cancel), true, cancel_r).clicked() {
            self.editor = None;
        }
    }
}
