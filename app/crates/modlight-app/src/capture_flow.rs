//! Captures on the window's side (`CaptureBase.Capture`, `UploadManager.RunImageTask`,
//! `TaskHelpers.ExecuteJob`, `HotkeyManager`): the delay, hiding the main window, taking
//! the picture off the UI thread, the overlay, the capture sound, the quick task menu and
//! the after-capture window, and the task that follows. Hotkeys arrive here too.
//!
//! The screen is ScreenCaptureKit in the real app and a fake desktop everywhere else
//! (`--qa-profile`, headless runs, tests): QA never asks macOS for Screen Recording.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use modlight_core::capture::geometry::IRect;
use modlight_core::capture::region::{RegionKind, RegionOutcome, RegionSession};
use modlight_core::capture::source::{CaptureError, FakeSource, SharedSource};
use modlight_core::capture::{self, CaptureKind, CaptureStep, Captured};
use modlight_core::hotkeys::{FakeBackend, HotkeyManager, HotkeySettings, HotkeyType};
use modlight_core::l10n::{Arg, format, s, t};
use modlight_core::recording::ScreenRecordOutput;
use modlight_core::tasks::host::{MessageIcon, NotificationSound};
use modlight_core::tasks::info::TaskMetadata;
use modlight_core::tasks::settings::{AfterCaptureTasks, TaskSettings};
use modlight_core::tasks::worker::WorkerTask;

use crate::app::Mode;
use crate::dialogs::after_capture::{AfterCapture, AfterCaptureOutcome};
use crate::dialogs::message::MessageBox;
use crate::region_overlay::{OverlayPurpose, RegionOverlay};
use crate::state::{AppState, Effect};

/// `ScreenRecordStartMethod`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordStartMethod {
    Region,
    ActiveWindow,
    CustomRegion,
    LastRegion,
}

/// A capture waiting for its delay or for the main window to get out of the way.
pub struct Scheduled {
    pub at: f64,
    pub kind: CaptureKind,
    pub settings: TaskSettings,
    pub auto_hide: bool,
    /// The main window was hidden for it.
    pub hidden: bool,
}

/// A picture (or a recording) waiting for the quick task menu or the after-capture window.
#[derive(Clone)]
pub struct PendingImage {
    pub metadata: TaskMetadata,
    pub settings: TaskSettings,
    /// A recording's file (`ShowAfterCaptureWindow(…, filePath)`).
    pub file: Option<PathBuf>,
    pub skip_quick_menu: bool,
}

/// Everything captures need, held by the app state.
pub struct Captures {
    pub source: SharedSource,
    pub mac: Option<Arc<crate::platform::capture::ScreenSource>>,
    pub overlay: Option<RegionOverlay>,
    pub scheduled: Vec<Scheduled>,
    /// A picture is being taken on the background pool.
    pub busy: bool,
    pub hotkeys: HotkeyManager,
    pub after_capture: VecDeque<(AfterCapture, PendingImage)>,
    pub quick_menu: Option<(crate::dialogs::quick_task_menu::QuickTaskMenu, PendingImage)>,
    pub recording: Option<crate::recording_ui::Recording>,
    pub auto_capture: crate::dialogs::auto_capture::AutoCaptureState,
    /// Hotkeys pressed this frame (QA and tests press them through here).
    pub simulated: Vec<usize>,
    /// The main window was hidden for a capture and comes back afterwards.
    pub main_hidden: bool,
    /// The quick task menu's editor.
    /// The screen recording options window (`FFmpegOptionsWindow`).
    pub ffmpeg_window: Option<crate::ffmpeg_window::FFmpegWindow>,
    pub quick_task_editor: Option<crate::dialogs::quick_task_editor::QuickTaskEditor>,
}

impl Captures {
    /// The fake desktop of QA runs: `<profile>/qa-capture` when it holds one, else the
    /// built-in sample.
    pub fn fake_source(profile: &std::path::Path) -> SharedSource {
        Arc::new(
            FakeSource::from_folder(&profile.join("qa-capture")).unwrap_or_else(FakeSource::sample),
        )
    }

    pub fn new(profile: &std::path::Path, qa: bool) -> Captures {
        let (source, mac): (SharedSource, _) = if qa {
            (Captures::fake_source(profile), None)
        } else {
            let mac = crate::platform::capture::ScreenSource::new();
            (mac.clone() as SharedSource, Some(mac))
        };
        Captures {
            source,
            mac,
            overlay: None,
            scheduled: Vec::new(),
            busy: false,
            hotkeys: HotkeyManager::new(Box::new(FakeBackend::default()), Vec::new(), false),
            after_capture: VecDeque::new(),
            quick_menu: None,
            recording: None,
            auto_capture: Default::default(),
            simulated: Vec::new(),
            main_hidden: false,
            ffmpeg_window: None,
            quick_task_editor: None,
        }
    }

    /// Whether a capture or a recording owns the screen right now.
    pub fn active(&self) -> bool {
        self.overlay.is_some() || self.busy || !self.scheduled.is_empty()
    }
}

impl AppState {
    // ------------------------------------------------------------------ hotkeys

    /// Load the hotkeys and, in the real app, register them (`HotkeyManager.UpdateHotkeys`).
    /// QA and headless runs keep a fake registration; nothing is taken from the system.
    pub fn start_hotkeys(&mut self) {
        let list = modlight_core::hotkeys::load(&self.config);
        let disabled = self
            .config
            .get("DisableHotkeys")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let backend: Box<dyn modlight_core::hotkeys::HotkeyBackend> =
            if self.mode == Mode::Normal && !self.env.qa {
                Box::new(crate::platform::hotkeys::SystemHotkeys::new())
            } else {
                Box::new(FakeBackend::default())
            };
        self.captures.hotkeys = HotkeyManager::new(backend, list, disabled);
        self.captures.hotkeys.register_all();
        if self.mode == Mode::Normal && !self.captures.hotkeys.failed().is_empty() {
            log::warn!(
                "{} hotkey(s) could not be registered",
                self.captures.hotkeys.failed().len()
            );
        }
    }

