//! The Download tab (`DownloadView`, `DownloadViewModel`): the address box, the YouTube
//! panel in the corner, the notices about missing tools, the format and destination
//! strip, and the queue. One download runs at a time; what a row is and how it changes
//! lives in `modlight_core::download::item`, the run in `modlight_core::download::run`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use egui::{Align2, Color32, Id, Order, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::components;
use modlight_core::download::cookies::YouTubeCookies;
use modlight_core::download::item::{Delivery, DownloadItem, DownloadQueue, State};
use modlight_core::download::run::{self, FormatResult, Progress, RunEvent, RunSpec};
use modlight_core::download::tool::{SearchPlaces, ToolPaths};
use modlight_core::download::{
    AudioFormat, Destination, DownloadFormat, DownloadLink, MediaKind, Quality, settings,
};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t};

use crate::components_window::AfterClose;
use crate::state::AppState;
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Field, Fill, Look, button_at, button_width, combo_rounded, icon_galley, layout,
    paint_icon, paint_progress, trimmed, wrapped,
};

/// What the running yt-dlp said since the last frame, per row: only the latest of each.
#[derive(Clone, Debug, Default)]
struct Live {
    progress: Option<Progress>,
    title: Option<String>,
    format: Option<FormatResult>,
}

/// The tab's state.
#[derive(Default)]
pub struct DownloadTab {
    pub queue: DownloadQueue,
    pub url_text: String,
    pub url_error: String,
    pub tools: ToolPaths,
    pub tools_read: bool,
    /// Something installed or replaced a tool: look again on the next frame.
    pub refresh_tools_soon: bool,
    pub signed_in: bool,
    pub signing_in: bool,
    pub sign_in_message: String,
    /// The panel under the YouTube button is open.
    pub popup_open: bool,
    /// The frame the tab was last drawn in, to notice it being shown.
    last_frame: Option<u64>,
    live: Arc<Mutex<HashMap<u64, Live>>>,
    /// A download waits for the components window to close (ffmpeg was offered).
    waiting_for_components: bool,
    scratch_swept: bool,
    /// The address field should take the focus.
    focus_url: bool,
}

impl DownloadTab {
    fn tool_ready(&self) -> bool {
        self.tools.yt_dlp.is_some()
    }
}

// ------------------------------------------------------------------ the model's verbs

impl AppState {
    fn cookies(&self) -> YouTubeCookies {
        YouTubeCookies::new(&self.env.profile)
    }

    /// `RefreshTool`: look for yt-dlp, ffmpeg and QuickJS again. Not in a QA render,
    /// whose fixtures say what is installed.
    pub fn downloads_refresh_tools(&mut self) {
        if self.mode == crate::app::Mode::Shot {
            return;
        }
        self.downloads.tools = SearchPlaces::for_profile(&self.env.profile).find_tools();
        self.downloads.tools_read = true;
    }

    /// `RefreshSignIn`: the disk, every time.
    pub fn downloads_refresh_sign_in(&mut self) {
        if self.mode == crate::app::Mode::Shot {
            return;
        }
        self.downloads.signed_in = self.cookies().is_signed_in();
    }

    /// `Refresh`: when the tab is shown.
    fn downloads_shown(&mut self) {
        self.downloads_refresh_tools();
        self.downloads_refresh_sign_in();
        self.downloads.focus_url = true;
        if !self.downloads.scratch_swept && self.mode != crate::app::Mode::Shot {
            self.downloads.scratch_swept = true;
            self.rt.spawn_blocking(|| {
                modlight_core::download::sweep_scratch(&modlight_core::download::scratch_folder());
                |_: &mut AppState, _: &egui::Context| {}
            });
        }
        self.downloads_pump();
    }

    fn current_format(&self) -> DownloadFormat {
        settings::current_format(&self.config)
    }

    /// The label of the entry in force: "Best available", "1080p", "MP3".
    fn current_format_label(&self) -> String {
        let format = self.current_format();
        if format.is_audio() {
            audio_label(format.audio_format).to_string()
        } else {
            video_label(format.quality)
        }
    }

    /// `StartAsync`: check the address, offer ffmpeg, ask for the folder, queue.
    pub fn downloads_start(&mut self) {
        if DownloadLink::parse(&self.downloads.url_text).is_none() {
            self.downloads.url_error = t(s::Download_BadUrl).to_string();
            return;
        }
        if self.downloads.tools.ffmpeg.is_none() {
            // Offered, and the answer is not acted on: the download happens either way.
            if !crate::components_window::require(
                self,
                components::FFMPEG,
                s::Components_FeatureDownload,
                AfterClose::DownloadContinue,
            ) {
                self.downloads.waiting_for_components = true;
                return;
            }
        }
        self.downloads_continue_start();
    }

    fn downloads_continue_start(&mut self) {
        let destination = settings::destination(&self.config);
        let fixed = settings::folder(&self.config);
        let mut chosen = None;
        if destination != Destination::Modlight && fixed.is_empty() {
            match self.pick_download_folder() {
                Some(folder) => chosen = Some(folder),
                // Closed the chooser: the address stays, nothing else happens.
                None => return,
            }
        }
        self.downloads_enqueue(chosen);
    }

    /// The folder chooser; in QA modes `<profile>/Downloads` without a panel.
    fn pick_download_folder(&mut self) -> Option<PathBuf> {
        if self.env.qa {
            return Some(self.env.profile.join("Downloads"));
        }
        rfd::FileDialog::new()
            .set_title(t(s::Download_FolderPickerTitle))
            .pick_folder()
    }

    /// `Enqueue`.
    pub fn downloads_enqueue(&mut self, chosen: Option<PathBuf>) -> bool {
        let Some(link) = DownloadLink::parse(&self.downloads.url_text) else {
            self.downloads.url_error = t(s::Download_BadUrl).to_string();
            return false;
        };
        let destination = settings::destination(&self.config);
        let scratch = destination == Destination::Modlight;
        let folder = if scratch {
            modlight_core::download::scratch_folder()
        } else {
            match chosen.or_else(|| {
                let fixed = settings::folder(&self.config);
                (!fixed.is_empty()).then(|| PathBuf::from(fixed))
            }) {
                Some(folder) => folder,
                None => {
                    self.downloads.url_error = t(s::Download_FolderMissing).to_string();
                    return false;
                }
            }
        };
        let label = self.current_format_label();
        let format = self.current_format();
        self.downloads
            .queue
            .enqueue(link, folder, scratch, destination, format, &label);
        self.downloads.url_text.clear();
        self.downloads.url_error.clear();
        self.downloads_pump();
        true
    }

    /// `Pump`: start the next waiting row if nothing is running.
    pub fn downloads_pump(&mut self) {
        if self.mode == crate::app::Mode::Shot || !self.downloads.tool_ready() {
            return;
        }
        let Some(id) = self.downloads.queue.next_to_start() else {
            return;
        };
        let Some(tool) = self.downloads.tools.yt_dlp.clone() else {
            return;
        };
        let cookies = self.cookies();
        let tools = self.downloads.tools.clone();
        let profile = self.env.profile.clone();
        let waker = self.rt.waker();
        let live = self.downloads.live.clone();
        let Some(item) = self.downloads.queue.get_mut(id) else {
            return;
        };
        let Some(cancel) = item.start() else {
            return;
        };
        let spec = RunSpec {
            tool,
            cookies: cookies.file_for(&item.link),
            link: item.link.clone(),
            folder: item.folder.clone(),
            format: item.format,
            ffmpeg: tools.ffmpeg,
            quickjs: tools.quickjs,
        };
        let events = Arc::new(move |event: RunEvent| {
            if let Ok(mut map) = live.lock() {
                let entry = map.entry(id).or_default();
                match event {
                    RunEvent::Progress(p) => entry.progress = Some(p),
                    RunEvent::Title(title) => entry.title = Some(title),
                    RunEvent::Format(f) => entry.format = Some(f),
                }
            }
            waker();
        });
        self.rt.spawn(async move {
            let outcome = run::run(&spec, &profile, events, &cancel).await;
            move |state: &mut AppState, _ctx: &egui::Context| state.downloads_finished(id, outcome)
        });
    }

