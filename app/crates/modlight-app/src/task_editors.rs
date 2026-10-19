//! The editors the task settings pages open over themselves (`TaskSettingsView`'s
//! overlays): an action (`ActionEditorOverlay`), a watch folder
//! (`WatchFolderEditorOverlay`) and the notification buttons
//! (`NotificationButtonsEditorOverlay`). Each sits on the modal veil over the page, and
//! Escape closes the top one without saving.

use egui::{Align, Layout, Order, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use serde_json::{Value, json};

use modlight_core::icons;
use modlight_core::l10n::{helpers, s, t};
use modlight_core::tasks::actions::ExternalProgram;
use modlight_core::tasks::settings::{NotificationActionButton, ToastClickAction};

use crate::commands::Command;
use crate::menus::Entry;
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Field, Look, button, checkbox_weighted, layout, paint_icon, trimmed, wrapped,
};

/// The editor open over the pages.
#[derive(Clone, Debug)]
pub enum TaskEditor {
    Action(ActionEditor),
    WatchFolder(WatchFolderEditor),
    Buttons(ButtonsEditor),
}

/// What closing an editor gave.
#[derive(Clone, Debug, PartialEq)]
pub enum EditorResult {
    /// Still open.
    Open,
    /// Cancelled (Cancel, the close button, Escape).
    Cancelled,
    /// The action to store at `index` (`None`: add it).
    Action(Option<usize>, Value),
    /// The watch folder to store at `index` (`None`: add it).
    WatchFolder(Option<usize>, Value),
    /// The new `ToastWindowButtons`.
    Buttons(Vec<Value>),
}

/// `ShowActionEditor`'s fields.
#[derive(Clone, Debug)]
pub struct ActionEditor {
    pub index: Option<usize>,
    original: Value,
    pub name: String,
    pub path: String,
    pub args: String,
    pub output_extension: String,
    pub extensions: String,
    pub hidden_window: bool,
    pub delete_input_file: bool,
    pub error: Option<String>,
    /// The field to focus (and select) next frame.
    focus: Option<&'static str>,
}

