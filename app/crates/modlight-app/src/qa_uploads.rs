//! QA fixtures for the upload flow's screens (phase 2): the Upload tab with transfers,
//! queued uploads and history, and every upload window drawn alone. Nothing here
//! touches a network, a clipboard or a disk outside the QA profile.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Duration as Span;
use egui::{Ui, Vec2, vec2};
use image::RgbaImage;
use modlight_core::l10n::{s, t, uploaders};
use modlight_core::tasks::host::{MessageIcon, NotificationConfig};
use modlight_core::tasks::info::{DataType, TaskInfo, TaskJob, TaskStatus};
use modlight_core::tasks::settings::TaskSettings;
use modlight_core::tasks::upload_manager::ClipboardContent;
use modlight_core::tasks::worker::WorkerTask;
use modlight_core::upload::progress::ProgressManager;
use modlight_core::upload::{QueueItem, QueueReason, QueueState};

use crate::dialogs::{
    after_upload, before_upload, clipboard_upload, drop_window, file_exists, message, paste,
    shorten_url, text_upload, upload_confirm, url_upload,
};
use crate::state::AppState;
use crate::upload_tab::HistoryRow;

const LINK: &str = "https://files.example.com/alex/q8";
const RAW: &str = "https://files.example.com/r/q8.jpg";

/// A task the way the Upload tab sees one in flight.
fn transfer(name: &str, length: i64, sent: i64, settings: &TaskSettings) -> WorkerTask {
    let task = WorkerTask::data_upload(DataType::File, vec![0], name, settings.clone());
    // A transfer that has run for four seconds at a steady pace.
    let start = Instant::now() - Duration::from_secs(4);
    let mut progress = ProgressManager::starting_at(length, 0, start);
    for step in 1..=16 {
        progress.update_progress_at(sent / 16, start + Duration::from_millis(250 * step));
    }
    task.with(|s| {
        s.info.job = TaskJob::FileUpload;
        s.status = TaskStatus::Working;
        s.info.status = t(s::UploadTask_DoUploadJob_Uploading).to_string();
        s.info.progress = Some(progress);
    });
    task
}

fn queued(name: &str, length: i64, state: &AppState) -> QueueItem {
    let now = state.env.now();
    QueueItem {
        file_name: name.into(),
        length,
        host: "https://files.example.com".into(),
        queued_at_utc: now - Span::minutes(12),
        deadline_utc: now + Span::days(14),
        ..QueueItem::default()
    }
}

fn row(state: &mut AppState, name: &str, minutes: i64) -> HistoryRow {
    HistoryRow {
        key: 0,
        task: None,
        name: name.into(),
        when: state.env.now() - Span::minutes(minutes),
        destination: "Modlight".into(),
        link: String::new(),
        failed: false,
        canceled: false,
        reason: String::new(),
        retry_path: None,
        copied_until: 0.0,
    }
}

/// An image upload's task as the before/after windows see it.
fn image_info(settings: &TaskSettings, picture: Option<RgbaImage>) -> TaskInfo {
    let mut info = TaskInfo::new(settings.clone());
    info.job = TaskJob::DataUpload;
    info.data_type = DataType::Image;
    info.file_name = "mountain-sunset.jpg".into();
    info.metadata.image = picture.map(Arc::new);
    info
}