    /// Write the hotkeys back (`SettingManager.SaveHotkeysConfigAsync`).
    pub fn save_hotkeys(&mut self) {
        modlight_core::hotkeys::save(&mut self.config, &self.captures.hotkeys.hotkeys);
        self.save_config();
    }

    /// Hotkeys pressed since the last frame, and those QA pressed. Whether any ran.
    pub fn pump_hotkeys(&mut self) -> bool {
        self.end_orphaned_hotkey_capture();
        let mut pressed: Vec<HotkeySettings> = Vec::new();
        if self.mode == Mode::Normal && !self.env.qa {
            for id in crate::platform::hotkeys::SystemHotkeys::pressed() {
                // `DisableHotkeysOnFullscreen`.
                let fullscreen_block = self
                    .config
                    .get("DisableHotkeysOnFullscreen")
                    .and_then(|v| v.as_bool())
                    == Some(true)
                    && crate::platform::system::active_window_fullscreen();
                if fullscreen_block {
                    continue;
                }
                if let Some(hotkey) = self.captures.hotkeys.triggered(id) {
                    pressed.push(hotkey.clone());
                }
            }
        }
        for index in std::mem::take(&mut self.captures.simulated) {
            if !self.captures.hotkeys.ignore
                && let Some(hotkey) = self.captures.hotkeys.hotkeys.get(index)
            {
                pressed.push(hotkey.clone());
            }
        }
        let any = !pressed.is_empty();
        for hotkey in pressed {
            let defaults = self
                .config
                .get("DefaultTaskSettings")
                .cloned()
                .unwrap_or_default();
            let settings = hotkey.task_settings(&defaults);
            self.execute_job(hotkey.job(), settings);
        }
        any
    }

    /// `TaskHelpers.ToggleHotkeys`.
    pub fn toggle_hotkeys(&mut self) {
        let disabled = !self.captures.hotkeys.disabled;
        self.config
            .set("DisableHotkeys", serde_json::json!(disabled));
        self.captures.hotkeys.toggle(disabled);
        self.save_config();
        let settings = self.default_task_settings();
        if settings.general_settings.play_sound_after_action {
            self.play_task_sound(NotificationSound::ActionCompleted, &settings);
        }
        if settings
            .general_settings
            .show_toast_notification_after_task_completed
        {
            let text = if disabled {
                t(s::TaskHelpers_ToggleHotkeys_Hotkeys_disabled_)
            } else {
                t(s::TaskHelpers_ToggleHotkeys_Hotkeys_enabled_)
            };
            self.show_notification_tip(text, "Modlight", None);
        }
    }

    /// `TaskHelpers.ExecuteJob`.
    pub fn execute_job(&mut self, job: HotkeyType, settings: TaskSettings) {
        log::info!("Executing: {}", job.text());
        use HotkeyType::*;
        match job {
            None => {}
            FileUpload => self.pick_and_upload(Option::None),
            FolderUpload => self.pick_folder_and_upload(),
            ClipboardUpload => self.clipboard_upload(settings),
            ClipboardUploadWithContentViewer => self.open_clipboard_viewer(settings, false),
            UploadText => self.open_text_upload(),
            UploadURL => self.open_url_upload(),
            DragDropUpload => self.open_drop_window(),
            ShortenURL => self.open_shorten_url(),
            StopUploads => self.uploads.tasks.stop_all(&self.uploads.ctx),
            PrintScreen => self.capture_with(CaptureKind::Fullscreen, settings, false),
            ActiveWindow => self.capture_with(CaptureKind::ActiveWindow, settings, false),
            ActiveMonitor => self.capture_with(CaptureKind::ActiveMonitor, settings, false),
            RectangleRegion => {
                self.capture_with(CaptureKind::Region(RegionKind::Default), settings, false)
            }
            RectangleLight => {
                self.capture_with(CaptureKind::Region(RegionKind::Light), settings, false)
            }
            RectangleTransparent => self.capture_with(
                CaptureKind::Region(RegionKind::Transparent),
                settings,
                false,
            ),
            CustomRegion => self.capture_with(CaptureKind::CustomRegion, settings, false),
            CustomWindow => self.capture_with(CaptureKind::CustomWindow, settings, false),
            LastRegion => self.capture_with(CaptureKind::LastRegion, settings, false),
            ScrollingCapture => self.open_scrolling_capture(settings),
            AutoCapture => self.open_auto_capture(settings),
            StartAutoCapture => self.start_auto_capture(settings),
            StopAutoCapture => self.stop_auto_capture(),
            ScreenRecorder => self.start_screen_recording(
                ScreenRecordOutput::FFmpeg,
                RecordStartMethod::Region,
                settings,
            ),
            ScreenRecorderActiveWindow => self.start_screen_recording(
                ScreenRecordOutput::FFmpeg,
                RecordStartMethod::ActiveWindow,
                settings,
            ),
            ScreenRecorderCustomRegion => self.start_screen_recording(
                ScreenRecordOutput::FFmpeg,
                RecordStartMethod::CustomRegion,
                settings,
            ),
            StartScreenRecorder => self.start_screen_recording(
                ScreenRecordOutput::FFmpeg,
                RecordStartMethod::LastRegion,
                settings,
            ),
            ScreenRecorderGIF => self.start_screen_recording(
                ScreenRecordOutput::Gif,
                RecordStartMethod::Region,
                settings,
            ),
            ScreenRecorderGIFActiveWindow => self.start_screen_recording(
                ScreenRecordOutput::Gif,
                RecordStartMethod::ActiveWindow,
                settings,
            ),
            ScreenRecorderGIFCustomRegion => self.start_screen_recording(
                ScreenRecordOutput::Gif,
                RecordStartMethod::CustomRegion,
                settings,
            ),
            StartScreenRecorderGIF => self.start_screen_recording(
                ScreenRecordOutput::Gif,
                RecordStartMethod::LastRegion,
                settings,
            ),
            StopScreenRecording => self.recording_stop(),
            PauseScreenRecording => self.recording_pause(),
            AbortScreenRecording => self.recording_abort(),
            DisableHotkeys => self.toggle_hotkeys(),
            OpenMainWindow => self.push(crate::commands::Command::ShowMainWindow),
            OpenScreenshotsFolder => self.open_screenshots_folder(),
            ExitShareX => self.quit_requested = true,
            ImageEditor => self.open_image_editor(),
            // The tools, pinning and the history windows (phase 5).
            _ => self.run_tool_job(job, settings),
        }
    }