    /// Fold what the runs said into their rows.
    fn downloads_apply_live(&mut self) {
        let binary = settings::binary_units(&self.config);
        let updates: Vec<(u64, Live)> = self
            .downloads
            .live
            .lock()
            .map(|mut map| map.drain().collect())
            .unwrap_or_default();
        for (id, live) in updates {
            if let Some(item) = self.downloads.queue.get_mut(id) {
                if let Some(title) = live.title {
                    item.named(&title);
                }
                if let Some(format) = live.format {
                    item.chose(&format);
                }
                if let Some(progress) = live.progress {
                    item.apply(&progress, binary);
                }
            }
        }
    }

    fn downloads_finished(&mut self, id: u64, outcome: run::Outcome) {
        self.downloads_apply_live();
        let binary = settings::binary_units(&self.config);
        let delivery = self
            .downloads
            .queue
            .get_mut(id)
            .and_then(|item| item.finish(&outcome, binary));
        match delivery {
            Some(Delivery::Saved(path)) => self.download_saved_notice(&path),
            Some(Delivery::Upload(path)) => self.download_upload(id, &path),
            None => {}
        }
        self.downloads_pump();
    }

    /// `Deliver`: through the same upload flow a capture takes, with the finished
    /// notice's buttons rebuilt around a file that is already on the server.
    fn download_upload(&mut self, id: u64, path: &std::path::Path) {
        use modlight_core::tasks::settings::{NotificationActionButton, ToastClickAction};
        let both = self
            .downloads
            .queue
            .get(id)
            .is_some_and(|i| i.destination == Destination::Both);
        let mut settings = self.default_task_settings();
        let general = &mut settings.general_settings;
        let mut buttons = Vec::new();
        if both {
            buttons.push(NotificationActionButton {
                label: Some(t(s::Download_ToastOpenFolder).to_string()),
                ..NotificationActionButton::new(ToastClickAction::OpenFolder)
            });
        }
        buttons.push(NotificationActionButton {
            label: Some(t(s::Download_ToastCopyLink).to_string()),
            ..NotificationActionButton::new(ToastClickAction::CopyUrl)
        });
        general.toast_window_buttons = buttons;
        general.toast_window_left_click_action = ToastClickAction::OpenUrl;
        general.toast_window_middle_click_action = ToastClickAction::CopyUrl;
        general.toast_window_right_click_action = ToastClickAction::CloseNotification;
        if !path.is_file() {
            if let Some(item) = self.downloads.queue.get_mut(id) {
                item.delivery_failed(&path.display().to_string());
            }
            return;
        }
        let ctx = self.uploads.ctx.clone();
        modlight_core::tasks::upload_manager::upload_file(
            &mut self.uploads.tasks,
            path,
            &settings,
            &ctx,
        );
    }

    /// `ShowSavedToast`: a local-only download never becomes a task, so it raises the
    /// desktop notice itself - with Upload on it, because the file is not on the server.
    fn download_saved_notice(&mut self, path: &std::path::Path) {
        use modlight_core::tasks::host::NotificationConfig;
        use modlight_core::tasks::settings::{NotificationActionButton, ToastClickAction};
        let general = self.default_task_settings().general_settings;
        if !general.show_toast_notification_after_task_completed {
            return;
        }
        if general.disable_notifications_on_fullscreen
            && self.mode == crate::app::Mode::Normal
            && modlight_macos::system::active_window_fullscreen()
        {
            return;
        }
        let button = |action, key| NotificationActionButton {
            label: Some(t(key).to_string()),
            ..NotificationActionButton::new(action)
        };
        let config = NotificationConfig {
            duration_ms: general.toast_window_duration * 1000.0,
            fade_ms: general.toast_window_fade_duration * 1000.0,
            placement: general.toast_window_placement,
            size: general.toast_window_size,
            left_click: ToastClickAction::OpenFile,
            right_click: ToastClickAction::CloseNotification,
            middle_click: ToastClickAction::OpenFolder,
            button_size: general.toast_window_button_size,
            buttons: vec![
                button(ToastClickAction::OpenFile, s::Download_ToastOpen),
                button(ToastClickAction::OpenFolder, s::Download_ToastOpenFolder),
                button(ToastClickAction::Upload, s::Download_ToastUpload),
            ],
            auto_hide: general.toast_window_auto_hide,
            file_path: path.display().to_string(),
            image: None,
            title: t(s::Download_ToastSavedTitle).to_string(),
            text: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            url: String::new(),
        };
        self.windows.notification =
            Some(crate::notification::Notification::task(config, self.time));
    }

    /// `SignInAsync`: open the sign-in window; when a session is captured, every row
    /// that failed for want of one runs again.
    pub fn downloads_sign_in(&mut self) {
        if self.downloads.signing_in {
            return;
        }
        self.downloads.popup_open = false;
        self.downloads.sign_in_message.clear();
        if !matches!(self.mode, crate::app::Mode::Normal | crate::app::Mode::Qa) {
            log::info!("QA: the YouTube sign-in window is not opened in automated runs.");
            return;
        }
        let language = match self.language {
            modlight_core::l10n::Language::Polish => "pl",
            modlight_core::l10n::Language::English => "en",
        };
        let opened = modlight_macos::webview::open_youtube_sign_in(
            t(s::Download_SignInWindowTitle),
            &modlight_core::download::cookies::login_url(language),
            self.colors.dark,
        );
        if opened {
            self.downloads.signing_in = true;
        } else {
            self.downloads.sign_in_message = t(s::Download_SignInFailed).to_string();
        }
    }

    /// What the sign-in window decided, when it has closed.
    pub fn downloads_poll_sign_in(&mut self) {
        let Some(result) = modlight_macos::webview::take_result() else {
            return;
        };
        self.downloads.signing_in = false;
        let mut captured = false;
        match result {
            modlight_macos::webview::SignInResult::Captured(cookies) => {
                let jar: Vec<modlight_core::download::cookies::NetscapeCookie> = cookies
                    .into_iter()
                    .map(|c| modlight_core::download::cookies::NetscapeCookie {
                        domain: c.domain,
                        path: c.path,
                        name: c.name,
                        value: c.value,
                        secure: c.secure,
                        expires_unix: c.expires_unix,
                    })
                    .collect();
                match self.cookies().save(&jar) {
                    Ok(count) if count > 0 => {
                        captured = true;
                        self.downloads.sign_in_message = t(s::Download_SignInDone).to_string();
                    }
                    _ => self.downloads.sign_in_message = t(s::Download_SignInFailed).to_string(),
                }
            }
            modlight_macos::webview::SignInResult::Failed(reason) => {
                log::warn!("Download: the sign-in window failed: {reason}");
                self.downloads.sign_in_message = t(s::Download_SignInFailed).to_string();
            }
            // Closed without finishing: not an error.
            modlight_macos::webview::SignInResult::Abandoned => {}
        }
        self.downloads_refresh_sign_in();
        if captured {
            self.downloads.queue.retry_sign_in_failures();
            self.downloads_pump();
        }
    }

    pub fn downloads_sign_out(&mut self) {
        self.cookies().sign_out();
        self.downloads.sign_in_message.clear();
        self.downloads_refresh_sign_in();
    }

    /// `UpdateToolAndRetryAsync`: fetch a newer yt-dlp and try this row again.
    fn download_update_tool(&mut self, id: u64) {
        let Some(item) = self.downloads.queue.get_mut(id) else {
            return;
        };
        if !item.can_update_tool() {
            return;
        }
        item.updating_tool = true;
        item.detail = t(s::Download_UpdatingTool).to_string();
        let profile = self.env.profile.clone();
        self.rt.spawn(async move {
            let result = match components::find(components::YT_DLP) {
                Some(component) => {
                    components::updates::update(
                        &profile,
                        component,
                        None,
                        false,
                        &tokio_util::sync::CancellationToken::new(),
                    )
                    .await
                }
                None => components::updates::UpdateResult {
                    outcome: components::updates::UpdateOutcome::NotTracked,
                    version: String::new(),
                    detail: String::new(),
                },
            };
            move |state: &mut AppState, _ctx: &egui::Context| {
                use components::updates::UpdateOutcome;
                if result.installed() {
                    state.downloads_refresh_tools();
                }
                let Some(item) = state.downloads.queue.get_mut(id) else {
                    return;
                };
                item.updating_tool = false;
                if result.installed() {
                    item.detail = format(
                        t(s::Download_UpdatedTool),
                        &[Arg::from(result.version.as_str())],
                    );
                    item.retry();
                    state.downloads_pump();
                    return;
                }
                item.detail = match result.outcome {
                    UpdateOutcome::AlreadyCurrent => format(
                        t(s::Download_UpdateToolCurrent),
                        &[Arg::from(result.version.as_str())],
                    ),
                    UpdateOutcome::CheckFailed => t(s::Download_UpdateToolOffline).to_string(),
                    UpdateOutcome::NotOurs if !result.detail.is_empty() => format(
                        t(s::Download_UpdateToolNotOurs),
                        &[Arg::from(result.detail.as_str())],
                    ),
                    UpdateOutcome::Cancelled => t(s::Download_UpdateToolFailedPlain).to_string(),
                    _ if !result.detail.is_empty() => format(
                        t(s::Download_UpdateToolFailed),
                        &[Arg::from(result.detail.as_str())],
                    ),
                    _ => t(s::Download_UpdateToolFailedPlain).to_string(),
                };
            }
        });
    }

