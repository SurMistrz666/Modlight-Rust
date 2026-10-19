//! The Upload tab (`UploadView`, `UploadViewModel`, `UploadRows`, `QueueRows`): what is
//! going up right now, what is waiting for a connection, and what has gone up before.
//!
//! A view over work that exists elsewhere: the in-flight rows are the task list's own
//! busy uploads, drawn fresh every frame; the waiting rows are the offline queue's
//! snapshot; the finished rows pool three sources (this session's tasks, the history
//! store for earlier sessions, and whatever the queue made of what it held), because
//! none of them alone is complete.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t};
use modlight_core::tasks::info::{TaskInfo, TaskStatus};
use modlight_core::tasks::worker::TaskState;
use modlight_core::upload::item::{QueueItem, QueueReason, QueueState};
use modlight_core::upload::progress::ProgressManager;
use modlight_core::upload::queue::QueueOutcome;
use modlight_core::upload::text::{countdown, sentence};

use crate::commands::Command;
use crate::state::AppState;
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Look, button_at, button_width, layout, paint_icon, paint_progress, trimmed, wrapped,
};

/// How many finished uploads stay on screen: a control panel, not a library.
pub const MAX_HISTORY_ROWS: usize = 100;

/// What separates two facts on one line, matching the My space cards.
pub const SEPARATOR: &str = "  ·  ";

/// How long the copy button admits to having copied something.
pub const COPY_ACKNOWLEDGEMENT: f64 = 2.0;

/// One finished upload, however it finished (`UploadHistoryViewModel`).
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryRow {
    pub key: u64,
    /// The task this row came from; `None` for the store and the queue.
    pub task: Option<u64>,
    pub name: String,
    pub when: DateTime<Utc>,
    /// The destination's name for the meta line ("Modlight").
    pub destination: String,
    pub link: String,
    pub failed: bool,
    pub canceled: bool,
    pub reason: String,
    pub retry_path: Option<PathBuf>,
    /// Until when the copy button shows a tick (UI seconds).
    pub copied_until: f64,
}

impl HistoryRow {
    pub fn has_link(&self) -> bool {
        !self.link.is_empty()
    }

    pub fn done(&self) -> bool {
        !self.failed && !self.canceled
    }

    pub fn can_retry(&self) -> bool {
        self.retry_path.is_some() && !self.done()
    }

    pub fn state_text(&self) -> &'static str {
        if self.failed {
            t(s::Upload_StateFailed)
        } else if self.canceled {
            t(s::Upload_StateCanceled)
        } else {
            t(s::Upload_StateDone)
        }
    }

    /// Time and destination, as one line (`when.ToString("g")`).
    pub fn meta(&self) -> String {
        let text = modlight_core::culture::local_short(&self.when);
        if self.destination.is_empty() {
            text
        } else {
            format!("{text}{SEPARATOR}{}", self.destination)
        }
    }
}

fn first_non_empty(candidates: &[&str]) -> String {
    candidates
        .iter()
        .find(|c| !c.trim().is_empty())
        .map(|c| c.to_string())
        .unwrap_or_default()
}

/// The path when the file is still there; retry needs it.
fn file_for(path: &str) -> Option<PathBuf> {
    (!path.is_empty() && Path::new(path).is_file()).then(|| PathBuf::from(path))
}

/// One sentence about why an upload failed (`UploadRow.ReasonFor`): the first error's
/// first line, the sentence the uploader puts at the front for exactly this.
pub fn reason_for(info: &TaskInfo) -> String {
    let Some(error) = info.result.errors.errors.first() else {
        return t(s::Upload_NoReason).to_string();
    };
    let text = error.text.lines().next().unwrap_or("").trim();
    if text.is_empty() {
        t(s::Upload_NoReason).to_string()
    } else {
        text.to_string()
    }
}

/// The file's display name, or "Untitled".
pub fn task_name(info: &TaskInfo) -> String {
    let from_path = Path::new(info.file_path())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = first_non_empty(&[&info.file_name, &from_path]);
    if name.is_empty() {
        t(s::ThumbnailItemViewModel_UntitledTask).to_string()
    } else {
        name
    }
}

/// The Lucide glyph for a kind of file (`UploadRow.IconFor`).
pub fn icon_for(path: &str) -> char {
    use modlight_core::names::file_helpers;
    if path.is_empty() {
        icons::FILE
    } else if file_helpers::is_video_file(path) {
        icons::FILE_VIDEO
    } else if file_helpers::is_image_file(path) {
        icons::FILE_IMAGE
    } else if file_helpers::is_text_file(path) {
        icons::FILE_TEXT
    } else {
        icons::FILE
    }
}

/// Whether a task belongs on this tab: an upload job, whatever became of it, or
/// anything that produced a link (the recent list restored at start).
pub fn is_upload(state: &TaskState) -> bool {
    state.info.is_upload_job()
        || !state.info.result.url.is_empty()
        || !state.info.result.shortened_url.is_empty()
}

