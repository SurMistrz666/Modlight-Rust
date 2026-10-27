//! The app's own update: the checks (`GitHubUpdateManager`'s timer, now against the
//! Downloads of the server the app is signed in to), the "update is available" question
//! (`UpdateMessageWindow`) and the downloader (`DownloaderWindow`), which installs.
//!
//! Checks run on start and every 30 minutes while `AutoCheckUpdate` is on, and on "Check
//! now" ([`check_now`], which puts the question up for a newer build), always only while
//! signed in (`modlight_core::update`). About checks when it opens and on its own "Check
//! now" without putting anything up ([`check_quietly`]); it links the question when a
//! newer build is there. The question asks first; Yes
//! downloads the build with the account's token and refuses it unless its size and
//! SHA-256 are the listing's, then prepares it (`update::install`: unpacked, and on a Mac
//! checked to be signed by the same developer), starts the helper that swaps it in once
//! the app has quit, and quits. A copy that cannot replace itself (not an app bundle, a
//! read-only place, an unsigned Mac build, another signer) says why and offers the
//! server's download page instead; when that is known before the download (`preflight`),
//! Yes opens the page at once.
//!
//! What other screens read: `state.updates.checking`, `last_checked`, `last_outcome`
//! (the last check's [`UpdateOutcome`]), `preflight` (whether this copy installs updates
//! itself), `server`, and the windows' `message` and `downloader`; [`this_build`] (or
//! `modlight_core::build_info`) for this build. What they call: [`check_now`],
//! [`check_quietly`], [`offer`] (the question for a found build).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeZone, Utc};
use egui::{Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::build_info::BuildIdentity;
use modlight_core::components::download::DownloadProgress;
use modlight_core::format::local_size;
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, helpers, port, t};
use modlight_core::update::install::{self, InstallProblem, Installed};
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

/// What happens to a verified download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallState {
    /// Not started (downloading, a failed download, a QA window).
    Idle,
    /// Being unpacked and checked.
    Preparing,
    /// The helper runs; the app is quitting to let it work.
    Restarting,
    /// Not installed, for this reason; the download page is offered instead.
    Refused(InstallProblem),
}

/// The downloader window's state.
pub struct Downloader {
    pub manifest: ReleaseManifest,
    pub status: DownloaderStatus,
    pub status_text: String,
    pub progress: Option<DownloadProgress>,
    pub file: Option<PathBuf>,
    /// After the download: preparing, restarting, or why not.
    pub install: InstallState,
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
            install: InstallState::Idle,
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
    /// A check is on its way (the timer's, "Check now"'s, About's).
    pub checking: bool,
    /// When the last check ended, and what it found.
    pub last_checked: Option<DateTime<Utc>>,
    pub last_outcome: Option<UpdateOutcome>,
    /// The server the last check asked; its download page is where a person goes when
    /// this copy does not install updates itself.
    pub server: Option<String>,
    /// Whether this copy can replace itself (asked in the background with the first
    /// check): `None` until known.
    pub preflight: Option<Result<Installed, InstallProblem>>,
    /// What earlier updates left was put away (once a run).
    pub cleaned: bool,
    /// "Check now" was asked for while another check ran: that one's answer puts the
    /// question up.
    wants_offer: bool,
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
            checking: false,
            last_checked: None,
            last_outcome: None,
            server: None,
            preflight: None,
            cleaned: false,
            wants_offer: false,
        }
    }
}

