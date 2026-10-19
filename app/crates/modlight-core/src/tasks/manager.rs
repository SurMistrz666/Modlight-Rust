//! The list of tasks and what happens when one ends (`TaskManager`): starting them
//! within the upload limit, writing finished uploads into the history and the recent
//! list, and the sound, notice and after-upload window that follow a task.
//!
//! Lives on the window's thread. Tasks report through the channel in `host.rs`; the
//! window hands each event to [`TaskManager::handle_event`] and performs the effects it
//! returns, so the order of things is upstream's: final status, history, notice, then
//! "finished" for the views.

use std::sync::Arc;

use crate::history::{HistoryItem, HistoryStore};
use crate::l10n::{format, s, t};
use crate::tasks::context::TaskContext;
use crate::tasks::host::{NotificationConfig, NotificationSound, TaskEffect, TaskEvent};
use crate::tasks::info::{TaskJob, TaskStatus};
use crate::tasks::naming;
use crate::tasks::recent::{RecentTask, RecentTaskManager};
use crate::tasks::settings::AfterUploadTasks;
use crate::tasks::worker::WorkerTask;
use crate::upload::queue::QueueOutcome;

/// What the window learns from one event.
#[derive(Clone, Debug, Default)]
pub struct EventOutcome {
    /// The task changed (`TaskChanged`): redraw rows and the progress.
    pub changed: bool,
    /// The task ended, once (`TaskFinished`).
    pub finished: bool,
    /// Platform work to do.
    pub effects: Vec<TaskEffect>,
    /// The task list's state moved; save the recent list if it is kept.
    pub recent_changed: bool,
}

#[derive(Default)]
pub struct TaskManager {
    pub tasks: Vec<WorkerTask>,
    pub recent: RecentTaskManager,
    pub history: Option<Arc<HistoryStore>>,
}

impl TaskManager {
    pub fn new(history: Option<Arc<HistoryStore>>) -> TaskManager {
        TaskManager {
            tasks: Vec::new(),
            recent: RecentTaskManager::default(),
            history,
        }
    }

    pub fn find(&self, id: u64) -> Option<&WorkerTask> {
        self.tasks.iter().find(|t| t.id == id)
    }

    /// `IsBusy`.
    pub fn is_busy(&self) -> bool {
        self.tasks.iter().any(WorkerTask::is_busy)
    }

    /// `Start`.
    pub fn start(&mut self, task: WorkerTask, ctx: &TaskContext) {
        let history = task.status() == TaskStatus::History;
        self.tasks.push(task);
        if !history {
            self.start_tasks(ctx);
        }
    }

    /// `Remove`: stopped and taken off the list.
    pub fn remove(&mut self, id: u64, ctx: &TaskContext) {
        if let Some(index) = self.tasks.iter().position(|t| t.id == id) {
            let task = self.tasks.remove(index);
            task.stop(ctx);
        }
    }

    /// `StartTasks`: as many queued tasks as the upload limit allows (0 = no limit).
    pub fn start_tasks(&mut self, ctx: &TaskContext) {
        let working = self.tasks.iter().filter(|t| t.is_working()).count() as i64;
        let queued: Vec<WorkerTask> = self
            .tasks
            .iter()
            .filter(|t| t.status() == TaskStatus::InQueue)
            .cloned()
            .collect();
        if queued.is_empty() {
            return;
        }
        let limit = ctx.app_settings().upload_limit;
        let count = if limit == 0 {
            queued.len()
        } else {
            (limit - working).clamp(0, queued.len() as i64) as usize
        };
        for task in queued.iter().take(count) {
            task.start(ctx);
        }
    }

    pub fn stop_all(&self, ctx: &TaskContext) {
        for task in &self.tasks {
            task.stop(ctx);
        }
    }