/// Bytes, speed and time left, as one line (`Upload_TransferProgress` + ETA).
pub fn transfer_detail(progress: &ProgressManager, binary: bool) -> String {
    use modlight_core::format::local_size;
    let mut text = format(
        t(s::Upload_TransferProgress),
        &[
            Arg::from(local_size(progress.position(), binary, 1)),
            Arg::from(local_size(progress.length(), binary, 1)),
            Arg::from(local_size(progress.speed() as i64, binary, 1)),
        ],
    );
    let remaining = progress.remaining();
    if remaining > std::time::Duration::ZERO {
        let seconds = remaining.as_secs_f64();
        let when = if seconds < 90.0 {
            format(t(s::Upload_EtaSeconds), &[Arg::Int(seconds.ceil() as i64)])
        } else {
            format(
                t(s::Upload_EtaMinutes),
                &[Arg::Int((seconds / 60.0).ceil() as i64)],
            )
        };
        text.push_str(SEPARATOR);
        text.push_str(&when);
    }
    text
}

#[derive(Default)]
pub struct UploadTab {
    pub history: Vec<HistoryRow>,
    next_key: u64,
    pub history_read: bool,
    pub loading_history: bool,
    pub history_error: String,
}

impl UploadTab {
    fn key(&mut self) -> u64 {
        self.next_key += 1;
        self.next_key
    }

    /// A row for a task this session ran.
    pub fn row_from_task(&mut self, id: u64, state: &TaskState) -> HistoryRow {
        let info = &state.info;
        let failed = state.status == TaskStatus::Failed;
        HistoryRow {
            key: self.key(),
            task: Some(id),
            name: task_name(info),
            link: first_non_empty(&[&info.result.shortened_url, &info.result.url]),
            when: info.task_end_time.unwrap_or_else(Utc::now),
            failed,
            canceled: state.status == TaskStatus::Stopped,
            reason: if failed {
                reason_for(info)
            } else {
                String::new()
            },
            retry_path: file_for(info.file_path()),
            destination: info.uploader_host().unwrap_or_default(),
            copied_until: 0.0,
        }
    }

    /// A row for what the queue made of an item.
    pub fn row_from_queue(&mut self, outcome: &QueueOutcome) -> HistoryRow {
        let failed = !outcome.succeeded && !outcome.canceled;
        let name = if outcome.item.file_name.is_empty() {
            t(s::ThumbnailItemViewModel_UntitledTask).to_string()
        } else {
            outcome.item.file_name.clone()
        };
        HistoryRow {
            key: self.key(),
            task: None,
            name,
            link: outcome.link.clone(),
            when: outcome.when,
            failed,
            canceled: outcome.canceled,
            reason: if failed {
                sentence(outcome.reason, &outcome.detail)
            } else {
                String::new()
            },
            retry_path: if outcome.item.owns_source {
                None
            } else {
                file_for(&outcome.item.source_path)
            },
            destination: "Modlight".into(),
            copied_until: 0.0,
        }
    }

    /// A row from the history store: always a success, there is nowhere for a failure.
    pub fn row_from_store(&mut self, item: &modlight_core::history::HistoryItem) -> HistoryRow {
        let from_path = Path::new(&item.file_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = first_non_empty(&[&item.file_name, &from_path]);
        HistoryRow {
            key: self.key(),
            task: None,
            name: if name.is_empty() {
                t(s::ThumbnailItemViewModel_UntitledTask).to_string()
            } else {
                name
            },
            link: first_non_empty(&[&item.shortened_url, &item.url]),
            when: item
                .date_time
                .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC),
            failed: false,
            canceled: false,
            reason: String::new(),
            retry_path: file_for(&item.file_path),
            destination: item.host.clone(),
            copied_until: 0.0,
        }
    }

    /// File one finished upload, newest first. A session row displaces a stored row with
    /// the same link (it knows whether it failed); a stored row for an upload this
    /// session already reported is dropped.
    pub fn insert(&mut self, row: HistoryRow) {
        if row.has_link()
            && let Some(index) = self.history.iter().position(|existing| {
                existing.key != row.key
                    && existing.link.eq_ignore_ascii_case(&row.link)
                    && (existing.task.is_none() || row.task.is_none())
            })
        {
            if self.history[index].task.is_some() && row.task.is_none() {
                return;
            }
            self.history.remove(index);
        }
        let index = self
            .history
            .iter()
            .position(|existing| existing.when < row.when)
            .unwrap_or(self.history.len());
        self.history.insert(index, row);
        self.history.truncate(MAX_HISTORY_ROWS);
    }

