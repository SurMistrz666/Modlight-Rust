//! The app's own update: the hourly check (`GitHubUpdateManager` with
//! `ShareXUpdateManager`'s Modlight checker), the "update is available" question
//! (`UpdateMessageWindow`) and the downloader (`DownloaderWindow`).
//!
//! Off unless the build has a release host (`modlight_core::update`). The downloader
//! fetches the release and refuses it unless its size and SHA-256 match the manifest.
//! Windows, as upstream: the verified installer (`.exe`) starts with `/UPDATE`
//! (`AutoStartInstall`), Install starts it again, and once it runs the app quits to make
//! way for it. A `.zip` (the folder build `build-app.ps1` makes) has nothing to run: it is
//! revealed in Explorer, as on macOS the `.dmg`/`.zip` is revealed in Finder; nothing else
//! is installed or run.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use egui::{Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::components::download::DownloadProgress;
use modlight_core::format::local_size;
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, helpers, s, t};
use modlight_core::update::{self, ReleaseManifest, UpdateOutcome};
use tokio_util::sync::CancellationToken;

use crate::state::AppState;
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Look, button_at, button_width, layout, paint_icon, paint_progress, trimmed, wrapped,
};

/// `UpdateMessageWindow`: 400 wide, its height follows the content.
pub const MESSAGE_WIDTH: f32 = 400.0;
/// `DownloaderWindow`: 560 x 310.
pub const DOWNLOADER_WIDTH: f32 = 560.0;
pub const DOWNLOADER_HEIGHT: f32 = 310.0;

/// `DownloaderWindowStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloaderStatus {
    Waiting,
    DownloadStarted,
    DownloadCompleted,
    Failed,
}

/// The downloader window's state.
pub struct Downloader {
    pub manifest: ReleaseManifest,
    pub status: DownloaderStatus,
    pub status_text: String,
    pub progress: Option<DownloadProgress>,
    pub file: Option<PathBuf>,
    cancel: Option<CancellationToken>,
    live: Arc<Mutex<Option<DownloadProgress>>>,
}

impl Downloader {
    pub fn new(manifest: ReleaseManifest) -> Downloader {
        Downloader {
            manifest,
            status: DownloaderStatus::Waiting,
            status_text: t(helpers::DownloaderForm_DownloaderForm_Waiting_).to_string(),
            progress: None,
            file: None,
            cancel: None,
            live: Arc::new(Mutex::new(None)),
        }
    }
}

/// The update's state in the app.
pub struct Updates {
    /// `AutoUpdateEnabled`: a "No" switches the timer off for this run.
    pub enabled: bool,
    /// The first check after launch activates its window; later ones do not.
    pub first_check: bool,
    pub scheduled: bool,
    /// The question, when there is an update.
    pub message: Option<ReleaseManifest>,
    /// The first check's window comes forward; a later one opens without taking the
    /// focus (`ActivateWindow`).
    pub message_activate: bool,
    pub message_height: f32,
    pub downloader: Option<Downloader>,
}

impl Default for Updates {
    fn default() -> Self {
        Updates {
            enabled: true,
            first_check: true,
            scheduled: false,
            message: None,
            message_activate: true,
            message_height: 260.0,
            downloader: None,
        }
    }
}

/// `AutoCheckUpdate` (on by default).
pub fn auto_check(state: &AppState) -> bool {
    state
        .config
        .get("AutoCheckUpdate")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true)
}

/// `ConfigureAutoUpdate`: only a real app with a release channel and the setting on
/// arms the timer, which fires at once and then every hour.
pub fn on_startup(state: &mut AppState) {
    if state.mode != crate::app::Mode::Normal
        || !update::has_release_channel()
        || !auto_check(state)
        || state.updates.scheduled
    {
        return;
    }
    state.updates.scheduled = true;
    schedule(state, std::time::Duration::ZERO);
}

/// Arm the timer: after `after`, the UI thread decides whether to check.
fn schedule(state: &mut AppState, after: std::time::Duration) {
    state.rt.spawn(async move {
        tokio::time::sleep(after).await;
        move |state: &mut AppState, _ctx: &egui::Context| timer_fired(state)
    });
}

