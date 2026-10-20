//! `TaskHelpers.ExecuteJob` for the tools: which window opens, with which options, and
//! what runs without one. `ToolsSettingsReference` is the task's `ToolsSettings` (the
//! hotkey's own when it overrides them, else the defaults); edits a window makes are
//! written back to the same place.

use std::path::PathBuf;

use modlight_core::hotkeys::HotkeyType;
use modlight_core::tasks::settings::TaskSettings;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::commands::Command;
use crate::state::AppState;

/// Where a task's tool options live in the settings, for writing them back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptionsHome {
    /// `DefaultTaskSettings.ToolsSettings`.
    Defaults,
    /// The `ToolsSettings` of the first hotkey running `job` that overrides them.
    Hotkey(HotkeyType),
}

impl OptionsHome {
    pub fn of(settings: &TaskSettings) -> OptionsHome {
        if settings.use_default_tools_settings {
            OptionsHome::Defaults
        } else {
            OptionsHome::Hotkey(settings.job)
        }
    }
}

/// Read one member of the task's `ToolsSettings`.
pub fn tool_options<T: DeserializeOwned + Default>(settings: &TaskSettings, member: &str) -> T {
    modlight_tools::settings::read(settings.tools_settings.get(member))
}

impl AppState {
    /// Write one member of `ToolsSettings` back where it came from, and save.
    pub fn save_tool_options<T: Serialize>(&mut self, home: &OptionsHome, member: &str, value: &T) {
        match home {
            OptionsHome::Defaults => {
                modlight_tools::settings::write(
                    self.config.values_mut(),
                    &["DefaultTaskSettings", "ToolsSettings", member],
                    value,
                );
            }
            OptionsHome::Hotkey(job) => {
                let mut list = self.captures.hotkeys.hotkeys.clone();
                let Some(hotkey) = list.iter_mut().find(|h| {
                    h.job() == *job
                        && h.task
                            .get("UseDefaultToolsSettings")
                            .and_then(|v| v.as_bool())
                            == Some(false)
                }) else {
                    return;
                };
                if let serde_json::Value::Object(task) = &mut hotkey.task {
                    modlight_tools::settings::write(task, &["ToolsSettings", member], value);
                }
                self.captures.hotkeys.hotkeys = list;
                self.save_hotkeys();
            }
        }
        self.save_config();
    }

    /// Write a member of `DefaultTaskSettings.CaptureSettings` (OCR options, the colour
    /// picker's palette choice).
    pub fn save_capture_options<T: Serialize>(&mut self, path: &[&str], value: &T) {
        let mut full = vec!["DefaultTaskSettings", "CaptureSettings"];
        full.extend_from_slice(path);
        modlight_tools::settings::write(self.config.values_mut(), &full, value);
        self.save_config();
    }

    /// Write an application member (`HistorySettings`, `ImageHistorySettings`).
    pub fn save_app_options<T: Serialize>(&mut self, member: &str, value: &T) {
        modlight_tools::settings::write(self.config.values_mut(), &[member], value);
        self.save_config();
    }
}

/// `ExecuteJob(job, filePath)` for the jobs `capture_flow` leaves to the tools.
pub fn run(state: &mut AppState, job: HotkeyType, settings: TaskSettings, file: Option<PathBuf>) {
    let _ = &file;
    use HotkeyType::*;
    match job {
        ColorPicker => state.open_color_picker(&settings),
        ScreenColorPicker => state.open_screen_color_picker(settings),
        // The image effects window, the tray menu toggle: other parts of the port.
        _ => state.push(Command::NotYet),
    }
}

/// Every frame: background results the tools wait for.
pub fn tick(state: &mut AppState, _ctx: &egui::Context) {
    state.tick_screen_pick();
}
