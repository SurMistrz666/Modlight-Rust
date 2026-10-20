//! QA fixtures for phase 3's settings screens: every task page, the editors the pages
//! open over themselves, the screen recording options window, the application pages and
//! the hotkey settings. Nothing here asks macOS for anything.

use egui::{Ui, Vec2, vec2};
use serde_json::json;

use crate::state::AppState;
use crate::task_editors::{ActionEditor, ButtonsEditor, TaskEditor, WatchFolderEditor};

/// Each settings screen and the section it shows.
pub const PAGES: &[(&str, &str)] = &[
    ("settings-capture-region", "task.capture-region"),
    ("settings-capture", "task.capture"),
    ("settings-image", "task.image"),
    ("settings-image-effects", "task.image-effects"),
    ("settings-thumbnail", "task.image-thumbnail"),
    ("settings-file-naming", "task.upload-file-naming"),
    ("settings-recorder", "task.capture-screen-recorder"),
    ("settings-ocr", "task.capture-ocr"),
    ("settings-clipboard-upload", "task.upload-clipboard"),
    ("settings-notifications", "task.general-notifications"),
    ("settings-tools", "task.tools"),
    ("settings-actions", "task.actions"),
    ("settings-watch-folders", "task.watch-folders"),
    ("settings-task-advanced", "task.advanced"),
    ("settings-action-editor", "task.actions"),
    ("settings-watch-folder-editor", "task.watch-folders"),
    (
        "settings-notification-buttons",
        "task.general-notifications",
    ),
    ("settings-integration", "application.integration"),
    ("settings-paths", "application.paths"),
    ("settings-backup", "application.settings"),
    ("settings-main-window", "application.main-window"),
    ("settings-copy-formats", "application.clipboard-formats"),
    (
        "settings-copy-format-editor",
        "application.clipboard-formats",
    ),
    ("settings-upload-limits", "application.upload"),
    ("settings-history", "application.history"),
    ("settings-print", "application.print"),
    ("settings-proxy", "application.proxy"),
    ("settings-advanced", "application.advanced"),
];

/// The screens this module draws.
pub const SCREENS: &[&str] = &[
    "settings-capture-region",
    "settings-capture",
    "settings-image",
    "settings-image-effects",
    "settings-thumbnail",
    "settings-file-naming",
    "settings-recorder",
    "settings-ocr",
    "settings-clipboard-upload",
    "settings-notifications",
    "settings-tools",
    "settings-actions",
    "settings-watch-folders",
    "settings-task-advanced",
    "settings-action-editor",
    "settings-watch-folder-editor",
    "settings-notification-buttons",
    "settings-integration",
    "settings-paths",
    "settings-backup",
    "settings-main-window",
    "settings-copy-formats",
    "settings-copy-format-editor",
    "settings-upload-limits",
    "settings-history",
    "settings-print",
    "settings-proxy",
    "settings-advanced",
    "settings-hotkeys",
    "settings-hotkeys-capturing",
    "settings-hotkeys-reset",
    "settings-hotkeys-disabled",
    "hotkey-task-settings",
    "hotkey-task-folder",
    "hotkey-task-capture",
    "ffmpeg-options",
    "ffmpeg-options-video",
    "ffmpeg-options-audio",
    "ffmpeg-options-advanced",
];

fn section_of(screen: &str) -> Option<&'static str> {
    PAGES
        .iter()
        .find(|(s, _)| *s == screen)
        .map(|(_, section)| *section)
}

/// Two actions and a watch folder, so the lists have rows.
fn sample_lists(state: &mut AppState) {
    let mut defaults = match state.config.get("DefaultTaskSettings") {
        Some(serde_json::Value::Object(map)) => map.clone(),
        _ => serde_json::Map::new(),
    };
    defaults.insert(
        "ExternalPrograms".into(),
        json!([
            {"IsActive": true, "Name": "Optimize PNG", "Path": "/opt/homebrew/bin/oxipng", "Args": "-o 4 \"$input\"", "OutputExtension": "", "Extensions": "png", "HiddenWindow": true, "DeleteInputFile": false},
            {"IsActive": false, "Name": "Convert to WebP", "Path": "/opt/homebrew/bin/cwebp", "Args": "\"$input\" -o \"$output\"", "OutputExtension": "webp", "Extensions": "png jpg", "HiddenWindow": true, "DeleteInputFile": false}
        ]),
    );
    defaults.insert(
        "WatchFolderList".into(),
        json!([
            {"FolderPath": "~/Desktop/Exports", "Filter": "*.png", "IncludeSubdirectories": false, "MoveFilesToScreenshotsFolder": false}
        ]),
    );
    state
        .config
        .set("DefaultTaskSettings", serde_json::Value::Object(defaults));
}