    // ------------------------------------------------------------------ captures

    /// `CaptureBase.Capture(autoHideForm)` with the default task settings.
    pub fn capture(&mut self, kind: CaptureKind, auto_hide: bool) {
        let settings = self.default_task_settings();
        self.capture_with(kind, settings, auto_hide);
    }

    /// `CaptureBase.Capture(taskSettings, autoHideForm)`.
    pub fn capture_with(&mut self, kind: CaptureKind, settings: TaskSettings, auto_hide: bool) {
        if self.captures.overlay.is_some() {
            return;
        }
        if settings.general_settings.toast_window_auto_hide {
            self.windows.notification = None;
        }
        let delay = settings.capture_settings.screenshot_delay.max(0.0);
        self.captures.scheduled.push(Scheduled {
            at: self.time + delay,
            kind,
            settings,
            auto_hide: auto_hide && self.mode == Mode::Normal,
            hidden: false,
        });
    }

    /// Run what is due: delays, the main window out of the way, the picture.
    pub fn tick_captures(&mut self, ctx: &egui::Context) {
        if let Some(mac) = &self.captures.mac {
            // AppKit's display names and visible frames are main-thread only.
            if !self.captures.scheduled.is_empty() {
                mac.refresh_on_main_thread();
            }
        }
        let now = self.time;
        let mut due = Vec::new();
        let mut index = 0;
        while index < self.captures.scheduled.len() {
            if self.captures.scheduled[index].at <= now && !self.captures.busy {
                due.push(self.captures.scheduled.remove(index));
            } else {
                index += 1;
            }
        }
        if !self.captures.scheduled.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        for mut item in due {
            if item.auto_hide && !item.hidden && !self.captures.main_hidden {
                // `Program.MainForm.Hide()` and 250 ms for it to go.
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.captures.main_hidden = true;
                item.hidden = true;
                item.at = now + 0.25;
                self.captures.scheduled.push(item);
                ctx.request_repaint_after(std::time::Duration::from_millis(260));
                continue;
            }
            self.take_capture(item);
        }
        if let Some(overlay) = self.captures.overlay.as_mut() {
            overlay.tick(now);
            if overlay.session.is_closed() {
                let overlay = self.captures.overlay.take().unwrap();
                self.overlay_closed(overlay, ctx);
            }
        }
        if self.captures.main_hidden
            && !self.captures.busy
            && self.captures.overlay.is_none()
            && self.captures.scheduled.is_empty()
        {
            self.captures.main_hidden = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }
        self.tick_auto_capture(ctx);
        self.tick_recording(ctx);
        // Files Finder handed to "Upload with Modlight".
        let files = crate::platform::integration::take_service_files();
        if !files.is_empty() {
            let settings = self.default_task_settings();
            self.upload_files_with(files, settings);
        }
    }

    fn take_capture(&mut self, item: Scheduled) {
        self.captures.busy = true;
        let source = self.captures.source.clone();
        let Scheduled { kind, settings, .. } = item;
        self.rt.spawn_blocking(move || {
            let step = capture::capture(kind, &settings, source.as_ref());
            move |state: &mut AppState, _ctx: &egui::Context| {
                state.captures.busy = false;
                state.capture_step(step, settings);
            }
        });
    }

    fn capture_step(&mut self, step: CaptureStep, settings: TaskSettings) {
        match step {
            CaptureStep::Done(captured) => self.after_capture(captured, settings),
            CaptureStep::Region(request) => {
                let mut session = RegionSession::new(request);
                session.render_scale = session.request.scale;
                let mut overlay =
                    RegionOverlay::new(session, OverlayPurpose::Capture, settings, self.time);
                overlay.enable_workspace(self.editor_options());
                self.captures.overlay = Some(overlay);
            }
            CaptureStep::Message(text) => self.uploads.windows.messages.push_back(MessageBox::new(
                "Modlight",
                &text,
                MessageIcon::Information,
            )),
            CaptureStep::Nothing(error) => self.capture_failed(error),
        }
    }

    /// A capture that took nothing: a refusal is said out loud, anything else is logged.
    fn capture_failed(&mut self, error: Option<CaptureError>) {
        match error {
            Some(CaptureError::NotPermitted) => {
                // macOS refused the pixels: the Screen Recording permission is off.
                self.uploads.windows.messages.push_back(MessageBox::new(
                    &format!("Modlight - {}", t(s::Error)),
                    &CaptureError::NotPermitted.to_string(),
                    MessageIcon::Warning,
                ));
            }
            Some(other) => log::warn!("Capture failed: {other}"),
            None => {}
        }
    }

