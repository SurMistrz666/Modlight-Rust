//! The window's side of uploading: it owns the task list and the offline queue, answers
//! the questions tasks ask, performs what they ask for (clipboard, notices, sounds,
//! windows), keeps the Upload tab's history, and turns menu entries and drops into
//! `UploadManager` calls. The rules live in `modlight-core::tasks`; this is the glue.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64};
use std::sync::{Arc, RwLock};

use modlight_core::history::HistoryStore;
use modlight_core::l10n::{Arg, format, s, t};
use modlight_core::tasks::context::TaskContext;
use modlight_core::tasks::host::{
    MessageIcon, NotificationSound, TaskAnswer, TaskChannel, TaskEffect, TaskEvent, TaskMessage,
    TaskQuestion,
};
use modlight_core::tasks::info::{DataType, TaskStatus};
use modlight_core::tasks::manager::TaskManager;
use modlight_core::tasks::settings::{AppUploadSettings, TaskSettings};
use modlight_core::tasks::upload_manager::{self, ClipboardContent};
use modlight_core::upload::queue::{AccountSlot, QueueAccount, QueueEvent, UploadQueue};
use modlight_core::upload::store::QueueStore;
use modlight_core::upload::transport::ApiTransport;
use tokio::sync::oneshot;

use crate::dialogs::after_upload::{AfterUpload, AfterUploadAction, create_formats};
use crate::dialogs::before_upload::{BeforeUpload, BeforeUploadOutcome};
use crate::dialogs::clipboard_upload::{ClipboardOutcome, ClipboardUpload};
use crate::dialogs::drop_window::{DropOutcome, DropWindow};
use crate::dialogs::file_exists::FileExists;
use crate::dialogs::message::MessageBox;
use crate::dialogs::paste::{PasteOutcome, PasteWindow};
use crate::dialogs::shorten_url::ShortenUrl;
use crate::dialogs::text_upload::TextUpload;
use crate::dialogs::upload_confirm::{UploadConfirm, UploadConfirmOutcome, UploadQuestion};
use crate::dialogs::url_upload::UrlUpload;
use crate::state::{AppState, Effect};
use crate::upload_tab::COPY_ACKNOWLEDGEMENT;

/// A question a task is waiting on, and where the answer goes.
pub struct PendingAsk {
    pub task: u64,
    pub question: TaskQuestion,
    pub reply: oneshot::Sender<TaskAnswer>,
}

/// What a multi-file confirmation is about (`files`), and the paths of the same
/// request that come after it (`rest`), uploaded whatever the answer, as upstream's
/// loop goes on after a nested `UploadFile(folder)` returns.
pub struct PendingFiles {
    pub files: Vec<PathBuf>,
    pub rest: Vec<PathBuf>,
    pub settings: TaskSettings,
}

/// The upload windows that are open.
#[derive(Default)]
pub struct UploadWindows {
    /// The large-file question of the task at the head of the asks.
    pub large_file: Option<UploadConfirm>,
    /// The many-files question, with what it is about.
    pub multi_upload: Option<(UploadConfirm, PendingFiles)>,
    /// Many-files questions waiting for that window (upstream's are modal, one at a
    /// time).
    pub multi_upload_waiting: VecDeque<PendingFiles>,
    pub file_exists: Option<FileExists>,
    pub before_upload: Option<BeforeUpload>,
    pub after_uploads: Vec<AfterUpload>,
    pub paste: Option<PasteWindow>,
    pub text_upload: Option<TextUpload>,
    pub shorten_url: Option<ShortenUrl>,
    pub url_upload: Option<UrlUpload>,
    pub clipboard_upload: Option<(ClipboardUpload, TaskSettings)>,
    pub drop_window: Option<DropWindow>,
    pub messages: VecDeque<MessageBox>,
}

/// Everything uploading needs, held by the app state.
pub struct Uploads {
    pub tasks: TaskManager,
    pub ctx: TaskContext,
    pub receiver: std::sync::mpsc::Receiver<TaskMessage>,
    pub queue: UploadQueue,
    pub account: AccountSlot,
    pub asks: VecDeque<PendingAsk>,
    pub windows: UploadWindows,
    next_window_key: u64,
    /// The watched folders and the channel their files arrive on.
    pub watchers: Vec<modlight_core::watch::WatchFolder>,
    pub watch_events: Option<std::sync::mpsc::Receiver<(PathBuf, usize)>>,
    pub watch_settings: Vec<modlight_core::watch::WatchFolderSettings>,
}

impl Uploads {
    /// Build the task machinery over `profile`. `waker` repaints the window when a task
    /// or the queue has news. Nothing is read or written until [`AppState::start_uploads`].
    pub fn new(
        api: modlight_api::Api,
        runtime: tokio::runtime::Handle,
        profile: &Path,
        settings: AppUploadSettings,
        waker: Arc<dyn Fn() + Send + Sync>,
    ) -> Uploads {
        let account: AccountSlot = Arc::new(RwLock::new(None));
        let tus = modlight_api::TusClient::new(env!("CARGO_PKG_VERSION"))
            .expect("the transfer client starts");
        let transport = Arc::new(ApiTransport::with_tus(api.clone(), tus.clone()));
        let queue = UploadQueue::new(
            QueueStore::new(profile),
            account.clone(),
            transport,
            Arc::new(crate::platform::network::network_available),
        );
        queue.set_waker(waker.clone());
        let (channel, receiver) = TaskChannel::new(waker);
        let auto_increment = settings.name_parser_auto_increment_number;
        let ctx = TaskContext {
            api,
            tus,
            account: account.clone(),
            queue: queue.clone(),
            channel,
            runtime,
            settings: Arc::new(RwLock::new(settings)),
            personal_folder: profile.to_path_buf(),
            auto_increment: Arc::new(AtomicI64::new(auto_increment)),
            strip_png_color_space: Arc::new(AtomicBool::new(false)),
        };
        Uploads {
            tasks: TaskManager::new(None),
            ctx,
            receiver,
            queue,
            account,
            asks: VecDeque::new(),
            windows: UploadWindows::default(),
            next_window_key: 0,
            watchers: Vec::new(),
            watch_events: None,
            watch_settings: Vec::new(),
        }
    }

    pub fn window_key(&mut self) -> u64 {
        self.next_window_key += 1;
        self.next_window_key
    }

    /// Whether a window of the upload flow blocks the main window.
    pub fn modal_open(&self) -> bool {
        self.windows.multi_upload.is_some()
    }
}

impl AppState {
    /// The application settings the task flow reads, as of now.
    pub fn app_upload_settings(&self) -> AppUploadSettings {
        self.uploads.ctx.app_settings()
    }

    /// Default task settings (`TaskSettings.GetDefaultTaskSettings`).
    pub fn default_task_settings(&self) -> TaskSettings {
        TaskSettings::defaults_from(&self.config)
    }

    /// Settings aimed at one folder (`TargetedSettings`).
    pub fn targeted_settings(&self, folder: Option<String>) -> TaskSettings {
        let mut settings = self.default_task_settings();
        settings.modlight_folder_id = folder;
        settings
    }