impl ActionEditor {
    /// `ShowActionEditor(action)`: `None` adds one, with `ExternalProgram`'s defaults.
    pub fn new(index: Option<usize>, action: Option<&Value>) -> ActionEditor {
        let defaults = serde_json::to_value(ExternalProgram::default()).unwrap_or(Value::Null);
        let values = action.filter(|v| v.is_object()).unwrap_or(&defaults);
        let text = |key: &str| {
            values
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let flag = |key: &str| values.get(key).and_then(Value::as_bool).unwrap_or(false);
        ActionEditor {
            index: if action.is_some() { index } else { None },
            original: action.cloned().unwrap_or(Value::Null),
            name: text("Name"),
            path: text("Path"),
            args: text("Args"),
            output_extension: text("OutputExtension"),
            extensions: text("Extensions"),
            hidden_window: flag("HiddenWindow"),
            delete_input_file: flag("DeleteInputFile"),
            error: None,
            focus: Some("name"),
        }
    }

    /// `OnSaveActionEditorClick`: the name and path are required.
    pub fn save(&mut self) -> Option<Value> {
        if self.name.trim().is_empty() {
            self.error = Some(t(s::ActionsForm_btnOK_Click_Name_can_t_be_empty_).to_string());
            self.focus = Some("name");
            return None;
        }
        if self.path.trim().is_empty() {
            self.error = Some(t(s::ActionsForm_btnOK_Click_File_path_can_t_be_empty_).to_string());
            self.focus = Some("path");
            return None;
        }
        let mut action = match &self.original {
            Value::Object(_) => self.original.clone(),
            _ => serde_json::to_value(ExternalProgram {
                is_active: true,
                ..ExternalProgram::default()
            })
            .unwrap_or_else(|_| json!({ "IsActive": true })),
        };
        action["Name"] = json!(self.name);
        action["Path"] = json!(self.path);
        action["Args"] = json!(self.args);
        action["OutputExtension"] = json!(self.output_extension);
        action["Extensions"] = json!(self.extensions);
        action["HiddenWindow"] = json!(self.hidden_window);
        action["DeleteInputFile"] = json!(self.delete_input_file);
        Some(action)
    }
}

/// `ShowWatchFolderEditor`'s fields.
#[derive(Clone, Debug)]
pub struct WatchFolderEditor {
    pub index: Option<usize>,
    original: Value,
    pub folder_path: String,
    pub filter: String,
    pub include_subdirectories: bool,
    pub move_files: bool,
    focus: bool,
}

impl WatchFolderEditor {
    pub fn new(index: Option<usize>, folder: Option<&Value>) -> WatchFolderEditor {
        let text = |key: &str| {
            folder
                .and_then(|f| f.get(key))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let flag = |key: &str| {
            folder
                .and_then(|f| f.get(key))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        WatchFolderEditor {
            index: if folder.is_some() { index } else { None },
            original: folder.cloned().unwrap_or(Value::Null),
            folder_path: text("FolderPath"),
            filter: text("Filter"),
            include_subdirectories: flag("IncludeSubdirectories"),
            move_files: flag("MoveFilesToScreenshotsFolder"),
            focus: true,
        }
    }

    pub fn save(&self) -> Value {
        let mut folder = match &self.original {
            Value::Object(_) => self.original.clone(),
            _ => serde_json::to_value(modlight_core::watch::WatchFolderSettings::default())
                .unwrap_or_else(|_| json!({})),
        };
        folder["FolderPath"] = json!(self.folder_path);
        folder["Filter"] = json!(self.filter);
        folder["IncludeSubdirectories"] = json!(self.include_subdirectories);
        folder["MoveFilesToScreenshotsFolder"] = json!(self.move_files);
        folder
    }
}

/// `ShowNotificationButtonsEditor`'s list.
#[derive(Clone, Debug)]
pub struct ButtonsEditor {
    pub items: Vec<Value>,
    pub selected: Option<usize>,
    add_open: bool,
}

impl ButtonsEditor {
    /// The buttons that name a known action, each action once (`addedActions`).
    pub fn new(buttons: Option<&Value>) -> ButtonsEditor {
        let list: Vec<Value> = match buttons {
            Some(Value::Array(list)) => list.clone(),
            _ => NotificationActionButton::defaults()
                .into_iter()
                .filter_map(|b| serde_json::to_value(b).ok())
                .collect(),
        };
        let mut seen = Vec::new();
        let mut items = Vec::new();
        for button in list {
            let Some(action) = button
                .get("Action")
                .and_then(Value::as_str)
                .and_then(ToastClickAction::parse)
            else {
                continue;
            };
            if seen.contains(&action) {
                continue;
            }
            seen.push(action);
            items.push(button);
        }
        let selected = (!items.is_empty()).then_some(0);
        ButtonsEditor {
            items,
            selected,
            add_open: false,
        }
    }

    fn action(item: &Value) -> ToastClickAction {
        item.get("Action")
            .and_then(Value::as_str)
            .and_then(ToastClickAction::parse)
            .unwrap_or(ToastClickAction::CloseNotification)
    }

    /// The actions not on the list yet, in the enum's order.
    pub fn missing(&self) -> Vec<ToastClickAction> {
        ToastClickAction::ALL
            .iter()
            .copied()
            .filter(|a| !self.items.iter().any(|i| ButtonsEditor::action(i) == *a))
            .collect()
    }

    pub fn add(&mut self, action: ToastClickAction) {
        if let Ok(value) = serde_json::to_value(NotificationActionButton::new(action)) {
            self.items.push(value);
            self.selected = Some(self.items.len() - 1);
        }
    }

    pub fn remove(&mut self) {
        let Some(index) = self.selected.filter(|i| *i < self.items.len()) else {
            return;
        };
        self.items.remove(index);
        self.selected = if self.items.is_empty() {
            None
        } else {
            Some(index.min(self.items.len() - 1))
        };
    }

    pub fn move_by(&mut self, offset: isize) {
        let Some(index) = self.selected.filter(|i| *i < self.items.len()) else {
            return;
        };
        let target = index as isize + offset;
        if target < 0 || target as usize >= self.items.len() {
            return;
        }
        let item = self.items.remove(index);
        self.items.insert(target as usize, item);
        self.selected = Some(target as usize);
    }
}

/// `GetLocalizedDescription` of a `ToastClickAction`.
pub fn toast_action_text(action: ToastClickAction) -> String {
    modlight_core::l10n::enum_text("ToastClickAction", action.name(), None)
}

/// The modal veil over `area` and the editor's panel; returns what closing it gave.
/// `dialogs` is false in automated runs, where Browse opens nothing.
pub fn show(
    ui: &mut Ui,
    colors: &Colors,
    area: Rect,
    editor: &mut TaskEditor,
    dialogs: bool,
) -> EditorResult {
    let ctx = ui.ctx().clone();
    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let mut result = EditorResult::Open;
    egui::Area::new(egui::Id::new("task-editor-veil"))
        .order(Order::Foreground)
        .fixed_pos(area.min)
        .show(&ctx, |ui| {
            let (veil, _) = ui.allocate_exact_size(area.size(), Sense::click_and_drag());
            ui.painter().rect_filled(veil, 0.0, colors.overlay_modal());
            result = match editor {
                TaskEditor::Action(editor) => action_ui(ui, colors, veil, editor, dialogs),
                TaskEditor::WatchFolder(editor) => {
                    watch_folder_ui(ui, colors, veil, editor, dialogs)
                }
                TaskEditor::Buttons(editor) => buttons_ui(ui, colors, veil, editor),
            };
        });
    if escape && result == EditorResult::Open {
        let menu_open = matches!(editor, TaskEditor::Buttons(b) if b.add_open);
        if let TaskEditor::Buttons(b) = editor {
            b.add_open = false;
        }
        if !menu_open {
            result = EditorResult::Cancelled;
        }
    }
    result
}

/// The panel: background, border, padding 16; returns the inner rectangle.
fn panel(ui: &Ui, colors: &Colors, veil: Rect, width: f32, height: f32) -> Rect {
    let width = width.min(veil.width() - 24.0);
    let height = height.min(veil.height() - 24.0);
    let rect = Rect::from_center_size(veil.center(), vec2(width, height));
    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, colors.background_panel());
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    rect.shrink(16.0)
}

/// The header: accent icon (21), title (19, SemiBold) over the hint; returns its height.
fn header(
    ui: &Ui,
    colors: &Colors,
    inner: Rect,
    glyph: char,
    title: &str,
    hint: &str,
    right: f32,
) -> f32 {
    let painter = ui.painter();
    let text_left = inner.left() + 21.0 + 9.0;
    let width = inner.right() - right - text_left;
    let title = layout(
        ui,
        trimmed(
            title,
            19.0,
            Weight::SemiBold,
            colors.text_on_panel(),
            width,
            1,
        ),
    );
    let hint = layout(
        ui,
        wrapped(
            hint,
            12.0,
            Weight::Normal,
            colors.text_secondary_on_panel(),
            width,
        ),
    );
    let height = title.size().y + 1.0 + 4.0 + hint.size().y + 4.0;
    paint_icon(
        ui,
        painter,
        pos2(inner.left() + 10.5, inner.top() + height / 2.0),
        glyph,
        21.0,
        colors.accent_start(),
    );
    painter.galley(
        pos2(text_left, inner.top()),
        title.clone(),
        colors.text_on_panel(),
    );
    painter.galley(
        pos2(text_left, inner.top() + title.size().y + 1.0 + 4.0),
        hint,
        colors.text_secondary_on_panel(),
    );
    height
}

/// Save and Cancel, right-aligned (MinWidth 84, 8 apart); returns (save, cancel).
fn footer(ui: &mut Ui, colors: &Colors, inner: Rect, height: f32) -> (bool, bool) {
    let row = Rect::from_min_max(pos2(inner.left(), inner.bottom() - height), inner.max);
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(row)
            .layout(Layout::right_to_left(Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 8.0;
    let look = Look {
        string_text: Some(colors.text()),
        min_width: 84.0,
        ..Look::button(colors).height(height).padding(10.0)
    };
    let cancel = button(
        &mut child,
        colors,
        &look,
        Content::label(t(s::TaskSettingsWindow_Cancel)),
        true,
    )
    .clicked();
    let save = button(
        &mut child,
        colors,
        &look,
        Content::label(t(s::TaskSettingsWindow_Save)),
        true,
    )
    .clicked();
    (save, cancel)
}

/// `TextBlock.action-editor-label` over its field (3 apart).
fn label(ui: &mut Ui, colors: &Colors, text: &str) {
    let galley = layout(
        ui,
        wrapped(
            text,
            13.0,
            Weight::Normal,
            colors.text_secondary_on_panel(),
            ui.available_width(),
        ),
    );
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter()
        .galley(rect.min, galley, colors.text_secondary_on_panel());
    ui.add_space(3.0);
}

fn hint(ui: &mut Ui, colors: &Colors, text: &str) {
    ui.add_space(4.0);
    let galley = layout(
        ui,
        wrapped(
            text,
            12.0,
            Weight::Normal,
            colors.text_secondary_on_panel(),
            ui.available_width(),
        ),
    );
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter()
        .galley(rect.min, galley, colors.text_secondary_on_panel());
    ui.add_space(4.0);
}

/// A 32-tall text box; `focus` asks for focus with everything selected.
fn field(
    ui: &mut Ui,
    colors: &Colors,
    id: egui::Id,
    text: &mut String,
    width: f32,
    focus: bool,
) -> egui::Response {
    let response = Field {
        height: 32.0,
        width: Some(width),
        id: Some(id),
        ..Field::new(text)
    }
    .show(ui, colors);
    if focus {
        response.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            let all = egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(text.chars().count()),
            );
            state.cursor.set_char_range(Some(all));
            state.store(ui.ctx(), id);
        }
    }
    response
}

/// The 32x32 folder button next to a path box.
fn browse_button(ui: &mut Ui, colors: &Colors) -> bool {
    let look = Look::button(colors)
        .height(32.0)
        .width(32.0)
        .padding(0.0)
        .icon(15.0, None);
    button(ui, colors, &look, Content::icon(icons::FOLDER), true)
        .on_hover_text(t(s::TaskSettingsWindow_BrowseWithEllipsis))
        .clicked()
}

/// Put `token` where the caret of the text box `id` is (`InsertArgumentToken`).
pub fn insert_at_caret(ctx: &egui::Context, id: egui::Id, text: &mut String, token: &str) {
    let chars = text.chars().count();
    let (start, end) = egui::TextEdit::load_state(ctx, id)
        .and_then(|s| s.cursor.char_range())
        .map(|r| {
            let (a, b) = (r.primary.index.0.min(chars), r.secondary.index.0.min(chars));
            (a.min(b), a.max(b))
        })
        .unwrap_or((chars, chars));
    let byte = |n: usize| {
        text.char_indices()
            .nth(n)
            .map(|(b, _)| b)
            .unwrap_or(text.len())
    };
    let (from, to) = (byte(start), byte(end));
    text.replace_range(from..to, token);
    let caret = start + token.chars().count();
    if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(caret),
            )));
        state.store(ctx, id);
    }
    ctx.memory_mut(|m| m.request_focus(id));
}

