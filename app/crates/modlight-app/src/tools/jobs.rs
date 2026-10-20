//! `TaskHelpers.ExecuteJob` for the tools: which window opens, with which options, and
//! what runs without one. `ToolsSettingsReference` is the task's `ToolsSettings` (the
//! hotkey's own when it overrides them, else the defaults); edits a window makes are
//! written back to the same place.

use std::path::PathBuf;

use modlight_core::hotkeys::HotkeyType;
use modlight_core::tasks::settings::TaskSettings;
use serde::Serialize;
use serde::de::DeserializeOwned;

use modlight_core::l10n::{mac, t};
use modlight_core::tasks::host::{MessageIcon, NotificationSound};

use crate::commands::Command;
use crate::state::AppState;

/// Where a task's options live in the settings, for writing them back: the defaults, or
/// the first hotkey running `job` that overrides the group (`UseDefault<group>` false).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptionsHome {
    Defaults,
    Hotkey(HotkeyType, &'static str),
}

impl OptionsHome {
    /// `ToolsSettingsReference`.
    pub fn of(settings: &TaskSettings) -> OptionsHome {
        if settings.use_default_tools_settings {
            OptionsHome::Defaults
        } else {
            OptionsHome::Hotkey(settings.job, "UseDefaultToolsSettings")
        }
    }

    /// `CaptureSettingsReference`.
    pub fn of_capture(settings: &TaskSettings) -> OptionsHome {
        if settings.use_default_capture_settings {
            OptionsHome::Defaults
        } else {
            OptionsHome::Hotkey(settings.job, "UseDefaultCaptureSettings")
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
    /// Opens whether or not a model came (upstream offers the download, then opens).
    BackgroundRemover {
        settings: Box<TaskSettings>,
    },
}

/// The components window closed: the tool opens if what it needs is there now.
pub fn after_components(state: &mut AppState, next: ToolContinuation, satisfied: bool) {
    if let ToolContinuation::BackgroundRemover { settings } = next {
        state.show_background_remover(*settings);
        return;
    }
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
        ToolContinuation::BackgroundRemover { settings } => {
            state.show_background_remover(*settings)
        }
    }
}

/// Read one member of the task's `ToolsSettings`.
pub fn tool_options<T: DeserializeOwned + Default>(settings: &TaskSettings, member: &str) -> T {
    modlight_tools::settings::read(settings.tools_settings.get(member))
}

impl AppState {
    /// Write a member of a task's settings (`path` from the `TaskSettings` object) back
    /// where it came from, and save.
    pub fn save_task_member<T: Serialize>(&mut self, home: &OptionsHome, path: &[&str], value: &T) {
        match home {
            OptionsHome::Defaults => {
                let mut full = vec!["DefaultTaskSettings"];
                full.extend_from_slice(path);
                modlight_tools::settings::write(self.config.values_mut(), &full, value);
            }
            OptionsHome::Hotkey(job, flag) => {
                let mut list = self.captures.hotkeys.hotkeys.clone();
                let Some(hotkey) = list.iter_mut().find(|h| {
                    h.job() == *job && h.task.get(*flag).and_then(|v| v.as_bool()) == Some(false)
                }) else {
                    return;
                };
                if let serde_json::Value::Object(task) = &mut hotkey.task {
                    modlight_tools::settings::write(task, path, value);
                }
                self.captures.hotkeys.hotkeys = list;
                self.save_hotkeys();
            }
        }
        self.save_config();
    }

    /// Write one member of `ToolsSettings` back where it came from.
    pub fn save_tool_options<T: Serialize>(&mut self, home: &OptionsHome, member: &str, value: &T) {
        self.save_task_member(home, &["ToolsSettings", member], value);
    }

