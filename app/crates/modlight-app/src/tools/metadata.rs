//! `MetadataWindow` and `MetadataViewModel`: a file's tags read by ExifTool, grouped,
//! searchable, copied as text; links in values open; "Strip metadata" after a confirming
//! overlay. `TaskHelpers.StripMetadata` (the hotkey) strips without the window.

use std::path::{Path, PathBuf};

use egui::{Align2, Id, Rect, Sense, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t, tools as tl};
use modlight_core::tasks::host::{MessageIcon, NotificationSound};
use modlight_core::tasks::settings::TaskSettings;
use modlight_tools::metadata::{self as md, Entry, ExifTool};

use super::jobs::ToolContinuation;
use super::kit::{self, Track};
use super::{ToolWindow, WindowSpec};
use crate::state::{AppState, Effect};
use crate::theme::Weight;

pub struct MetadataWindow {
    pub key: u64,
    pub file: String,
    pub entries: Vec<Entry>,
    pub busy: bool,
    pub confirming_strip: bool,
    pub search: String,
    pub empty_message: String,
    exiftool: Option<ExifTool>,
    pub started: bool,
    /// QA renders show this path instead of the fixture's temporary one.
    pub qa_display: Option<String>,
}

impl MetadataWindow {
    pub fn new(key: u64, file: Option<String>, exiftool: Option<ExifTool>) -> MetadataWindow {
        MetadataWindow {
            key,
            file: file.unwrap_or_default(),
            entries: Vec::new(),
            busy: false,
            confirming_strip: false,
            search: String::new(),
            empty_message: t(tl::MetadataViewModel_Open_or_drop).to_string(),
            exiftool,
            started: false,
            qa_display: None,
        }
    }

    fn exists(&self) -> bool {
        Path::new(&self.file).is_file()
    }

    pub fn file_name(&self) -> String {
        if self.exists() {
            Path::new(self.qa_display.as_deref().unwrap_or(&self.file))
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            t(tl::MetadataViewModel_No_file_selected).to_string()
        }
    }

    /// `WindowTitle`.
    pub fn title(&self) -> String {
        if self.exists() {
            format(
                t(tl::MetadataViewModel_Window_title_file),
                &[Arg::from(self.file_name())],
            )
        } else {
            t(tl::MetadataViewModel_Window_title).to_string()
        }
    }

    pub fn can_copy(&self) -> bool {
        !self.busy && !self.entries.is_empty()
    }

    pub fn can_strip(&self) -> bool {
        !self.busy && self.exists()
    }

    /// `LoadMetadataAsync`.
    fn load(&mut self, state: &mut AppState) {
        self.busy = true;
        self.confirming_strip = false;
        self.entries.clear();
        self.empty_message = t(tl::MetadataViewModel_Reading_metadata).to_string();
        let tool = self.exiftool.clone();
        let file = PathBuf::from(&self.file);
        let key = self.key;
        state.rt.spawn_blocking(move || {
            let result = match &tool {
                Some(tool) => tool.read(&file),
                None => Err(t(tl::MetadataService_ExifTool_not_found).to_string()),
            };
            move |state: &mut AppState, _ctx: &egui::Context| {
                if let Some(window) = state.tools.window_mut::<MetadataWindow>(key) {
                    window.loaded(result);
                }
            }
        });
    }

    pub fn loaded(&mut self, result: Result<Vec<Entry>, String>) {
        self.busy = false;
        match result {
            Ok(entries) => {
                self.empty_message = if entries.is_empty() {
                    t(tl::MetadataViewModel_No_metadata).to_string()
                } else {
                    t(tl::MetadataViewModel_No_search_matches).to_string()
                };
                self.entries = entries;
            }
            Err(error) => {
                log::warn!("Failed to read file metadata: {error}");
                self.empty_message =
                    format(t(tl::MetadataViewModel_Unable_read), &[Arg::from(error)]);
            }
        }
    }

    /// `OpenFileAsync`.
    fn open_file(&mut self, state: &mut AppState, file: &Path) {
        if self.busy || !file.is_file() {
            return;
        }
        self.file = file.to_string_lossy().into_owned();
        self.search.clear();
        self.load(state);
    }