fn action_ui(
    ui: &mut Ui,
    colors: &Colors,
    veil: Rect,
    editor: &mut ActionEditor,
    dialogs: bool,
) -> EditorResult {
    let inner = panel(ui, colors, veil, 640.0, 590.0);
    let title = if editor.index.is_none() {
        t(s::TaskSettingsWindow_AddAction)
    } else {
        t(s::TaskSettingsWindow_EditAction)
    };
    let head = header(
        ui,
        colors,
        inner,
        icons::ZAP,
        title,
        t(s::TaskSettingsWindow_RunAnExternalProgramAsPartOfACaptureWorkflow),
        40.0,
    );
    // The close button (`icon-button`, x).
    let close_rect = Rect::from_min_size(
        pos2(inner.right() - 32.0, inner.top() + (head - 32.0) / 2.0),
        vec2(32.0, 32.0),
    );
    let close_look = Look::icon_button(colors, 32.0, 8.0, colors.control_background_hover())
        .icon(16.0, Some(colors.text_on_panel()));
    let close = crate::widgets::button_at(
        ui,
        colors,
        &close_look,
        Content::icon(icons::X),
        true,
        close_rect,
    )
    .on_hover_text(t(s::TaskSettingsWindow_Close))
    .clicked();
    let body = Rect::from_min_max(
        pos2(inner.left(), inner.top() + head + 12.0),
        pos2(inner.right(), inner.bottom() - 32.0 - 12.0),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(body)
            .layout(Layout::top_down(Align::Min)),
    );
    let focus = editor.focus.take();
    egui::ScrollArea::vertical()
        .id_salt("action-editor")
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
            let width = ui.available_width() - 4.0;
            label(ui, colors, t(s::TaskSettingsWindow_Name));
            field(ui, colors, egui::Id::new("action-name"), &mut editor.name, width, focus == Some("name"));
            ui.add_space(8.0);
            label(ui, colors, t(s::TaskSettingsWindow_FilePath));
            ui.horizontal(|ui| {
                field(ui, colors, egui::Id::new("action-path"), &mut editor.path, width - 32.0 - 6.0, focus == Some("path"));
                if browse_button(ui, colors)
                    && dialogs
                    && let Some(path) = rfd::FileDialog::new()
                        .set_title(t(s::TaskSettingsWindow_ChooseProgram))
                        .pick_file()
                {
                    editor.path = path.display().to_string();
                    editor.error = None;
                }
            });
            ui.add_space(8.0);
            label(ui, colors, t(s::TaskSettingsWindow_Arguments));
            let args_id = egui::Id::new("action-args");
            let response = field(ui, colors, args_id, &mut editor.args, width, false);
            let tokens = [
                ("$input", t(helpers::ActionsCodeMenuEntry_FilePath_File_path)),
                (
                    "$output",
                    t(helpers::ActionsCodeMenuEntry_OutputFilePath_File_path_without_extension____Output_file_name_extension_),
                ),
            ];
            if let Some(Command::Local(index)) = crate::menus::context(colors, &response, || {
                tokens
                    .iter()
                    .enumerate()
                    .map(|(i, (token, text))| Entry::plain(format!("{token} \u{2014} {text}"), Command::Local(i)))
                    .collect()
            }) && let Some((token, _)) = tokens.get(index)
            {
                insert_at_caret(ui.ctx(), args_id, &mut editor.args, token);
            }
            hint(ui, colors, t(s::TaskSettingsWindow_RightClickToInsertAnInputOrOutputFileToken));
            ui.add_space(8.0);
            label(ui, colors, t(s::TaskSettingsWindow_OutputFileNameExtensionEmptyUsesTheSameFileName));
            field(ui, colors, egui::Id::new("action-output"), &mut editor.output_extension, width, false);
            ui.add_space(8.0);
            label(ui, colors, t(s::TaskSettingsWindow_ExtensionFilterExampleJpgPngMp4));
            field(ui, colors, egui::Id::new("action-extensions"), &mut editor.extensions, width, false);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 18.0;
                checkbox_weighted(
                    ui,
                    colors,
                    &mut editor.hidden_window,
                    Some(t(s::TaskSettingsWindow_HiddenWindow)),
                    true,
                    Weight::Normal,
                );
                // `UpdateDeleteInputFileState`: only with an output extension.
                let can_delete = !editor.output_extension.is_empty();
                checkbox_weighted(
                    ui,
                    colors,
                    &mut editor.delete_input_file,
                    Some(t(s::TaskSettingsWindow_DeleteInputFile)),
                    can_delete,
                    Weight::Normal,
                );
            });
            if let Some(error) = &editor.error {
                ui.add_space(8.0);
                let galley = layout(ui, wrapped(error, 13.0, Weight::Normal, colors.cancel_border(), width));
                let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
                ui.painter().galley(rect.min, galley, colors.cancel_border());
            }
        });
    let (save, cancel) = footer(ui, colors, inner, 32.0);
    if close || cancel {
        return EditorResult::Cancelled;
    }
    if save && let Some(action) = editor.save() {
        return EditorResult::Action(editor.index, action);
    }
    EditorResult::Open
}

