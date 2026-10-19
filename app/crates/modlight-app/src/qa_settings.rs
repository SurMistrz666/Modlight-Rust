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