    /// Start the queue and read the history; called once when the window opens.
    pub fn start_uploads(&mut self) {
        self.sync_upload_settings();
        self.sync_account_slot();
        self.uploads.queue.initialize(&self.rt.handle());
        // `HistoryManagerSQLite`: History.db in the personal folder.
        let path = self
            .env
            .profile
            .join(modlight_core::history::HISTORY_FILE_NAME);
        match HistoryStore::open(&path) {
            Ok(store) => self.uploads.tasks.history = Some(Arc::new(store)),
            Err(error) => log::warn!("Modlight: the history could not be opened: {error}"),
        }
        let recent: Option<Vec<modlight_core::tasks::recent::RecentTask>> = self
            .config
            .get("RecentTasks")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let settings = self.app_upload_settings();
        self.uploads
            .tasks
            .recent
            .init(settings.recent_tasks_max_count, recent);
        if settings.recent_tasks_save
            && settings.recent_tasks_show_in_main_window
            && !self.uploads.tasks.recent.tasks.is_empty()
        {
            // `UpdateMainWindowList`: the recent uploads come back as finished rows.
            let restored: Vec<_> = self.uploads.tasks.recent.tasks.iter().cloned().collect();
            for recent in restored {
                let task = modlight_core::tasks::worker::WorkerTask::history(&recent);
                let id = task.id;
                let ctx = self.uploads.ctx.clone();
                self.uploads.tasks.start(task.clone(), &ctx);
                self.upload_tab.task_finished(id, &task.snapshot());
            }
        }
        self.load_upload_history();
        self.update_watch_folders();
    }

    /// Hand the settings file's upload members to the tasks.
    pub fn sync_upload_settings(&mut self) {
        let settings = AppUploadSettings::from_config(&self.config);
        if let Ok(mut shared) = self.uploads.ctx.settings.write() {
            *shared = settings;
        }
    }

    /// Keep the queue's idea of who is signed in in step with the account.
    pub fn sync_account_slot(&mut self) {
        let wanted = self.account.credential().map(|credential| QueueAccount {
            host: credential.host,
            token: credential.token,
            tag: self.account.account().tag.clone(),
            display_name: self.account.display_name(),
        });
        let current = self.uploads.account.read().ok().map(|a| a.clone());
        if current.as_ref() != Some(&wanted)
            && let Ok(mut slot) = self.uploads.account.write()
        {
            let changed = slot.is_some() != wanted.is_some();
            *slot = wanted;
            if changed {
                self.uploads.queue.wake();
            }
        }
    }