    /// A task ended: its row goes into the history (replacing any it had).
    pub fn task_finished(&mut self, id: u64, state: &TaskState) {
        self.history.retain(|row| row.task != Some(id));
        if !is_upload(state) || state.status.is_busy() {
            return;
        }
        let row = self.row_from_task(id, state);
        self.insert(row);
    }

    /// Drop the stored rows before reading the store again (`Refresh`).
    pub fn forget_stored(&mut self) {
        self.history.retain(|row| row.task.is_some());
        self.history_error.clear();
    }

    pub fn has_history_error(&self) -> bool {
        !self.history_error.is_empty()
    }
}

// ---------------------------------------------------------------------- drawing

/// A transfer row as drawn (`TransferViewModel`).
struct TransferRow {
    id: u64,
    icon: char,
    name: String,
    state: String,
    detail: String,
    fraction: Option<f32>,
    can_cancel: bool,
}

fn transfer_rows(state: &AppState) -> Vec<TransferRow> {
    let binary = state.app_upload_settings().binary_units;
    let mut rows: Vec<TransferRow> = state
        .uploads
        .tasks
        .tasks
        .iter()
        .map(|task| (task.id, task.snapshot()))
        .filter(|(_, s)| is_upload(s) && s.status.is_busy())
        .map(|(id, s)| {
            let info = &s.info;
            let state_text = if !info.status.is_empty() {
                info.status.clone()
            } else if s.status == TaskStatus::InQueue {
                t(s::Upload_StateQueued).to_string()
            } else {
                t(s::Upload_StateWorking).to_string()
            };
            let (fraction, detail) = match info.progress.as_ref().filter(|p| p.length() > 0) {
                Some(progress) => (
                    Some((progress.percentage().clamp(0.0, 100.0) / 100.0) as f32),
                    transfer_detail(progress, binary),
                ),
                None => (None, String::new()),
            };
            TransferRow {
                id,
                icon: icon_for(if info.file_path().is_empty() {
                    &info.file_name
                } else {
                    info.file_path()
                }),
                name: task_name(info),
                state: state_text,
                detail,
                fraction,
                can_cancel: s.status.is_busy(),
            }
        })
        .collect();
    // Newest first.
    rows.reverse();
    rows
}

/// A queue row as drawn (`QueueRowViewModel`).
struct QueueRow {
    id: String,
    icon: char,
    badge: char,
    stopped: bool,
    name: String,
    state: String,
    detail: String,
    progress: Option<Option<f32>>,
    can_send_now: bool,
}

fn queue_row(item: &QueueItem, now: DateTime<Utc>, binary: bool) -> QueueRow {
    let name = if item.file_name.is_empty() {
        t(s::ThumbnailItemViewModel_UntitledTask).to_string()
    } else {
        item.file_name.clone()
    };
    let icon = icon_for(&item.file_name);
    match item.state {
        QueueState::Sending => {
            let measured = item.progress.as_ref().filter(|p| p.length() > 0);
            QueueRow {
                id: item.id.clone(),
                icon,
                badge: icons::CLOUD_UPLOAD,
                stopped: false,
                name,
                state: t(s::UploadQueue_StateSending).to_string(),
                detail: measured
                    .map(|p| transfer_detail(p, binary))
                    .unwrap_or_default(),
                progress: Some(measured.map(|p| (p.percentage().clamp(0.0, 100.0) / 100.0) as f32)),
                can_send_now: false,
            }
        }
        QueueState::Blocked => QueueRow {
            id: item.id.clone(),
            icon,
            badge: icons::OCTAGON_ALERT,
            stopped: true,
            name,
            state: t(s::UploadQueue_StateStopped).to_string(),
            detail: sentence(item.reason, &item.detail),
            progress: None,
            can_send_now: true,
        },
        QueueState::Waiting => {
            let why = sentence(item.reason, &item.detail);
            let when = countdown(item.next_attempt_utc, now);
            QueueRow {
                id: item.id.clone(),
                icon,
                // No network gets its own glyph: the most common reason, and the one a
                // person can act on without reading anything.
                badge: if item.reason == QueueReason::NoNetwork {
                    icons::WIFI_OFF
                } else {
                    icons::CLOCK
                },
                stopped: false,
                name,
                state: t(s::UploadQueue_StateWaiting).to_string(),
                detail: if why.is_empty() {
                    when
                } else {
                    format!("{why}{SEPARATOR}{when}")
                },
                progress: None,
                can_send_now: true,
            }
        }
    }
}

/// `Button.up-quiet`: 32 tall, no surface until hovered, 13 px secondary text.
fn quiet(colors: &Colors) -> Look {
    let mut look = Look::strip(colors);
    look.height = 32.0;
    look.padding_x = 10.0;
    look.radius = 8.0;
    look.font_size = 13.0;
    look.weight = Weight::Normal;
    look.spacing = 7.0;
    look.icon_size = 15.0;
    look
}