/// The timer fired. The setting is read now, not when the timer was armed: upstream
/// disposes the timer as soon as automatic checks are switched off, so an hour-old timer
/// must not reach the release host (or put up the question) after the person said no.
fn timer_fired(state: &mut AppState) {
    if !state.updates.enabled || !auto_check(state) {
        state.updates.scheduled = false;
        return;
    }
    state.rt.spawn(async move {
        let outcome = update::check().await;
        move |state: &mut AppState, _ctx: &egui::Context| {
            checked(state, outcome);
            if state.updates.enabled && auto_check(state) {
                schedule(state, update::CHECK_INTERVAL);
            } else {
                state.updates.scheduled = false;
            }
        }
    });
}

/// A check completed: offer what it found, unless a question is already up.
pub fn checked(state: &mut AppState, outcome: UpdateOutcome) {
    if !state.updates.enabled
        || state.updates.message.is_some()
        || state.updates.downloader.is_some()
    {
        return;
    }
    if let UpdateOutcome::UpdateAvailable(manifest) = outcome {
        state.updates.message = Some(manifest);
        state.updates.message_activate = state.updates.first_check;
    }
    state.updates.first_check = false;
}

/// Whether a release file is an installer (`.exe`), which Windows starts, rather than a
/// file that is only shown in the file manager (the `.zip` folder build, the Mac's
/// `.dmg`).
fn is_installer(file_name: &str) -> bool {
    file_name.to_ascii_lowercase().ends_with(".exe")
}

/// Whether this system installs the release itself: Windows, when it is an installer.
fn installs(file_name: &str) -> bool {
    cfg!(windows) && is_installer(file_name)
}

/// The question's text (`BuildMessage`): what will happen, both versions, the size.
pub fn message_text(manifest: &ReleaseManifest) -> String {
    // Windows installs an installer; a `.zip`, and anything on a Mac, is downloaded,
    // verified and shown in the file manager: "download it", not "download and install
    // it" (upstream's portable question).
    let mut text = String::new();
    text.push_str(t(if installs(&manifest.file_name) {
        helpers::UpdateMessageBox_UpdateMessageBox_
    } else {
        helpers::UpdateMessageBox_UpdateMessageBox_Portable
    }));
    text.push_str("\n\n");
    text.push_str(t(helpers::UpdateMessageBox_UpdateMessageBox_CurrentVersion));
    text.push_str(": ");
    text.push_str(env!("CARGO_PKG_VERSION"));
    text.push('\n');
    text.push_str(t(helpers::UpdateMessageBox_UpdateMessageBox_LatestVersion));
    text.push_str(": ");
    text.push_str(&manifest.version_text);
    if manifest.size > 0 {
        text.push('\n');
        text.push_str(t(helpers::UpdateMessageWindow_DownloadSize));
        text.push_str(": ");
        text.push_str(&local_size(manifest.size, false, 2));
    }
    text
}