    /// The average progress of the tasks uploading now, for the title and the tray
    /// (`UpdateProgressUI`); `None` when nothing is uploading.
    pub fn average_progress(&self) -> Option<f64> {
        let working: Vec<_> = self
            .tasks
            .iter()
            .map(WorkerTask::snapshot)
            .filter(|s| s.status == TaskStatus::Working)
            .collect();
        if working.is_empty() {
            return None;
        }
        let measured: Vec<f64> = working
            .iter()
            .filter_map(|s| s.info.progress.as_ref().map(|p| p.percentage()))
            .collect();
        if measured.is_empty() {
            Some(0.0)
        } else {
            Some(measured.iter().sum::<f64>() / measured.len() as f64)
        }
    }

    /// Follow one task event.
    pub fn handle_event(&mut self, id: u64, event: TaskEvent, ctx: &TaskContext) -> EventOutcome {
        let mut outcome = EventOutcome {
            changed: true,
            ..EventOutcome::default()
        };
        if event == TaskEvent::UploadProgressChanged
            && self
                .find(id)
                .is_some_and(|t| t.status() != TaskStatus::Working)
        {
            outcome.changed = false;
        }
        if event == TaskEvent::TaskCompleted
            && let Some(task) = self.find(id).cloned()
        {
            self.task_completed(&task, ctx, &mut outcome);
            outcome.finished = true;
            self.start_tasks(ctx);
        }
        outcome
    }

    /// `Task_TaskCompleted`.
    fn task_completed(&mut self, task: &WorkerTask, ctx: &TaskContext, outcome: &mut EventOutcome) {
        let state = task.snapshot();
        let info = &state.info;
        let app = ctx.app_settings();
        let result_text = info.summary();

        if !result_text.is_empty() {
            if app.history_save_tasks
                && (!app.history_check_url
                    || !info.result.url.is_empty()
                    || !info.result.shortened_url.is_empty())
            {
                self.append_history(info.history_item(), ctx);
            }
            self.recent.set_max_count(app.recent_tasks_max_count);
            self.recent.add(RecentTask {
                file_path: info.file_path().to_string(),
                url: info.result.url.clone(),
                thumbnail_url: info.result.thumbnail_url.clone(),
                deletion_url: info.result.deletion_url.clone(),
                shortened_url: info.result.shortened_url.clone(),
                ..RecentTask::default()
            });
            outcome.recent_changed = true;
        }

        let general = &info.settings.general_settings;
        match state.status {
            TaskStatus::Stopped => log::info!("Task stopped. File name: {}", info.file_name),
            TaskStatus::Failed => {
                log::info!(
                    "Task failed. File name: {}, Errors:\n{}",
                    info.file_name,
                    info.result.errors
                );
                if general.play_sound_after_upload {
                    outcome.effects.push(TaskEffect::PlaySound {
                        sound: NotificationSound::Error,
                        custom_path: general
                            .use_custom_error_sound
                            .then(|| general.custom_error_sound_path.clone())
                            .flatten()
                            .filter(|p| !p.is_empty()),
                    });
                }
                if let Some(error) = info.result.errors.errors.first() {
                    let title = if error.title.is_empty() {
                        t(s::TaskManager_task_UploadCompleted_Error).to_string()
                    } else {
                        error.title.clone()
                    };
                    if general.show_toast_notification_after_task_completed
                        && !error.text.is_empty()
                    {
                        outcome.effects.push(TaskEffect::ShowNotificationTip {
                            text: error.text.clone(),
                            title: format!("Modlight - {title}"),
                            duration_ms: Some(5000.0),
                        });
                    }
                }
            }
            _ => {
                log::info!(
                    "Task completed. File name: {}, Duration: {} ms",
                    info.file_name,
                    info.task_duration().as_millis()
                );
                if !state.stop_requested && info.job != TaskJob::ShareUrl && !result_text.is_empty()
                {
                    if general.play_sound_after_upload {
                        outcome.effects.push(TaskEffect::PlaySound {
                            sound: NotificationSound::TaskCompleted,
                            custom_path: general
                                .use_custom_task_completed_sound
                                .then(|| general.custom_task_completed_sound_path.clone())
                                .flatten()
                                .filter(|p| !p.is_empty()),
                        });
                    }
                    let format_text = &info.settings.advanced_settings.balloon_tip_content_format;
                    let text = if format_text.is_empty() {
                        result_text.clone()
                    } else {
                        naming::parse_upload_info(info, format_text, ctx)
                    };
                    if general.show_toast_notification_after_task_completed && !text.is_empty() {
                        outcome.effects.push(TaskEffect::ShowNotification(Box::new(
                            NotificationConfig {
                                duration_ms: general.toast_window_duration * 1000.0,
                                fade_ms: general.toast_window_fade_duration * 1000.0,
                                placement: general.toast_window_placement,
                                size: general.toast_window_size,
                                left_click: general.toast_window_left_click_action,
                                right_click: general.toast_window_right_click_action,
                                middle_click: general.toast_window_middle_click_action,
                                button_size: general.toast_window_button_size,
                                buttons: general.toast_window_buttons.clone(),
                                auto_hide: general.toast_window_auto_hide,
                                file_path: info.file_path().to_string(),
                                image: state.image.clone(),
                                title: format!(
                                    "Modlight - {}",
                                    t(s::TaskManager_task_UploadCompleted_ShareX___Task_completed)
                                ),
                                text,
                                url: info.result.to_string(),
                            },
                        )));
                        if info
                            .settings
                            .after_upload_job
                            .has(AfterUploadTasks::ShowAfterUploadWindow)
                            && info.is_upload_job()
                        {
                            outcome
                                .effects
                                .push(TaskEffect::ShowAfterUploadWindow(Box::new(info.clone())));
                        }
                    }
                }
            }
        }
        // The notice holds its own copy of the picture.
        task.release_image();
    }