    fn overlay_closed(&mut self, mut overlay: RegionOverlay, ctx: &egui::Context) {
        // The annotations drawn on the desktop go with the picture (`GetSnapshot`).
        let annotated = overlay.annotated_image();
        let RegionOverlay {
            mut session,
            purpose,
            settings,
            ..
        } = overlay;
        let modified = annotated.is_some();
        if let Some(image) = annotated {
            session.request.image = Some(image);
        }
        let Some(outcome) = session.outcome.clone() else {
            return;
        };
        match purpose {
            OverlayPurpose::Capture => {
                if session.request.kind == RegionKind::Transparent {
                    // The area is taken after the overlay has gone.
                    if let RegionOutcome::Completed { .. } = outcome {
                        self.captures.busy = true;
                        let source = self.captures.source.clone();
                        let request = session.request.clone();
                        self.rt.spawn_blocking(move || {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            let captured =
                                capture::finish_region(&request, &outcome, source.as_ref(), false);
                            move |state: &mut AppState, _ctx: &egui::Context| {
                                state.captures.busy = false;
                                if let Some(captured) = captured {
                                    state.after_capture(captured, settings);
                                }
                            }
                        });
                    }
                } else if let Some(captured) = capture::finish_region(
                    &session.request,
                    &outcome,
                    self.captures.source.as_ref(),
                    modified,
                ) {
                    self.after_capture(captured, settings);
                }
            }
            OverlayPurpose::Tool(target) => {
                // A tool's picture: the area as the overlay left it, with where it was.
                // No capture: "Last region" keeps repeating the last capture's area.
                let picture = match &outcome {
                    RegionOutcome::Completed { screen_rect, .. } => capture::region_picture(
                        &session.request,
                        &outcome,
                        self.captures.source.as_ref(),
                    )
                    .map(|image| (Arc::new(image), *screen_rect)),
                    _ => None,
                };
                self.tool_region_closed(target, picture);
            }
            OverlayPurpose::AutoCaptureRegion => {
                if let RegionOutcome::Completed { screen_rect, .. } = outcome {
                    self.auto_capture_region_chosen(screen_rect);
                }
            }
            OverlayPurpose::CustomRegion(target) => {
                if let RegionOutcome::Completed { screen_rect, .. } = outcome {
                    self.set_task_member(
                        target,
                        &["CaptureSettings", "CaptureCustomRegion"],
                        serde_json::json!(screen_rect.to_text()),
                    );
                }
            }
            OverlayPurpose::ScrollingRegion => {
                if let RegionOutcome::Completed {
                    screen_rect,
                    window,
                    ..
                } = outcome
                {
                    crate::scrolling::region_chosen(self, screen_rect, window);
                }
            }
            OverlayPurpose::Recording(output) => {
                if let RegionOutcome::Completed {
                    screen_rect,
                    window,
                    ..
                } = outcome
                {
                    let mut metadata = TaskMetadata::default();
                    if let Some(window) = window {
                        metadata.window_title = Some(window.title);
                        metadata.process_name = Some(window.process_name);
                    }
                    self.begin_recording(output, screen_rect, metadata, settings, true, ctx);
                }
            }
        }
    }

    /// `CaptureBase.AfterCapture`.
    pub fn after_capture(&mut self, captured: Captured, settings: TaskSettings) {
        if captured.metadata.image.is_none() {
            return;
        }
        self.play_task_sound(NotificationSound::Capture, &settings);
        let settings = capture::after_capture_settings(&captured, &settings);
        self.run_image_task(PendingImage {
            metadata: captured.metadata,
            settings,
            file: None,
            skip_quick_menu: false,
        });
    }

    /// `UploadManager.RunImageTask` with the quick task menu and the after-capture window.
    pub fn run_image_task(&mut self, pending: PendingImage) {
        let job = pending.settings.after_capture_job;
        if !pending.skip_quick_menu
            && job.has(AfterCaptureTasks::ShowQuickTaskMenu)
            && pending.file.is_none()
        {
            let menu = crate::dialogs::quick_task_menu::QuickTaskMenu::new(&self.config);
            self.captures.quick_menu = Some((menu, pending));
            return;
        }
        if job.has(AfterCaptureTasks::ShowAfterCaptureWindow) {
            let window = AfterCapture::new(
                self.uploads.window_key(),
                &pending.settings,
                &pending.metadata,
                pending.file.as_deref(),
                &self.uploads.ctx,
            );
            self.captures.after_capture.push_back((window, pending));
            return;
        }
        self.start_image_task(pending, None);
    }

    fn start_image_task(&mut self, pending: PendingImage, custom_name: Option<String>) {
        let ctx = self.uploads.ctx.clone();
        match pending.file {
            Some(path) => {
                // `FinishRecording`: a new name renames the file first.
                let mut path = path;
                if let Some(name) = custom_name.as_deref().filter(|n| !n.is_empty()) {
                    match rename_recording(&path, name) {
                        Ok(renamed) => path = renamed,
                        // `RenameFile` says why and the file keeps its name.
                        Err(error) => self.uploads.windows.messages.push_back(MessageBox::new(
                            &format!("Modlight - {}", t(s::Error)),
                            &format!(
                                "{}\n{error}",
                                t(modlight_core::l10n::helpers::FileHelpers_Rename_file_error)
                            ),
                            MessageIcon::Error,
                        )),
                    }
                }
                if let Some(task) = WorkerTask::file_job(
                    &path,
                    pending.metadata,
                    pending.settings,
                    custom_name.as_deref(),
                    &ctx,
                ) {
                    self.uploads.tasks.start(task, &ctx);
                }
            }
            None => {
                let task = WorkerTask::image_upload(
                    pending.metadata,
                    pending.settings,
                    custom_name.as_deref(),
                    &ctx,
                );
                self.uploads.tasks.start(task, &ctx);
            }
        }
    }

    /// The after-capture window answered.
    pub fn after_capture_finished(&mut self, key: u64, outcome: AfterCaptureOutcome) {
        let Some(index) = self
            .captures
            .after_capture
            .iter()
            .position(|(w, _)| w.key == key)
        else {
            return;
        };
        let (_, mut pending) = self.captures.after_capture.remove(index).unwrap();
        match outcome {
            AfterCaptureOutcome::Continue {
                settings,
                file_name,
            } => {
                pending.settings = *settings;
                self.start_image_task(pending, Some(file_name));
            }
            AfterCaptureOutcome::Cancel => {}
        }
    }

    /// The quick task menu answered (`TaskInfoSelected`).
    pub fn quick_menu_finished(
        &mut self,
        choice: Option<
            Option<(
                AfterCaptureTasks,
                modlight_core::tasks::settings::AfterUploadTasks,
            )>,
        >,
    ) {
        let Some((_, mut pending)) = self.captures.quick_menu.take() else {
            return;
        };
        match choice {
            // Closed without a choice: nothing happens, as upstream's menu closing does.
            None => {}
            // "Continue": the task as it was.
            Some(None) => {
                pending.skip_quick_menu = true;
                self.run_image_task(pending);
            }
            Some(Some((after_capture, after_upload))) => {
                pending.settings.after_capture_job = after_capture;
                pending.settings.after_upload_job = after_upload;
                pending.skip_quick_menu = true;
                self.run_image_task(pending);
            }
        }
    }