impl Updates {
    /// Whether this copy installs updates itself, as far as is known (unknown counts as
    /// yes: it is asked again before anything is installed).
    pub fn installs(&self) -> bool {
        !matches!(self.preflight, Some(Err(_)))
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

/// This build as the update windows show it: the real one, or in a headless render the
/// fixture's (a picture must not change with every commit).
pub fn this_build(state: &AppState) -> BuildIdentity {
    if state.mode == crate::app::Mode::Shot {
        BuildIdentity {
            version: modlight_core::build_info::VERSION.to_string(),
            build: "1a2b3c4d".into(),
            built: Utc.with_ymd_and_hms(2026, 10, 20, 9, 41, 0).single(),
        }
    } else {
        BuildIdentity::current()
    }
}

/// `ConfigureAutoUpdate`: in the real app, what earlier updates left is put away, and with
/// the setting on the timer is armed: it fires at once and then every 30 minutes.
pub fn on_startup(state: &mut AppState) {
    if state.mode != crate::app::Mode::Normal {
        return;
    }
    if !state.updates.cleaned {
        state.updates.cleaned = true;
        state.rt.spawn_blocking(|| {
            install::cleanup(&|path: &std::path::Path| {
                crate::platform::system::move_to_trash(path)
            });
            |_: &mut AppState, _: &egui::Context| {}
        });
    }
    if !auto_check(state) || state.updates.scheduled {
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
/// disposes the timer as soon as automatic checks are switched off, so an old timer must
/// not reach the server (or put up the question) after the person said no. Signed out,
/// nothing is asked and the timer waits for the next round.
fn timer_fired(state: &mut AppState) {
    if !state.updates.enabled || !auto_check(state) {
        state.updates.scheduled = false;
        return;
    }
    if !begin_check(state, Check::Timer) {
        schedule(state, update::CHECK_INTERVAL);
    }
}

/// Who asked for a check, which decides what its answer does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Check {
    /// The timer: the question for a newer build (unless the person said No this run),
    /// then the next round.
    Timer,
    /// "Check now": the question for a newer build, whatever an earlier No said.
    Manual,
    /// The About window: only `state.updates` (About shows its link).
    Quiet,
}

/// "Check now": ask the server at once, whatever the timer and an earlier No; a newer
/// build opens the question. Signed out (or while a check runs) it does nothing.
pub fn check_now(state: &mut AppState) {
    if state.mode == crate::app::Mode::Shot {
        return;
    }
    begin_check(state, Check::Manual);
}

/// A check whose answer only goes to `state.updates` (the About window opening, or its
/// "Check now": About shows what was found and links the question).
pub fn check_quietly(state: &mut AppState) {
    if state.mode == crate::app::Mode::Shot {
        return;
    }
    begin_check(state, Check::Quiet);
}

/// Start one check; false when none was started (signed out, or one is running).
fn begin_check(state: &mut AppState, kind: Check) -> bool {
    if state.updates.checking {
        state.updates.wants_offer |= kind == Check::Manual;
        return false;
    }
    let Some(credential) = state.credential() else {
        return false;
    };
    state.updates.checking = true;
    let api = state.api.clone();
    let preflight = state.updates.preflight.is_none() && state.mode == crate::app::Mode::Normal;
    state.rt.spawn(async move {
        let outcome = update::check(&api, &credential).await;
        let installed = if preflight {
            tokio::task::spawn_blocking(install::preflight).await.ok()
        } else {
            None
        };
        move |state: &mut AppState, _ctx: &egui::Context| {
            if let Some(installed) = installed {
                if let Err(problem) = &installed {
                    log::info!("Update: this copy does not install updates itself: {problem:?}");
                }
                state.updates.preflight = Some(installed);
            }
            finished(state, &credential.host, &credential.token, outcome, kind);
        }
    });
    true
}

/// A check ended. An answer about an account that is no longer signed in is dropped (as
/// every other answer is, `MACOS.md`); the timer's check arms the next one.
fn finished(state: &mut AppState, host: &str, token: &str, outcome: UpdateOutcome, kind: Check) {
    state.updates.checking = false;
    // "Check now" pressed while this check ran: its answer is the one asked for.
    let asked = std::mem::take(&mut state.updates.wants_offer);
    let current = state
        .credential()
        .is_some_and(|c| c.host == host && c.token == token);
    if current {
        state.updates.last_checked = Some(state.env.now());
        state.updates.server = Some(host.to_string());
        state.updates.last_outcome = Some(outcome.clone());
        match kind {
            Check::Manual => offer(state, outcome),
            _ if asked => offer(state, outcome),
            Check::Timer => checked(state, outcome),
            Check::Quiet => {}
        }
    }
    if kind == Check::Timer {
        if state.updates.enabled && auto_check(state) {
            schedule(state, update::CHECK_INTERVAL);
        } else {
            state.updates.scheduled = false;
        }
    }
}

/// The timer's check completed: offer what it found, unless a question is already up or
/// the person said No this run.
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

/// Put the question up for a found update now (a check the person asked for, the About
/// window's link): it comes forward, whatever an earlier No said.
pub fn offer(state: &mut AppState, outcome: UpdateOutcome) {
    if let UpdateOutcome::UpdateAvailable(manifest) = outcome
        && state.updates.message.is_none()
        && state.updates.downloader.is_none()
    {
        state.updates.message = Some(manifest);
        state.updates.message_activate = true;
    }
}

/// The question's text (`BuildMessage`): what will happen, both builds, the size.
pub fn message_text(manifest: &ReleaseManifest, current: &BuildIdentity, installs: bool) -> String {
    // A copy that installs updates itself asks "download and install it?"; one that
    // cannot (known before the download) sends the person to the download page, upstream's
    // portable question.
    let mut text = String::new();
    text.push_str(t(if installs {
        helpers::UpdateMessageBox_UpdateMessageBox_
    } else {
        helpers::UpdateMessageBox_UpdateMessageBox_Portable
    }));
    text.push_str("\n\n");
    text.push_str(t(helpers::UpdateMessageBox_UpdateMessageBox_CurrentVersion));
    text.push_str(": ");
    // With the commit: two builds of one version must not read the same.
    text.push_str(&current.label());
    text.push('\n');
    text.push_str(t(helpers::UpdateMessageBox_UpdateMessageBox_LatestVersion));
    text.push_str(": ");
    text.push_str(&manifest.label());
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
    let current = this_build(state);
    let installs = state.updates.installs();
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
            &message_text(&manifest, &current, installs),
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
        state.open_url(&update::download_page_url(&manifest.server));
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

/// The question was answered: Yes downloads and installs (or, for a copy known not to
/// install updates itself, opens the server's download page), No stops asking this run.
pub fn message_answered(state: &mut AppState, yes: bool) {
    let Some(manifest) = state.updates.message.take() else {
        return;
    };
    if !yes {
        state.updates.enabled = false;
        return;
    }
    if !state.updates.installs() {
        state.open_url(&update::download_page_url(&manifest.server));
        // The person fetches it from there: the timer does not ask again this run.
        state.updates.enabled = false;
        return;
    }
    let mut downloader = Downloader::new(manifest);
    start_download(state, &mut downloader);
    state.updates.downloader = Some(downloader);
}

/// `StartDownloadAsync`: reuse a verified copy, else fetch with the account's token, then
/// verify.
fn start_download(state: &mut AppState, downloader: &mut Downloader) {
    if downloader.status != DownloaderStatus::Waiting || state.mode == crate::app::Mode::Shot {
        return;
    }
    let Some(credential) = state.credential() else {
        downloader.status = DownloaderStatus::Failed;
        downloader.status_text = t(port::UpdateSignedOut).to_string();
        return;
    };
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
        let result = update::download(&manifest, &credential, &folder, &sink, &cancel).await;
        move |state: &mut AppState, _ctx: &egui::Context| download_done(state, result)
    });
}

fn download_done(state: &mut AppState, result: update::DownloadResult) {
    let Some(downloader) = state.updates.downloader.as_mut() else {
        return;
    };
    downloader.cancel = None;
    let failed = |downloader: &mut Downloader, text: &str| {
        downloader.status = DownloaderStatus::Failed;
        downloader.status_text = text.to_string();
        downloader.progress = None;
    };
    match result {
        update::DownloadResult::Verified(path) => {
            downloader.status = DownloaderStatus::DownloadCompleted;
            downloader.status_text =
                t(helpers::DownloaderForm_fileDownloader_DownloadCompleted_Download_completed_)
                    .to_string();
            downloader.file = Some(path);
            // `AutoStartInstall`: the person said yes to "download and install".
            begin_install(state);
        }
        update::DownloadResult::Cancelled => {
            state.updates.downloader = None;
        }
        update::DownloadResult::Incomplete(detail) => {
            log::warn!("Update: the download stopped short: {detail}");
            failed(downloader, t(helpers::DownloaderWindow_DownloadIncomplete));
        }
        update::DownloadResult::VerificationFailed => {
            failed(downloader, t(helpers::DownloaderWindow_VerificationFailed));
        }
        update::DownloadResult::OtherServer => {
            failed(downloader, t(port::UpdateOtherServer));
        }
    }
}

/// `InstallAsync`: unpack and check the verified download in the background, then start
/// the helper and quit. Only the real app installs anything.
fn begin_install(state: &mut AppState) {
    let real = state.mode == crate::app::Mode::Normal && !state.env.qa;
    let Some(downloader) = state.updates.downloader.as_mut() else {
        return;
    };
    let Some(zip) = downloader.file.clone() else {
        return;
    };
    if !real {
        log::info!("QA mode: not installing {}", zip.display());
        return;
    }
    if matches!(
        downloader.install,
        InstallState::Preparing | InstallState::Restarting
    ) {
        return;
    }
    downloader.install = InstallState::Preparing;
    downloader.status_text = t(port::UpdatePreparing).to_string();
    state.rt.spawn_blocking(move || {
        let prepared =
            install::preflight().and_then(|installed| install::prepare(&zip, &installed));
        move |state: &mut AppState, _ctx: &egui::Context| prepared_done(state, prepared)
    });
}

fn prepared_done(state: &mut AppState, prepared: Result<install::Prepared, InstallProblem>) {
    let Some(downloader) = state.updates.downloader.as_mut() else {
        // The window was closed meanwhile: nothing starts; the next start clears it away.
        return;
    };
    let started = prepared.and_then(|prepared| install::start(&prepared));
    match started {
        Ok(()) => {
            downloader.install = InstallState::Restarting;
            downloader.status_text = t(port::UpdateRestarting).to_string();
            // Making way for the helper: a running copy cannot be replaced.
            state.quit_requested = true;
        }
        Err(problem) => {
            log::warn!("Update: not installed: {problem:?}");
            if matches!(
                problem,
                InstallProblem::SignerDiffers | InstallProblem::NotAnUpdate(_)
            ) && let Some(file) = downloader.file.take()
            {
                // Not to be offered again from the cache.
                modlight_core::components::verify::try_delete(&file);
            }
            downloader.status_text = problem.text();
            downloader.install = InstallState::Refused(problem.clone());
            if !matches!(problem, InstallProblem::StartFailed(_)) {
                state.updates.preflight = Some(Err(problem));
            }
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
    let busy = matches!(
        downloader.install,
        InstallState::Preparing | InstallState::Restarting
    );
    let label = match (downloader.status, &downloader.install) {
        (DownloaderStatus::Waiting, _) => t(helpers::DownloaderWindow_Download),
        (DownloaderStatus::DownloadStarted, _) => t(helpers::DownloaderForm_StartDownload_Cancel),
        (DownloaderStatus::DownloadCompleted, InstallState::Refused(_)) => {
            t(port::OpenDownloadsPage)
        }
        (DownloaderStatus::DownloadCompleted, _) => {
            t(helpers::DownloaderForm_fileDownloader_DownloadCompleted_Install)
        }
        (DownloaderStatus::Failed, _) => t(helpers::DownloaderWindow_Close),
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
        (DownloaderStatus::DownloadCompleted, _) => Some(1.0),
        (_, Some(p)) if p.total > 0 => Some(p.received as f32 / p.total as f32),
        _ => Some(0.0),
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
    // Preparing shows a bar that moves without a measure.
    paint_progress(
        ui,
        &colors,
        bar,
        if downloader.install == InstallState::Preparing {
            None
        } else {
            fraction
        },
    );
    let mut close = false;
    let server = downloader.manifest.server.clone();
    let clicked = button_here(ui, &colors, &look, Content::label(label), !busy, button).clicked();
    let (install_now, open_page) = if clicked {
        match (downloader.status, &downloader.install) {
            (DownloaderStatus::Waiting, _) => (false, false),
            (DownloaderStatus::DownloadStarted, _) => {
                if let Some(cancel) = &downloader.cancel {
                    cancel.cancel();
                }
                close = true;
                (false, false)
            }
            (DownloaderStatus::DownloadCompleted, InstallState::Refused(_)) => {
                close = true;
                (false, true)
            }
            (DownloaderStatus::DownloadCompleted, _) => (true, false),
            (DownloaderStatus::Failed, _) => {
                close = true;
                (false, false)
            }
        }
    } else {
        (false, false)
    };
    if open_page {
        state.open_url(&update::download_page_url(&server));
    }
    if install_now {
        begin_install(state);
    }
    if state.updates.downloader.as_ref().is_some_and(|d| {
        d.status == DownloaderStatus::DownloadStarted || d.install == InstallState::Preparing
    }) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
    }
    close
}

/// The downloader closed: a download in flight stops (`OnClosing`).
/// A refused install, once read, ends the timer's questions for this run: the same build
/// would only be refused again.
pub fn downloader_closed(state: &mut AppState) {
    let Some(downloader) = state.updates.downloader.take() else {
        return;
    };
    if let Some(cancel) = downloader.cancel {
        cancel.cancel();
    }
    if matches!(downloader.install, InstallState::Refused(_)) {
        state.updates.enabled = false;
    }
}

/// QA: a downloader showing a finished, verified download, or one under way.
pub fn qa_downloader(manifest: ReleaseManifest, done: bool) -> Downloader {
    let mut downloader = Downloader::new(manifest);
    if done {
        downloader.status = DownloaderStatus::DownloadCompleted;
        downloader.status_text =
            t(helpers::DownloaderForm_fileDownloader_DownloadCompleted_Download_completed_)
                .to_string();
        downloader.file = Some(PathBuf::from(crate::qa::fixture_path(&format!(
            "/tmp/Modlight/Update/{}",
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

    fn release(server: &str) -> ReleaseManifest {
        let build = modlight_api::parse_build(
            r#"{"id":"m2","platform":"macos","arch":"arm64","version":"0.7.9","build":"9f8e7d6c","built":"2026-10-27T14:03:05Z","size":41200000,"sha256":"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824","filename":"Modlight-0.7.9-9f8e7d6c-macos-arm64.zip","note":"Faster uploads","url":"/downloads/m2/Modlight-0.7.9-9f8e7d6c-macos-arm64.zip"}"#,
        )
        .unwrap();
        update::release_from_build(&build, server).unwrap()
    }

    fn drain_until(app: &mut AppState, done: impl Fn(&AppState) -> bool) {
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done(app) && Instant::now() < deadline {
            for completion in app.rt.drain() {
                completion(app, &ctx);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The timer was armed at start; the person then switched automatic checks off. When
    /// it fires it must not check (a request to the server, and maybe the update question)
    /// and must not arm again: upstream disposes the timer when the setting goes off
    /// (`ConfigureAutoUpdate`).
    #[test]
    fn a_timer_armed_before_the_setting_went_off_checks_nothing() {
        let mut app = state(&Look::default(), false);
        app.updates.scheduled = true;
        schedule(&mut app, Duration::ZERO);
        app.config.set("AutoCheckUpdate", serde_json::json!(false));
        drain_until(&mut app, |app| !app.updates.scheduled);
        assert!(!app.updates.scheduled, "the timer is not armed again");
        assert!(
            app.updates.first_check,
            "no check ran after the setting went off"
        );
        assert!(!app.updates.checking);
    }

    /// Signed out, the timer asks nobody and waits for its next round.
    #[test]
    fn signed_out_nothing_is_asked() {
        let mut app = state(&Look::default(), false);
        assert!(app.credential().is_none());
        app.updates.scheduled = true;
        timer_fired(&mut app);
        assert!(!app.updates.checking, "no check without an account");
        assert!(app.updates.scheduled, "the timer waits for the next round");
        check_now(&mut app);
        assert!(!app.updates.checking);
        assert_eq!(app.updates.last_outcome, None);
    }

    #[test]
    fn the_timers_find_is_offered_once_and_a_no_stops_it() {
        let mut app = state(&Look::default(), false);
        let found = UpdateOutcome::UpdateAvailable(release("https://files.example.com"));
        checked(&mut app, found.clone());
        assert!(app.updates.message.is_some());
        assert!(
            app.updates.message_activate,
            "the first check comes forward"
        );
        message_answered(&mut app, false);
        assert!(!app.updates.enabled);
        checked(&mut app, found.clone());
        assert!(app.updates.message.is_none(), "No holds for this run");
        // Asked for by the person, it is offered anyway, in front.
        offer(&mut app, found);
        assert!(app.updates.message.is_some());
        assert!(app.updates.message_activate);
        offer(&mut app, UpdateOutcome::UpToDate);
        assert!(app.updates.message.is_some());
    }

    /// A copy known not to replace itself sends Yes to the server's download page.
    #[test]
    fn a_copy_that_cannot_install_opens_the_download_page() {
        let mut app = state(&Look::default(), false);
        app.updates.preflight = Some(Err(InstallProblem::Unsigned));
        assert!(!app.updates.installs());
        app.updates.message = Some(release("https://files.example.com"));
        message_answered(&mut app, true);
        assert!(app.updates.downloader.is_none());
        assert_eq!(
            app.effects,
            vec![crate::state::Effect::OpenUrl(
                "https://files.example.com/download".into()
            )]
        );
        assert!(
            !app.updates.enabled,
            "the timer does not ask again this run"
        );
    }

    #[test]
    fn the_question_names_both_builds() {
        modlight_core::l10n::set_thread_language(Some(modlight_core::l10n::Language::English));
        let current = BuildIdentity {
            version: "0.7.8".into(),
            build: "1a2b3c4d".into(),
            built: None,
        };
        let manifest = release("https://files.example.com");
        let text = message_text(&manifest, &current, true);
        assert!(text.starts_with("Would you like to download and install it?"));
        assert!(text.contains("Current version: 0.7.8 (1a2b3c4d)"));
        assert!(text.contains("Latest version: 0.7.9 (9f8e7d6c)"));
        let text = message_text(&manifest, &current, false);
        assert!(text.starts_with("Would you like to download it?"));
        modlight_core::l10n::set_thread_language(None);
    }

    /// A finished download in a QA window is never installed, and nothing quits.
    #[test]
    fn qa_never_installs() {
        let mut app = state(&Look::default(), false);
        app.updates.downloader = Some(qa_downloader(release("https://files.example.com"), true));
        begin_install(&mut app);
        let downloader = app.updates.downloader.as_ref().unwrap();
        assert_eq!(downloader.install, InstallState::Idle);
        assert!(!app.quit_requested);
    }

    /// A refused install keeps the reason, offers the page and remembers that this copy
    /// does not install updates itself.
    #[test]
    fn a_refused_install_says_why_and_offers_the_page() {
        let mut app = state(&Look::default(), false);
        let folder = tempfile::tempdir().unwrap();
        let zip = folder
            .path()
            .join("Modlight-0.7.9-9f8e7d6c-macos-arm64.zip");
        std::fs::write(&zip, "verified, but signed by somebody else").unwrap();
        let mut downloader = qa_downloader(release("https://files.example.com"), true);
        downloader.file = Some(zip.clone());
        app.updates.downloader = Some(downloader);
        prepared_done(&mut app, Err(InstallProblem::SignerDiffers));
        let downloader = app.updates.downloader.as_ref().unwrap();
        assert_eq!(
            downloader.install,
            InstallState::Refused(InstallProblem::SignerDiffers)
        );
        assert_eq!(downloader.status_text, InstallProblem::SignerDiffers.text());
        assert!(downloader.file.is_none(), "a foreign build is not kept");
        assert!(!zip.exists(), "nor offered again from the download folder");
        assert!(!app.quit_requested);
        assert_eq!(
            app.updates.preflight,
            Some(Err(InstallProblem::SignerDiffers))
        );
        assert!(app.updates.enabled);
        downloader_closed(&mut app);
        assert!(app.updates.downloader.is_none());
        assert!(
            !app.updates.enabled,
            "read and closed: no more questions this run"
        );
    }

    /// "Check now" pressed while About's quiet check runs: that answer puts the question up.
    #[test]
    fn check_now_during_a_running_check_is_answered_by_it() {
        let mut app = state(&Look::default(), true);
        let credential = app.credential().expect("the QA account");
        app.updates.checking = true;
        check_now(&mut app);
        assert!(app.updates.wants_offer);
        finished(
            &mut app,
            &credential.host,
            &credential.token,
            UpdateOutcome::UpdateAvailable(release(&credential.host)),
            Check::Quiet,
        );
        assert!(!app.updates.wants_offer);
        assert!(app.updates.message.is_some(), "the question is up");
        assert!(app.updates.last_checked.is_some());
        assert_eq!(
            app.updates.server.as_deref(),
            Some(credential.host.as_str())
        );
    }

    /// An answer about an account that signed out meanwhile is dropped.
    #[test]
    fn an_answer_for_another_account_is_dropped() {
        let mut app = state(&Look::default(), false);
        app.updates.checking = true;
        finished(
            &mut app,
            "https://files.example.com",
            "token-0123456789",
            UpdateOutcome::UpdateAvailable(release("https://files.example.com")),
            Check::Manual,
        );
        assert!(!app.updates.checking);
        assert_eq!(app.updates.last_outcome, None);
        assert!(app.updates.message.is_none());
    }
}
