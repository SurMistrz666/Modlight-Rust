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

/// A tool waiting for a component (ffmpeg, ExifTool, the model) to be installed.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolContinuation {
    VideoConverter {
        file: Option<PathBuf>,
        settings: Box<TaskSettings>,
    },
    Metadata {
        file: Option<PathBuf>,
    },
    StripMetadata {
        file: PathBuf,
        settings: Box<TaskSettings>,
    },
}

/// The components window closed: the tool opens if what it needs is there now.
pub fn after_components(state: &mut AppState, next: ToolContinuation, satisfied: bool) {
    if !satisfied {
        return;
    }
    match next {
        ToolContinuation::VideoConverter { file, settings } => {
            state.open_video_converter(file, *settings)
        }
        ToolContinuation::Metadata { file } => state.open_metadata(file),
        ToolContinuation::StripMetadata { file, settings } => {
            state.strip_metadata(Some(file), *settings)
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
    use HotkeyType as H;
    match job {
        H::ColorPicker => state.open_color_picker(&settings),
        H::ScreenColorPicker => state.open_screen_color_picker(settings),
        H::ImageViewer => match file {
            Some(file) => {
                if file.is_file() {
                    state.open_image_viewer_file(&file);
                }
            }
            None => state.open_image_viewer(),
        },
        H::QRCode => match file {
            Some(file) => state.open_qr_code_scan(Some(file), None),
            None => state.open_qr_code(None),
        },
        H::QRCodeDecodeFromScreen => {
            state.open_qr_code_scan(None, Some(super::qr::ScanMode::Screen))
        }
        H::QRCodeScanRegion => state.open_qr_code_scan(None, Some(super::qr::ScanMode::Region)),
        H::PinToScreen => match file {
            Some(file) => state.pin_file(&file, &settings),
            None => state.open_pin_startup(settings),
        },
        H::PinToScreenFromScreen => state.pin_from_screen(settings),
        H::PinToScreenFromClipboard => state.pin_from_clipboard(settings),
        H::PinToScreenFromFile => match file {
            Some(file) => state.pin_file(&file, &settings),
            None => state.pin_from_file(settings),
        },
        H::PinToScreenCloseAll => state.pin_close_all(settings),
        H::ImageCombiner => state.open_image_combiner(Vec::new(), settings),
        H::Metadata => state.open_metadata(file),
        H::StripMetadata => state.strip_metadata(file, settings),
        H::VideoConverter => match file {
            Some(file) => state.open_video_converter(Some(file), settings),
            None => state.open_video_converter(None, settings),
        },
        // The image effects window, the tray menu toggle: other parts of the port.
        _ => state.push(Command::NotYet),
    }
}

/// Every frame: background results the tools wait for.
pub fn tick(state: &mut AppState, _ctx: &egui::Context) {
    static PANIC_HOOK: std::sync::Once = std::sync::Once::new();
    if state.mode == crate::app::Mode::Normal {
        PANIC_HOOK.call_once(super::boxes::install_panic_hook);
        state.tick_panics();
    }
    state.tick_screen_pick();
}