/// The question's content. `Some(true)` for Yes, `Some(false)` for No.
pub fn message_ui(ui: &mut Ui, state: &mut AppState) -> Option<bool> {
    let colors = state.colors.clone();
    let manifest = state.updates.message.clone()?;
    let full = ui.max_rect();
    let painter = ui.painter().clone();
    painter.rect_filled(full, 0.0, colors.background_main());
    let left = full.left() + 18.0;
    let width = full.width() - 36.0;
    let mut y = full.top() + 18.0;
    // Header: the download glyph on a panel tile, the heading in the accent.
    let tile = Rect::from_min_size(pos2(left, y), vec2(42.0, 42.0));
    painter.rect_filled(tile, 6.0, colors.background_panel());
    paint_icon(
        ui,
        &painter,
        tile.center(),
        icons::DOWNLOAD,
        21.0,
        colors.accent_start(),
    );
    let header = layout(
        ui,
        wrapped(
            t(helpers::UpdateMessageWindow_Header),
            20.0,
            Weight::SemiBold,
            colors.heading(),
            width - 52.0,
        ),
    );
    let hh = header.size().y;
    painter.galley(
        pos2(tile.right() + 10.0, tile.center().y - hh / 2.0),
        header,
        colors.heading(),
    );
    y += 42.0_f32.max(hh) + 14.0;
    // The card: the message, the notes, the changelog link.
    let inner_w = width - 30.0;
    let message = layout(
        ui,
        wrapped(
            &message_text(&manifest),
            15.0,
            Weight::Normal,
            colors.text(),
            inner_w,
        ),
    );
    let notes = (!manifest.notes.is_empty()).then(|| {
        layout(
            ui,
            wrapped(
                &manifest.notes,
                14.0,
                Weight::Normal,
                colors.text_secondary(),
                inner_w,
            ),
        )
    });
    let link_text = t(helpers::UpdateMessageWindow_View_changelog);
    let link = layout(
        ui,
        trimmed(
            link_text,
            15.0,
            Weight::Normal,
            colors.accent_start(),
            inner_w,
            1,
        ),
    );
    let notes_h = notes
        .as_ref()
        .map(|g| g.size().y.min(160.0) + 12.0)
        .unwrap_or(0.0);
    let card_h = 15.0 + message.size().y + notes_h + 12.0 + link.size().y + 15.0;
    let card = Rect::from_min_size(pos2(left, y), vec2(width, card_h));
    painter.rect_filled(card, 6.0, colors.background_panel());
    painter.rect_stroke(
        card,
        6.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let mut cy = card.top() + 15.0;
    let mh = message.size().y;
    painter.galley(pos2(card.left() + 15.0, cy), message, colors.text());
    cy += mh;
    if let Some(notes) = notes {
        cy += 12.0;
        let clip = Rect::from_min_size(
            pos2(card.left() + 15.0, cy),
            vec2(inner_w, notes.size().y.min(160.0)),
        );
        painter
            .with_clip_rect(clip)
            .galley(clip.min, notes, colors.text_secondary());
        cy += clip.height();
    }
    cy += 12.0;
    let link_rect = Rect::from_min_size(pos2(card.left() + 15.0, cy), link.size());
    painter.galley(link_rect.min, link, colors.accent_start());
    if ui
        .interact(link_rect, egui::Id::new("update-changelog"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        state.open_url(&update::download_page_url());
    }
    y = card.bottom() + 14.0;
    // Yes (primary) and No, right-aligned.
    let mut yes_look = Look::primary(&colors).height(32.0).padding(14.0);
    yes_look.min_width = 96.0;
    let mut no_look = Look::compact(&colors).height(32.0).padding(14.0);
    no_look.min_width = 96.0;
    let yes = t(helpers::MyMessageBox_MyMessageBox_Yes);
    let no = t(helpers::MyMessageBox_MyMessageBox_No);
    let no_w = button_width(ui, &no_look, Content::label(no));
    let yes_w = button_width(ui, &yes_look, Content::label(yes));
    let no_rect = Rect::from_min_size(pos2(left + width - no_w, y), vec2(no_w, 32.0));
    let yes_rect = Rect::from_min_size(pos2(no_rect.left() - 7.0 - yes_w, y), vec2(yes_w, 32.0));
    let mut answer = None;
    if button_here(ui, &colors, &yes_look, Content::label(yes), true, yes_rect).clicked() {
        answer = Some(true);
    }
    if button_here(ui, &colors, &no_look, Content::label(no), true, no_rect).clicked() {
        answer = Some(false);
    }
    state.updates.message_height = (y + 32.0 + 18.0 - full.top()).round();
    answer
}

/// The question was answered: Yes opens the downloader, No stops asking this run.
pub fn message_answered(state: &mut AppState, yes: bool) {
    let Some(manifest) = state.updates.message.take() else {
        return;
    };
    if yes {
        let mut downloader = Downloader::new(manifest);
        start_download(state, &mut downloader);
        state.updates.downloader = Some(downloader);
    } else {
        state.updates.enabled = false;
    }
}

/// `StartDownloadAsync`: reuse a verified copy, else fetch, then verify.
fn start_download(state: &mut AppState, downloader: &mut Downloader) {
    if downloader.status != DownloaderStatus::Waiting || state.mode == crate::app::Mode::Shot {
        return;
    }
    downloader.status = DownloaderStatus::DownloadStarted;
    downloader.status_text =
        t(helpers::DownloaderForm_StartDownload_Getting_file_size_).to_string();
    let cancel = CancellationToken::new();
    downloader.cancel = Some(cancel.clone());
    let manifest = downloader.manifest.clone();
    let live = downloader.live.clone();
    let waker = state.rt.waker();
    state.rt.spawn(async move {
        let folder = update::download_folder();
        let sink = move |p: DownloadProgress| {
            if let Ok(mut slot) = live.lock() {
                *slot = Some(p);
            }
            waker();
        };
        let result = update::download(&manifest, &folder, &sink, &cancel).await;
        move |state: &mut AppState, _ctx: &egui::Context| download_done(state, result)
    });
}

fn download_done(state: &mut AppState, result: update::DownloadResult) {
    let Some(downloader) = state.updates.downloader.as_mut() else {
        return;
    };
    downloader.cancel = None;
    match result {
        update::DownloadResult::Verified(path) => {
            downloader.status = DownloaderStatus::DownloadCompleted;
            downloader.status_text =
                t(helpers::DownloaderForm_fileDownloader_DownloadCompleted_Download_completed_)
                    .to_string();
            downloader.file = Some(path.clone());
            // `AutoStartInstall`.
            use_release(state, path);
        }
        update::DownloadResult::Cancelled => {
            state.updates.downloader = None;
        }
        update::DownloadResult::Incomplete(detail) => {
            log::warn!("Update: the download stopped short: {detail}");
            downloader.status = DownloaderStatus::Failed;
            downloader.status_text = t(helpers::DownloaderWindow_DownloadIncomplete).to_string();
            downloader.progress = None;
        }
        update::DownloadResult::VerificationFailed => {
            downloader.status = DownloaderStatus::Failed;
            downloader.status_text = t(helpers::DownloaderWindow_VerificationFailed).to_string();
            downloader.progress = None;
        }
    }
}

/// What a verified release is for (`AutoStartInstall`, and the button after it): on
/// Windows an installer starts; a `.zip` there, and the file on a Mac, is shown in the
/// file manager.
fn use_release(state: &mut AppState, path: PathBuf) {
    #[cfg(windows)]
    if path
        .file_name()
        .is_some_and(|name| installs(&name.to_string_lossy()))
    {
        install(state);
        return;
    }
    state.effects.push(crate::state::Effect::Reveal(path));
}

/// Windows, `InstallAsync`: start the verified installer and close the window, or say why
/// it did not start and keep Install (`KeepInstallable`). Only the real app starts it.
/// Once it runs the app quits (`DownloadUpdateAsync`'s `ApplicationExit.Request`), so the
/// installer can replace the program.
#[cfg(windows)]
fn install(state: &mut AppState) {
    let real = state.mode == crate::app::Mode::Normal && !state.env.qa;
    let Some(downloader) = state.updates.downloader.as_mut() else {
        return;
    };
    let Some(file) = downloader.file.clone() else {
        return;
    };
    if !real {
        log::info!("QA mode: not starting {}", file.display());
        return;
    }
    downloader.status_text = t(helpers::DownloaderWindow_StartingInstaller).to_string();
    match crate::win32::start_installer(&file) {
        crate::win32::InstallerStart::Started => {
            log::info!("Update: installer started - \"{}\" /UPDATE", file.display());
            state.updates.downloader = None;
            // Making way for it: a running Modlight.exe cannot be replaced.
            state.quit_requested = true;
        }
        crate::win32::InstallerStart::Cancelled => {
            log::info!("Update: the elevation prompt for the installer was declined");
            downloader.status_text = t(helpers::DownloaderWindow_InstallCancelled).to_string();
        }
        crate::win32::InstallerStart::Failed(error) => {
            log::warn!("Update: the installer could not be started: {error}");
            downloader.status_text = format(
                t(helpers::DownloaderWindow_InstallFailed),
                &[Arg::from(error)],
            );
        }
    }
}

/// The downloader's content. True when it asked to close.
pub fn downloader_ui(ui: &mut Ui, state: &mut AppState) -> bool {
    let colors = state.colors.clone();
    let Some(downloader) = state.updates.downloader.as_mut() else {
        return true;
    };
    if let Some(p) = downloader.live.lock().ok().and_then(|mut l| l.take()) {
        if downloader.status == DownloaderStatus::DownloadStarted {
            downloader.status_text =
                t(helpers::DownloaderForm_StartDownload_Downloading_).to_string();
        }
        downloader.progress = Some(p);
    }
    let full = ui.max_rect();
    let painter = ui.painter().clone();
    painter.rect_filled(full, 0.0, colors.background_main());
    let left = full.left() + 18.0;
    let width = full.width() - 36.0;
    let top = full.top() + 18.0;
    // Header.
    let tile = Rect::from_min_size(pos2(left, top), vec2(42.0, 42.0));
    painter.rect_filled(tile, 6.0, colors.background_panel());
    paint_icon(
        ui,
        &painter,
        tile.center(),
        icons::DOWNLOAD,
        21.0,
        colors.accent_start(),
    );
    let heading = layout(
        ui,
        trimmed(
            t(helpers::DownloaderWindow_Heading),
            20.0,
            Weight::SemiBold,
            colors.heading(),
            width - 52.0,
            1,
        ),
    );
    let description = layout(
        ui,
        wrapped(
            t(helpers::DownloaderWindow_Description),
            13.0,
            Weight::SemiBold,
            colors.text_secondary(),
            width - 52.0,
        ),
    );
    let text_h = heading.size().y + 2.0 + description.size().y;
    let ty = tile.center().y - text_h / 2.0;
    let heading_h = heading.size().y;
    painter.galley(pos2(tile.right() + 10.0, ty), heading, colors.heading());
    painter.galley(
        pos2(tile.right() + 10.0, ty + heading_h + 2.0),
        description,
        colors.text_secondary(),
    );
    // The action button, bottom right.
    let label = match downloader.status {
        DownloaderStatus::Waiting => t(helpers::DownloaderWindow_Download),
        DownloaderStatus::DownloadStarted => t(helpers::DownloaderForm_StartDownload_Cancel),
        DownloaderStatus::DownloadCompleted if installs(&downloader.manifest.file_name) => {
            t(helpers::DownloaderForm_fileDownloader_DownloadCompleted_Install)
        }
        DownloaderStatus::DownloadCompleted => t(s::Download_ShowInFolder),
        DownloaderStatus::Failed => t(helpers::DownloaderWindow_Close),
    };
    let mut look = Look::primary(&colors).height(34.0).padding(16.0);
    look.min_width = 112.0;
    let bw = button_width(ui, &look, Content::label(label));
    let button = Rect::from_min_size(
        pos2(left + width - bw, full.bottom() - 18.0 - 34.0),
        vec2(bw, 34.0),
    );
    // The card between them.
    let card = Rect::from_min_max(
        pos2(left, tile.bottom().max(ty + text_h) + 12.0),
        pos2(left + width, button.top() - 12.0),
    );
    painter.rect_filled(card, 6.0, colors.background_panel());
    painter.rect_stroke(
        card,
        6.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let inner = card.shrink(15.0);
    let file_line = layout(
        ui,
        trimmed(
            &format(
                t(helpers::DownloaderForm_DownloaderForm_Filename___0_),
                &[Arg::from(downloader.manifest.file_name.as_str())],
            ),
            13.0,
            Weight::SemiBold,
            colors.text_secondary(),
            inner.width(),
            1,
        ),
    );
    let status_line = layout(
        ui,
        wrapped(
            &format(
                t(helpers::DownloaderForm_ChangeStatus_Status___0_),
                &[Arg::from(downloader.status_text.as_str())],
            ),
            13.0,
            Weight::SemiBold,
            colors.text_secondary(),
            inner.width(),
        ),
    );
    let mut y = inner.top();
    let fh = file_line.size().y;
    painter.galley(pos2(inner.left(), y), file_line, colors.text_secondary());
    y += fh + 8.0;
    let sh = status_line.size().y;
    painter.galley(pos2(inner.left(), y), status_line, colors.text_secondary());
    y += sh + 8.0;
    let bar = Rect::from_min_size(
        pos2(inner.left(), inner.bottom() - 18.0),
        vec2(inner.width(), 18.0),
    );
    let fraction = match (downloader.status, downloader.progress) {
        (DownloaderStatus::DownloadCompleted, _) => 1.0,
        (_, Some(p)) if p.total > 0 => p.received as f32 / p.total as f32,
        _ => 0.0,
    };
    if let Some(p) = downloader
        .progress
        .filter(|_| downloader.status == DownloaderStatus::DownloadStarted)
    {
        let percent = if p.total > 0 {
            p.received as f64 * 100.0 / p.total as f64
        } else {
            0.0
        };
        let text = format!(
            "{}: {}%\n{}: {}/s\n{}: {} / {}",
            t(helpers::DownloaderForm_FileDownloader_ProgressChanged_Progress),
            modlight_core::culture::Culture::current().fixed(percent, 1),
            t(helpers::DownloaderForm_FileDownloader_ProgressChanged_DownloadSpeed),
            local_size(p.bytes_per_second as i64, false, 2),
            t(helpers::DownloaderForm_FileDownloader_ProgressChanged_FileSize),
            local_size(p.received, false, 2),
            local_size(p.total, false, 2),
        );
        let g = layout(
            ui,
            wrapped(
                &text,
                13.0,
                Weight::SemiBold,
                colors.text_secondary(),
                inner.width(),
            ),
        );
        let gh = g.size().y;
        let space = (bar.top() - 8.0 - y).max(gh);
        painter.galley(
            pos2(inner.left(), y + (space - gh) / 2.0),
            g,
            colors.text_secondary(),
        );
    }
    paint_progress(ui, &colors, bar, Some(fraction));
    let mut close = false;
    if button_here(ui, &colors, &look, Content::label(label), true, button).clicked() {
        match downloader.status {
            DownloaderStatus::Waiting => {}
            DownloaderStatus::DownloadStarted => {
                if let Some(cancel) = &downloader.cancel {
                    cancel.cancel();
                }
                close = true;
            }
            DownloaderStatus::DownloadCompleted => {
                if let Some(path) = downloader.file.clone() {
                    use_release(state, path);
                }
            }
            DownloaderStatus::Failed => close = true,
        }
    }
    if state
        .updates
        .downloader
        .as_ref()
        .is_some_and(|d| d.status == DownloaderStatus::DownloadStarted)
    {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
    }
    close
}

/// The downloader closed: a download in flight stops (`OnClosing`).
pub fn downloader_closed(state: &mut AppState) {
    if let Some(downloader) = state.updates.downloader.take()
        && let Some(cancel) = downloader.cancel
    {
        cancel.cancel();
    }
}

/// QA: a downloader showing a finished, verified download.
pub fn qa_downloader(manifest: ReleaseManifest, done: bool) -> Downloader {
    let mut downloader = Downloader::new(manifest);
    if done {
        downloader.status = DownloaderStatus::DownloadCompleted;
        downloader.status_text =
            t(helpers::DownloaderForm_fileDownloader_DownloadCompleted_Download_completed_)
                .to_string();
        downloader.file = Some(PathBuf::from(crate::qa::fixture_path(&format!(
            "/tmp/Modlight/{}",
            downloader.manifest.file_name
        ))));
    } else {
        downloader.status = DownloaderStatus::DownloadStarted;
        downloader.status_text = t(helpers::DownloaderForm_StartDownload_Downloading_).to_string();
        downloader.progress = Some(DownloadProgress {
            received: 18_400_000,
            total: 41_200_000,
            bytes_per_second: 6_300_000.0,
        });
    }
    downloader
}

/// `button_at` in a child `Ui`: a hand-placed button must not move the layout's cursor.
fn button_here(
    ui: &mut Ui,
    colors: &Colors,
    look: &Look,
    content: Content,
    enabled: bool,
    rect: Rect,
) -> egui::Response {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    button_at(&mut child, colors, look, content, enabled, rect)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::qa::{Look, state};

    /// The timer was armed at start; the person then switched automatic checks off. When
    /// it fires it must not check (a request to the release host, and maybe the update
    /// question) and must not arm again: upstream disposes the timer when the setting goes
    /// off (`ConfigureAutoUpdate`).
    #[test]
    fn a_timer_armed_before_the_setting_went_off_checks_nothing() {
        let ctx = egui::Context::default();
        let mut app = state(&Look::default(), false);
        app.updates.scheduled = true;
        schedule(&mut app, Duration::ZERO);
        app.config.set("AutoCheckUpdate", serde_json::json!(false));

        let deadline = Instant::now() + Duration::from_secs(5);
        while app.updates.scheduled && Instant::now() < deadline {
            for completion in app.rt.drain() {
                completion(&mut app, &ctx);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!app.updates.scheduled, "the timer is not armed again");
        assert!(
            app.updates.first_check,
            "no check ran after the setting went off"
        );
    }

    #[test]
    fn only_an_installer_is_started() {
        // Windows' release may be the installer or the folder build (`build-app.ps1`
        // makes only a `.zip`): only the installer has anything to run with /UPDATE.
        assert!(is_installer("Modlight-setup.exe"));
        assert!(is_installer("Modlight-0.7.8-setup.EXE"));
        assert!(!is_installer("Modlight-0.7.8-windows-x64.zip"));
        assert!(!is_installer("Modlight-0.7.8.dmg"));
        // A Mac never installs anything; Windows installs only the installer.
        assert_eq!(installs("Modlight-setup.exe"), cfg!(windows));
        assert!(!installs("Modlight-0.7.8-windows-x64.zip"));
    }
}