    fn browse(&mut self, state: &mut AppState) {
        if self.busy {
            return;
        }
        if let Some(path) = state.tool_open_file(t(tl::MetadataWindow_Open_file_dialog), &[]) {
            self.open_file(state, &path);
        }
    }

    /// `ConfirmStripAsync`.
    fn strip(&mut self, state: &mut AppState) {
        if !self.can_strip() {
            return;
        }
        self.confirming_strip = false;
        self.busy = true;
        let tool = self.exiftool.clone();
        let file = PathBuf::from(&self.file);
        let key = self.key;
        state.rt.spawn_blocking(move || {
            let result = match &tool {
                Some(tool) => tool.strip(&file),
                None => Err(t(tl::MetadataService_ExifTool_not_found).to_string()),
            };
            move |state: &mut AppState, _ctx: &egui::Context| {
                if result.is_ok() {
                    let settings = state.default_task_settings();
                    state.play_task_sound(NotificationSound::ActionCompleted, &settings);
                }
                super::with_window::<MetadataWindow, _>(state, key, |window, state| {
                    window.busy = false;
                    match result {
                        Ok(()) => window.load(state),
                        Err(error) => {
                            log::warn!("Failed to strip file metadata: {error}");
                            window.empty_message =
                                format(t(tl::MetadataViewModel_Unable_strip), &[Arg::from(error)]);
                        }
                    }
                });
            }
        });
    }
}