    /// `TaskHelpers.PlayNotificationSoundAsync` for the window's own events.
    pub fn play_task_sound(&mut self, sound: NotificationSound, settings: &TaskSettings) {
        let general = &settings.general_settings;
        let (wanted, custom, path) = match sound {
            NotificationSound::Capture => (
                general.play_sound_after_capture,
                general.use_custom_capture_sound,
                general.custom_capture_sound_path.clone(),
            ),
            NotificationSound::TaskCompleted => (
                general.play_sound_after_upload,
                general.use_custom_task_completed_sound,
                general.custom_task_completed_sound_path.clone(),
            ),
            NotificationSound::ActionCompleted => (
                general.play_sound_after_action,
                general.use_custom_action_completed_sound,
                general.custom_action_completed_sound_path.clone(),
            ),
            NotificationSound::Error => (
                true,
                general.use_custom_error_sound,
                general.custom_error_sound_path.clone(),
            ),
        };
        if wanted {
            let custom = custom.then_some(path).flatten().filter(|p| !p.is_empty());
            self.effects.push(Effect::PlaySound(sound, custom));
        }
    }

    /// `QuickTaskMenuEditorIntegration.Show`.
    pub fn open_quick_task_menu_editor(&mut self) {
        if self.captures.quick_task_editor.is_none() {
            let presets = crate::dialogs::quick_task_menu::QuickTaskInfo::presets(&self.config);
            self.captures.quick_task_editor = Some(
                crate::dialogs::quick_task_editor::QuickTaskEditor::new(presets),
            );
        }
    }

    /// Save what the editor changed (`SettingManager.SaveApplicationConfigAsync`).
    pub fn save_quick_task_presets(&mut self) {
        if let Some(editor) = self.captures.quick_task_editor.as_mut()
            && editor.dirty
        {
            editor.dirty = false;
            let value = editor.to_value();
            self.config.set("QuickTaskPresets", value);
            self.save_config();
        }
    }

    /// The default task settings' capture delay (`SetAvaloniaScreenshotDelay`).
    pub fn set_screenshot_delay(&mut self, seconds: f64) {
        self.set_default_capture_member("ScreenshotDelay", serde_json::json!(seconds));
    }

    /// `ShowCursor` of the default task settings.
    pub fn toggle_show_cursor(&mut self) {
        let current = self.default_task_settings().capture_settings.show_cursor;
        self.set_default_capture_member("ShowCursor", serde_json::json!(!current));
    }

    fn set_default_capture_member(&mut self, member: &str, value: serde_json::Value) {
        let mut defaults = self
            .config
            .get("DefaultTaskSettings")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        let capture = defaults
            .entry("CaptureSettings")
            .or_insert_with(|| serde_json::json!({}));
        if !capture.is_object() {
            *capture = serde_json::json!({});
        }
        capture[member] = value;
        self.config
            .set("DefaultTaskSettings", serde_json::Value::Object(defaults));
        self.save_config();
    }

    /// `Settings.ScreenRecordRegion`.
    pub fn screen_record_region(&self) -> IRect {
        self.config
            .get("ScreenRecordRegion")
            .and_then(IRect::parse_value)
            .unwrap_or_default()
    }

    pub fn set_screen_record_region(&mut self, rect: IRect) {
        self.config
            .set("ScreenRecordRegion", serde_json::json!(rect.to_text()));
        self.config_dirty = true;
    }

    /// The tray and the capture menu's monitor list: "1. 1440x900".
    pub fn monitor_entries(&self) -> Vec<(String, IRect, bool)> {
        self.captures
            .source
            .screens()
            .iter()
            .enumerate()
            .map(|(i, s)| {
                (
                    format!("{}. {}x{}", i + 1, s.bounds.width, s.bounds.height),
                    s.bounds,
                    i == 0,
                )
            })
            .collect()
    }

    /// The window list for the capture menu (titles cut at 50, "...").
    pub fn window_entries(&self) -> Vec<(String, modlight_core::capture::source::WindowInfo)> {
        self.captures
            .source
            .windows()
            .into_iter()
            .map(|w| {
                let mut title = if w.title.is_empty() {
                    w.process_name.clone()
                } else {
                    w.title.clone()
                };
                if title.chars().count() > 50 {
                    title = title.chars().take(50).collect::<String>() + "...";
                }
                (title, w)
            })
            .collect()
    }

    /// `ScreenshotDelay0S` with the current delay, "0.#".
    pub fn screenshot_delay_text(&self) -> String {
        let delay = self
            .default_task_settings()
            .capture_settings
            .screenshot_delay;
        let text = if delay.fract().abs() < 1e-9 {
            format!("{}", delay as i64)
        } else {
            modlight_core::culture::Culture::current().fixed(delay, 1)
        };
        format(t(s::ScreenshotDelay0S), &[Arg::from(text)])
    }
}

/// `FinishRecording`'s `FileHelpers.RenameFile(path, name + ext)`: the recording under
/// the name typed in the after-capture window (the same name in another case is no
/// rename, `OrdinalIgnoreCase`). A file of that name already there is never replaced:
/// .NET's `File.Move` refuses it, where `rename` would silently overwrite it.
fn rename_recording(path: &std::path::Path, name: &str) -> std::io::Result<PathBuf> {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if stem.to_lowercase() == name.to_lowercase() {
        return Ok(path.to_path_buf());
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    let target = path.with_file_name(if ext.is_empty() {
        name.to_string()
    } else {
        format!("{name}.{ext}")
    });
    if std::fs::symlink_metadata(&target).is_ok() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("\"{}\" already exists", target.display()),
        ));
    }
    std::fs::rename(path, &target)?;
    Ok(target)
}

// ---------------------------------------------------------------------- auto capture

impl AppState {
    fn auto_capture_region(&self) -> IRect {
        self.config
            .get("AutoCaptureRegion")
            .and_then(IRect::parse_value)
            .unwrap_or_default()
    }