/// Set up `screen`; `None` when it is not one of these. Returns the window size.
pub fn prepare(screen: &str, state: &mut AppState, picture: Option<RgbaImage>) -> Option<Vec2> {
    let settings = state.default_task_settings();
    let windows = &mut state.uploads.windows;
    let size = match screen {
        "tab-upload-busy" => {
            state.tab = crate::state::Tab::Upload;
            for task in [
                transfer("holiday-video.mov", 412_000_000, 268_000_000, &settings),
                transfer("design-notes.pdf", 8_400_000, 1_100_000, &settings),
            ] {
                state.uploads.tasks.tasks.push(task);
            }
            let now = state.env.now();
            let mut waiting = queued("screenshot-2026-10-19.png", 2_480_000, state);
            waiting.reason = QueueReason::Unreachable;
            waiting.next_attempt_utc = now + Span::seconds(95);
            waiting.server_attempts = 0;
            let mut blocked = queued("raw-footage.mp4", 3_900_000_000, state);
            blocked.state = QueueState::Blocked;
            blocked.reason = QueueReason::OutOfSpace;
            state.queue_snapshot = vec![waiting, blocked];
            state.queue_snapshot_at = f64::INFINITY;
            let mut done = row(state, "mountain-sunset.jpg", 5);
            done.link = LINK.into();
            let mut failed = row(state, "huge-video.mov", 40);
            failed.failed = true;
            failed.reason = t(uploaders::Modlight_File_too_large).to_string();
            failed.retry_path = Some(PathBuf::from("/Users/alex/Movies/huge-video.mov"));
            let mut canceled = row(state, "notes.txt", 180);
            canceled.canceled = true;
            for row in [canceled, failed, done] {
                state.upload_tab.insert(row);
            }
            state.upload_tab.history_read = true;
            crate::app::MAIN_SIZE
        }
        "paste" | "paste-languages" => {
            let mut window = paste::PasteWindow::new(Some(
                "fn main() {\n    let greeting = \"Hello from Modlight\";\n    println!(\"{greeting}\");\n}\n"
                    .into(),
            ));
            window.model.title = "hello".into();
            window.language_open = screen == "paste-languages";
            windows.paste = Some(window);
            vec2(paste::WIDTH, paste::HEIGHT)
        }
        "text-upload" => {
            windows.text_upload = Some(text_upload::TextUpload::new(Some(
                "Meeting notes, 19 October\n\n- ship the uploads\n- check the queue after a restart\n"
                    .into(),
            )));
            vec2(text_upload::WIDTH, text_upload::HEIGHT)
        }
        "url-upload" => {
            windows.url_upload = Some(url_upload::UrlUpload::new(Some(
                "https://example.com/pictures/mountain-sunset.jpg",
            )));
            vec2(url_upload::WIDTH, 240.0)
        }
        "shorten-url" => {
            windows.shorten_url = Some(shorten_url::ShortenUrl::new(Some(
                "https://example.com/a/rather/long/address?with=parameters&and=more",
            )));
            vec2(shorten_url::WIDTH, 560.0)
        }
        "clipboard-upload" => {
            windows.clipboard_upload = Some((
                clipboard_upload::ClipboardUpload::new(picture.map(ClipboardContent::Image), true),
                settings,
            ));
            vec2(clipboard_upload::WIDTH, clipboard_upload::HEIGHT)
        }
        "clipboard-upload-text" => {
            windows.clipboard_upload = Some((
                clipboard_upload::ClipboardUpload::new(
                    Some(ClipboardContent::Text(
                        "The quick brown fox jumps over the lazy dog.\nZażółć gęślą jaźń.".into(),
                    )),
                    true,
                ),
                settings,
            ));
            vec2(clipboard_upload::WIDTH, clipboard_upload::HEIGHT)
        }
        "drop-window" => {
            let app = state.app_upload_settings();
            let window = drop_window::DropWindow::new(
                app.drop_size,
                app.drop_offset,
                app.drop_alignment,
                app.drop_opacity,
                app.drop_hover_opacity,
            );
            let size = window.window_size();
            state.uploads.windows.drop_window = Some(window);
            size
        }
        "before-upload" => {
            windows.before_upload = Some(before_upload::BeforeUpload::new(
                1,
                image_info(&settings, picture),
                true,
            ));
            vec2(before_upload::WIDTH, before_upload::HEIGHT)
        }
        "after-upload" => {
            let mut info = image_info(&settings, picture);
            info.result.url = RAW.into();
            info.result.thumbnail_url = RAW.into();
            info.result.shortened_url = "https://files.example.com/s/alex/sunset".into();
            let ctx = state.uploads.ctx.clone();
            let formats = after_upload::create_formats(&info, &[], &|pattern| {
                modlight_core::tasks::naming::parse_upload_info(&info, pattern, &ctx)
            });
            windows
                .after_uploads
                .push(after_upload::AfterUpload::new(1, info, formats, false));
            vec2(after_upload::WIDTH, after_upload::HEIGHT)
        }
        "large-file" => {
            windows.large_file = Some(upload_confirm::UploadConfirm::new(
                upload_confirm::UploadQuestion::LargeFile,
            ));
            vec2(upload_confirm::WIDTH, 180.0)
        }
        "multi-upload" => {
            windows.multi_upload = Some((
                upload_confirm::UploadConfirm::new(upload_confirm::UploadQuestion::ManyFiles(24)),
                crate::uploads::PendingFiles {
                    files: Vec::new(),
                    settings,
                },
            ));
            vec2(upload_confirm::WIDTH, 180.0)
        }
        "file-exists" => {
            windows.file_exists = Some(file_exists::FileExists::new(
                PathBuf::from("/Users/alex/Pictures/Screenshots/2026-10/mountain-sunset.png"),
                PathBuf::from("/Users/alex/Pictures/Screenshots/2026-10/mountain-sunset (2).png"),
            ));
            vec2(file_exists::WIDTH, 360.0)
        }
        "message-box" => {
            let text = format!(
                "{}\n{}",
                t(uploaders::Modlight_File_too_large),
                "This file is larger than the 2 GB the server accepts."
            );
            windows.messages.push_back(message::MessageBox::new(
                "Modlight",
                &text,
                MessageIcon::Error,
            ));
            vec2(message::WIDTH, 160.0)
        }
        "notification-upload" => {
            let general = settings.general_settings.clone();
            let config = NotificationConfig {
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
                file_path: String::new(),
                image: picture.map(Arc::new),
                title: format!(
                    "Modlight - {}",
                    t(s::TaskManager_task_UploadCompleted_ShareX___Task_completed)
                ),
                text: LINK.into(),
                url: LINK.into(),
            };
            let notice = crate::notification::Notification::task(config, 0.0);
            let size = notice.window_size();
            state.windows.notification = Some(notice);
            size
        }
        _ => return None,
    };
    Some(size)
}