impl ToolWindow for MetadataWindow {
    fn spec(&self) -> WindowSpec {
        WindowSpec::resizable(&self.title(), vec2(900.0, 650.0), vec2(700.0, 500.0))
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        if !self.started {
            // `StartAsync`: the file, or the open panel.
            self.started = true;
            if self.exists() {
                self.load(state);
            } else {
                self.browse(state);
            }
        }
        kit::fill(ui, &colors);
        let dropped: Option<PathBuf> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .find(|p| p.is_file())
        });
        if let Some(path) = dropped {
            self.open_file(state, &path);
        }
        let full = kit::inset(ui.max_rect(), 16.0);
        let header_h = 12.0 * 2.0 + 40.0 + if self.busy { 8.0 + 5.0 } else { 0.0 };
        let rows = kit::rows(full, &[Track::Px(header_h), Track::Star(1.0)], 12.0);

        // The file panel.
        kit::panel(ui, &colors, rows[0]);
        let inner = rows[0].shrink(12.0);
        let line = Rect::from_min_size(inner.min, vec2(inner.width(), 40.0));
        let open_label = t(tl::MetadataWindow_Open);
        let copy_label = t(tl::MetadataWindow_Copy_all);
        let strip_label = t(tl::MetadataWindow_Strip_metadata);
        let widths = [
            kit::button_width(ui, &colors, Some(icons::FOLDER_OPEN), Some(open_label)),
            kit::button_width(ui, &colors, Some(icons::COPY), Some(copy_label)),
            kit::button_width(ui, &colors, Some(icons::ERASER), Some(strip_label)),
        ];
        let cols = kit::columns(
            line,
            &[
                Track::Px(28.0),
                Track::Star(1.0),
                Track::Px(widths[0]),
                Track::Px(widths[1]),
                Track::Px(widths[2]),
            ],
            10.0,
        );
        crate::widgets::paint_icon(
            ui,
            ui.painter(),
            cols[0].center(),
            icons::FILE_SEARCH,
            28.0,
            colors.text_secondary(),
        );
        let name_rect = Rect::from_min_size(
            pos2(cols[1].left(), cols[1].center().y - 19.0),
            vec2(cols[1].width(), 20.0),
        );
        kit::heading(ui, &colors, name_rect, &self.file_name());
        let description = if self.exists() {
            self.qa_display.clone().unwrap_or_else(|| self.file.clone())
        } else {
            t(tl::MetadataViewModel_Open_or_drop_supported).to_string()
        };
        kit::path_text(
            ui,
            &colors,
            Rect::from_min_size(
                pos2(cols[1].left(), name_rect.bottom() + 2.0),
                vec2(cols[1].width(), 16.0),
            ),
            &description,
        );
        if kit::button_at(
            ui,
            &colors,
            cols[2],
            Some(icons::FOLDER_OPEN),
            Some(open_label),
            !self.busy,
        )
        .clicked()
        {
            self.browse(state);
        }
        if kit::button_at(
            ui,
            &colors,
            cols[3],
            Some(icons::COPY),
            Some(copy_label),
            self.can_copy(),
        )
        .clicked()
        {
            state
                .effects
                .push(Effect::CopyText(md::copy_text(&self.entries)));
        }
        if kit::button_at(
            ui,
            &colors,
            cols[4],
            Some(icons::ERASER),
            Some(strip_label),
            self.can_strip(),
        )
        .clicked()
        {
            self.confirming_strip = true;
        }
        if self.busy {
            let bar = Rect::from_min_size(
                pos2(inner.left(), line.bottom() + 8.0),
                vec2(inner.width(), 5.0),
            );
            crate::widgets::paint_progress(ui, &colors, bar, None);
        }

        // The metadata panel.
        kit::panel(ui, &colors, rows[1]);
        let head = Rect::from_min_size(
            rows[1].min + vec2(12.0, 12.0),
            vec2(rows[1].width() - 24.0, 40.0),
        );
        let head_cols = kit::columns(head, &[Track::Star(1.0), Track::Px(260.0)], 10.0);
        let title = t(tl::MetadataWindow_Metadata);
        let title_w = kit::text_width(ui, title, 15.0, Weight::SemiBold);
        kit::heading(
            ui,
            &colors,
            Rect::from_min_size(head_cols[0].min, vec2(title_w + 1.0, 40.0)),
            title,
        );
        kit::text_at(
            ui,
            Rect::from_min_size(
                pos2(head_cols[0].left() + title_w + 8.0, head_cols[0].top()),
                vec2(200.0, 40.0),
            ),
            Align2::LEFT_CENTER,
            &md::count_text(self.entries.len()),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
        );
        let can_search = self.can_copy();
        kit::text_box_at(
            ui,
            &colors,
            head_cols[1],
            Id::new(("metadata-search", self.key)),
            &mut self.search,
            t(tl::MetadataWindow_Search_metadata),
            can_search,
            false,
        );
        crate::widgets::paint_icon(
            ui,
            ui.painter(),
            pos2(head_cols[1].right() - 12.0 - 7.0, head_cols[1].center().y),
            icons::SEARCH,
            14.0,
            colors.text_secondary(),
        );
        let list = Rect::from_min_max(
            pos2(rows[1].left() + 12.0, head.bottom() + 12.0),
            pos2(rows[1].right() - 12.0, rows[1].bottom() - 12.0),
        );
        let groups = md::groups(&self.entries, &self.search);
        if groups.is_empty() {
            let galley = crate::widgets::layout(
                ui,
                crate::widgets::wrapped(
                    &self.empty_message,
                    13.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    420.0,
                ),
            );
            let total = 48.0 + 10.0 + galley.size().y;
            let top = list.center().y - total / 2.0;
            crate::widgets::paint_icon(
                ui,
                ui.painter(),
                pos2(list.center().x, top + 24.0),
                icons::TAGS,
                48.0,
                colors.text_secondary(),
            );
            ui.painter().galley(
                pos2(list.center().x - galley.size().x / 2.0, top + 58.0),
                galley,
                colors.text_secondary(),
            );
        } else {
            let mut child = kit::column(ui, list);
            child.set_clip_rect(list);
            let mut open = None;
            egui::ScrollArea::vertical()
                .id_salt(("metadata-list", self.key))
                .auto_shrink([false, false])
                .show(&mut child, |ui| {
                    let width = ui.available_width() - 10.0;
                    for group in &groups {
                        let top = ui.cursor().min.y;
                        let x = ui.cursor().min.x;
                        // The group's header.
                        let header = Rect::from_min_size(pos2(x, top), vec2(width, 20.0 + 14.0));
                        ui.painter().rect_filled(
                            header,
                            egui::CornerRadius {
                                nw: 4,
                                ne: 4,
                                sw: 0,
                                se: 0,
                            },
                            colors.control_background(),
                        );
                        kit::text_at(
                            ui,
                            header.shrink2(vec2(10.0, 0.0)),
                            Align2::LEFT_CENTER,
                            &group.name,
                            15.0,
                            Weight::SemiBold,
                            colors.text(),
                        );
                        kit::text_at(
                            ui,
                            header.shrink2(vec2(10.0, 0.0)),
                            Align2::RIGHT_CENTER,
                            &format!("{} tags", group.entries.len()),
                            13.0,
                            Weight::Normal,
                            colors.text_secondary(),
                        );
                        let mut y = header.bottom();
                        for (index, entry) in group.entries.iter().enumerate() {
                            let value_w = width - 20.0 - 190.0 - 12.0;
                            let value_color = if entry.url.is_some() {
                                colors.accent_start()
                            } else {
                                colors.text()
                            };
                            let value = crate::widgets::layout(
                                ui,
                                crate::widgets::wrapped(
                                    &entry.value,
                                    13.0,
                                    Weight::Normal,
                                    value_color,
                                    value_w.max(40.0),
                                ),
                            );
                            let tag = crate::widgets::layout(
                                ui,
                                crate::widgets::wrapped(
                                    &entry.tag,
                                    13.0,
                                    Weight::Normal,
                                    colors.text_secondary(),
                                    190.0,
                                ),
                            );
                            let h = value.size().y.max(tag.size().y).max(32.0);
                            let row =
                                Rect::from_min_size(pos2(x + 10.0, y + 2.0), vec2(width - 20.0, h));
                            ui.painter().galley(
                                pos2(row.left(), row.center().y - tag.size().y / 2.0),
                                tag,
                                colors.text_secondary(),
                            );
                            let value_pos = pos2(
                                row.left() + 190.0 + 12.0,
                                row.center().y - value.size().y / 2.0,
                            );
                            let value_rect = Rect::from_min_size(value_pos, value.size());
                            if entry.url.is_some() {
                                ui.painter().hline(
                                    value_rect.left()
                                        ..=value_rect.right().min(value_pos.x + value.size().x),
                                    value_rect.bottom() - 1.0,
                                    egui::Stroke::new(1.0, colors.accent_start()),
                                );
                                let response = ui
                                    .interact(
                                        value_rect,
                                        Id::new(("metadata-url", group.name.as_str(), index)),
                                        Sense::click(),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                                if response.clicked() {
                                    open = entry.url.clone();
                                }
                            }
                            ui.painter().galley(value_pos, value, value_color);
                            y = row.bottom() + 2.0;
                        }
                        let border = Rect::from_min_max(pos2(x, top), pos2(x + width, y));
                        ui.painter().rect_stroke(
                            border,
                            4.0,
                            egui::Stroke::new(1.0, colors.border()),
                            egui::StrokeKind::Inside,
                        );
                        ui.allocate_exact_size(vec2(width + 10.0, y - top + 7.0), Sense::hover());
                    }
                });
            if let Some(url) = open {
                state.open_url(&url);
            }
        }

        // The strip confirmation over everything.
        if self.confirming_strip {
            let full = ui.max_rect();
            ui.painter().rect_filled(full, 0.0, colors.overlay_modal());
            let _ = ui.interact(full, Id::new(("metadata-modal", self.key)), Sense::click());
            let text = t(tl::MetadataWindow_This_permanently_removes_all_non_essential_metadata_from_the_selected_file_This_action_cannot_be_undone);
            let galley = crate::widgets::layout(
                ui,
                crate::widgets::wrapped(
                    text,
                    13.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    470.0 - 36.0,
                ),
            );
            let h = 18.0 * 2.0 + 28.0 + 14.0 + galley.size().y + 14.0 + 40.0;
            let card = Rect::from_center_size(full.center(), vec2(470.0, h));
            kit::panel(ui, &colors, card);
            let inner = card.shrink(18.0);
            crate::widgets::paint_icon(
                ui,
                ui.painter(),
                pos2(inner.left() + 12.0, inner.top() + 14.0),
                icons::TRIANGLE_ALERT,
                24.0,
                colors.cancel_border(),
            );
            kit::heading(
                ui,
                &colors,
                Rect::from_min_size(
                    pos2(inner.left() + 34.0, inner.top() + 4.0),
                    vec2(inner.width() - 34.0, 20.0),
                ),
                t(tl::MetadataWindow_Strip_all_metadata),
            );
            ui.painter().galley(
                pos2(inner.left(), inner.top() + 28.0 + 14.0),
                galley,
                colors.text_secondary(),
            );
            let buttons = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 40.0), inner.max);
            let items = [
                (
                    Some(icons::ERASER),
                    Some(t(tl::MetadataWindow_Strip_metadata_2)),
                ),
                (None, Some(t(tl::MetadataWindow_Cancel))),
            ];
            let rects = kit::right_aligned(ui, &colors, buttons, &items, 8.0);
            if kit::button_at(ui, &colors, rects[0], items[0].0, items[0].1, true).clicked() {
                self.strip(state);
            }
            if kit::button_at(ui, &colors, rects[1], items[1].0, items[1].1, true).clicked() {
                self.confirming_strip = false;
            }
        }
        false
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl AppState {
    /// `CheckExifTool`: the component, offered when missing; `None` while it is not there.
    fn check_exiftool(&mut self, then: ToolContinuation) -> Option<ExifTool> {
        if let Some(tool) = self.exiftool() {
            return Some(tool);
        }
        let then = crate::components_window::AfterClose::Tool(Box::new(then));
        if crate::components_window::require(self, "exiftool", s::Components_FeatureMetadata, then)
        {
            let path = self
                .env
                .profile
                .join(modlight_core::components::TOOLS_FOLDER)
                .join("ExifTool")
                .join("exiftool");
            self.uploads
                .windows
                .messages
                .push_back(crate::dialogs::message::MessageBox::new(
                    t(s::TaskHelpers_ExifToolMissingTitle),
                    &format(
                        t(s::TaskHelpers_ExifToolDoesNotExist),
                        &[Arg::from(path.display().to_string())],
                    ),
                    MessageIcon::Warning,
                ));
        }
        None
    }

    /// The installed ExifTool (QA runs may point `MODLIGHT_QA_EXIFTOOL` at a stand-in).
    pub fn exiftool(&self) -> Option<ExifTool> {
        if self.env.qa
            && let Some(path) = std::env::var_os("MODLIGHT_QA_EXIFTOOL").map(PathBuf::from)
        {
            return Some(ExifTool::direct(path));
        }
        ExifTool::installed(&self.env.profile)
    }

    /// `OpenMetadataWindow(filePath)`.
    pub fn open_metadata(&mut self, file: Option<PathBuf>) {
        let Some(tool) = self.check_exiftool(ToolContinuation::Metadata { file: file.clone() })
        else {
            return;
        };
        let file = file.map(|f| f.to_string_lossy().into_owned());
        self.tools
            .open_keyed(|key| Box::new(MetadataWindow::new(key, file, Some(tool))));
    }

    /// `StripMetadata(filePath, taskSettings)`: a file picked when none was given.
    pub fn strip_metadata(&mut self, file: Option<PathBuf>, settings: TaskSettings) {
        let file = match file {
            Some(file) => file,
            None => match self.tool_open_file(t(tl::MetadataWindow_Open_file_dialog), &[]) {
                Some(file) => file,
                None => return,
            },
        };
        let Some(tool) = self.check_exiftool(ToolContinuation::StripMetadata {
            file: file.clone(),
            settings: Box::new(settings.clone()),
        }) else {
            return;
        };
        self.rt.spawn_blocking(move || {
            let result = tool.strip(&file);
            move |state: &mut AppState, _ctx: &egui::Context| match result {
                Ok(()) => state.play_task_sound(NotificationSound::ActionCompleted, &settings),
                // `e.ShowError()`: a message box with the error.
                Err(error) => state.uploads.windows.messages.push_back(
                    crate::dialogs::message::MessageBox::new(
                        &format!("Modlight - {}", t(s::Error)),
                        &error,
                        MessageIcon::Error,
                    ),
                ),
            }
        });
    }
}