    /// What the auto-capture window shows (`Program.Settings.AutoCapture*`).
    pub fn auto_capture_view(&self) -> crate::dialogs::auto_capture::AutoCaptureView {
        crate::dialogs::auto_capture::AutoCaptureView {
            region: self.auto_capture_region(),
            repeat_time: self
                .config
                .get("AutoCaptureRepeatTime")
                .and_then(|v| v.as_f64())
                .unwrap_or(60.0),
            minimize: self
                .config
                .get("AutoCaptureMinimizeToTray")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            wait_uploads: self
                .config
                .get("AutoCaptureWaitUpload")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        }
    }

    /// `AutoCaptureWindowIntegration.Show`.
    pub fn open_auto_capture(&mut self, settings: TaskSettings) {
        let region = self.auto_capture_region();
        let auto = &mut self.captures.auto_capture;
        auto.settings = Some(settings);
        auto.window_open = true;
        auto.hidden = false;
        if auto.custom_region.is_empty() {
            auto.custom_region = region;
        }
    }

    /// `AutoCaptureWindowIntegration.Start`.
    pub fn start_auto_capture(&mut self, settings: TaskSettings) {
        if self.captures.auto_capture.running {
            return;
        }
        self.open_auto_capture(settings);
        self.auto_capture_execute();
    }

    /// `AutoCaptureWindowIntegration.Stop`.
    pub fn stop_auto_capture(&mut self) {
        if self.captures.auto_capture.running {
            self.auto_capture_execute();
        }
    }

    /// `Execute`: start or stop.
    pub fn auto_capture_execute(&mut self) {
        let view = self.auto_capture_view();
        let now = self.time;
        let auto = &mut self.captures.auto_capture;
        if auto.running {
            auto.running = false;
            auto.count = 0;
            return;
        }
        if view.region.is_empty() {
            return;
        }
        auto.running = true;
        auto.delay = view.repeat_time.max(1.0);
        auto.wait_uploads = view.wait_uploads;
        // The first shot comes after one second, as the timer's first interval.
        auto.waited_from = now;
        auto.next_at = now + 1.0;
        if view.minimize {
            auto.hidden = true;
        }
    }

    /// `RegionCaptureTasks.GetRectangleRegion` for the task settings' `Select region...`,
    /// with the region options of the settings being edited.
    pub fn select_custom_region(&mut self, target: crate::task_pages::TaskTarget) {
        if self.captures.overlay.is_some() || self.captures.busy {
            return;
        }
        let settings = self.task_settings_of(target);
        let source = self.captures.source.clone();
        self.captures.busy = true;
        self.rt.spawn_blocking(move || {
            let request = capture::region_request(RegionKind::Default, &settings, source.as_ref());
            move |state: &mut AppState, _ctx: &egui::Context| {
                state.captures.busy = false;
                if let Ok(mut request) = request {
                    request.enable_annotations = false;
                    let session = RegionSession::new(request);
                    state.captures.overlay = Some(RegionOverlay::new(
                        session,
                        OverlayPurpose::CustomRegion(target),
                        settings,
                        state.time,
                    ));
                }
            }
        });
    }

    pub fn auto_capture_action(&mut self, action: crate::dialogs::auto_capture::AutoCaptureAction) {
        use crate::dialogs::auto_capture::AutoCaptureAction as A;
        match action {
            A::Execute => self.auto_capture_execute(),
            A::SelectRegion => {
                let settings = self
                    .captures
                    .auto_capture
                    .settings
                    .clone()
                    .unwrap_or_else(|| self.default_task_settings());
                let source = self.captures.source.clone();
                self.captures.busy = true;
                self.rt.spawn_blocking(move || {
                    let request =
                        capture::region_request(RegionKind::Default, &settings, source.as_ref());
                    move |state: &mut AppState, _ctx: &egui::Context| {
                        state.captures.busy = false;
                        if let Ok(mut request) = request {
                            request.enable_annotations = false;
                            let session = RegionSession::new(request);
                            state.captures.overlay = Some(RegionOverlay::new(
                                session,
                                OverlayPurpose::AutoCaptureRegion,
                                settings,
                                state.time,
                            ));
                        }
                    }
                });
            }
            A::Fullscreen => {
                self.captures.auto_capture.custom_region = self.auto_capture_region();
                self.captures.auto_capture.fullscreen = true;
                let bounds = self.captures.source.screen_bounds();
                self.config
                    .set("AutoCaptureRegion", serde_json::json!(bounds.to_text()));
                self.save_config();
            }
            A::CustomRegion => {
                self.captures.auto_capture.fullscreen = false;
                let region = self.captures.auto_capture.custom_region;
                self.config
                    .set("AutoCaptureRegion", serde_json::json!(region.to_text()));
                self.save_config();
            }
            A::RepeatTime(value) => {
                self.config
                    .set("AutoCaptureRepeatTime", serde_json::json!(value));
                self.save_config();
            }
            A::AutoMinimize(value) => {
                self.config
                    .set("AutoCaptureMinimizeToTray", serde_json::json!(value));
                self.save_config();
            }
            A::WaitUploads(value) => {
                self.config
                    .set("AutoCaptureWaitUpload", serde_json::json!(value));
                self.save_config();
            }
        }
    }

    /// The region chosen for the auto capture (`SelectRegion`).
    pub fn auto_capture_region_chosen(&mut self, rect: IRect) {
        self.config
            .set("AutoCaptureRegion", serde_json::json!(rect.to_text()));
        self.captures.auto_capture.custom_region = rect;
        self.save_config();
    }