/// `Button.up-quiet.up-icon`: 34 by 32, an icon only.
fn quiet_icon(colors: &Colors) -> Look {
    let mut look = quiet(colors);
    look.width = Some(34.0);
    look.padding_x = 0.0;
    look.icon_size = 16.0;
    look
}

/// The tab, filling `rect`.
pub fn ui(ui: &mut Ui, state: &mut AppState, rect: Rect) {
    let colors = state.colors.clone();
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, colors.background_main());

    // The way in: a panel band with the drop hint and two buttons.
    let band_inner_width = rect.width() - 32.0;
    let paste_label = t(s::Paste_Action);
    let choose_label = t(s::Upload_ChooseFiles);
    let button_look = Look::button(&colors);
    let paste_w = button_width(
        ui,
        &button_look,
        Content::both(icons::CLIPBOARD_TYPE, paste_label),
    );
    let choose_w = button_width(
        ui,
        &button_look,
        Content::both(icons::FILE_UP, choose_label),
    );
    let text_width =
        (band_inner_width - 32.0 - 38.0 - 14.0 - 14.0 - paste_w - 8.0 - choose_w).max(60.0);
    let title = layout(
        ui,
        wrapped(
            t(s::Upload_DropTitle),
            15.0,
            Weight::SemiBold,
            colors.text(),
            text_width,
        ),
    );
    let body = layout(
        ui,
        wrapped(
            t(s::Upload_DropBody),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            text_width,
        ),
    );
    let text_height = title.size().y + 4.0 + body.size().y;
    let panel_height = 16.0 + text_height.max(40.0).max(38.0) + 16.0;
    let band = Rect::from_min_size(
        rect.min,
        vec2(rect.width(), 16.0 + panel_height + 16.0 + 1.0),
    );
    painter.rect_filled(band, 0.0, colors.background_panel());
    painter.hline(
        band.x_range(),
        band.bottom() - 0.5,
        Stroke::new(1.0, colors.border()),
    );
    let panel = Rect::from_min_size(
        pos2(band.left() + 16.0, band.top() + 16.0),
        vec2(band_inner_width, panel_height),
    );
    painter.rect_filled(panel, 10.0, colors.control_background());
    painter.rect_stroke(
        panel,
        10.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let badge = Rect::from_min_size(
        pos2(panel.left() + 16.0, panel.center().y - 19.0),
        vec2(38.0, 38.0),
    );
    painter.rect_filled(badge, 8.0, colors.control_background_hover());
    paint_icon(
        ui,
        &painter,
        badge.center(),
        icons::UPLOAD,
        18.0,
        colors.accent_start(),
    );
    let text_left = badge.right() + 14.0;
    let text_top = panel.center().y - text_height / 2.0;
    let title_h = title.size().y;
    painter.galley(pos2(text_left, text_top), title, colors.text());
    painter.galley(
        pos2(text_left, text_top + title_h + 4.0),
        body,
        colors.text_secondary(),
    );
    let choose_rect = Rect::from_min_size(
        pos2(panel.right() - 16.0 - choose_w, panel.center().y - 20.0),
        vec2(choose_w, 40.0),
    );
    let paste_rect = Rect::from_min_size(
        pos2(choose_rect.left() - 8.0 - paste_w, choose_rect.top()),
        vec2(paste_w, 40.0),
    );
    if button_at(
        ui,
        &colors,
        &button_look,
        Content::both(icons::CLIPBOARD_TYPE, paste_label),
        true,
        paste_rect,
    )
    .clicked()
    {
        state.push(Command::PasteText);
    }
    if button_at(
        ui,
        &colors,
        &button_look,
        Content::both(icons::FILE_UP, choose_label),
        true,
        choose_rect,
    )
    .clicked()
    {
        state.push(Command::UploadTo(None));
    }

    let content = Rect::from_min_max(pos2(rect.left(), band.bottom()), rect.max);
    let transfers = transfer_rows(state);
    let now = state.env.now();
    let binary = state.app_upload_settings().binary_units;
    let queue: Vec<QueueRow> = state
        .queue_snapshot
        .iter()
        .map(|item| queue_row(item, now, binary))
        .collect();
    let signed_out = !state.account.is_signed_in();
    let tab = &state.upload_tab;
    let show_empty = transfers.is_empty()
        && queue.is_empty()
        && tab.history.is_empty()
        && !tab.loading_history
        && !tab.has_history_error();

    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(content)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(content.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical()
        .id_salt("upload-scroll")
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let width = content.width() - 16.0 - 12.0;
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    let mut first = true;
                    let mut gap = |ui: &mut Ui| {
                        if !first {
                            ui.add_space(18.0);
                        }
                        first = false;
                    };
                    if !transfers.is_empty() {
                        gap(ui);
                        section_label(ui, &colors, t(s::Upload_ActiveTitle), width);
                        ui.add_space(10.0);
                        for row in &transfers {
                            if transfer_card(ui, &colors, row, width) {
                                state.pending.push(Command::CancelTask(row.id));
                            }
                        }
                    }
                    if !queue.is_empty() {
                        gap(ui);
                        section_label(ui, &colors, t(s::UploadQueue_SectionTitle), width);
                        ui.add_space(10.0);
                        for row in &queue {
                            match queue_card(ui, &colors, row, width) {
                                Some(true) => {
                                    state.pending.push(Command::QueueSendNow(row.id.clone()))
                                }
                                Some(false) => {
                                    state.pending.push(Command::QueueCancel(row.id.clone()))
                                }
                                None => {}
                            }
                        }
                    }
                    if !state.upload_tab.history.is_empty() {
                        gap(ui);
                        let (label_rect, _) =
                            ui.allocate_exact_size(vec2(width, 32.0), Sense::hover());
                        let galley = layout(
                            ui,
                            trimmed(
                                t(s::Upload_HistoryTitle),
                                12.0,
                                Weight::SemiBold,
                                colors.text_secondary(),
                                width - 40.0,
                                1,
                            ),
                        );
                        crate::widgets::paint_text_left(
                            ui.painter(),
                            label_rect.left(),
                            label_rect.center().y,
                            galley,
                            colors.text_secondary(),
                        );
                        let refresh = Rect::from_min_size(
                            pos2(label_rect.right() - 34.0, label_rect.top()),
                            vec2(34.0, 32.0),
                        );
                        let response = button_at(
                            ui,
                            &colors,
                            &quiet_icon(&colors),
                            Content::icon(icons::REFRESH_CW),
                            true,
                            refresh,
                        );
                        if response
                            .clone()
                            .on_hover_text(t(s::Upload_Refresh))
                            .clicked()
                        {
                            state.pending.push(Command::RefreshUploadHistory);
                        }
                        ui.add_space(10.0);
                        let time = state.time;
                        let rows = state.upload_tab.history.clone();
                        for row in &rows {
                            if let Some(action) = history_card(ui, &colors, row, width, time) {
                                state.pending.push(match action {
                                    HistoryAction::Copy => Command::CopyHistoryLink(row.key),
                                    HistoryAction::Open => Command::OpenUrl(row.link.clone()),
                                    HistoryAction::Retry => Command::RetryUpload(
                                        row.retry_path.clone().unwrap_or_default(),
                                    ),
                                });
                            }
                        }
                    }
                    if state.upload_tab.loading_history {
                        gap(ui);
                        crate::widgets::text_wrapped(
                            ui,
                            t(s::Upload_LoadingHistory),
                            12.0,
                            Weight::Normal,
                            colors.text_secondary(),
                        );
                    }
                    if state.upload_tab.has_history_error() {
                        gap(ui);
                        if history_error(ui, &colors, &state.upload_tab.history_error, width) {
                            state.pending.push(Command::RefreshUploadHistory);
                        }
                    }
                });
            });
            ui.add_space(16.0);
        });

    if show_empty {
        empty_panel(ui, state, &colors, content, signed_out);
    }
}