/// Set the state up for `screen`; the window size it wants.
pub fn prepare(screen: &str, state: &mut AppState) -> Option<Vec2> {
    if let Some(section) = section_of(screen) {
        if matches!(screen, "settings-actions" | "settings-action-editor") {
            sample_lists(state);
        }
        if matches!(
            screen,
            "settings-copy-formats" | "settings-copy-format-editor"
        ) {
            state.config.set(
                "ClipboardContentFormats",
                json!([
                    {"Description": "Markdown link", "Format": "[$filename]($url)"},
                    {"Description": "HTML image", "Format": "<img src=\"$url\" alt=\"$filenamenoext\">"}
                ]),
            );
        }
        let mut window = crate::settings::SettingsWindow::new(section, &state.options.skin);
        window.task_pages.selected_action = (screen == "settings-actions").then_some(0);
        window.task_pages.editor = match screen {
            "settings-action-editor" => {
                let action = state
                    .config
                    .get("DefaultTaskSettings")
                    .and_then(|d| d.get("ExternalPrograms"))
                    .and_then(|l| l.get(1))
                    .cloned();
                Some(TaskEditor::Action(ActionEditor::new(
                    Some(1),
                    action.as_ref(),
                )))
            }
            "settings-watch-folder-editor" => {
                Some(TaskEditor::WatchFolder(WatchFolderEditor::new(None, None)))
            }
            "settings-notification-buttons" => Some(TaskEditor::Buttons(ButtonsEditor::new(None))),
            _ => None,
        };
        if screen == "settings-copy-formats" {
            window.app_pages.selected_format = Some(0);
        }
        if screen == "settings-copy-format-editor" {
            let format = state
                .config
                .get("ClipboardContentFormats")
                .and_then(|l| l.get(0))
                .cloned();
            window.app_pages.editor = Some(crate::app_pages::FormatEditor::new(
                Some(0),
                format.as_ref(),
            ));
        }
        state.windows.settings = Some(window);
        return Some(vec2(crate::settings::WIDTH, crate::settings::HEIGHT));
    }
    if screen.starts_with("settings-hotkeys") {
        state.captures.hotkeys = crate::qa_capture::registered_hotkeys(true);
        if screen == "settings-hotkeys-disabled" {
            state.captures.hotkeys.toggle(true);
        }
        // One hotkey with its own settings, for the pencil.
        if let Some(hotkey) = state.captures.hotkeys.hotkeys.get_mut(1) {
            hotkey.task["UseDefaultAfterCaptureJob"] = json!(false);
        }
        let mut window = crate::settings::SettingsWindow::new("hotkeys.list", &state.options.skin);
        window.hotkeys.selected = Some(1);
        if screen == "settings-hotkeys-capturing" {
            window.hotkeys.capturing = Some(2);
            if let Some(hotkey) = state.captures.hotkeys.hotkeys.get_mut(2) {
                hotkey.info = modlight_core::hotkeys::HotkeyInfo::default();
            }
        }
        window.hotkeys.confirm_reset = screen == "settings-hotkeys-reset";
        state.windows.settings = Some(window);
        return Some(vec2(crate::settings::WIDTH, crate::settings::HEIGHT));
    }
    if screen.starts_with("hotkey-task") {
        state.captures.hotkeys = crate::qa_capture::registered_hotkeys(false);
        let index = 0;
        if let Some(hotkey) = state.captures.hotkeys.hotkeys.get_mut(index) {
            hotkey.task["UseDefaultAfterCaptureJob"] = json!(false);
            hotkey.task["AfterCaptureJob"] =
                json!("CopyImageToClipboard, SaveImageToFile, UploadImageToHost");
            hotkey.task["UseDefaultUploadSettings"] = json!(false);
            hotkey.task["Description"] = json!("");
        }
        state.open_hotkey_task_settings(index);
        let mut window = state.windows.hotkey_task.take().expect("hotkey window");
        window.section = match screen {
            "hotkey-task-folder" => "task.upload-modlight",
            "hotkey-task-capture" => "task.capture",
            _ => "task.task",
        }
        .to_string();
        window.reveal(&window.section.clone());
        if screen == "hotkey-task-folder" {
            window.task_pages.folders = crate::task_pages::ModlightFolders {
                requested: true,
                loading: false,
                status: String::new(),
                root: Some("root".into()),
                folders: Some(vec![
                    ("f1".into(), "Design".into(), "Design".into()),
                    ("f2".into(), "Design / Mockups".into(), "Mockups".into()),
                    ("f3".into(), "Screenshots".into(), "Screenshots".into()),
                ]),
            };
        }
        state.windows.settings = Some(window);
        return Some(vec2(crate::settings::WIDTH, crate::settings::HEIGHT));
    }
    if screen.starts_with("ffmpeg-options") {
        state.open_ffmpeg_options(crate::task_pages::TaskTarget::Defaults);
        if let Some(window) = state.captures.ffmpeg_window.as_mut() {
            window.tab = match screen {
                "ffmpeg-options-video" => 1,
                "ffmpeg-options-audio" => 2,
                "ffmpeg-options-advanced" => 3,
                _ => 0,
            };
        }
        return Some(vec2(
            crate::ffmpeg_window::WIDTH,
            crate::ffmpeg_window::HEIGHT,
        ));
    }
    None
}

pub fn is_dialog(screen: &str) -> bool {
    SCREENS.contains(&screen)
}

/// Draw a screen of this module; `Some(height)` when it asks for its own height.
pub fn draw(screen: &str, ui: &mut Ui, state: &mut AppState) -> Option<Option<f32>> {
    if !is_dialog(screen) {
        return None;
    }
    if screen.starts_with("ffmpeg-options") {
        let colors = state.colors.clone();
        if let Some(window) = state.captures.ffmpeg_window.as_mut() {
            let _ = crate::ffmpeg_window::ui(ui, window, &colors);
        }
        return Some(None);
    }
    let asks = crate::settings::ui(ui, state);
    let _ = asks;
    Some(None)
}