    /// Stop everything: the app is going away.
    pub fn downloads_shutdown(&mut self) {
        self.downloads.queue.shutdown();
        modlight_macos::webview::close_youtube_sign_in();
    }
}

/// The components window closed; the Download tab carries on.
pub fn after_components(state: &mut AppState, then: AfterClose, _satisfied: bool) {
    state.downloads_refresh_tools();
    match then {
        AfterClose::DownloadRefresh => state.downloads_pump(),
        AfterClose::DownloadContinue => {
            if std::mem::take(&mut state.downloads.waiting_for_components) {
                state.downloads_continue_start();
            }
        }
        AfterClose::Recording(..) => {}
    }
}

/// Every frame in the real app: the sign-in window's answer, and tools that changed.
pub fn tick(state: &mut AppState) {
    state.downloads_poll_sign_in();
    if std::mem::take(&mut state.downloads.refresh_tools_soon) {
        state.downloads_refresh_tools();
        state.downloads_pump();
    }
}

// ------------------------------------------------------------------ texts

pub fn video_label(quality: Quality) -> String {
    match quality.resolution() {
        0 => t(s::Download_QualityBest).to_string(),
        height => format(t(s::Download_QualityHeight), &[Arg::from(height)]),
    }
}

pub fn audio_label(audio: AudioFormat) -> &'static str {
    match audio {
        AudioFormat::Mp3 => t(s::Download_AudioMp3),
        AudioFormat::M4a => t(s::Download_AudioM4a),
    }
}

fn format_note(choice: &DownloadFormat) -> String {
    if choice.is_audio() {
        match choice.audio_format {
            AudioFormat::Mp3 => t(s::Download_FormatNoteMp3).to_string(),
            AudioFormat::M4a => t(s::Download_FormatNoteM4a).to_string(),
        }
    } else {
        match choice.quality.resolution() {
            0 => t(s::Download_FormatNoteBest).to_string(),
            height => format(t(s::Download_FormatNoteHeight), &[Arg::from(height)]),
        }
    }
}

fn format_hint(format: &DownloadFormat) -> String {
    // The recipe of the entry, as the picker builds it (YouTube, with ffmpeg).
    let recipe = if format.is_audio() {
        DownloadFormat::new(MediaKind::Audio, Quality::Best, format.audio_format).recipe(true)
    } else {
        DownloadFormat::new(MediaKind::Video, format.quality, AudioFormat::M4a).recipe(true)
    };
    modlight_core::l10n::format(t(s::Download_QualityHint), &[Arg::from(recipe)])
}

// ------------------------------------------------------------------ looks

/// `Button.dl-primary`: 38 tall, 16 padding, 10 corners, the solid accent.
fn primary_look(colors: &Colors) -> Look {
    Look {
        height: 38.0,
        padding_x: 16.0,
        radius: 10.0,
        fill: Fill::Solid(colors.accent_start()),
        hover_fill: Fill::Solid(colors.accent_start()),
        pressed_fill: Fill::Solid(colors.accent_start()),
        border: None,
        hover_border: None,
        text: colors.accent_foreground(),
        hover_text: colors.accent_foreground(),
        string_text: Some(colors.accent_foreground()),
        icon_size: 16.0,
        hover_opacity: 0.88,
        ..Look::button(colors)
    }
}

/// `Button.dl-secondary`: the app's own button, 32 tall, 12 padding, 8 corners.
fn secondary_look(colors: &Colors) -> Look {
    Look {
        icon_size: 14.0,
        string_text: Some(colors.text()),
        ..Look::button(colors).height(32.0).padding(12.0).radius(8.0)
    }
}

/// `Button.dl-config`: 34 tall, 12 padding, 10 corners.
fn config_look(colors: &Colors) -> Look {
    Look::button(colors).height(34.0).padding(12.0).radius(10.0)
}

/// `Button.dl-quiet`: a link until pointed at.
fn quiet_look(colors: &Colors) -> Look {
    Look {
        height: 30.0,
        padding_x: 10.0,
        radius: 8.0,
        fill: Fill::None,
        hover_fill: Fill::Solid(colors.control_background_hover()),
        pressed_fill: Fill::Solid(colors.control_background_hover()),
        border: None,
        hover_border: None,
        text: colors.text_secondary(),
        hover_text: colors.text_secondary(),
        string_text: Some(colors.text_secondary()),
        ..Look::button(colors)
    }
}

/// `Button.dl-action`: a 32 px square, the glyph in Text.Secondary, Text on hover.
fn action_look(colors: &Colors, size: f32) -> Look {
    Look {
        icon_size: size,
        hover_text: colors.text(),
        ..Look::icon_button(colors, 32.0, 8.0, colors.control_background_hover())
    }
}

fn galley(
    ui: &Ui,
    text: &str,
    size: f32,
    weight: Weight,
    color: Color32,
    width: f32,
) -> Arc<egui::Galley> {
    layout(ui, wrapped(text, size, weight, color, width.max(20.0)))
}

fn one_line(
    ui: &Ui,
    text: &str,
    size: f32,
    weight: Weight,
    color: Color32,
    width: f32,
) -> Arc<egui::Galley> {
    layout(ui, trimmed(text, size, weight, color, width.max(20.0), 1))
}

/// The status dot: red with no session, green with one, accent while a window is open.
fn dot_color(colors: &Colors, signed_in: bool, busy: bool) -> Color32 {
    if busy {
        colors.accent_start()
    } else if signed_in {
        colors.continue_border()
    } else {
        colors.cancel_border()
    }
}

/// `Border.dl-segments` with its buttons; returns the index clicked.
fn segments(
    ui: &mut Ui,
    colors: &Colors,
    at: egui::Pos2,
    items: &[(char, &str, bool)],
) -> (Rect, Option<usize>) {
    let mut widths = Vec::new();
    for (glyph, label, selected) in items {
        let weight = if *selected {
            Weight::SemiBold
        } else {
            Weight::Normal
        };
        let icon_w = icon_galley(ui, *glyph, 14.0, colors.text()).size().x;
        let text_w = layout(ui, crate::widgets::job(label, 13.0, weight, colors.text()))
            .size()
            .x;
        widths.push(11.0 + icon_w + 7.0 + text_w + 11.0);
    }
    let total = widths.iter().sum::<f32>() + 2.0 * (items.len() as f32 - 1.0) + 4.0 + 2.0;
    let outer = Rect::from_min_size(at, vec2(total, 28.0 + 4.0 + 2.0));
    let painter = ui.painter().clone();
    painter.rect_filled(outer, 8.0, colors.control_background());
    painter.rect_stroke(
        outer,
        8.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let mut x = outer.left() + 3.0;
    let mut clicked = None;
    for (index, ((glyph, label, selected), width)) in items.iter().zip(widths).enumerate() {
        let rect = Rect::from_min_size(pos2(x, outer.top() + 3.0), vec2(width, 28.0));
        let response = ui.allocate_rect(rect, Sense::click());
        if *selected || response.hovered() {
            painter.rect_filled(rect, 6.0, colors.control_background_hover());
        }
        let weight = if *selected {
            Weight::SemiBold
        } else {
            Weight::Normal
        };
        let opacity = if *selected { 1.0 } else { 0.8 };
        let text_color = colors.text().gamma_multiply(opacity);
        let icon_color = if *selected {
            colors.accent_start()
        } else {
            text_color
        };
        let icon = icon_galley(ui, *glyph, 14.0, icon_color);
        let text = layout(ui, crate::widgets::job(label, 13.0, weight, text_color));
        let iw = icon.size().x;
        let (ih, th) = (icon.size().y, text.size().y);
        painter.galley(
            pos2(rect.left() + 11.0, rect.center().y - ih / 2.0),
            icon,
            icon_color,
        );
        painter.galley(
            pos2(rect.left() + 11.0 + iw + 7.0, rect.center().y - th / 2.0),
            text,
            text_color,
        );
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            clicked = Some(index);
        }
        x += width + 2.0;
    }
    (outer, clicked)
}