fn watch_folder_ui(
    ui: &mut Ui,
    colors: &Colors,
    veil: Rect,
    editor: &mut WatchFolderEditor,
    dialogs: bool,
) -> EditorResult {
    let inner = panel(ui, colors, veil, 600.0, 480.0);
    let title = if editor.index.is_none() {
        t(s::TaskSettingsWindow_AddWatchFolder)
    } else {
        t(s::TaskSettingsWindow_EditWatchFolder)
    };
    let head = header(
        ui,
        colors,
        inner,
        icons::FOLDER_SEARCH,
        title,
        t(s::TaskSettingsWindow_UploadFilesWhenTheyAreCreatedInThisFolder),
        0.0,
    );
    let body = Rect::from_min_max(
        pos2(inner.left(), inner.top() + head + 12.0),
        pos2(inner.right(), inner.bottom() - 32.0 - 12.0),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(body)
            .layout(Layout::top_down(Align::Min)),
    );
    let focus = std::mem::take(&mut editor.focus);
    egui::ScrollArea::vertical()
        .id_salt("watch-folder-editor")
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
            let width = ui.available_width() - 4.0;
            label(ui, colors, t(s::TaskSettingsWindow_FolderPath));
            ui.horizontal(|ui| {
                field(
                    ui,
                    colors,
                    egui::Id::new("watch-path"),
                    &mut editor.folder_path,
                    width - 32.0 - 6.0,
                    focus,
                );
                if browse_button(ui, colors)
                    && dialogs
                    && let Some(path) = rfd::FileDialog::new()
                        .set_title(t(s::TaskSettingsWindow_ChooseWatchFolder))
                        .pick_folder()
                {
                    editor.folder_path = path.display().to_string();
                }
            });
            ui.add_space(8.0);
            label(ui, colors, t(s::TaskSettingsWindow_Filter));
            field(
                ui,
                colors,
                egui::Id::new("watch-filter"),
                &mut editor.filter,
                width,
                false,
            );
            hint(ui, colors, t(s::TaskSettingsWindow_ExamplePng));
            ui.add_space(8.0);
            checkbox_weighted(
                ui,
                colors,
                &mut editor.include_subdirectories,
                Some(t(s::TaskSettingsWindow_IncludeSubdirectories)),
                true,
                Weight::Normal,
            );
            ui.add_space(3.0);
            checkbox_weighted(
                ui,
                colors,
                &mut editor.move_files,
                Some(t(s::TaskSettingsWindow_MoveFileToScreenshotsFolder)),
                true,
                Weight::Normal,
            );
        });
    let (save, cancel) = footer(ui, colors, inner, 32.0);
    if cancel {
        return EditorResult::Cancelled;
    }
    if save {
        return EditorResult::WatchFolder(editor.index, editor.save());
    }
    EditorResult::Open
}