fn section_label(ui: &mut Ui, colors: &Colors, text: &str, width: f32) {
    let galley = layout(
        ui,
        trimmed(
            text,
            12.0,
            Weight::SemiBold,
            colors.text_secondary(),
            width,
            1,
        ),
    );
    let (rect, _) = ui.allocate_exact_size(vec2(width, galley.size().y), Sense::hover());
    ui.painter()
        .galley(rect.min, galley, colors.text_secondary());
}

/// The card frame every row shares (`Border.up-card`): 12 padding, 8 below.
fn card_frame(ui: &mut Ui, colors: &Colors, width: f32, height: f32) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(width, height + 24.0 + 8.0), Sense::hover());
    let card = Rect::from_min_size(rect.min, vec2(width, height + 24.0));
    ui.painter()
        .rect_filled(card, 10.0, colors.control_background());
    ui.painter().rect_stroke(
        card,
        10.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    card
}

fn badge(ui: &Ui, colors: &Colors, card: Rect, glyph: char, color: Color32, opacity: f32) {
    let rect = Rect::from_min_size(
        pos2(card.left() + 12.0, card.center().y - 19.0),
        vec2(38.0, 38.0),
    );
    ui.painter()
        .rect_filled(rect, 8.0, colors.control_background_hover());
    let mut painter = ui.painter().clone();
    painter.set_opacity(opacity);
    paint_icon(ui, &painter, rect.center(), glyph, 18.0, color);
}