    /// `OnScreenshotTimerTick` and `TakeScreenshot`.
    pub fn tick_auto_capture(&mut self, ctx: &egui::Context) {
        if !self.captures.auto_capture.running {
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
        let now = self.time;
        if now < self.captures.auto_capture.next_at || self.captures.busy {
            return;
        }
        if self.captures.auto_capture.wait_uploads && self.uploads.tasks.is_busy() {
            self.captures.auto_capture.next_at = now + 1.0;
            return;
        }
        let delay = self.captures.auto_capture.delay;
        self.captures.auto_capture.waited_from = now;
        self.captures.auto_capture.next_at = now + delay;
        self.captures.auto_capture.count += 1;
        let rect = self.auto_capture_region();
        if rect.is_empty() {
            return;
        }
        let mut settings = self
            .captures
            .auto_capture
            .settings
            .clone()
            .unwrap_or_else(|| self.default_task_settings());
        settings.after_capture_job = settings
            .after_capture_job
            .remove(AfterCaptureTasks::AnnotateImage);
        settings.general_settings.play_sound_after_upload = false;
        settings.general_settings.play_sound_after_action = false;
        settings
            .general_settings
            .show_toast_notification_after_task_completed = false;
        let source = self.captures.source.clone();
        let cursor = settings.capture_settings.show_cursor;
        self.captures.busy = true;
        self.rt.spawn_blocking(move || {
            let picture = source.capture_rect(rect, cursor);
            move |state: &mut AppState, _ctx: &egui::Context| {
                state.captures.busy = false;
                if let Ok(picture) = picture {
                    // `RunImageTask(bitmap, settings, true, true)`: no menu, no window.
                    let mut settings = settings;
                    settings.after_capture_job = settings
                        .after_capture_job
                        .remove(AfterCaptureTasks::ShowQuickTaskMenu)
                        .remove(AfterCaptureTasks::ShowAfterCaptureWindow);
                    state.start_image_task(
                        PendingImage {
                            metadata: TaskMetadata {
                                image: Some(Arc::new(picture)),
                                ..TaskMetadata::default()
                            },
                            settings,
                            file: None,
                            skip_quick_menu: true,
                        },
                        None,
                    );
                }
            }
        });
    }

    /// Scrolling capture (`TaskHelpers.OpenScrollingCapture`).
    pub fn open_scrolling_capture(&mut self, settings: TaskSettings) {
        crate::scrolling::open(self, settings);
    }
}

// ---------------------------------------------------------------------- menus

/// `MainMenuBuilder.BuildCaptureMenu`. `from_tray`: captures started from the tray do not
/// hide the main window (`autoHide = !_trayMenu`).
pub fn capture_menu(state: &AppState, from_tray: bool) -> Vec<crate::menus::Entry> {
    use crate::commands::Command;
    use crate::menus::{Entry, Toggle};
    use modlight_core::icons;
    let auto_hide = !from_tray;
    let windows: Vec<Entry> = {
        let list = state.window_entries();
        if list.is_empty() {
            vec![Entry::Item {
                label: t(s::MainMenuBuilder_NoWindowsFound).to_string(),
                icon: Some(icons::APP_WINDOW),
                enabled: false,
                toggle: Toggle::None,
                command: None,
            }]
        } else {
            list.into_iter()
                .map(|(title, window)| Entry::Item {
                    label: title,
                    icon: None,
                    enabled: true,
                    toggle: Toggle::None,
                    command: Some(Command::Capture(CaptureKind::Window(window), auto_hide)),
                })
                .collect()
        }
    };
    let monitors: Vec<Entry> = state
        .monitor_entries()
        .into_iter()
        .map(|(label, bounds, first)| {
            Entry::item(
                label,
                if first {
                    icons::MONITOR
                } else {
                    icons::MONITOR_UP
                },
                Command::Capture(CaptureKind::Monitor(bounds), auto_hide),
            )
        })
        .collect();
    let settings = state.default_task_settings();
    let current = settings.capture_settings.screenshot_delay;
    let culture = modlight_core::culture::Culture::current();
    let delays: Vec<Entry> = (0..6)
        .map(|delay| Entry::Item {
            label: format(
                t(s::ScreenshotDelay0S),
                &[Arg::from(culture.general(delay as f64))],
            ),
            icon: Some(if delay == 0 {
                icons::TIMER_OFF
            } else {
                icons::TIMER
            }),
            enabled: true,
            toggle: Toggle::Radio((current - delay as f64).abs() < 0.01),
            command: Some(Command::SetScreenshotDelay(delay)),
        })
        .collect();
    vec![
        Entry::item(
            t(s::MainMenuBuilder_Fullscreen),
            icons::MAXIMIZE,
            Command::Capture(CaptureKind::Fullscreen, auto_hide),
        ),
        Entry::submenu(
            t(s::MainMenuBuilder_Window),
            icons::APP_WINDOW,
            true,
            windows,
        ),
        Entry::submenu(
            t(s::MainMenuBuilder_Monitor),
            icons::MONITOR,
            true,
            monitors,
        ),
        Entry::item(
            t(s::MainMenuBuilder_Region),
            icons::SCAN,
            Command::Capture(CaptureKind::Region(RegionKind::Default), auto_hide),
        ),
        Entry::item(
            t(s::MainMenuBuilder_RegionLight),
            icons::SQUARE,
            Command::Capture(CaptureKind::Region(RegionKind::Light), auto_hide),
        ),
        Entry::item(
            t(s::MainMenuBuilder_RegionTransparent),
            icons::SQUARE_DASHED,
            Command::Capture(CaptureKind::Region(RegionKind::Transparent), auto_hide),
        ),
        Entry::item(
            t(s::MainMenuBuilder_LastRegion),
            icons::LAYERS,
            Command::Capture(CaptureKind::LastRegion, auto_hide),
        ),
        Entry::item(
            t(s::MainMenuBuilder_ScreenRecording),
            icons::VIDEO,
            Command::StartRecording(ScreenRecordOutput::FFmpeg, RecordStartMethod::Region),
        ),
        Entry::item(
            t(s::MainMenuBuilder_ScreenRecordingGif),
            icons::FILM,
            Command::StartRecording(ScreenRecordOutput::Gif, RecordStartMethod::Region),
        ),
        Entry::item(
            t(s::MainMenuBuilder_ScrollingCapture),
            icons::SCROLL_TEXT,
            Command::OpenScrollingCapture,
        ),
        Entry::item(
            t(s::MainMenuBuilder_AutoCapture),
            icons::CLOCK,
            Command::OpenAutoCapture,
        ),
        Entry::Separator,
        Entry::check(
            t(s::MainMenuBuilder_ShowCursor),
            icons::MOUSE_POINTER_2,
            settings.capture_settings.show_cursor,
            Command::ToggleShowCursor,
        ),
        Entry::submenu(state.screenshot_delay_text(), icons::TIMER, true, delays),
    ]
}

/// `MainMenuBuilder.BuildToolsMenu`: every tool; those of later phases say so when
/// picked.
pub fn tools_menu() -> Vec<crate::menus::Entry> {
    use crate::commands::Command;
    use crate::menus::Entry;
    use modlight_core::icons;
    let job = |job: HotkeyType| Command::RunJob(job);
    vec![
        Entry::item(
            t(s::MainMenuBuilder_ColorPicker),
            icons::PALETTE,
            job(HotkeyType::ColorPicker),
        ),
        Entry::item(
            t(s::MainMenuBuilder_ScreenColorPicker),
            icons::PIPETTE,
            job(HotkeyType::ScreenColorPicker),
        ),
        Entry::item(
            t(s::MainMenuBuilder_PinToScreenDialog),
            icons::PIN,
            job(HotkeyType::PinToScreen),
        ),
        Entry::Separator,
        Entry::item(
            t(s::MainMenuBuilder_ImageEditor),
            icons::IMAGE,
            job(HotkeyType::ImageEditor),
        ),
        Entry::item(
            t(s::MainMenuBuilder_ImageEffects),
            icons::WAND_SPARKLES,
            job(HotkeyType::ImageEffects),
        ),
        Entry::item(
            t(s::MainMenuBuilder_ImageViewer),
            icons::EYE,
            job(HotkeyType::ImageViewer),
        ),
        Entry::item(
            t(s::MainMenuBuilder_BackgroundRemover),
            icons::ERASER,
            job(HotkeyType::BackgroundRemover),
        ),
        Entry::item(
            t(s::MainMenuBuilder_ImageCombiner),
            icons::COMBINE,
            job(HotkeyType::ImageCombiner),
        ),
        Entry::Separator,
        Entry::item(
            t(s::MainMenuBuilder_VideoConverter),
            icons::FILE_VIDEO,
            job(HotkeyType::VideoConverter),
        ),
        Entry::Separator,
        Entry::item(
            t(s::MainMenuBuilder_OCR),
            icons::SCAN_TEXT,
            job(HotkeyType::OCR),
        ),
        Entry::item(
            t(s::MainMenuBuilder_QRCode),
            icons::QR_CODE,
            job(HotkeyType::QRCode),
        ),
        Entry::item(
            t(s::MainMenuBuilder_Metadata),
            icons::TAGS,
            job(HotkeyType::Metadata),
        ),
    ]
}

/// `HotkeyProblemViewModel.Describe`: the keys the system would not give, or nothing. On
/// Windows upstream's words, which blame a running ShareX when its mutex exists and the
/// Snipping Tool otherwise; on a Mac the port's own.
pub fn hotkey_problem(state: &AppState) -> Option<String> {
    let failed = state.captures.hotkeys.failed();
    if failed.is_empty() {
        return None;
    }
    let keys = failed
        .iter()
        .map(|h| h.info.text())
        .collect::<Vec<_>>()
        .join(", ");
    #[cfg(windows)]
    let text = if state.mode == Mode::Normal && !state.env.qa && crate::win32::sharex_running() {
        s::MainWindow_HotkeyProblemShareX
    } else {
        s::MainWindow_HotkeyProblem
    };
    #[cfg(not(windows))]
    let text = modlight_core::l10n::mac::HotkeyProblem;
    Some(format(t(text), &[Arg::from(keys)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name typed in the after-capture window renames the recording first; a file
    /// of that name already there stays as it is (upstream's `File.Move` refuses to
    /// replace it), and the recording keeps its own name.
    #[test]
    fn renaming_a_recording_never_replaces_another_file() {
        let folder = tempfile::tempdir().unwrap();
        let recording = folder.path().join("Recording_01.mp4");
        let earlier = folder.path().join("demo.mp4");
        std::fs::write(&recording, b"new recording").unwrap();
        std::fs::write(&earlier, b"an earlier recording").unwrap();
        let refused = rename_recording(&recording, "demo").unwrap_err();
        assert_eq!(refused.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&earlier).unwrap(), b"an earlier recording");
        assert_eq!(std::fs::read(&recording).unwrap(), b"new recording");
        // The same name in another case is no rename at all.
        assert_eq!(
            rename_recording(&recording, "recording_01").unwrap(),
            recording
        );
        // A free name is taken.
        let renamed = rename_recording(&recording, "talk").unwrap();
        assert_eq!(renamed, folder.path().join("talk.mp4"));
        assert_eq!(std::fs::read(&renamed).unwrap(), b"new recording");
        assert!(!recording.exists());
    }

    /// The area a tool asked for (OCR, QR, pin from screen) is no capture: upstream's
    /// `RegionCaptureTasks.GetRegionImage` leaves `LastRegionRectangle` alone, so "Last
    /// region" still repeats the last capture's area.
    #[test]
    fn a_tools_area_is_not_the_last_region() {
        use modlight_core::capture::geometry::Rect;
        use modlight_core::capture::{LastRegion, last_region, set_last_region};
        let mut state = crate::qa::state(&crate::qa::Look::default(), false);
        let captured = IRect::new(1, 2, 30, 40);
        set_last_region(LastRegion {
            rect: captured,
            ..LastRegion::default()
        });
        let settings = TaskSettings::default();
        let request = capture::region_request(
            RegionKind::Default,
            &settings,
            state.captures.source.as_ref(),
        )
        .unwrap();
        let mut session = RegionSession::new(request);
        session.complete(Rect::new(10.0, 10.0, 50.0, 50.0), false);
        let overlay = RegionOverlay::new(
            session,
            OverlayPurpose::Tool(crate::tools::region::RegionFor::QrScan(7)),
            settings,
            0.0,
        );
        state.overlay_closed(overlay, &egui::Context::default());
        assert_eq!(last_region().rect, captured);
        set_last_region(LastRegion::default());
    }
}