// ------------------------------------------------------------------ drawing

/// What a click asked for, carried out after drawing.
enum Ask {
    TogglePopup,
    ClosePopup,
    SignIn,
    SignOut,
    Start,
    InstallTool,
    InstallFFmpeg,
    Recheck,
    Kind(MediaKind),
    Quality(Quality),
    Audio(AudioFormat),
    Destination(Destination),
    ChooseFolder,
    ForgetFolder,
    ClearFinished,
    Pause(u64),
    Resume(u64),
    RowSignIn,
    UpdateTool(u64),
    Retry(u64),
    Reveal(u64),
    Cancel(u64),
    Remove(u64),
}

/// Draw the tab into `rect`.
pub fn ui(ui: &mut Ui, state: &mut AppState, rect: Rect) {
    let frame = ui.ctx().cumulative_frame_nr();
    let shown = state
        .downloads
        .last_frame
        .is_none_or(|last| frame > last + 1);
    state.downloads.last_frame = Some(frame);
    if shown {
        state.downloads_shown();
    }
    state.downloads_apply_live();

    let colors = state.colors.clone();
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, colors.background_main());
    let mut asks: Vec<Ask> = Vec::new();
    let left = rect.left() + 18.0;
    let width = (rect.width() - 36.0).max(200.0);
    let mut y = rect.top() + 18.0;

    // --- the header: the hint and the YouTube button in the corner
    let config_label = t(s::Download_YouTubeConfig);
    let config_w = 12.0
        + 9.0
        + 9.0
        + layout(
            ui,
            crate::widgets::job(config_label, 13.0, Weight::Normal, colors.text()),
        )
        .size()
        .x
        + 9.0
        + icon_galley(ui, icons::CHEVRON_DOWN, 14.0, colors.text())
            .size()
            .x
        + 12.0;
    let hint = galley(
        ui,
        t(s::Download_Hint),
        13.0,
        Weight::Normal,
        colors.text_secondary(),
        width - config_w - 16.0,
    );
    let head_h = hint.size().y.max(34.0);
    let hint_h = hint.size().y;
    painter.galley(
        pos2(left, y + (head_h - hint_h) / 2.0),
        hint,
        colors.text_secondary(),
    );
    let config_rect = Rect::from_min_size(
        pos2(left + width - config_w, y + (head_h - 34.0) / 2.0),
        vec2(config_w, 34.0),
    );
    let status_text = sign_in_status(state);
    let response = button_here(
        ui,
        &colors,
        &config_look(&colors),
        Content::label(""),
        true,
        config_rect,
    )
    .on_hover_text(&status_text);
    {
        let cx = config_rect.left() + 12.0;
        painter.circle_filled(
            pos2(cx + 4.5, config_rect.center().y),
            4.5,
            dot_color(
                &colors,
                state.downloads.signed_in,
                state.downloads.signing_in,
            ),
        );
        let label = layout(
            ui,
            crate::widgets::job(config_label, 13.0, Weight::Normal, colors.text_secondary()),
        );
        let lw = label.size().x;
        let lh = label.size().y;
        painter.galley(
            pos2(cx + 9.0 + 9.0, config_rect.center().y - lh / 2.0),
            label,
            colors.text_secondary(),
        );
        paint_icon(
            ui,
            &painter,
            pos2(cx + 18.0 + lw + 9.0 + 7.0, config_rect.center().y),
            icons::CHEVRON_DOWN,
            14.0,
            colors.text_secondary(),
        );
    }
    if response.clicked() {
        asks.push(Ask::TogglePopup);
    }
    y += head_h + 12.0;

    // --- the address and the Download button
    let download_label = t(s::Download_Start);
    let primary = primary_look(&colors);
    let dl_w = button_width(ui, &primary, Content::both(icons::DOWNLOAD, download_label));
    let field_rect = Rect::from_min_size(pos2(left, y), vec2(width - dl_w - 8.0, 38.0));
    let mut field_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(field_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let field_id = Id::new("download-url");
    let before = state.downloads.url_text.clone();
    let field = Field {
        hint: t(s::Download_UrlWatermark),
        height: 38.0,
        width: Some(field_rect.width()),
        padding_x: 10.0,
        id: Some(field_id),
        ..Field::new(&mut state.downloads.url_text)
    }
    .show(&mut field_ui, &colors);
    if state.downloads.url_text != before {
        // The complaint was about what was in the box.
        state.downloads.url_error.clear();
    }
    if std::mem::take(&mut state.downloads.focus_url) && state.mode != crate::app::Mode::Shot {
        ui.memory_mut(|m| m.request_focus(field_id));
    }
    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        asks.push(Ask::Start);
    }
    let dl_rect = Rect::from_min_size(pos2(left + width - dl_w, y), vec2(dl_w, 38.0));
    let ready = state.downloads.tool_ready();
    let mut look = primary.clone();
    if !ready {
        look.fill = Fill::Solid(colors.control_background_disabled());
        look.text = colors.text_secondary();
        look.string_text = Some(colors.text_secondary());
    }
    if button_here(
        ui,
        &colors,
        &look,
        Content::both(icons::DOWNLOAD, download_label),
        ready,
        dl_rect,
    )
    .clicked()
    {
        asks.push(Ask::Start);
    }
    y += 38.0;

    if !state.downloads.url_error.is_empty() {
        y += 12.0;
        let g = galley(
            ui,
            &state.downloads.url_error,
            13.0,
            Weight::Normal,
            colors.cancel_border(),
            width,
        );
        let h = g.size().y;
        painter.galley(pos2(left, y), g, colors.cancel_border());
        y += h;
    }

    // --- yt-dlp missing, ffmpeg missing
    if !ready {
        y += 12.0;
        y = notice(
            ui,
            &colors,
            pos2(left, y),
            width,
            t(s::Download_ToolMissingTitle),
            t(s::Download_ToolMissingBody),
            &[
                (
                    icons::EXTERNAL_LINK,
                    t(s::Download_ToolInstall),
                    Ask::InstallTool,
                ),
                (icons::REFRESH_CW, t(s::Download_ToolRecheck), Ask::Recheck),
            ],
            &mut asks,
        );
    }
    let format = state.current_format();
    if state.downloads.tools.ffmpeg.is_none() {
        y += 12.0;
        let body = if format.is_audio() {
            t(s::Download_FFmpegMissingAudio)
        } else {
            t(s::Download_FFmpegMissingVideo)
        };
        y = notice(
            ui,
            &colors,
            pos2(left, y),
            width,
            t(s::Download_FFmpegMissingTitle),
            body,
            &[
                (
                    icons::DOWNLOAD,
                    t(s::Download_FFmpegInstall),
                    Ask::InstallFFmpeg,
                ),
                (icons::REFRESH_CW, t(s::Download_ToolRecheck), Ask::Recheck),
            ],
            &mut asks,
        );
    }

    // --- the strip: format, destination, folder
    y += 12.0;
    y = settings_bar(ui, state, &colors, pos2(left, y), width, &mut asks);

    // --- the queue, or the empty card
    let has_items = !state.downloads.queue.items.is_empty();
    if has_items {
        y += 16.0;
        let label = one_line(
            ui,
            t(s::Download_QueueTitle),
            13.0,
            Weight::SemiBold,
            colors.text(),
            width,
        );
        let lh = label.size().y;
        painter.galley(pos2(left, y + (30.0 - lh) / 2.0), label, colors.text());
        if state.downloads.queue.has_finished() {
            let quiet = quiet_look(&colors);
            let clear = t(s::Download_ClearFinished);
            let w = button_width(ui, &quiet, Content::label(clear));
            let r = Rect::from_min_size(pos2(left + width - w, y), vec2(w, 30.0));
            if button_here(ui, &colors, &quiet, Content::label(clear), true, r).clicked() {
                asks.push(Ask::ClearFinished);
            }
        }
        y += 30.0 + 8.0;
        let area = Rect::from_min_max(pos2(rect.left(), y), rect.max);
        queue_rows(ui, state, &colors, area, &mut asks);
    } else {
        let area = Rect::from_min_max(pos2(rect.left(), y), rect.max);
        empty_card(ui, state, &colors, area);
    }

    if state.downloads.popup_open {
        youtube_panel(ui, state, &colors, config_rect, &mut asks);
    }
    if state
        .downloads
        .queue
        .items
        .iter()
        .any(DownloadItem::is_active)
    {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(200));
    }
    handle(state, asks);
}