/// The name on the left and the state on the right of one line.
fn name_line(
    ui: &Ui,
    colors: &Colors,
    left: f32,
    top: f32,
    width: f32,
    name: &str,
    state: &str,
) -> f32 {
    let state_galley = layout(
        ui,
        trimmed(
            state,
            12.0,
            Weight::Normal,
            colors.text_secondary(),
            width * 0.5,
            1,
        ),
    );
    let state_w = state_galley.size().x;
    let name_galley = layout(
        ui,
        trimmed(
            name,
            15.0,
            Weight::Normal,
            colors.text(),
            (width - 12.0 - state_w).max(20.0),
            1,
        ),
    );
    let height = name_galley.size().y;
    let painter = ui.painter();
    painter.galley(pos2(left, top), name_galley, colors.text());
    let state_h = state_galley.size().y;
    painter.galley(
        pos2(left + width - state_w, top + (height - state_h) / 2.0),
        state_galley,
        colors.text_secondary(),
    );
    height
}

fn transfer_card(ui: &mut Ui, colors: &Colors, row: &TransferRow, width: f32) -> bool {
    let cancel_label = t(s::Upload_Cancel);
    let look = quiet(colors);
    let cancel_w = if row.can_cancel {
        button_width(ui, &look, Content::both(icons::X, cancel_label))
    } else {
        0.0
    };
    let text_left_offset = 12.0 + 38.0 + 14.0;
    let text_width =
        width - text_left_offset - 12.0 - if row.can_cancel { cancel_w + 14.0 } else { 0.0 };
    let name_h = 20.0;
    let detail_h: f32 = if row.detail.is_empty() {
        0.0
    } else {
        7.0 + 15.0
    };
    let height = (name_h + 7.0 + 4.0 + detail_h).max(38.0);
    let card = card_frame(ui, colors, width, height);
    badge(ui, colors, card, row.icon, colors.text_secondary(), 1.0);
    let left = card.left() + text_left_offset;
    let top = card.center().y - height / 2.0;
    let name_height = name_line(ui, colors, left, top, text_width, &row.name, &row.state);
    let bar = Rect::from_min_size(pos2(left, top + name_height + 7.0), vec2(text_width, 4.0));
    paint_progress(ui, colors, bar, row.fraction);
    if !row.detail.is_empty() {
        let galley = layout(
            ui,
            trimmed(
                &row.detail,
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                text_width,
                1,
            ),
        );
        ui.painter().galley(
            pos2(left, bar.bottom() + 7.0),
            galley,
            colors.text_secondary(),
        );
    }
    if row.can_cancel {
        let rect = Rect::from_min_size(
            pos2(card.right() - 12.0 - cancel_w, card.center().y - 16.0),
            vec2(cancel_w, 32.0),
        );
        return button_at(
            ui,
            colors,
            &look,
            Content::both(icons::X, cancel_label),
            true,
            rect,
        )
        .clicked();
    }
    false
}

/// `Some(true)` = send now, `Some(false)` = cancel.
fn queue_card(ui: &mut Ui, colors: &Colors, row: &QueueRow, width: f32) -> Option<bool> {
    let look = quiet(colors);
    let send_label = t(s::UploadQueue_SendNow);
    let send_w = if row.can_send_now {
        button_width(ui, &look, Content::both(icons::SEND, send_label))
    } else {
        0.0
    };
    let buttons_w = send_w + if row.can_send_now { 4.0 } else { 0.0 } + 34.0;
    let text_left_offset = 12.0 + 38.0 + 14.0;
    let text_width = width - text_left_offset - 12.0 - buttons_w - 14.0;
    let detail = (!row.detail.is_empty()).then(|| {
        layout(
            ui,
            wrapped_or_trimmed(&row.detail, colors.text_secondary(), text_width),
        )
    });
    let mut height = 20.0;
    if row.progress.is_some() {
        height += 7.0 + 4.0;
    }
    if let Some(detail) = &detail {
        height += 7.0 + detail.size().y;
    }
    let height = height.max(38.0);
    let card = card_frame(ui, colors, width, height);
    let badge_color = if row.stopped {
        colors.cancel_border()
    } else {
        colors.text_secondary()
    };
    badge(ui, colors, card, row.badge, badge_color, 1.0);
    let _ = row.icon;
    let left = card.left() + text_left_offset;
    let top = card.center().y - height / 2.0;
    let mut y = top + name_line(ui, colors, left, top, text_width, &row.name, &row.state);
    if let Some(fraction) = row.progress {
        let bar = Rect::from_min_size(pos2(left, y + 7.0), vec2(text_width, 4.0));
        paint_progress(ui, colors, bar, fraction);
        y = bar.bottom();
    }
    if let Some(detail) = detail {
        ui.painter()
            .galley(pos2(left, y + 7.0), detail, colors.text_secondary());
    }
    let mut outcome = None;
    let close = Rect::from_min_size(
        pos2(card.right() - 12.0 - 34.0, card.center().y - 16.0),
        vec2(34.0, 32.0),
    );
    if button_at(
        ui,
        colors,
        &quiet_icon(colors),
        Content::icon(icons::X),
        true,
        close,
    )
    .on_hover_text(t(s::Upload_Cancel))
    .clicked()
    {
        outcome = Some(false);
    }
    if row.can_send_now {
        let send = Rect::from_min_size(
            pos2(close.left() - 4.0 - send_w, close.top()),
            vec2(send_w, 32.0),
        );
        if button_at(
            ui,
            colors,
            &look,
            Content::both(icons::SEND, send_label),
            true,
            send,
        )
        .clicked()
        {
            outcome = Some(true);
        }
    }
    outcome
}