    /// `OnQueuedUploadFinished`: a file the queue got out is written into the same two
    /// books a finished task is. Only successes: the store has nowhere for a failure.
    pub fn queued_upload_finished(&mut self, outcome: &QueueOutcome, ctx: &TaskContext) -> bool {
        if !outcome.succeeded || outcome.link.is_empty() {
            return false;
        }
        let url = if outcome.view_url.is_empty() {
            outcome.link.clone()
        } else {
            outcome.view_url.clone()
        };
        let file_path = if outcome.item.owns_source {
            String::new()
        } else {
            outcome.item.source_path.clone()
        };
        if ctx.app_settings().history_save_tasks {
            self.append_history(
                HistoryItem {
                    file_name: outcome.item.file_name.clone(),
                    file_path: file_path.clone(),
                    date_time: Some(outcome.when),
                    host: "Modlight".into(),
                    url: url.clone(),
                    thumbnail_url: outcome.raw_url.clone(),
                    ..HistoryItem::default()
                },
                ctx,
            );
        }
        self.recent.add(RecentTask {
            file_path,
            url,
            thumbnail_url: outcome.raw_url.clone(),
            ..RecentTask::default()
        });
        true
    }

    /// `AppendHistoryItemAsync`: off the window's thread; a failure is shown.
    fn append_history(&self, item: HistoryItem, ctx: &TaskContext) {
        let Some(history) = self.history.clone() else {
            return;
        };
        let channel = ctx.channel.clone();
        ctx.runtime.spawn_blocking(move || {
            let mut items = [item];
            if let Err(error) = history.append(&mut items) {
                log::warn!("Modlight: the history could not be written: {error}");
                channel.effect(TaskEffect::MessageBox {
                    text: error.to_string(),
                    title: "Modlight".into(),
                    icon: crate::tasks::host::MessageIcon::Error,
                });
            }
        });
    }

    /// The window's title while uploads run: "Modlight 0.7.7 Dev - 42.0%".
    pub fn title(&self) -> String {
        match self.average_progress() {
            Some(progress) => format(
                "{0} - {1:0.0}%",
                &[
                    crate::l10n::Arg::from(crate::title()),
                    crate::l10n::Arg::from(progress),
                ],
            ),
            None => crate::title(),
        }
    }
}