fn sign_in_status(state: &AppState) -> String {
    if state.downloads.signing_in {
        t(s::Download_SignInBusy).to_string()
    } else if state.downloads.signed_in {
        t(s::Download_SignInStatusIn).to_string()
    } else {
        t(s::Download_SignInStatusOut).to_string()
    }
}

/// `Border.dl-notice` with the warning glyph, a title, a body and buttons. Returns the
/// bottom edge.
#[allow(clippy::too_many_arguments)]
fn notice(
    ui: &mut Ui,
    colors: &Colors,
    at: egui::Pos2,
    width: f32,
    title: &str,
    body: &str,
    buttons: &[(char, &str, Ask)],
    asks: &mut Vec<Ask>,
) -> f32 {
    let inner_w = width - 28.0 - 2.0;
    let text_w = inner_w - 18.0 - 12.0;
    let title_g = one_line(ui, title, 13.0, Weight::SemiBold, colors.text(), text_w);
    let body_g = galley(
        ui,
        body,
        13.0,
        Weight::Normal,
        colors.text_secondary(),
        text_w,
    );
    let height = 1.0 + 12.0 + title_g.size().y + 8.0 + body_g.size().y + 8.0 + 32.0 + 12.0 + 1.0;
    let rect = Rect::from_min_size(at, vec2(width, height));
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 10.0, colors.control_background());
    painter.rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let inner_left = rect.left() + 15.0;
    let top = rect.top() + 13.0;
    let glyph = icon_galley(ui, icons::TRIANGLE_ALERT, 18.0, colors.cancel_border());
    painter.galley(pos2(inner_left, top + 2.0), glyph, colors.cancel_border());
    let text_left = inner_left + 18.0 + 12.0;
    let th = title_g.size().y;
    painter.galley(pos2(text_left, top), title_g, colors.text());
    let bh = body_g.size().y;
    painter.galley(
        pos2(text_left, top + th + 8.0),
        body_g,
        colors.text_secondary(),
    );
    let mut x = text_left;
    let by = top + th + 8.0 + bh + 8.0;
    let look = secondary_look(colors);
    for (glyph, label, ask) in buttons {
        let w = button_width(ui, &look, Content::both(*glyph, label));
        let r = Rect::from_min_size(pos2(x, by), vec2(w, 32.0));
        if button_here(ui, colors, &look, Content::both(*glyph, label), true, r).clicked() {
            asks.push(match ask {
                Ask::InstallTool => Ask::InstallTool,
                Ask::InstallFFmpeg => Ask::InstallFFmpeg,
                _ => Ask::Recheck,
            });
        }
        x += w + 8.0;
    }
    rect.bottom()
}

/// `Border.dl-bar`: what to fetch, where it goes, and (when a copy stays) which folder.
fn settings_bar(
    ui: &mut Ui,
    state: &mut AppState,
    colors: &Colors,
    at: egui::Pos2,
    width: f32,
    asks: &mut Vec<Ask>,
) -> f32 {
    let destination = settings::destination(&state.config);
    let format = state.current_format();
    let show_folder = destination != Destination::Modlight;
    let row_h = 34.0;
    let height = 1.0
        + 12.0
        + row_h
        + 10.0
        + row_h
        + if show_folder { 10.0 + 32.0 } else { 0.0 }
        + 12.0
        + 1.0;
    let bar = Rect::from_min_size(at, vec2(width, height));
    let painter = ui.painter().clone();
    painter.rect_filled(bar, 10.0, colors.background_panel());
    painter.rect_stroke(
        bar,
        10.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let left = bar.left() + 15.0;
    let right = bar.right() - 15.0;
    let mut y = bar.top() + 13.0;

    let label =
        |ui: &Ui, text: &str| one_line(ui, text, 13.0, Weight::SemiBold, colors.text(), 300.0);

    // Format: Video / Audio, the quality or container picker, and what it means.
    let format_label = label(ui, t(s::Download_FormatLabel));
    let (fw, fh) = (format_label.size().x, format_label.size().y);
    painter.galley(
        pos2(left, y + (row_h - fh) / 2.0),
        format_label,
        colors.text(),
    );
    let video = format.kind == MediaKind::Video;
    let (seg, clicked) = segments(
        ui,
        colors,
        pos2(left + fw + 12.0, y),
        &[
            (icons::FILM, t(s::Download_FormatVideo), video),
            (icons::MUSIC, t(s::Download_FormatAudio), !video),
        ],
    );
    match clicked {
        Some(0) => asks.push(Ask::Kind(MediaKind::Video)),
        Some(1) => asks.push(Ask::Kind(MediaKind::Audio)),
        _ => {}
    }
    let quality_label = label(
        ui,
        if video {
            t(s::Download_QualityLabel)
        } else {
            t(s::Download_AudioFormatLabel)
        },
    );
    let (qw, qh) = (quality_label.size().x, quality_label.size().y);
    let qx = seg.right() + 12.0;
    painter.galley(
        pos2(qx, y + (row_h - qh) / 2.0),
        quality_label,
        colors.text(),
    );
    let combo_x = qx + qw + 12.0;
    let combo_w = 150.0;
    let mut combo_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_size(
                pos2(combo_x, y + 2.0),
                vec2(combo_w, 30.0),
            ))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let hint = format_hint(&format);
    if video {
        let options: Vec<(Quality, String)> =
            Quality::ALL.iter().map(|q| (*q, video_label(*q))).collect();
        if let Some(picked) = combo_rounded(
            &mut combo_ui,
            colors,
            "download-quality",
            combo_w,
            30.0,
            8.0,
            &format.quality,
            &options,
        ) {
            asks.push(Ask::Quality(picked));
        }
    } else {
        let options: Vec<(AudioFormat, String)> = AudioFormat::ALL
            .iter()
            .map(|a| (*a, audio_label(*a).to_string()))
            .collect();
        if let Some(picked) = combo_rounded(
            &mut combo_ui,
            colors,
            "download-audio",
            combo_w,
            30.0,
            8.0,
            &format.audio_format,
            &options,
        ) {
            asks.push(Ask::Audio(picked));
        }
    }
    ui.interact(
        Rect::from_min_size(pos2(combo_x, y + 2.0), vec2(combo_w, 30.0)),
        Id::new("download-quality-tip"),
        Sense::hover(),
    )
    .on_hover_text(hint);
    let note_x = combo_x + combo_w + 12.0;
    if right - note_x > 30.0 {
        let note_text = format_note(&format);
        let note = one_line(
            ui,
            &note_text,
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            right - note_x,
        );
        let nh = note.size().y;
        let note_rect = Rect::from_min_size(pos2(note_x, y + (row_h - nh) / 2.0), note.size());
        painter.galley(note_rect.min, note, colors.text_secondary());
        ui.interact(note_rect, Id::new("download-format-note"), Sense::hover())
            .on_hover_text(note_text);
    }
    y += row_h + 10.0;

    // Destination: this Mac, Modlight, or both.
    let dest_label = label(ui, t(s::Download_DestinationLabel));
    let (dw, dh) = (dest_label.size().x, dest_label.size().y);
    painter.galley(
        pos2(left, y + (row_h - dh) / 2.0),
        dest_label,
        colors.text(),
    );
    let (seg, clicked) = segments(
        ui,
        colors,
        pos2(left + dw + 12.0, y),
        &[
            (
                icons::HARD_DRIVE,
                t(s::Download_DestinationDisk),
                destination == Destination::Disk,
            ),
            (
                icons::CLOUD_UPLOAD,
                t(s::Download_DestinationModlight),
                destination == Destination::Modlight,
            ),
            (
                icons::COPY,
                t(s::Download_DestinationBoth),
                destination == Destination::Both,
            ),
        ],
    );
    if let Some(index) = clicked {
        asks.push(Ask::Destination(Destination::ALL[index]));
    }
    let note_x = seg.right() + 12.0;
    if right - note_x > 30.0 {
        let text = match destination {
            Destination::Modlight => t(s::Download_NoteModlight),
            Destination::Both => t(s::Download_NoteBoth),
            Destination::Disk => t(s::Download_NoteDisk),
        };
        let note = one_line(
            ui,
            text,
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            right - note_x,
        );
        let nh = note.size().y;
        painter.galley(
            pos2(note_x, y + (row_h - nh) / 2.0),
            note,
            colors.text_secondary(),
        );
    }
    y += row_h;

    // The folder: only when a copy is kept.
    if show_folder {
        y += 10.0;
        let folder_label = label(ui, t(s::Download_FolderLabel));
        let (lw, lh) = (folder_label.size().x, folder_label.size().y);
        painter.galley(
            pos2(left, y + (32.0 - lh) / 2.0),
            folder_label,
            colors.text(),
        );
        let fixed = settings::folder(&state.config);
        let look = secondary_look(colors);
        let choose = t(s::Download_FolderChoose);
        let forget = t(s::Download_FolderForget);
        let choose_w = button_width(ui, &look, Content::both(icons::FOLDER, choose));
        let forget_w = if fixed.is_empty() {
            0.0
        } else {
            button_width(ui, &look, Content::label(forget))
        };
        let mut bx = right - forget_w - if fixed.is_empty() { 0.0 } else { 8.0 } - choose_w;
        let text_x = left + lw + 8.0;
        let folder_text = if fixed.is_empty() {
            t(s::Download_FolderAsk).to_string()
        } else {
            fixed.clone()
        };
        let text = one_line(
            ui,
            &folder_text,
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            bx - 8.0 - text_x,
        );
        let th = text.size().y;
        painter.galley(
            pos2(text_x, y + (32.0 - th) / 2.0),
            text,
            colors.text_secondary(),
        );
        let r = Rect::from_min_size(pos2(bx, y), vec2(choose_w, 32.0));
        if button_here(
            ui,
            colors,
            &look,
            Content::both(icons::FOLDER, choose),
            true,
            r,
        )
        .clicked()
        {
            asks.push(Ask::ChooseFolder);
        }
        bx += choose_w + 8.0;
        if !fixed.is_empty() {
            let r = Rect::from_min_size(pos2(bx, y), vec2(forget_w, 32.0));
            if button_here(ui, colors, &look, Content::label(forget), true, r).clicked() {
                asks.push(Ask::ForgetFolder);
            }
        }
    }
    bar.bottom()
}