/// The detail line of a queue row: one trimmed line (`up-meta`).
fn wrapped_or_trimmed(text: &str, color: Color32, width: f32) -> egui::text::LayoutJob {
    trimmed(text, 12.0, Weight::Normal, color, width, 1)
}

enum HistoryAction {
    Copy,
    Open,
    Retry,
}

fn history_card(
    ui: &mut Ui,
    colors: &Colors,
    row: &HistoryRow,
    width: f32,
    time: f64,
) -> Option<HistoryAction> {
    let look = quiet(colors);
    let retry_label = t(s::Upload_Retry);
    let retry_w = if row.can_retry() {
        button_width(ui, &look, Content::both(icons::ROTATE_CCW, retry_label))
    } else {
        0.0
    };
    let link_buttons = if row.has_link() {
        34.0 + 4.0 + 34.0
    } else {
        0.0
    };
    let mut buttons_w = link_buttons;
    if row.can_retry() {
        buttons_w += if buttons_w > 0.0 { 4.0 } else { 0.0 } + retry_w;
    }
    let text_left_offset = 12.0 + 38.0 + 14.0;
    let text_width = width
        - text_left_offset
        - 12.0
        - if buttons_w > 0.0 {
            buttons_w + 14.0
        } else {
            0.0
        };
    let reason = (!row.reason.is_empty()).then(|| {
        layout(
            ui,
            wrapped(
                &row.reason,
                12.0,
                Weight::Normal,
                colors.cancel_border(),
                text_width,
            ),
        )
    });
    let mut height = 20.0 + 4.0 + 15.0;
    if row.has_link() {
        height += 4.0 + 15.0;
    }
    if let Some(reason) = &reason {
        height += 4.0 + reason.size().y;
    }
    let height = height.max(38.0);
    let card = card_frame(ui, colors, width, height);
    let (glyph, color, opacity) = if row.failed {
        (icons::TRIANGLE_ALERT, colors.cancel_border(), 1.0)
    } else if row.canceled {
        (icons::BAN, colors.text_secondary(), 0.6)
    } else {
        (icons::CIRCLE_CHECK, colors.continue_border(), 1.0)
    };
    badge(ui, colors, card, glyph, color, opacity);
    let left = card.left() + text_left_offset;
    let top = card.center().y - height / 2.0;
    let mut y = top
        + name_line(
            ui,
            colors,
            left,
            top,
            text_width,
            &row.name,
            row.state_text(),
        );
    let meta = layout(
        ui,
        trimmed(
            &row.meta(),
            12.0,
            Weight::Normal,
            colors.text_secondary(),
            text_width,
            1,
        ),
    );
    let meta_h = meta.size().y;
    ui.painter()
        .galley(pos2(left, y + 4.0), meta, colors.text_secondary());
    y += 4.0 + meta_h;
    if row.has_link() {
        let link = layout(
            ui,
            trimmed(
                &row.link,
                12.0,
                Weight::Normal,
                colors.accent_start(),
                text_width,
                1,
            ),
        );
        let link_h = link.size().y;
        ui.painter()
            .galley(pos2(left, y + 4.0), link, colors.accent_start());
        y += 4.0 + link_h;
    }
    if let Some(reason) = reason {
        ui.painter()
            .galley(pos2(left, y + 4.0), reason, colors.cancel_border());
    }

    let mut action = None;
    let mut x = card.right() - 12.0;
    if row.can_retry() {
        let rect = Rect::from_min_size(
            pos2(x - retry_w, card.center().y - 16.0),
            vec2(retry_w, 32.0),
        );
        if button_at(
            ui,
            colors,
            &look,
            Content::both(icons::ROTATE_CCW, retry_label),
            true,
            rect,
        )
        .clicked()
        {
            action = Some(HistoryAction::Retry);
        }
        x = rect.left() - 4.0;
    }
    if row.has_link() {
        let open = Rect::from_min_size(pos2(x - 34.0, card.center().y - 16.0), vec2(34.0, 32.0));
        if button_at(
            ui,
            colors,
            &quiet_icon(colors),
            Content::icon(icons::EXTERNAL_LINK),
            true,
            open,
        )
        .on_hover_text(t(s::Upload_OpenInBrowser))
        .clicked()
        {
            action = Some(HistoryAction::Open);
        }
        let copied = row.copied_until > time;
        let copy =
            Rect::from_min_size(pos2(open.left() - 4.0 - 34.0, open.top()), vec2(34.0, 32.0));
        let (glyph, tip) = if copied {
            (icons::CHECK, t(s::Upload_Copied))
        } else {
            (icons::COPY, t(s::Upload_CopyLink))
        };
        if button_at(
            ui,
            colors,
            &quiet_icon(colors),
            Content::icon(glyph),
            true,
            copy,
        )
        .on_hover_text(tip)
        .clicked()
        {
            action = Some(HistoryAction::Copy);
        }
        if copied {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(
                    (row.copied_until - time).max(0.05),
                ));
        }
    }
    action
}