    /// Write a member of `CaptureSettings` (OCR options, the colour picker's palette).
    pub fn save_capture_options<T: Serialize>(
        &mut self,
        home: &OptionsHome,
        path: &[&str],
        value: &T,
    ) {
        let mut full = vec!["CaptureSettings"];
        full.extend_from_slice(path);
        self.save_task_member(home, &full, value);
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
        H::OCR => match file {
            Some(file) => state.ocr_file(file, settings),
            None => state.ocr_region(settings),
        },
        H::IndexFolder => state.index_folder(file, settings),
        H::AnalyzeImage => state.open_analyze_image(file, settings),
        H::BackgroundRemover => state.open_background_remover(settings),
        H::ActiveWindowBorderless => state.active_window_borderless(settings),
        H::ActiveWindowTopMost => state.active_window_top_most(settings),
        H::StripMetadata => state.strip_metadata(file, settings),
        H::VideoConverter => match file {
            Some(file) => state.open_video_converter(Some(file), settings),
            None => state.open_video_converter(None, settings),
        },
        // The image effects window, the tray menu toggle: other parts of the port.
        _ => state.push(Command::NotYet),
    }
}

/// `Indexer.Index` with the task's `IndexerSettings` and the application's `BinaryUnits`.
pub fn folder_indexer(
    folder: &std::path::Path,
    settings: &TaskSettings,
    binary_units: bool,
) -> Option<(String, &'static str)> {
    let mut indexer: modlight_tools::settings::IndexerSettings =
        tool_options(settings, "IndexerSettings");
    indexer.binary_units = binary_units;
    let text = modlight_tools::indexer::index(folder, &indexer, chrono::Utc::now());
    (!text.is_empty()).then(|| (text, indexer.output.extension()))
}

impl AppState {
    /// `UploadManager.IndexFolder()`: a folder chosen, indexed off the UI thread, uploaded.
    pub fn index_folder(&mut self, folder: Option<PathBuf>, settings: TaskSettings) {
        let folder = match folder {
            Some(folder) => folder,
            None => match self.tool_pick_folder(
                modlight_core::l10n::t(
                    modlight_core::l10n::tools::DirectoryIndexerWindow_Select_folder_to_index_dialog,
                ),
                None,
            ) {
                Some(folder) => folder,
                None => return,
            },
        };
        if !folder.is_dir() {
            return;
        }
        let binary = self.app_upload_settings().binary_units;
        self.rt.spawn_blocking(move || {
            let index = folder_indexer(&folder, &settings, binary);
            move |state: &mut AppState, _ctx: &egui::Context| {
                if let Some((source, extension)) = index {
                    let ctx = state.uploads.ctx.clone();
                    modlight_core::tasks::upload_manager::start_index_upload(
                        &mut state.uploads.tasks,
                        &source,
                        extension,
                        &settings,
                        &ctx,
                    );
                }
            }
        });
    }
}

impl AppState {
    /// After-capture "Analyze image" (`TaskHelpers.AnalyzeImage(filePath)`).
    pub fn task_analyze_image(&mut self, file: PathBuf, settings: TaskSettings) {
        self.open_analyze_image(Some(file), settings);
    }

    /// `TaskHelpers.PrintImage(image)`.
    pub fn print_image(&mut self, image: image::RgbaImage) {
        let _ = image;
        self.push(Command::NotYet);
    }
}

impl AppState {
    fn window_tool_message(&mut self, job: HotkeyType, text: &str, icon: MessageIcon) {
        self.uploads
            .windows
            .messages
            .push_back(crate::dialogs::message::MessageBox::new(
                &format!("Modlight - {}", job.text()),
                text,
                icon,
            ));
    }

    /// `MakeActiveWindowBorderless` the macOS way (`modlight_tools::borderless`): the
    /// active window fills its screen, and goes back the second time. QA and headless
    /// runs never touch other apps' windows.
    pub fn active_window_borderless(&mut self, settings: TaskSettings) {
        if self.env.qa || self.mode != crate::app::Mode::Normal {
            log::info!("QA run: the active window is left as it is");
            return;
        }
        let options: modlight_tools::settings::BorderlessWindowSettings =
            tool_options(&settings, "BorderlessWindowSettings");
        let working_area = options.exclude_taskbar_area;
        let outcome = modlight_macos::ax::toggle_active_window(|window, screens| {
            use modlight_tools::borderless::{Frame, Screen, target_frame};
            let frame = |r: (f64, f64, f64, f64)| Frame::new(r.0, r.1, r.2, r.3);
            let screens: Vec<Screen> = screens
                .iter()
                .map(|(whole, visible)| Screen {
                    frame: frame(*whole),
                    visible: frame(*visible),
                })
                .collect();
            target_frame(&frame(window), &screens, working_area)
                .map(|f| (f.x, f.y, f.width, f.height))
        });
        use modlight_macos::ax::Toggled;
        match outcome {
            Toggled::Filled | Toggled::Restored => {
                self.play_task_sound(NotificationSound::ActionCompleted, &settings)
            }
            Toggled::NotTrusted => self.window_tool_message(
                HotkeyType::ActiveWindowBorderless,
                t(mac::BorderlessNeedsAccessibility),
                MessageIcon::Information,
            ),
            Toggled::NoWindow => {}
            Toggled::Failed(error) => self.window_tool_message(
                HotkeyType::ActiveWindowBorderless,
                &error,
                MessageIcon::Error,
            ),
        }
    }

    /// `MakeActiveWindowTopMost`: macOS has no public way to keep another app's window
    /// above the others (window levels belong to the app that owns the window), so the
    /// hotkey says so and points at Pin to screen.
    pub fn active_window_top_most(&mut self, settings: TaskSettings) {
        let _ = settings;
        self.window_tool_message(
            HotkeyType::ActiveWindowTopMost,
            t(mac::TopMostUnavailable),
            MessageIcon::Information,
        );
    }
}

/// Every frame: background results the tools wait for.
pub fn tick(state: &mut AppState, _ctx: &egui::Context) {
    static INDEXER: std::sync::Once = std::sync::Once::new();
    INDEXER.call_once(|| modlight_core::tasks::upload_manager::set_folder_indexer(folder_indexer));
    static PANIC_HOOK: std::sync::Once = std::sync::Once::new();
    if state.mode == crate::app::Mode::Normal && !state.env.qa {
        static VISION: std::sync::Once = std::sync::Once::new();
        VISION.call_once(|| {
            std::thread::spawn(super::ocr::use_vision_languages);
        });
    }
    if state.mode == crate::app::Mode::Normal {
        PANIC_HOOK.call_once(super::boxes::install_panic_hook);
        state.tick_panics();
    }
    state.tick_screen_pick();
}