/// The queue's rows, newest first, in a scroller.
fn queue_rows(ui: &mut Ui, state: &mut AppState, colors: &Colors, area: Rect, asks: &mut Vec<Ask>) {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(area)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(area.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical()
        .id_salt("download-queue")
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let width = area.width() - 18.0 - 10.0;
            for item in &state.downloads.queue.items {
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    row(ui, colors, item, width, asks);
                });
                ui.add_space(8.0);
            }
            ui.add_space(10.0);
        });
}

fn row(ui: &mut Ui, colors: &Colors, item: &DownloadItem, width: f32, asks: &mut Vec<Ask>) {
    // The buttons on the right, in upstream's order.
    let buttons: Vec<(char, f32, &str, Ask)> = [
        (
            item.can_pause(),
            icons::CIRCLE_PAUSE,
            17.0,
            t(s::Download_Pause),
            Ask::Pause(item.id),
        ),
        (
            item.can_resume(),
            icons::CIRCLE_PLAY,
            17.0,
            t(s::Download_Resume),
            Ask::Resume(item.id),
        ),
        (
            item.can_sign_in(),
            icons::LOG_IN,
            16.0,
            t(s::Download_YouTubeConfig),
            Ask::RowSignIn,
        ),
        (
            item.can_update_tool(),
            icons::CIRCLE_ARROW_UP,
            17.0,
            t(s::Download_UpdateAndRetry),
            Ask::UpdateTool(item.id),
        ),
        (
            item.can_retry(),
            icons::ROTATE_CW,
            16.0,
            t(s::Download_Retry),
            Ask::Retry(item.id),
        ),
        (
            item.can_reveal(),
            icons::FOLDER_OPEN,
            16.0,
            t(s::Download_ShowInFolder),
            Ask::Reveal(item.id),
        ),
        (
            item.can_cancel(),
            icons::X,
            16.0,
            t(s::Download_Cancel),
            Ask::Cancel(item.id),
        ),
        (
            item.is_finished(),
            icons::TRASH_2,
            16.0,
            t(s::Download_Remove),
            Ask::Remove(item.id),
        ),
    ]
    .into_iter()
    .filter(|b| b.0)
    .map(|(_, glyph, size, tip, ask)| (glyph, size, tip, ask))
    .collect();
    let actions_w = buttons.len() as f32 * 32.0 + (buttons.len().max(1) as f32 - 1.0) * 2.0;
    let inner_w = width - 26.0;
    let center_w = (inner_w - 36.0 - 12.0 - 12.0 - actions_w).max(80.0);

    // Middle column: name + chip + site, note, progress, detail + numbers.
    let chip_g = one_line(
        ui,
        &item.format_text,
        11.0,
        Weight::SemiBold,
        colors.text_secondary(),
        200.0,
    );
    let source_g = one_line(
        ui,
        &item.link.source,
        12.0,
        Weight::Normal,
        colors.text_secondary(),
        200.0,
    );
    let chip_w = chip_g.size().x + 14.0;
    let side_w = chip_w + 8.0 + source_g.size().x;
    let name_g = one_line(
        ui,
        &item.title,
        15.0,
        Weight::Normal,
        colors.text(),
        center_w - side_w - 12.0,
    );
    let note_g = (!item.note.is_empty()).then(|| {
        one_line(
            ui,
            &item.note,
            12.0,
            Weight::Normal,
            colors.cancel_border(),
            center_w,
        )
    });
    let numbers: Vec<(&str, f32)> = if item.has_numbers() {
        vec![
            (item.size_text.as_str(), 0.0),
            (item.speed_text.as_str(), 74.0),
            (item.eta_text.as_str(), 62.0),
        ]
    } else {
        Vec::new()
    };
    let number_gs: Vec<(Arc<egui::Galley>, f32)> = numbers
        .iter()
        .map(|(text, min)| {
            (
                one_line(
                    ui,
                    text,
                    12.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    200.0,
                ),
                *min,
            )
        })
        .collect();
    let numbers_w: f32 = number_gs
        .iter()
        .map(|(g, min)| g.size().x.max(*min))
        .sum::<f32>()
        + 12.0 * (number_gs.len().max(1) as f32 - 1.0);
    let detail_color = if item.is_failed() {
        colors.cancel_border()
    } else {
        colors.text_secondary()
    };
    let detail_w = center_w
        - if number_gs.is_empty() {
            0.0
        } else {
            numbers_w + 12.0
        };
    let detail_g = one_line(
        ui,
        &item.detail,
        13.0,
        Weight::Normal,
        detail_color,
        detail_w,
    );
    let line1 = name_g.size().y.max(chip_g.size().y + 4.0);
    let mut center_h = line1;
    if let Some(g) = &note_g {
        center_h += 6.0 + g.size().y;
    }
    if item.show_progress() {
        center_h += 6.0 + 4.0;
    }
    let detail_line = detail_g.size().y.max(
        number_gs
            .iter()
            .map(|(g, _)| g.size().y)
            .fold(0.0, f32::max),
    );
    center_h += 6.0 + detail_line;
    let height = 2.0 + 24.0 + center_h.max(36.0).max(32.0);

    let (card, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(card, 10.0, colors.control_background());
    let hovered = response.hovered() || ui.rect_contains_pointer(card);
    painter.rect_stroke(
        card,
        10.0,
        Stroke::new(
            1.0,
            if hovered {
                colors.accent_start()
            } else {
                colors.border()
            },
        ),
        StrokeKind::Inside,
    );
    let inner = card.shrink(13.0);
    // Badge: the state at a glance.
    let badge = Rect::from_min_size(inner.min, vec2(36.0, 36.0));
    painter.rect_filled(badge, 8.0, colors.control_background_hover());
    let glyph_color = if item.is_done() {
        colors.continue_border()
    } else if item.is_failed() {
        colors.cancel_border()
    } else {
        colors.accent_start()
    };
    paint_icon(ui, &painter, badge.center(), item.icon(), 18.0, glyph_color);

    let cx = badge.right() + 12.0;
    let mut y = inner.center().y - center_h / 2.0;
    let name_h = name_g.size().y;
    let name_rect = Rect::from_min_size(pos2(cx, y + (line1 - name_h) / 2.0), name_g.size());
    painter.galley(name_rect.min, name_g, colors.text());
    ui.interact(
        name_rect,
        Id::new(("download-title", item.id)),
        Sense::hover(),
    )
    .on_hover_text(&item.link.url);
    // Chip and site, at the right of the first line.
    let side_x = cx + center_w - side_w;
    let chip = Rect::from_min_size(
        pos2(side_x, y + (line1 - chip_g.size().y - 4.0) / 2.0),
        vec2(chip_w, chip_g.size().y + 4.0),
    );
    painter.rect_filled(chip, 6.0, colors.control_background_hover());
    let chip_text_h = chip_g.size().y;
    painter.galley(
        pos2(chip.left() + 7.0, chip.center().y - chip_text_h / 2.0),
        chip_g,
        colors.text_secondary(),
    );
    let sh = source_g.size().y;
    painter.galley(
        pos2(chip.right() + 8.0, y + (line1 - sh) / 2.0),
        source_g,
        colors.text_secondary(),
    );
    y += line1;
    if let Some(g) = note_g {
        y += 6.0;
        let r = Rect::from_min_size(pos2(cx, y), g.size());
        let h = g.size().y;
        painter.galley(r.min, g, colors.cancel_border());
        ui.interact(r, Id::new(("download-note", item.id)), Sense::hover())
            .on_hover_text(&item.note);
        y += h;
    }
    if item.show_progress() {
        y += 6.0;
        let bar = Rect::from_min_size(pos2(cx, y), vec2(center_w, 4.0));
        let fraction = (!item.is_indeterminate()).then_some((item.percent / 100.0) as f32);
        paint_progress(ui, colors, bar, fraction);
        y += 4.0;
    }
    y += 6.0;
    let dh = detail_g.size().y;
    let detail_rect = Rect::from_min_size(pos2(cx, y + (detail_line - dh) / 2.0), detail_g.size());
    painter.galley(detail_rect.min, detail_g, detail_color);
    ui.interact(
        detail_rect,
        Id::new(("download-detail", item.id)),
        Sense::hover(),
    )
    .on_hover_text(&item.detail);
    let mut nx = cx + center_w;
    for (g, min) in number_gs.into_iter().rev() {
        let w = g.size().x.max(min);
        let gh = g.size().y;
        // Right-aligned inside its fixed width.
        painter.galley(
            pos2(nx - g.size().x, y + (detail_line - gh) / 2.0),
            g,
            colors.text_secondary(),
        );
        nx -= w + 12.0;
    }

    // The buttons.
    let mut bx = inner.right() - actions_w;
    for (glyph, size, tip, ask) in buttons {
        let r = Rect::from_min_size(pos2(bx, inner.center().y - 16.0), vec2(32.0, 32.0));
        let look = action_look(colors, size);
        if button_here(ui, colors, &look, Content::icon(glyph), true, r)
            .on_hover_text(tip)
            .clicked()
        {
            asks.push(ask);
        }
        bx += 32.0 + 2.0;
    }
}

/// `Border.dl-card`: nothing queued, and why the space is empty.
fn empty_card(ui: &mut Ui, state: &AppState, colors: &Colors, area: Rect) {
    let width = 560.0_f32.min(area.width() - 36.0).max(200.0);
    let text_w = width - 44.0 - 2.0 - 42.0 - 13.0;
    let title = galley(
        ui,
        t(s::Download_EmptyTitle),
        17.0,
        Weight::SemiBold,
        colors.text(),
        text_w,
    );
    let body = galley(
        ui,
        t(s::Download_EmptyBody),
        14.0,
        Weight::Normal,
        colors.text_secondary(),
        text_w,
    );
    let path_note = state.downloads.tools.yt_dlp.as_ref().map(|path| {
        galley(
            ui,
            &format(
                t(s::Download_ToolPath),
                &[Arg::from(path.display().to_string())],
            ),
            13.0,
            Weight::Normal,
            colors.text_secondary().gamma_multiply(0.75),
            width - 46.0,
        )
    });
    let text_h = title.size().y + 6.0 + body.size().y;
    let head_h = text_h.max(42.0);
    let height =
        23.0 + head_h + path_note.as_ref().map(|g| 14.0 + g.size().y).unwrap_or(0.0) + 23.0;
    // Centred in what is left, but never over the strip above it.
    let top = (area.center().y - height / 2.0).max(area.top() + 8.0);
    let rect = Rect::from_min_size(
        pos2(area.center().x - width / 2.0, top),
        vec2(width, height),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, 10.0, colors.background_panel());
    painter.rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let badge = Rect::from_min_size(
        pos2(
            rect.left() + 23.0,
            rect.top() + 23.0 + (head_h - 42.0) / 2.0,
        ),
        vec2(42.0, 42.0),
    );
    painter.rect_filled(badge, 8.0, colors.control_background_hover());
    paint_icon(
        ui,
        painter,
        badge.center(),
        icons::DOWNLOAD,
        20.0,
        colors.accent_start(),
    );
    let left = badge.right() + 13.0;
    let top = rect.top() + 23.0 + (head_h - text_h) / 2.0;
    let th = title.size().y;
    painter.galley(pos2(left, top), title, colors.text());
    painter.galley(pos2(left, top + th + 6.0), body, colors.text_secondary());
    if let Some(g) = path_note {
        painter.galley(
            pos2(rect.left() + 23.0, rect.top() + 23.0 + head_h + 14.0),
            g,
            colors.text_secondary().gamma_multiply(0.75),
        );
    }
}

/// The panel under the YouTube button: the state, the way in and the way out.
fn youtube_panel(
    ui: &mut Ui,
    state: &mut AppState,
    colors: &Colors,
    anchor: Rect,
    asks: &mut Vec<Ask>,
) {
    let width = 380.0;
    let at = pos2(anchor.right() - width, anchor.bottom() + 6.0);
    let signed_in = state.downloads.signed_in;
    let busy = state.downloads.signing_in;
    let status = sign_in_status(state);
    let message = state.downloads.sign_in_message.clone();
    let area = egui::Area::new(Id::new("download-youtube-panel"))
        .order(Order::Foreground)
        .fixed_pos(at)
        .show(ui.ctx(), |ui| {
            // `Border.popup-border`: control fill, border, 10 corners, a soft shadow.
            egui::Frame::new()
                .fill(colors.control_background())
                .stroke(Stroke::new(1.0, colors.border()))
                .corner_radius(10)
                .inner_margin(egui::Margin::same(16))
                .shadow(egui::Shadow {
                    offset: [0, 4],
                    blur: 16,
                    spread: 0,
                    color: Color32::from_black_alpha(0x40),
                })
                .show(ui, |ui| {
                    let inner_w = width - 32.0 - 2.0;
                    ui.set_width(inner_w);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    // Badge, title, dot and state.
                    let title = one_line(
                        ui,
                        t(s::Download_SignInPanelTitle),
                        13.0,
                        Weight::SemiBold,
                        colors.text(),
                        inner_w - 48.0,
                    );
                    let state_g = one_line(
                        ui,
                        &status,
                        15.0,
                        Weight::Normal,
                        colors.text(),
                        inner_w - 48.0 - 19.0,
                    );
                    let head_h = (title.size().y + 5.0 + state_g.size().y).max(36.0);
                    let (head, _) = ui.allocate_exact_size(vec2(inner_w, head_h), Sense::hover());
                    let painter = ui.painter().clone();
                    let badge = Rect::from_min_size(
                        pos2(head.left(), head.center().y - 18.0),
                        vec2(36.0, 36.0),
                    );
                    painter.rect_filled(badge, 8.0, colors.control_background_hover());
                    paint_icon(
                        ui,
                        &painter,
                        badge.center(),
                        icons::CIRCLE_USER_ROUND,
                        18.0,
                        colors.accent_start(),
                    );
                    let tx = badge.right() + 12.0;
                    let ty = head.center().y - (title.size().y + 5.0 + state_g.size().y) / 2.0;
                    let title_h = title.size().y;
                    painter.galley(pos2(tx, ty), title, colors.text());
                    let sy = ty + title_h + 5.0;
                    let sh = state_g.size().y;
                    painter.circle_filled(
                        pos2(tx + 5.5, sy + sh / 2.0),
                        5.5,
                        dot_color(colors, signed_in, busy),
                    );
                    painter.galley(pos2(tx + 11.0 + 8.0, sy), state_g, colors.text());
                    ui.add_space(12.0);
                    let explain = if signed_in {
                        t(s::Download_SignInStateIn)
                    } else {
                        t(s::Download_SignInStateOut)
                    };
                    crate::widgets::text_wrapped(
                        ui,
                        explain,
                        13.0,
                        Weight::Normal,
                        colors.text_secondary(),
                    );
                    ui.add_space(12.0);
                    let (sep, _) = ui.allocate_exact_size(vec2(inner_w, 1.0), Sense::hover());
                    ui.painter().rect_filled(sep, 0.0, colors.separator());
                    ui.add_space(12.0);
                    if !signed_in && !busy {
                        let look = primary_look(colors);
                        let (r, _) = ui.allocate_exact_size(vec2(inner_w, 38.0), Sense::hover());
                        if button_here(
                            ui,
                            colors,
                            &look,
                            Content::both(icons::LOG_IN, t(s::Download_SignIn)),
                            true,
                            r,
                        )
                        .clicked()
                        {
                            asks.push(Ask::SignIn);
                        }
                        ui.add_space(12.0);
                    }
                    if signed_in && !busy {
                        let look = secondary_look(colors);
                        let half = (inner_w - 8.0) / 2.0;
                        let (r, _) = ui.allocate_exact_size(vec2(inner_w, 32.0), Sense::hover());
                        let again = Rect::from_min_size(r.min, vec2(half, 32.0));
                        let out = Rect::from_min_size(
                            pos2(r.left() + half + 8.0, r.top()),
                            vec2(half, 32.0),
                        );
                        if button_here(
                            ui,
                            colors,
                            &look,
                            Content::both(icons::ROTATE_CW, t(s::Download_SignInAgain)),
                            true,
                            again,
                        )
                        .clicked()
                        {
                            asks.push(Ask::SignIn);
                        }
                        if button_here(
                            ui,
                            colors,
                            &look,
                            Content::both(icons::LOG_OUT, t(s::Download_SignOut)),
                            true,
                            out,
                        )
                        .clicked()
                        {
                            asks.push(Ask::SignOut);
                        }
                        ui.add_space(12.0);
                    }
                    if !message.is_empty() {
                        crate::widgets::text_wrapped(
                            ui,
                            &message,
                            13.0,
                            Weight::Normal,
                            colors.text_secondary(),
                        );
                        ui.add_space(12.0);
                    }
                    crate::widgets::text_wrapped(
                        ui,
                        t(s::Download_SignInNote),
                        13.0,
                        Weight::Normal,
                        colors.text_secondary().gamma_multiply(0.75),
                    );
                });
        });
    // Light dismiss: a click anywhere else closes the panel.
    let panel = area.response.rect;
    let clicked_outside = ui.input(|i| {
        i.pointer.any_pressed()
            && i.pointer
                .interact_pos()
                .is_some_and(|p| !panel.contains(p) && !anchor.contains(p))
    });
    if clicked_outside || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        asks.push(Ask::ClosePopup);
    }
    let _ = Align2::CENTER_CENTER;
}