/// `Border.up-notice` with the error, and Refresh. True when Refresh was pressed.
fn history_error(ui: &mut Ui, colors: &Colors, text: &str, width: f32) -> bool {
    let look = quiet(colors);
    let refresh = t(s::Upload_Refresh);
    let refresh_w = button_width(ui, &look, Content::label(refresh));
    let text_width = width - 14.0 - 16.0 - 10.0 - 10.0 - refresh_w - 14.0;
    let galley = layout(
        ui,
        wrapped(
            text,
            12.0,
            Weight::Normal,
            colors.text_secondary(),
            text_width,
        ),
    );
    let height = galley.size().y.max(32.0) + 22.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 10.0, colors.control_background());
    painter.rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    paint_icon(
        ui,
        painter,
        pos2(rect.left() + 14.0 + 8.0, rect.center().y),
        icons::TRIANGLE_ALERT,
        16.0,
        colors.cancel_border(),
    );
    let text_h = galley.size().y;
    painter.galley(
        pos2(
            rect.left() + 14.0 + 16.0 + 10.0,
            rect.center().y - text_h / 2.0,
        ),
        galley,
        colors.text_secondary(),
    );
    let button = Rect::from_min_size(
        pos2(rect.right() - 14.0 - refresh_w, rect.center().y - 16.0),
        vec2(refresh_w, 32.0),
    );
    button_at(ui, colors, &look, Content::label(refresh), true, button).clicked()
}

/// `Border.up-empty`: 520 wide, centred, which of the two nothings this is.
fn empty_panel(ui: &mut Ui, state: &mut AppState, colors: &Colors, area: Rect, signed_out: bool) {
    let (title, body) = if signed_out {
        (t(s::Upload_SignedOutTitle), t(s::Upload_SignedOutBody))
    } else {
        (t(s::Upload_EmptyTitle), t(s::Upload_EmptyBody))
    };
    let width = 520.0_f32.min(area.width() - 32.0);
    let text_width = width - 44.0 - 42.0 - 13.0;
    let title_galley = layout(
        ui,
        wrapped(title, 15.0, Weight::SemiBold, colors.text(), text_width),
    );
    let body_galley = layout(
        ui,
        wrapped(
            body,
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            text_width,
        ),
    );
    let text_h = title_galley.size().y + 6.0 + body_galley.size().y;
    let top_h = text_h.max(42.0);
    let height = 22.0 + top_h + 14.0 + 40.0 + 22.0;
    let rect = Rect::from_center_size(area.center(), vec2(width, height));
    let painter = ui.painter();
    painter.rect_filled(rect, 10.0, colors.background_panel());
    painter.rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let badge = Rect::from_min_size(
        pos2(rect.left() + 22.0, rect.top() + 22.0),
        vec2(42.0, 42.0),
    );
    painter.rect_filled(badge, 8.0, colors.control_background_hover());
    paint_icon(
        ui,
        painter,
        badge.center(),
        icons::UPLOAD,
        20.0,
        colors.accent_start(),
    );
    let left = badge.right() + 13.0;
    let top = rect.top() + 22.0 + (top_h - text_h) / 2.0;
    let title_h = title_galley.size().y;
    painter.galley(pos2(left, top), title_galley, colors.text());
    painter.galley(
        pos2(left, top + title_h + 6.0),
        body_galley,
        colors.text_secondary(),
    );
    let look = Look::button(colors);
    let (glyph, label, command) = if signed_out {
        (icons::LOG_IN, t(s::MainWindow_SignIn), Command::ShowSignIn)
    } else {
        (
            icons::FILE_UP,
            t(s::Upload_ChooseFiles),
            Command::UploadTo(None),
        )
    };
    let w = button_width(ui, &look, Content::both(glyph, label));
    let button = Rect::from_min_size(
        pos2(rect.left() + 22.0, rect.top() + 22.0 + top_h + 14.0),
        vec2(w, 40.0),
    );
    if button_at(ui, colors, &look, Content::both(glyph, label), true, button).clicked() {
        state.push(command);
    }
    let _ = Align2::CENTER_CENTER;
}