/// Draw a dialog screen alone into `ui`; `Some(height)` when the dialog sizes itself.
pub fn draw(screen: &str, ui: &mut Ui, state: &mut AppState) -> Option<Option<f32>> {
    let colors = state.colors.clone();
    let now = state.time;
    let windows = &mut state.uploads.windows;
    let height = match screen {
        "paste" | "paste-languages" => {
            if let Some(w) = windows.paste.as_mut() {
                let _ = w.ui(ui, &colors);
            }
            None
        }
        "text-upload" => {
            if let Some(w) = windows.text_upload.as_mut() {
                let _ = w.ui(ui, &colors);
            }
            None
        }
        "url-upload" => windows.url_upload.as_mut().map(|w| {
            let _ = w.ui(ui, &colors);
            w.content_height
        }),
        "shorten-url" => windows.shorten_url.as_mut().map(|w| {
            let _ = w.ui(ui, &colors);
            w.content_height
        }),
        "clipboard-upload" | "clipboard-upload-text" => {
            if let Some((w, _)) = windows.clipboard_upload.as_mut() {
                let _ = w.ui(ui, &colors);
            }
            None
        }
        "drop-window" => {
            if let Some(w) = windows.drop_window.as_mut() {
                // The real window is transparent; the shot shows it over the desktop
                // colour the window would sit on.
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, colors.background_main());
                let _ = w.ui(ui, &colors, false, Vec::new());
            }
            None
        }
        "before-upload" => {
            if let Some(w) = windows.before_upload.as_mut() {
                let _ = w.ui(ui, &colors);
            }
            None
        }
        "after-upload" => {
            if let Some(w) = windows.after_uploads.first_mut() {
                let _ = w.ui(ui, &colors, now);
            }
            None
        }
        "large-file" => windows.large_file.as_mut().map(|w| {
            let _ = w.ui(ui, &colors);
            w.content_height
        }),
        "multi-upload" => windows.multi_upload.as_mut().map(|(w, _)| {
            let _ = w.ui(ui, &colors);
            w.content_height
        }),
        "file-exists" => windows.file_exists.as_mut().map(|w| {
            let _ = w.ui(ui, &colors);
            w.content_height
        }),
        "message-box" => windows.messages.front_mut().map(|w| {
            let _ = w.ui(ui, &colors);
            w.content_height
        }),
        "notification-upload" => state.windows.notification.as_mut().map(|n| {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, colors.background_main());
            let _ = n.ui(ui, &colors, 1.0);
            n.window_size().y
        }),
        _ => return None,
    };
    Some(height)
}

/// Whether `screen` is one of these windows drawn alone.
pub fn is_dialog(screen: &str) -> bool {
    matches!(
        screen,
        "paste"
            | "paste-languages"
            | "text-upload"
            | "url-upload"
            | "shorten-url"
            | "clipboard-upload"
            | "clipboard-upload-text"
            | "drop-window"
            | "before-upload"
            | "after-upload"
            | "large-file"
            | "multi-upload"
            | "file-exists"
            | "message-box"
            | "notification-upload"
    )
}