fn buttons_ui(
    ui: &mut Ui,
    colors: &Colors,
    veil: Rect,
    editor: &mut ButtonsEditor,
) -> EditorResult {
    let full = {
        let width = 600.0_f32.min(veil.width() - 24.0);
        let height = 520.0_f32.min(veil.height() - 24.0);
        Rect::from_center_size(veil.center(), vec2(width, height))
    };
    let inner = panel(ui, colors, veil, full.width(), full.height());
    let head = header(
        ui,
        colors,
        inner,
        icons::BELL,
        t(s::TaskSettingsWindow_NotificationButtons),
        t(s::TaskSettingsWindow_ChooseTheActionsShownAtTheBottomOfAfterCaptureNotifications),
        0.0,
    );
    // The toolbar: Add..., Remove, up, down.
    let bar = Rect::from_min_size(
        pos2(inner.left(), inner.top() + head + 12.0),
        vec2(inner.width(), 32.0),
    );
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(bar)
            .layout(Layout::left_to_right(Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 6.0;
    let index = editor.selected.filter(|i| *i < editor.items.len());
    let missing = editor.missing();
    let compact = Look::button(colors)
        .height(32.0)
        .padding(10.0)
        .icon(14.0, None)
        .spacing(7.0);
    let add = button(
        &mut row,
        colors,
        &compact,
        Content::both(icons::PLUS, t(s::TaskSettingsWindow_AddWithEllipsis)),
        !missing.is_empty(),
    );
    if add.clicked() {
        editor.add_open = !editor.add_open;
    }
    let remove = button(
        &mut row,
        colors,
        &compact,
        Content::both(icons::TRASH_2, t(s::TaskSettingsWindow_Remove)),
        index.is_some(),
    );
    if remove.clicked() {
        editor.remove();
    }
    let square = Look::button(colors)
        .height(32.0)
        .width(32.0)
        .padding(0.0)
        .icon(16.0, None);
    let up = button(
        &mut row,
        colors,
        &square,
        Content::icon(icons::CHEVRON_UP),
        index.is_some_and(|i| i > 0),
    )
    .on_hover_text(t(s::TaskSettingsWindow_MoveUp));
    if up.clicked() {
        editor.move_by(-1);
    }
    let down = button(
        &mut row,
        colors,
        &square,
        Content::icon(icons::CHEVRON_DOWN),
        index.is_some_and(|i| i + 1 < editor.items.len()),
    )
    .on_hover_text(t(s::TaskSettingsWindow_MoveDown));
    if down.clicked() {
        editor.move_by(1);
    }
    // The add menu (`PlacementMode.BottomEdgeAlignedLeft`), the missing actions only.
    let mut open = editor.add_open;
    if let Some(Command::Local(pick)) =
        crate::menus::popup_open_left(colors, &add, &mut open, || {
            missing
                .iter()
                .enumerate()
                .map(|(i, action)| {
                    Entry::item(
                        toast_action_text(*action),
                        crate::notification::ToastAction::parse(action.name())
                            .map(|a| a.icon())
                            .unwrap_or(icons::BELL),
                        Command::Local(i),
                    )
                })
                .collect()
        })
        && let Some(action) = missing.get(pick)
    {
        editor.add(*action);
        open = false;
    }
    editor.add_open = open;
    // The list.
    let list = Rect::from_min_max(
        pos2(inner.left(), bar.bottom() + 12.0),
        pos2(inner.right(), inner.bottom() - 36.0 - 12.0),
    );
    ui.painter()
        .rect_filled(list, 0.0, colors.background_main());
    ui.painter().rect_stroke(
        list,
        0.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    if editor.items.is_empty() {
        let galley = layout(
            ui,
            wrapped(
                t(s::TaskSettingsWindow_NoButtonsConfiguredTheNotificationToolbarWillBeHidden),
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                list.width() - 24.0,
            ),
        );
        ui.painter().galley(
            list.center() - galley.size() / 2.0,
            galley,
            colors.text_secondary(),
        );
    } else {
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(list.shrink(1.0))
                .layout(Layout::top_down(Align::Min)),
        );
        egui::ScrollArea::vertical()
            .id_salt("notification-buttons")
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut clicked = None;
                for (i, item) in editor.items.iter().enumerate() {
                    let action = ButtonsEditor::action(item);
                    let response = crate::settings_kit::list_row(
                        ui,
                        colors,
                        editor.selected == Some(i),
                        28.0,
                        |ui, r| {
                            let glyph = crate::notification::ToastAction::parse(action.name())
                                .map(|a| a.icon())
                                .unwrap_or(icons::BELL);
                            paint_icon(
                                ui,
                                ui.painter(),
                                pos2(r.left() + 8.0 + 13.0, r.center().y),
                                glyph,
                                16.0,
                                colors.accent_start(),
                            );
                            crate::controls::label_at(
                                ui,
                                Rect::from_min_max(
                                    pos2(r.left() + 8.0 + 26.0 + 7.0, r.top()),
                                    r.max,
                                ),
                                &toast_action_text(action),
                                Weight::Normal,
                                colors.text(),
                            );
                        },
                    );
                    if response.clicked() {
                        clicked = Some(i);
                    }
                }
                if clicked.is_some() {
                    editor.selected = clicked;
                }
            });
    }
    let (save, cancel) = footer(ui, colors, inner, 36.0);
    if cancel {
        return EditorResult::Cancelled;
    }
    if save {
        return EditorResult::Buttons(editor.items.clone());
    }
    EditorResult::Open
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_action_is_active_and_needs_a_name_and_a_path() {
        let mut editor = ActionEditor::new(None, None);
        assert_eq!(editor.args, "\"$input\"");
        assert!(editor.save().is_none());
        assert!(editor.error.is_some());
        editor.name = "Optimize".into();
        assert!(editor.save().is_none());
        editor.path = "/usr/bin/true".into();
        let saved = editor.save().unwrap();
        assert_eq!(saved["IsActive"], json!(true));
        assert_eq!(saved["Name"], json!("Optimize"));
        assert_eq!(saved["Args"], json!("\"$input\""));
    }

    #[test]
    fn an_edited_action_keeps_what_the_editor_does_not_show() {
        let original =
            json!({"IsActive": false, "Name": "A", "Path": "/a", "Args": "", "Unknown": 1});
        let mut editor = ActionEditor::new(Some(2), Some(&original));
        editor.name = "B".into();
        let saved = editor.save().unwrap();
        assert_eq!(saved["IsActive"], json!(false));
        assert_eq!(saved["Unknown"], json!(1));
        assert_eq!(saved["Name"], json!("B"));
        assert_eq!(editor.index, Some(2));
    }

    #[test]
    fn the_buttons_editor_drops_unknown_and_repeated_actions() {
        let list = json!([
            {"Action": "CopyUrl"},
            {"Action": "Nope"},
            {"Action": "CopyUrl"},
            {"Action": "OpenFolder"}
        ]);
        let mut editor = ButtonsEditor::new(Some(&list));
        assert_eq!(editor.items.len(), 2);
        assert_eq!(editor.selected, Some(0));
        assert!(!editor.missing().contains(&ToastClickAction::CopyUrl));
        editor.move_by(1);
        assert_eq!(editor.selected, Some(1));
        assert_eq!(editor.items[1]["Action"], json!("CopyUrl"));
        editor.remove();
        assert_eq!(editor.selected, Some(0));
        editor.add(ToastClickAction::Upload);
        assert_eq!(editor.items.last().unwrap()["Action"], json!("Upload"));
    }

    #[test]
    fn a_token_goes_where_the_caret_is() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("t");
        let mut text = String::from("ab");
        insert_at_caret(&ctx, id, &mut text, "$input");
        assert_eq!(text, "ab$input");
    }
}