    /// The work of one frame: account, task messages, queue events, watched folders.
    pub fn pump_uploads(&mut self, ctx: &egui::Context) {
        self.sync_account_slot();
        // `SetCurrentFolder` publishes the open folder as the ambient upload target.
        if modlight_core::upload::target::current() != self.space.current_folder {
            modlight_core::upload::target::set(self.space.current_folder.as_deref());
        }
        let messages: Vec<TaskMessage> = self.uploads.receiver.try_iter().collect();
        for message in messages {
            match message {
                TaskMessage::Event { task, event } => self.task_event(task, event, ctx),
                TaskMessage::Ask {
                    task,
                    question,
                    reply,
                } => self.task_ask(task, question, reply),
                TaskMessage::Effect(effect) => self.task_effect(effect, ctx),
            }
        }
        let mut snapshot_due = false;
        for event in self.uploads.queue.drain_events() {
            match event {
                QueueEvent::Changed | QueueEvent::Progressed => snapshot_due = true,
                QueueEvent::Finished(outcome) => {
                    snapshot_due = true;
                    let row = self.upload_tab.row_from_queue(&outcome);
                    self.upload_tab.insert(row);
                    let ctx_tasks = self.uploads.ctx.clone();
                    if self
                        .uploads
                        .tasks
                        .queued_upload_finished(&outcome, &ctx_tasks)
                    {
                        self.save_recent_tasks();
                    }
                    if outcome.succeeded {
                        // `OnQueuedUploadLanded`: the library hears about it too.
                        self.note_upload_landed(
                            outcome.item.folder_id.as_deref(),
                            outcome.item.length,
                        );
                        self.refresh_account_strip();
                    }
                }
                QueueEvent::Notice(text) => {
                    let title = format!("Modlight - {}", t(s::UploadQueue_SectionTitle));
                    self.show_notification_tip(&text, &title, None);
                }
            }
        }
        if snapshot_due || self.queue_snapshot_stale() {
            self.queue_snapshot = self.uploads.queue.snapshot();
            self.queue_snapshot_at = self.time;
        }
        if !self.queue_snapshot.is_empty() {
            // The countdowns tick once a second.
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
        self.answer_sign_in_asks();
        self.pump_watch_folders();
        self.open_next_ask();
    }

    fn queue_snapshot_stale(&self) -> bool {
        !self.queue_snapshot.is_empty() && self.time - self.queue_snapshot_at >= 1.0
    }

    // ------------------------------------------------------------------ events

    fn task_event(&mut self, id: u64, event: TaskEvent, ctx: &egui::Context) {
        let task_ctx = self.uploads.ctx.clone();
        let outcome = self.uploads.tasks.handle_event(id, event, &task_ctx);
        if outcome.changed && event != TaskEvent::UploadProgressChanged {
            // An upload that just landed changed the storage figure (throttled).
            self.refresh_account_strip();
        }
        for effect in outcome.effects {
            self.task_effect(effect, ctx);
        }
        if outcome.recent_changed {
            self.save_recent_tasks();
        }
        if outcome.finished
            && let Some(task) = self.uploads.tasks.find(id).cloned()
        {
            let state = task.snapshot();
            self.upload_tab.task_finished(id, &state);
            self.report_finished(&state);
            if self
                .app_upload_settings()
                .save_settings_after_task_completed
                && !self.uploads.tasks.is_busy()
            {
                self.save_config();
            }
        }
    }

    /// `ReportFinished`: the notice in the window's corner, and the library's tile.
    fn report_finished(&mut self, state: &modlight_core::tasks::worker::TaskState) {
        let info = &state.info;
        if !info.is_upload_job() || state.status == TaskStatus::Stopped {
            return;
        }
        let folder = info
            .modlight_landed_folder_id
            .clone()
            .or_else(|| info.modlight_folder_id.clone());
        let landed = state.status != TaskStatus::Failed
            && !matches!(
                info.job,
                modlight_core::tasks::info::TaskJob::ShortenUrl
                    | modlight_core::tasks::info::TaskJob::ShareUrl
            )
            && !info.result.url.is_empty();
        let name = crate::upload_tab::task_name(info);
        let link = if !info.result.shortened_url.is_empty() {
            info.result.shortened_url.clone()
        } else {
            info.result.url.clone()
        };
        if state.status == TaskStatus::Failed || link.is_empty() {
            let reason = crate::upload_tab::reason_for(info);
            self.toast_upload(t(s::MainWindow_ToastFailed), &name, &reason, true);
        } else {
            let place = self.space.folder_name_for(folder.as_deref());
            let title = if place.is_empty() {
                t(s::MainWindow_ToastUploaded).to_string()
            } else {
                format(t(s::MainWindow_ToastUploadedTo), &[Arg::from(place)])
            };
            self.toast_upload(&title, &name, &link, false);
        }
        if landed {
            let size = if !info.file_path().is_empty() {
                std::fs::metadata(info.file_path())
                    .map(|m| m.len() as i64)
                    .unwrap_or_else(|_| info.progress.as_ref().map(|p| p.length()).unwrap_or(0))
            } else {
                info.progress.as_ref().map(|p| p.length()).unwrap_or(0)
            };
            self.note_upload_landed(folder.as_deref(), size);
        }
    }

    fn save_recent_tasks(&mut self) {
        let settings = self.app_upload_settings();
        if settings.recent_tasks_save {
            let list = self.uploads.tasks.recent.to_vec();
            self.config.set(
                "RecentTasks",
                serde_json::to_value(list).unwrap_or(serde_json::Value::Null),
            );
        } else {
            self.config.set("RecentTasks", serde_json::Value::Null);
        }
        self.config_dirty = true;
    }

    // ------------------------------------------------------------------ questions

    fn task_ask(&mut self, task: u64, question: TaskQuestion, reply: oneshot::Sender<TaskAnswer>) {
        if matches!(question, TaskQuestion::SignIn) {
            self.show_sign_in();
        }
        // Answered at once: the save panel is modal. The editor answers when it closes.
        match question {
            TaskQuestion::SaveAs { folder, file_name } => {
                let path = self.save_as_path(&folder, &file_name);
                let _ = reply.send(TaskAnswer::SaveAs(path));
                return;
            }
            TaskQuestion::Annotate(image) => {
                self.annotate_for_task(image, false, reply);
                return;
            }
            TaskQuestion::Beautify(image) => {
                self.annotate_for_task(image, true, reply);
                return;
            }
            TaskQuestion::ImageEffects(image, settings) => {
                self.image_effects_for_task(image, *settings, reply);
                return;
            }
            _ => {}
        }
        self.uploads.asks.push_back(PendingAsk {
            task,
            question,
            reply,
        });
    }

    /// Every task waiting for a sign-in hears the answer when the window closes.
    fn answer_sign_in_asks(&mut self) {
        if self.windows.sign_in.is_some() {
            return;
        }
        let signed_in = self.account.is_signed_in();
        let mut kept = VecDeque::new();
        while let Some(ask) = self.uploads.asks.pop_front() {
            if matches!(ask.question, TaskQuestion::SignIn) {
                self.sync_account_slot();
                let _ = ask.reply.send(TaskAnswer::SignIn(signed_in));
            } else {
                kept.push_back(ask);
            }
        }
        self.uploads.asks = kept;
    }

    /// Show the window for the oldest question nobody is looking at yet.
    fn open_next_ask(&mut self) {
        let windows = &self.uploads.windows;
        if windows.large_file.is_some()
            || windows.file_exists.is_some()
            || windows.before_upload.is_some()
        {
            return;
        }
        let Some(ask) = self
            .uploads
            .asks
            .iter()
            .find(|a| !matches!(a.question, TaskQuestion::SignIn))
        else {
            return;
        };
        let signed_in = self.account.is_signed_in();
        match &ask.question {
            TaskQuestion::LargeFile { .. } => {
                self.uploads.windows.large_file =
                    Some(UploadConfirm::new(UploadQuestion::LargeFile));
            }
            TaskQuestion::FileExists { path } => {
                let unique = modlight_core::tasks::naming::unique_file_path(path);
                self.uploads.windows.file_exists = Some(FileExists::new(path.clone(), unique));
            }
            TaskQuestion::BeforeUpload(info) => {
                self.uploads.windows.before_upload =
                    Some(BeforeUpload::new(ask.task, (**info).clone(), signed_in));
            }
            TaskQuestion::SignIn
            | TaskQuestion::SaveAs { .. }
            | TaskQuestion::Annotate(_)
            | TaskQuestion::Beautify(_)
            | TaskQuestion::ImageEffects(..) => {}
        }
    }

    /// `SaveImageToFileWithDialog`: the save panel, starting in the last folder it saved
    /// to, else the screenshots folder (`HelpersOptions.LastSaveDirectory`). QA and
    /// headless runs save into `<profile>/Saved` without a panel.
    fn save_as_path(&mut self, folder: &Path, file_name: &str) -> Option<PathBuf> {
        if self.env.qa || self.mode != crate::app::Mode::Normal {
            let folder = self.env.profile.join("Saved");
            let _ = std::fs::create_dir_all(&folder);
            return Some(folder.join(file_name));
        }
        let last = self
            .config
            .get("LastSaveDirectory")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .filter(|p| p.is_dir());
        let start = last.unwrap_or_else(|| folder.to_path_buf());
        let extension = Path::new(file_name)
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut dialog = rfd::FileDialog::new()
            .set_directory(&start)
            .set_file_name(file_name)
            .set_title(format!(
                "{} {file_name}",
                t(s::UploadTask_DoAfterCaptureJobs_Choose_a_folder_to_save)
            ));
        if !extension.is_empty() {
            dialog = dialog.add_filter(format!("*.{extension}"), &[extension.as_str()]);
        }
        let path = dialog.save_file()?;
        if let Some(parent) = path.parent() {
            self.config.set(
                "LastSaveDirectory",
                serde_json::json!(parent.to_string_lossy()),
            );
            self.config_dirty = true;
        }
        Some(path)
    }

    /// Answer the oldest question of this kind.
    fn answer(&mut self, matches: impl Fn(&TaskQuestion) -> bool, answer: TaskAnswer) {
        if let Some(index) = self.uploads.asks.iter().position(|a| matches(&a.question))
            && let Some(ask) = self.uploads.asks.remove(index)
        {
            let _ = ask.reply.send(answer);
        }
    }

    pub fn large_file_finished(&mut self, outcome: UploadConfirmOutcome) {
        self.uploads.windows.large_file = None;
        self.answer(
            |q| matches!(q, TaskQuestion::LargeFile { .. }),
            TaskAnswer::LargeFile {
                should_continue: outcome.go_on,
                dont_show_again: outcome.dont_show_again,
            },
        );
    }

    pub fn file_exists_finished(&mut self, path: Option<PathBuf>) {
        self.uploads.windows.file_exists = None;
        self.answer(
            |q| matches!(q, TaskQuestion::FileExists { .. }),
            TaskAnswer::FileExists(path),
        );
    }

    pub fn before_upload_finished(&mut self, outcome: BeforeUploadOutcome) {
        let Some(window) = self.uploads.windows.before_upload.as_ref() else {
            return;
        };
        let task = window.task;
        let answer = match outcome {
            BeforeUploadOutcome::CopyImage => {
                if let Some(image) = window.image.clone() {
                    self.effects.push(Effect::CopyImage(image));
                }
                return;
            }
            BeforeUploadOutcome::Upload(settings) => TaskAnswer::BeforeUpload {
                accepted: true,
                settings,
            },
            BeforeUploadOutcome::Cancel => TaskAnswer::BeforeUpload {
                accepted: false,
                settings: Box::new(window.info.settings.clone()),
            },
        };
        self.uploads.windows.before_upload = None;
        if let Some(index) = self
            .uploads
            .asks
            .iter()
            .position(|a| a.task == task && matches!(a.question, TaskQuestion::BeforeUpload(_)))
            && let Some(ask) = self.uploads.asks.remove(index)
        {
            let _ = ask.reply.send(answer);
        }
    }

    // ------------------------------------------------------------------ effects

    pub fn task_effect(&mut self, effect: TaskEffect, _ctx: &egui::Context) {
        match effect {
            TaskEffect::CopyText(text) => self.copy_text(&text),
            TaskEffect::CopyImage(image) => self.effects.push(Effect::CopyImage(image)),
            TaskEffect::CopyFile(path) => self.effects.push(Effect::CopyFiles(vec![path])),
            TaskEffect::ClearClipboard => self.effects.push(Effect::ClearClipboard),
            TaskEffect::OpenUrl(url) => self.open_url(&url),
            TaskEffect::RevealFile(path) => self.effects.push(Effect::Reveal(path)),
            TaskEffect::PlaySound { sound, custom_path } => {
                self.effects.push(Effect::PlaySound(sound, custom_path))
            }
            TaskEffect::ShowNotification(config) => self.show_task_notification(*config),
            TaskEffect::ShowNotificationTip {
                text,
                title,
                duration_ms,
            } => self.show_notification_tip(&text, &title, duration_ms),
            TaskEffect::ShowAfterUploadWindow(info) => self.show_after_upload(*info),
            TaskEffect::MessageBox { text, title, icon } => self
                .uploads
                .windows
                .messages
                .push_back(MessageBox::new(&title, &text, icon)),
            TaskEffect::ShowSignIn => self.show_sign_in(),
            // `TaskHelpers.PinToScreen(image)` (phase 5, `tools::pin`).
            TaskEffect::PinToScreen(image) => {
                let settings = self.default_task_settings();
                self.pin_picture(image, &settings);
            }
            // The other tools a task hands work to (phase 5, `tools`).
            TaskEffect::Ocr(image, settings) => self.ocr_image((*image).clone(), None, *settings),
            TaskEffect::AnalyzeImage(file, settings) => self.task_analyze_image(file, *settings),
            TaskEffect::ScanQrCode(file) => self.open_qr_code_scan(Some(file), None),
            TaskEffect::PrintImage(image) => self.print_image((*image).clone()),
            TaskEffect::DisableLargeFileWarning => {
                self.config
                    .set("ShowLargeFileSizeWarning", serde_json::json!(0));
                self.save_config();
                self.sync_upload_settings();
            }
            TaskEffect::SetAutoIncrement(value) => {
                self.config
                    .set("NameParserAutoIncrementNumber", serde_json::json!(value));
                self.config_dirty = true;
            }
            TaskEffect::DisableMultiUploadWarning => {
                self.config
                    .set("ShowMultiUploadWarning", serde_json::json!(false));
                self.save_config();
                self.sync_upload_settings();
            }
            TaskEffect::SetFileUploadDefaultDirectory(folder) => {
                self.config
                    .set("FileUploadDefaultDirectory", serde_json::json!(folder));
                self.save_config();
                self.sync_upload_settings();
            }
        }
    }

    /// `NotificationWindow.RunAction`: what a click on the notice or one of its buttons
    /// does. The picture is copied before the notice goes; everything else after.
    pub fn notification_action(&mut self, action: crate::notification::ToastAction, dismiss: bool) {
        use crate::notification::ToastAction;
        let Some(notice) = self.windows.notification.as_ref() else {
            return;
        };
        let path = notice.file_path.clone();
        let url = notice.url.clone();
        let image = notice.image.clone();
        if action == ToastAction::CopyImageToClipboard {
            self.copy_notice_image(&path, image);
        }
        if dismiss {
            self.windows.notification = None;
        }
        let file = PathBuf::from(&path);
        match action {
            ToastAction::CloseNotification | ToastAction::CopyImageToClipboard => {}
            // `AnnotateImageFromFile`; pinning arrives with phase 5.
            ToastAction::AnnotateImage => {
                if !path.is_empty() {
                    self.annotate_file(&file);
                }
            }
            // `TaskHelpers.PinToScreen(filePath)` (phase 5, `tools::pin`).
            ToastAction::PinToScreen => {
                if !path.is_empty() {
                    let settings = self.default_task_settings();
                    self.pin_file(&file, &settings);
                }
            }
            ToastAction::CopyFile => {
                if !path.is_empty() {
                    self.effects.push(Effect::CopyFiles(vec![file]));
                }
            }
            ToastAction::CopyFilePath => {
                if !path.is_empty() {
                    self.copy_text(&path);
                }
            }
            ToastAction::CopyUrl => {
                let text = if url.is_empty() { path } else { url };
                self.copy_text(&text);
            }
            ToastAction::OpenFile => {
                if !path.is_empty() {
                    self.effects.push(Effect::OpenFile(file));
                }
            }
            ToastAction::OpenFolder => {
                if !path.is_empty() {
                    self.effects.push(Effect::Reveal(file));
                }
            }
            ToastAction::OpenUrl => {
                if !url.is_empty() {
                    self.open_url(&url);
                } else if !path.is_empty() {
                    self.effects.push(Effect::OpenFile(file));
                }
            }
            ToastAction::Upload => {
                if !path.is_empty() {
                    let settings = self.default_task_settings();
                    self.upload_files_with(vec![file], settings);
                }
            }
            ToastAction::DeleteFile => {
                if !path.is_empty() && file.is_file() {
                    self.windows.confirm = Some(crate::dialogs::confirm::Confirm::new(
                        t(s::MainForm_tsmiDeleteSelectedFile_Click_File_delete_confirmation),
                        t(s::MainForm_tsmiDeleteSelectedFile_Click_Do_you_really_want_to_delete_this_file_),
                        crate::dialogs::confirm::ConfirmAction::DeleteLocalFile(file),
                    ));
                }
            }
        }
    }

    /// `CopyImageToClipboard`: the file on disk first, the picture in memory as the
    /// fallback, never a path or a link; when there is neither, the person is told.
    fn copy_notice_image(&mut self, path: &str, image: Option<Arc<image::RgbaImage>>) {
        let from_file = (!path.is_empty()
            && modlight_core::names::file_helpers::is_image_file(path))
        .then(|| modlight_core::image_decode::open(Path::new(path)).ok())
        .flatten()
        .map(|picture| Arc::new(picture.into_rgba8()));
        match from_file.or(image) {
            Some(picture) => self.effects.push(Effect::CopyImage(picture)),
            None => {
                self.show_notification_tip(t(s::Notification_CopyImageFailed), "Modlight", None)
            }
        }
    }

    /// `NotificationWindow.Show` for a finished task.
    fn show_task_notification(&mut self, config: modlight_core::tasks::host::NotificationConfig) {
        self.windows.notification =
            Some(crate::notification::Notification::task(config, self.time));
    }

    fn show_after_upload(&mut self, info: modlight_core::tasks::info::TaskInfo) {
        let custom: Vec<(String, String)> = self
            .config
            .get("ClipboardContentFormats")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|f| {
                        Some((
                            f.get("Description")?.as_str().unwrap_or("").to_string(),
                            f.get("Format")?.as_str().unwrap_or("").to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let ctx = self.uploads.ctx.clone();
        let formats = create_formats(&info, &custom, &|pattern| {
            modlight_core::tasks::naming::parse_upload_info(&info, pattern, &ctx)
        });
        let auto_close = info.settings.advanced_settings.auto_close_after_upload_form;
        let key = self.uploads.window_key();
        self.uploads
            .windows
            .after_uploads
            .push(AfterUpload::new(key, info, formats, auto_close));
    }

    pub fn after_upload_action(&mut self, key: u64, action: AfterUploadAction) {
        let Some(index) = self
            .uploads
            .windows
            .after_uploads
            .iter()
            .position(|w| w.key == key)
        else {
            return;
        };
        match action {
            AfterUploadAction::CopyImage => {
                if let Some(image) = self.uploads.windows.after_uploads[index].image.clone() {
                    self.effects.push(Effect::CopyImage(image));
                }
            }
            AfterUploadAction::CopyText(text) => self.copy_text(&text),
            AfterUploadAction::OpenUrl(url) => self.open_url(&url),
            AfterUploadAction::OpenFile(path) => self.effects.push(Effect::OpenFile(path)),
            AfterUploadAction::RevealFile(path) => self.effects.push(Effect::Reveal(path)),
            AfterUploadAction::Close => {
                self.uploads.windows.after_uploads.remove(index);
            }
        }
    }

    /// Fetch an after-upload window's picture from its link (`LoadRemotePreviewAsync`).
    pub fn load_after_upload_preview(&mut self, key: u64, url: String) {
        let api = self.api.clone();
        self.rt.spawn(async move {
            let image = api
                .download_image(&url)
                .await
                .and_then(|picture| modlight_core::image_decode::from_bytes(&picture.bytes).ok())
                .map(|picture| picture.into_rgba8());
            Box::new(move |state: &mut AppState, _ctx: &egui::Context| {
                if let Some(image) = image
                    && let Some(window) = state
                        .uploads
                        .windows
                        .after_uploads
                        .iter_mut()
                        .find(|w| w.key == key)
                {
                    window.set_image(Arc::new(image));
                }
            })
        });
    }

    // ------------------------------------------------------------------ entry points

    /// `UploadFile(string[])`: many files ask first, unless told not to.
    pub fn upload_files_with(&mut self, files: Vec<PathBuf>, settings: TaskSettings) {
        if files.is_empty() {
            return;
        }
        if self.needs_multi_upload_question(files.len()) {
            self.ask_multi_upload(
                PendingFiles {
                    files,
                    rest: Vec::new(),
                    settings,
                },
                false,
            );
            return;
        }
        self.upload_each(files, settings);
    }

    fn needs_multi_upload_question(&self, count: usize) -> bool {
        upload_manager::needs_confirmation(
            count,
            self.app_upload_settings().show_multi_upload_warning,
        )
    }

    /// `foreach (file in files) UploadFile(file)`: a file becomes a task, a folder
    /// becomes its files, every level down, and asks first when there are many; the
    /// paths after such a folder wait for the answer.
    fn upload_each(&mut self, files: Vec<PathBuf>, settings: TaskSettings) {
        let ctx = self.uploads.ctx.clone();
        let mut paths = files.into_iter();
        while let Some(path) = paths.next() {
            if path.is_file() {
                upload_manager::upload_file(&mut self.uploads.tasks, &path, &settings, &ctx);
            } else if path.is_dir() {
                let inner = upload_manager::expand_paths(std::slice::from_ref(&path));
                if inner.is_empty() {
                    continue;
                }
                if self.needs_multi_upload_question(inner.len()) {
                    let rest = paths.collect();
                    self.ask_multi_upload(
                        PendingFiles {
                            files: inner,
                            rest,
                            settings,
                        },
                        true,
                    );
                    return;
                }
                upload_manager::upload_files(&mut self.uploads.tasks, &inner, &settings, &ctx);
            }
        }
    }

    /// Show the many-files question, or line it up behind the one that is showing. A
    /// question raised while an answer is being carried out goes first.
    fn ask_multi_upload(&mut self, pending: PendingFiles, nested: bool) {
        let windows = &mut self.uploads.windows;
        if windows.multi_upload.is_none() {
            let question = UploadConfirm::new(UploadQuestion::ManyFiles(pending.files.len()));
            windows.multi_upload = Some((question, pending));
        } else if nested {
            windows.multi_upload_waiting.push_front(pending);
        } else {
            windows.multi_upload_waiting.push_back(pending);
        }
    }

    pub fn multi_upload_finished(&mut self, outcome: UploadConfirmOutcome) {
        let Some((_, pending)) = self.uploads.windows.multi_upload.take() else {
            return;
        };
        // `ShowMultiUploadWarning = !result.DontShowAgain`.
        if outcome.dont_show_again {
            self.config
                .set("ShowMultiUploadWarning", serde_json::json!(false));
            self.save_config();
            self.sync_upload_settings();
        }
        let mut paths = Vec::new();
        if outcome.go_on {
            paths.extend(pending.files);
        }
        paths.extend(pending.rest);
        self.upload_each(paths, pending.settings);
        if self.uploads.windows.multi_upload.is_none()
            && let Some(next) = self.uploads.windows.multi_upload_waiting.pop_front()
        {
            let question = UploadConfirm::new(UploadQuestion::ManyFiles(next.files.len()));
            self.uploads.windows.multi_upload = Some((question, next));
        }
    }

    /// Files dropped on a folder tile (`TargetedSettings(folder.Id)`).
    pub fn upload_paths_to(&mut self, paths: Vec<PathBuf>, folder: Option<String>) {
        let settings = self.targeted_settings(folder);
        self.upload_files_with(paths, settings);
    }

    /// Files dropped anywhere else on the window (`DragDropUpload` with the defaults).
    pub fn drop_on_window(&mut self, paths: Vec<PathBuf>) {
        let settings = self.default_task_settings();
        self.upload_files_with(paths, settings);
    }

    /// The file picker (`UploadManager.UploadFile()`), into `folder` when one is named.
    pub fn pick_and_upload(&mut self, folder: Option<String>) {
        if self.env.qa {
            return;
        }
        if let Some(id) = folder.as_deref() {
            self.space.select_folder_tile(Some(id));
        }
        let dialog = rfd::FileDialog::new().set_title(format!(
            "Modlight - {}",
            t(s::UploadManager_UploadFile_File_upload)
        ));
        let Some(paths) = self.starting_in_last_folder(dialog).pick_files() else {
            return;
        };
        if let Some(parent) = paths.first().and_then(|p| p.parent()) {
            self.task_effect(
                TaskEffect::SetFileUploadDefaultDirectory(parent.to_string_lossy().into_owned()),
                &egui::Context::default(),
            );
        }
        let settings = self.targeted_settings(folder);
        self.upload_files_with(paths, settings);
    }

    /// `UploadFolder`.
    pub fn pick_folder_and_upload(&mut self) {
        if self.env.qa {
            return;
        }
        let dialog = rfd::FileDialog::new().set_title(format!(
            "Modlight - {}",
            t(s::UploadManager_UploadFolder_Folder_upload)
        ));
        let Some(folder) = self.starting_in_last_folder(dialog).pick_folder() else {
            return;
        };
        self.task_effect(
            TaskEffect::SetFileUploadDefaultDirectory(folder.to_string_lossy().into_owned()),
            &egui::Context::default(),
        );
        // `UploadFile(selectedPath)`: the folder's files, asked about when many.
        let settings = self.default_task_settings();
        self.upload_files_with(vec![folder], settings);
    }

    /// The picker starts in `FileUploadDefaultDirectory` when it exists, else on the
    /// Desktop.
    fn starting_in_last_folder(&self, dialog: rfd::FileDialog) -> rfd::FileDialog {
        let app = self.app_upload_settings();
        if let Some(directory) = app
            .file_upload_default_directory
            .as_deref()
            .filter(|d| !d.is_empty() && Path::new(d).is_dir())
        {
            dialog.set_directory(directory)
        } else if let Some(desktop) = dirs::desktop_dir() {
            dialog.set_directory(desktop)
        } else {
            dialog
        }
    }

    /// Read the clipboard. On a Mac a copied file also leaves its icon and its name on
    /// the pasteboard, so the file list is asked for first; otherwise upstream's order.
    pub fn read_clipboard(&mut self) -> Option<ClipboardContent> {
        if !matches!(self.mode, crate::app::Mode::Normal | crate::app::Mode::Qa) || self.env.qa {
            return self.qa_clipboard.clone();
        }
        let mut clipboard = arboard::Clipboard::new().ok()?;
        if let Ok(files) = clipboard.get().file_list()
            && !files.is_empty()
        {
            return Some(ClipboardContent::Files(files));
        }
        // The picture's bytes, decoded here under the app's limits (arboard's own
        // decoding has none).
        if let Some(bytes) = crate::platform::pasteboard::image_bytes() {
            match modlight_core::image_decode::from_bytes(&bytes) {
                Ok(picture) => return Some(ClipboardContent::Image(picture.into_rgba8())),
                Err(error) => log::warn!("Modlight: the clipboard's picture was not read: {error}"),
            }
        }
        if let Ok(text) = clipboard.get_text()
            && !text.is_empty()
        {
            return Some(ClipboardContent::Text(text));
        }
        None
    }

    /// The clipboard's text, for the windows that start from it (`GetText`): read even
    /// when a picture or files are on the clipboard too.
    pub fn clipboard_text(&mut self) -> Option<String> {
        if !matches!(self.mode, crate::app::Mode::Normal | crate::app::Mode::Qa) || self.env.qa {
            return match &self.qa_clipboard {
                Some(ClipboardContent::Text(text)) => Some(text.clone()),
                _ => None,
            };
        }
        arboard::Clipboard::new()
            .ok()?
            .get_text()
            .ok()
            .filter(|text| !text.is_empty())
    }

    /// `ClipboardUploadMainWindow`: the viewer first, unless told not to show it.
    pub fn clipboard_upload_main_window(&mut self) {
        let settings = self.default_task_settings();
        if self.app_upload_settings().show_clipboard_content_viewer {
            self.open_clipboard_viewer(settings, true);
        } else {
            self.clipboard_upload(settings);
        }
    }

    /// `ClipboardUploadWithContentViewer`.
    pub fn open_clipboard_viewer(&mut self, settings: TaskSettings, show_dont_show_again: bool) {
        if self.uploads.windows.clipboard_upload.is_some() {
            return;
        }
        let content = self.read_clipboard();
        self.uploads.windows.clipboard_upload = Some((
            ClipboardUpload::new(content, show_dont_show_again),
            settings,
        ));
    }

    pub fn clipboard_viewer_finished(&mut self, outcome: ClipboardOutcome) {
        let Some((window, settings)) = self.uploads.windows.clipboard_upload.take() else {
            return;
        };
        if window.show_dont_show_again && window.dont_show_again {
            self.config
                .set("ShowClipboardContentViewer", serde_json::json!(false));
            self.save_config();
            self.sync_upload_settings();
        }
        if outcome == ClipboardOutcome::Upload
            && let Some(content) = window.content
        {
            self.run_clipboard_content(content, settings);
        }
    }

    /// `ClipboardUpload`: whatever is there, straight away.
    pub fn clipboard_upload(&mut self, settings: TaskSettings) {
        if let Some(content) = self.read_clipboard() {
            self.run_clipboard_content(content, settings);
        }
    }

    fn run_clipboard_content(&mut self, content: ClipboardContent, settings: TaskSettings) {
        match content {
            ClipboardContent::Files(files) => self.upload_files_with(files, settings),
            other => {
                let ctx = self.uploads.ctx.clone();
                upload_manager::clipboard_upload(&mut self.uploads.tasks, other, &settings, &ctx);
            }
        }
    }

    pub fn open_paste_window(&mut self) {
        if self.uploads.windows.paste.is_none() {
            let text = self.clipboard_text();
            self.uploads.windows.paste = Some(PasteWindow::new(text));
        }
    }

    pub fn paste_finished(&mut self, outcome: PasteOutcome) {
        self.uploads.windows.paste = None;
        if let PasteOutcome::Send(bytes, name) = outcome {
            // A data upload of kind File: the name carries the language.
            let settings = self.default_task_settings();
            let ctx = self.uploads.ctx.clone();
            upload_manager::upload_data(
                &mut self.uploads.tasks,
                DataType::File,
                bytes,
                &name,
                &settings,
                &ctx,
            );
        }
    }

    pub fn open_text_upload(&mut self) {
        if self.uploads.windows.text_upload.is_none() {
            let text = self.clipboard_text();
            self.uploads.windows.text_upload = Some(TextUpload::new(text));
        }
    }

    pub fn text_upload_finished(&mut self, text: Option<String>) {
        self.uploads.windows.text_upload = None;
        if let Some(text) = text.filter(|t| !t.is_empty()) {
            let settings = self.default_task_settings();
            let ctx = self.uploads.ctx.clone();
            upload_manager::upload_text(&mut self.uploads.tasks, &text, &settings, false, &ctx);
        }
    }

    pub fn open_url_upload(&mut self) {
        if self.uploads.windows.url_upload.is_none() {
            let text = self
                .clipboard_text()
                .map(|t| t.trim().to_string())
                .filter(|t| modlight_core::names::url_helpers::is_valid_url(t));
            self.uploads.windows.url_upload = Some(UrlUpload::new(text.as_deref()));
        }
    }

    pub fn url_upload_finished(&mut self, url: Option<String>) {
        self.uploads.windows.url_upload = None;
        if let Some(url) = url {
            let settings = self.default_task_settings();
            let ctx = self.uploads.ctx.clone();
            upload_manager::download_and_upload(&mut self.uploads.tasks, &url, &settings, &ctx);
        }
    }

    pub fn open_shorten_url(&mut self) {
        if self.uploads.windows.shorten_url.is_none() {
            let text = self
                .clipboard_text()
                .map(|t| t.trim().to_string())
                .filter(|t| modlight_core::names::url_helpers::is_valid_url(t));
            self.uploads.windows.shorten_url = Some(ShortenUrl::new(text.as_deref()));
        }
    }

    pub fn shorten_url_finished(
        &mut self,
        request: Option<crate::dialogs::shorten_url::ShortenRequest>,
    ) {
        self.uploads.windows.shorten_url = None;
        if let Some(request) = request {
            let mut settings = self.default_task_settings();
            settings.modlight_short_link = Some(request.options);
            let ctx = self.uploads.ctx.clone();
            upload_manager::shorten_url(&mut self.uploads.tasks, &request.url, &settings, &ctx);
        }
    }

    /// `TaskHelpers.OpenDropWindow`.
    pub fn open_drop_window(&mut self) {
        let app = self.app_upload_settings();
        self.uploads.windows.drop_window = Some(DropWindow::new(
            app.drop_size,
            app.drop_offset,
            app.drop_alignment,
            app.drop_opacity,
            app.drop_hover_opacity,
        ));
    }

    pub fn drop_window_outcome(&mut self, outcome: DropOutcome) {
        match outcome {
            DropOutcome::Close => self.uploads.windows.drop_window = None,
            DropOutcome::Dropped(files) => {
                let settings = self.default_task_settings();
                self.upload_files_with(files, settings);
            }
            DropOutcome::StartDrag => {}
        }
    }

    /// The Upload tab's history: read the store once, off the window's thread.
    pub fn load_upload_history(&mut self) {
        let Some(history) = self.uploads.tasks.history.clone() else {
            return;
        };
        self.upload_tab.history_read = true;
        self.upload_tab.loading_history = true;
        self.rt.spawn_blocking(move || {
            let items = history.load();
            Box::new(move |state: &mut AppState, _ctx: &egui::Context| {
                state.upload_tab.loading_history = false;
                match items {
                    Ok(mut items) => {
                        items.retain(|item| !item.url.is_empty());
                        items.sort_by_key(|item| std::cmp::Reverse(item.date_time));
                        items.truncate(crate::upload_tab::MAX_HISTORY_ROWS);
                        for item in &items {
                            let row = state.upload_tab.row_from_store(item);
                            state.upload_tab.insert(row);
                        }
                    }
                    Err(error) => {
                        state.upload_tab.history_error = format(
                            t(s::Upload_HistoryUnreadable),
                            &[Arg::from(error.to_string())],
                        );
                    }
                }
            })
        });
    }

    pub fn refresh_upload_history(&mut self) {
        self.upload_tab.forget_stored();
        self.load_upload_history();
    }

    pub fn copy_history_link(&mut self, key: u64) {
        let time = self.time;
        if let Some(row) = self.upload_tab.history.iter_mut().find(|r| r.key == key) {
            let link = row.link.clone();
            row.copied_until = time + COPY_ACKNOWLEDGEMENT;
            self.copy_text(&link);
        }
    }

    pub fn retry_upload(&mut self, path: PathBuf) {
        if path.is_file() {
            let settings = self.default_task_settings();
            let ctx = self.uploads.ctx.clone();
            upload_manager::upload_file(&mut self.uploads.tasks, &path, &settings, &ctx);
        }
    }

    pub fn cancel_task(&mut self, id: u64) {
        let ctx = self.uploads.ctx.clone();
        if let Some(task) = self.uploads.tasks.find(id) {
            task.stop(&ctx);
        }
    }

    /// Play one of upstream's sounds, or the person's own file.
    pub fn play_sound(sound: NotificationSound, custom: Option<String>) {
        // A custom sound is played and nothing else, even when it cannot be.
        if let Some(path) = custom.filter(|p| !p.is_empty()) {
            if !crate::platform::sound::play_file(Path::new(&path)) {
                log::warn!("Modlight: the custom sound could not be played: {path}");
            }
            return;
        }
        let bytes: &[u8] = match sound {
            NotificationSound::Capture => {
                include_bytes!("../../../assets/upstream/sounds/CaptureSound.wav")
            }
            NotificationSound::TaskCompleted => {
                include_bytes!("../../../assets/upstream/sounds/TaskCompletedSound.wav")
            }
            NotificationSound::ActionCompleted => {
                include_bytes!("../../../assets/upstream/sounds/ActionCompletedSound.wav")
            }
            NotificationSound::Error => {
                include_bytes!("../../../assets/upstream/sounds/ErrorSound.wav")
            }
        };
        crate::platform::sound::play_bytes(bytes);
    }

    // ------------------------------------------------------------------ watch folders

    /// `WatchFolderManager.UpdateWatchFolders`: the default task settings' folders.
    pub fn update_watch_folders(&mut self) {
        self.uploads.watchers.clear();
        self.uploads.watch_events = None;
        if self.env.qa || self.mode != crate::app::Mode::Normal {
            return;
        }
        let settings = self.default_task_settings();
        if !settings.watch_folder_enabled {
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let waker = self.uploads.ctx.channel.clone();
        for (index, folder) in settings.watch_folder_list.iter().enumerate() {
            let sender = sender.clone();
            let waker = waker.clone();
            let path = modlight_core::tasks::naming::expand_folder_variables(
                folder.folder_path.as_deref().unwrap_or_default(),
            );
            match modlight_core::watch::WatchFolder::start(
                Path::new(&path),
                folder,
                move |path: PathBuf| {
                    let _ = sender.send((path, index));
                    waker.wake();
                },
            ) {
                Ok(watcher) => self.uploads.watchers.push(watcher),
                Err(error) => log::warn!("Modlight: a watch folder could not be started: {error}"),
            }
        }
        self.uploads.watch_settings = settings.watch_folder_list.clone();
        self.uploads.watch_events = Some(receiver);
    }

    fn pump_watch_folders(&mut self) {
        let Some(receiver) = self.uploads.watch_events.as_ref() else {
            return;
        };
        let arrived: Vec<(PathBuf, usize)> = receiver.try_iter().collect();
        for (path, index) in arrived {
            let Some(folder) = self.uploads.watch_settings.get(index).cloned() else {
                continue;
            };
            let settings = self.default_task_settings();
            let mut destination = path.clone();
            if folder.move_files_to_screenshots_folder {
                let ctx = self.uploads.ctx.clone();
                let target = ctx
                    .screenshots_folder(&settings, None)
                    .join(path.file_name().unwrap_or_default());
                // `HandleExistsFile` without a window: a watched folder runs unattended,
                // so an existing name takes a unique one rather than asking.
                let target = if target.exists() {
                    modlight_core::tasks::naming::unique_file_path(&target)
                } else {
                    target
                };
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                // `File.Move` also moves between volumes, by copying and deleting.
                let moved = std::fs::rename(&path, &target).is_ok()
                    || (std::fs::copy(&path, &target).is_ok()
                        && std::fs::remove_file(&path).is_ok());
                if moved {
                    destination = target;
                } else {
                    log::warn!(
                        "Modlight: {} could not be moved to the screenshots folder.",
                        path.display()
                    );
                }
            }
            let ctx = self.uploads.ctx.clone();
            upload_manager::upload_file(&mut self.uploads.tasks, &destination, &settings, &ctx);
        }
    }
}

/// The message box effect's icon names, for logs.
pub fn icon_name(icon: MessageIcon) -> &'static str {
    match icon {
        MessageIcon::Information => "information",
        MessageIcon::Warning => "warning",
        MessageIcon::Error => "error",
    }
}

/// `t` for a format string with one number, used by the windows' titles.
pub fn count_text(template: &str, count: usize) -> String {
    format(template, &[Arg::from(count)])
}

// ---------------------------------------------------------------------- menus

use crate::commands::Command;
use crate::menus::Entry;
use modlight_core::icons;
use modlight_core::l10n::helpers as h;
use modlight_core::tasks::settings::{AfterCaptureTasks, AfterUploadTasks, FlagNames};

/// `MainMenuBuilder.BuildUploadMenu`.
pub fn upload_menu() -> Vec<Entry> {
    vec![
        Entry::item(
            t(s::MainMenuBuilder_UploadFile),
            icons::FILE_UP,
            Command::UploadTo(None),
        ),
        Entry::item(
            t(s::MainMenuBuilder_UploadFolder),
            icons::FOLDER_UP,
            Command::UploadFolder,
        ),
        Entry::item(
            t(s::MainMenuBuilder_UploadClipboard),
            icons::CLIPBOARD,
            Command::ClipboardUpload,
        ),
        Entry::item(
            t(s::MainMenuBuilder_UploadText),
            icons::FILE_TEXT,
            Command::UploadText,
        ),
        Entry::item(
            t(s::MainMenuBuilder_UploadUrl),
            icons::LINK,
            Command::UploadUrl,
        ),
        Entry::item(
            t(s::MainMenuBuilder_DragAndDropUpload),
            icons::MOUSE_POINTER_2,
            Command::DragDropUpload,
        ),
        Entry::item(
            t(s::MainMenuBuilder_ShortenUrl),
            icons::LINK_2,
            Command::ShortenUrl,
        ),
    ]
}

/// The after-upload task toggles (`BuildAfterUploadMenu`), in the enum's order.
pub fn after_upload_menu(current: AfterUploadTasks) -> Vec<Entry> {
    let options: [(AfterUploadTasks, &str, char); 6] = [
        (
            AfterUploadTasks::ShowAfterUploadWindow,
            t(h::AfterUploadTasks_ShowAfterUploadWindow),
            icons::APP_WINDOW,
        ),
        (
            AfterUploadTasks::UseURLShortener,
            t(h::AfterUploadTasks_UseURLShortener),
            icons::LINK_2,
        ),
        (
            AfterUploadTasks::ShareURL,
            t(h::AfterUploadTasks_ShareURL),
            icons::SHARE_2,
        ),
        (
            AfterUploadTasks::CopyURLToClipboard,
            t(h::AfterUploadTasks_CopyURLToClipboard),
            icons::CLIPBOARD_COPY,
        ),
        (
            AfterUploadTasks::OpenURL,
            t(h::AfterUploadTasks_OpenURL),
            icons::EXTERNAL_LINK,
        ),
        (
            AfterUploadTasks::ShowQRCode,
            t(h::AfterUploadTasks_ShowQRCode),
            icons::QR_CODE,
        ),
    ];
    options
        .into_iter()
        .map(|(task, label, icon)| {
            Entry::check(
                label,
                icon,
                current.has(task),
                Command::ToggleAfterUploadTask(task.bits()),
            )
        })
        .collect()
}

/// The after-capture task toggles (`BuildAfterCaptureMenu`), in the enum's order.
pub fn after_capture_menu(current: AfterCaptureTasks) -> Vec<Entry> {
    let options: [(AfterCaptureTasks, &str, char); 22] = [
        (
            AfterCaptureTasks::ShowQuickTaskMenu,
            t(h::AfterCaptureTasks_ShowQuickTaskMenu),
            icons::MENU,
        ),
        (
            AfterCaptureTasks::ShowAfterCaptureWindow,
            t(h::AfterCaptureTasks_ShowAfterCaptureWindow),
            icons::APP_WINDOW,
        ),
        (
            AfterCaptureTasks::BeautifyImage,
            t(h::AfterCaptureTasks_BeautifyImage),
            icons::SPARKLES,
        ),
        (
            AfterCaptureTasks::AddImageEffects,
            t(h::AfterCaptureTasks_AddImageEffects),
            icons::WAND_SPARKLES,
        ),
        (
            AfterCaptureTasks::AnnotateImage,
            t(h::AfterCaptureTasks_AnnotateImage),
            icons::PEN_LINE,
        ),
        (
            AfterCaptureTasks::CopyImageToClipboard,
            t(h::AfterCaptureTasks_CopyImageToClipboard),
            icons::CLIPBOARD_COPY,
        ),
        (
            AfterCaptureTasks::PinToScreen,
            t(h::AfterCaptureTasks_PinToScreen),
            icons::PIN,
        ),
        (
            AfterCaptureTasks::SendImageToPrinter,
            t(h::AfterCaptureTasks_SendImageToPrinter),
            icons::PRINTER,
        ),
        (
            AfterCaptureTasks::SaveImageToFile,
            t(h::AfterCaptureTasks_SaveImageToFile),
            icons::SAVE,
        ),
        (
            AfterCaptureTasks::SaveImageToFileWithDialog,
            t(h::AfterCaptureTasks_SaveImageToFileWithDialog),
            icons::SAVE_PEN,
        ),
        (
            AfterCaptureTasks::SaveThumbnailImageToFile,
            t(h::AfterCaptureTasks_SaveThumbnailImageToFile),
            icons::IMAGE_DOWN,
        ),
        (
            AfterCaptureTasks::PerformActions,
            t(h::AfterCaptureTasks_PerformActions),
            icons::TERMINAL,
        ),
        (
            AfterCaptureTasks::CopyFileToClipboard,
            t(h::AfterCaptureTasks_CopyFileToClipboard),
            icons::CLIPBOARD_COPY,
        ),
        (
            AfterCaptureTasks::CopyFilePathToClipboard,
            t(h::AfterCaptureTasks_CopyFilePathToClipboard),
            icons::CLIPBOARD_LIST,
        ),
        (
            AfterCaptureTasks::CopyFolderPathToClipboard,
            t(h::AfterCaptureTasks_CopyFolderPathToClipboard),
            icons::FOLDER_BOOKMARK,
        ),
        (
            AfterCaptureTasks::ShowInExplorer,
            t(h::AfterCaptureTasks_ShowInExplorer),
            icons::FOLDER_OPEN,
        ),
        (
            AfterCaptureTasks::AnalyzeImage,
            t(h::AfterCaptureTasks_AnalyzeImage),
            icons::BOT,
        ),
        (
            AfterCaptureTasks::ScanQRCode,
            t(h::AfterCaptureTasks_ScanQRCode),
            icons::QR_CODE,
        ),
        (
            AfterCaptureTasks::DoOCR,
            t(h::AfterCaptureTasks_DoOCR),
            icons::SCAN_TEXT,
        ),
        (
            AfterCaptureTasks::ShowBeforeUploadWindow,
            t(h::AfterCaptureTasks_ShowBeforeUploadWindow),
            icons::APP_WINDOW,
        ),
        (
            AfterCaptureTasks::UploadImageToHost,
            t(h::AfterCaptureTasks_UploadImageToHost),
            icons::UPLOAD_CLOUD,
        ),
        (
            AfterCaptureTasks::DeleteFile,
            t(h::AfterCaptureTasks_DeleteFile),
            icons::TRASH_2,
        ),
    ];
    options
        .into_iter()
        .map(|(task, label, icon)| {
            Entry::check(
                label,
                icon,
                current.has(task),
                Command::ToggleAfterCaptureTask(task.bits()),
            )
        })
        .collect()
}

impl AppState {
    /// `AfterUploadJob.Swap(task)` on the default task settings.
    pub fn toggle_after_upload_task(&mut self, bit: u32) {
        let current = self.default_task_settings().after_upload_job;
        let flipped = AfterUploadTasks(current.0 ^ bit);
        self.set_default_task_member("AfterUploadJob", serde_json::json!(flipped.names()));
    }

    /// `AfterCaptureJob.Swap(task)` on the default task settings.
    pub fn toggle_after_capture_task(&mut self, bit: u32) {
        let current = self.default_task_settings().after_capture_job;
        let flipped = AfterCaptureTasks(current.0 ^ bit);
        self.set_default_task_member("AfterCaptureJob", serde_json::json!(flipped.names()));
    }

    /// Write one member of `DefaultTaskSettings`, keeping everything else as it is.
    fn set_default_task_member(&mut self, member: &str, value: serde_json::Value) {
        let mut object = self
            .config
            .get("DefaultTaskSettings")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        object.insert(member.to_string(), value);
        self.config
            .set("DefaultTaskSettings", serde_json::Value::Object(object));
        self.save_config();
    }

    /// `MainFormCommand.ScreenshotsFolder`: open the folder captures go to.
    pub fn open_screenshots_folder(&mut self) {
        let settings = self.default_task_settings();
        let folder = self.uploads.ctx.screenshots_folder(&settings, None);
        let _ = std::fs::create_dir_all(&folder);
        self.effects.push(Effect::OpenFile(folder));
    }
}