fn handle(state: &mut AppState, asks: Vec<Ask>) {
    for ask in asks {
        match ask {
            Ask::TogglePopup => {
                if state.downloads.popup_open {
                    state.downloads.popup_open = false;
                } else {
                    state.downloads_refresh_sign_in();
                    state.downloads.popup_open = true;
                }
            }
            Ask::ClosePopup => state.downloads.popup_open = false,
            Ask::SignIn => state.downloads_sign_in(),
            Ask::SignOut => state.downloads_sign_out(),
            Ask::RowSignIn => {
                state.downloads_refresh_sign_in();
                state.downloads.popup_open = true;
            }
            Ask::Start => state.downloads_start(),
            Ask::InstallTool => {
                if crate::components_window::require(
                    state,
                    components::YT_DLP,
                    s::Components_FeatureDownload,
                    AfterClose::DownloadRefresh,
                ) {
                    state.downloads_refresh_tools();
                }
            }
            Ask::InstallFFmpeg => {
                if crate::components_window::require(
                    state,
                    components::FFMPEG,
                    s::Components_FeatureDownload,
                    AfterClose::DownloadRefresh,
                ) {
                    state.downloads_refresh_tools();
                }
            }
            Ask::Recheck => {
                state.downloads_refresh_tools();
                state.downloads_pump();
            }
            Ask::Kind(kind) => {
                settings::set_media_kind(&mut state.config, kind);
                state.save_config();
            }
            Ask::Quality(quality) => {
                settings::set_video_quality(&mut state.config, quality);
                state.save_config();
            }
            Ask::Audio(audio) => {
                settings::set_audio_format(&mut state.config, audio);
                state.save_config();
            }
            Ask::Destination(destination) => {
                settings::set_destination(&mut state.config, destination);
                state.save_config();
            }
            Ask::ChooseFolder => {
                if let Some(folder) = state.pick_download_folder() {
                    settings::set_folder(&mut state.config, &folder.display().to_string());
                    state.save_config();
                }
            }
            Ask::ForgetFolder => {
                settings::set_folder(&mut state.config, "");
                state.save_config();
            }
            Ask::ClearFinished => state.downloads.queue.clear_finished(),
            Ask::Pause(id) => {
                if let Some(item) = state.downloads.queue.get_mut(id) {
                    item.pause();
                }
                state.downloads_pump();
            }
            Ask::Resume(id) => {
                if let Some(item) = state.downloads.queue.get_mut(id) {
                    item.resume();
                }
                state.downloads_pump();
            }
            Ask::UpdateTool(id) => state.download_update_tool(id),
            Ask::Retry(id) => {
                if let Some(item) = state.downloads.queue.get_mut(id) {
                    item.retry();
                }
                state.downloads_pump();
            }
            Ask::Reveal(id) => {
                if let Some(path) = state
                    .downloads
                    .queue
                    .get(id)
                    .and_then(|i| i.file_path.clone())
                {
                    state.effects.push(crate::state::Effect::Reveal(path));
                }
            }
            Ask::Cancel(id) => {
                if let Some(item) = state.downloads.queue.get_mut(id) {
                    item.cancel();
                }
                state.downloads_pump();
            }
            Ask::Remove(id) => {
                state.downloads.queue.remove(id);
                state.downloads_pump();
            }
        }
    }
}

/// QA: a state of the tab without running anything.
pub fn qa_fill(tab: &mut DownloadTab, tools: ToolPaths, signed_in: bool) {
    tab.tools = tools;
    tab.tools_read = true;
    tab.signed_in = signed_in;
    // Drawing notices the tab being shown; a fixture counts as already shown.
    tab.last_frame = Some(u64::MAX - 1);
}

/// QA: the queue, to fill with rows in every state.
pub fn qa_queue(tab: &mut DownloadTab) -> &mut DownloadQueue {
    &mut tab.queue
}

/// QA: open the YouTube panel.
pub fn qa_open_panel(tab: &mut DownloadTab, message: &str) {
    tab.popup_open = true;
    tab.sign_in_message = message.to_string();
}

/// QA: a row as a running download looks after some progress.
pub fn qa_progress(item: &mut DownloadItem, progress: Progress, state: State) {
    item.start();
    item.apply(&progress, false);
    if state == State::Working {
        item.apply(
            &Progress {
                is_working: true,
                ..progress
            },
            false,
        );
    }
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
