//! The external components list (`ComponentsView`, `ComponentsViewModel`,
//! `ComponentRowViewModel`), drawn in two hosts like upstream's one control: the
//! Settings -> Components page and the window a feature opens when the file it needs is
//! missing (`ComponentsWindow`, `ComponentsWindowIntegration.Require`).
//!
//! One row state per component, shared by both hosts: what is installed is read from
//! the disk when drawn, versions come from asking the binaries, progress from the
//! install running in the background.

use std::sync::{Arc, Mutex};

use egui::{Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::components::install::{
    self, InstallOutcome, InstallProgress, InstallResult, Stage,
};
use modlight_core::components::updates::{self, UpdateOutcome, UpdateResult};
use modlight_core::components::{self, Component, settings as component_settings, tool_release};
use modlight_core::format::local_size;
use modlight_core::l10n::{Arg, format, s, t};
use tokio_util::sync::CancellationToken;

use crate::state::AppState;
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Look, button_at, button_width, checkbox, layout, paint_progress, wrapped,
};

/// `ComponentsWindow`: 620 x 640, at least 520 x 420.
pub const WIDTH: f32 = 620.0;
pub const HEIGHT: f32 = 640.0;
pub const MIN_WIDTH: f32 = 520.0;
pub const MIN_HEIGHT: f32 = 420.0;

/// What a closed components window lets the feature that opened it do next.
#[derive(Clone, Debug, PartialEq)]
pub enum AfterClose {
    /// The Download tab was starting a download: carry on (folder, queue).
    DownloadContinue,
    /// The Download tab asked for a tool: look for the tools again.
    DownloadRefresh,
    /// A recording found ffmpeg missing (`TaskHelpers.CheckFFmpeg`): record when it is
    /// there now, so the hotkey need not be pressed again.
    Recording(
        modlight_core::recording::ScreenRecordOutput,
        crate::capture_flow::RecordStartMethod,
        Box<modlight_core::tasks::settings::TaskSettings>,
    ),
}

/// The window, when it is open.
#[derive(Clone, Debug, Default)]
pub struct ComponentsWindow {
    /// The component whose absence sent the person here.
    pub required: Option<String>,
    /// "The Download tab needs FFmpeg", or empty.
    pub headline: String,
    /// What to do once it closes.
    pub then: Vec<AfterClose>,
}

/// One row (`ComponentRowViewModel`).
#[derive(Clone, Debug)]
pub struct Row {
    pub id: &'static str,
    pub stage: Stage,
    pub progress_value: f32,
    pub progress_text: String,
    pub message: String,
    pub failed: bool,
    pub installed_version: String,
    pub version_asked: bool,
    pub updating: bool,
    /// Started from the window: closing the window stops it.
    pub from_window: bool,
    cancel: Option<CancellationToken>,
    live: Arc<Mutex<Option<InstallProgress>>>,
}

impl Row {
    fn new(id: &'static str) -> Row {
        Row {
            id,
            stage: Stage::Idle,
            progress_value: 0.0,
            progress_text: String::new(),
            message: String::new(),
            failed: false,
            installed_version: String::new(),
            version_asked: false,
            updating: false,
            from_window: false,
            cancel: None,
            live: Arc::new(Mutex::new(None)),
        }
    }

    pub fn component(&self) -> &'static Component {
        components::find(self.id).expect("a catalogued component")
    }

    pub fn is_busy(&self) -> bool {
        !matches!(self.stage, Stage::Idle | Stage::Done)
    }

    fn indeterminate(&self) -> bool {
        matches!(
            self.stage,
            Stage::Queued | Stage::Verifying | Stage::Installing
        )
    }

    /// `SetStage`: the stage and the status line that names it.
    fn set_stage(&mut self, stage: Stage) {
        self.stage = stage;
        let key = match stage {
            Stage::Queued => Some(s::Components_Queued),
            Stage::Downloading => Some(s::Components_Downloading),
            Stage::Verifying => Some(s::Components_Verifying),
            Stage::Installing => Some(s::Components_Installing),
            _ => None,
        };
        if let Some(key) = key {
            self.message = t(key).to_string();
        }
    }

    /// `Report`.
    fn report(&mut self, value: InstallProgress) {
        self.set_stage(value.stage);
        if value.stage == Stage::Downloading && value.total > 0 {
            self.progress_value = (value.received as f32 / value.total as f32 * 100.0).min(100.0);
            self.progress_text = format(
                t(s::Components_Progress),
                &[
                    Arg::from(local_size(value.received, false, 2)),
                    Arg::from(local_size(value.total, false, 2)),
                    Arg::from(local_size(value.bytes_per_second as i64, false, 2)),
                ],
            );
        } else {
            self.progress_text.clear();
        }
    }

    /// Take what the background install reported since the last frame.
    fn take_live(&mut self) {
        let latest = self.live.lock().ok().and_then(|mut l| l.take());
        if let Some(value) = latest
            && self.cancel.is_some()
        {
            self.report(value);
        }
    }
}

/// Both hosts' state.
#[derive(Clone, Debug)]
pub struct Components {
    pub rows: Vec<Row>,
    pub window: Option<ComponentsWindow>,
    pub checking: bool,
    /// The version reading and the scheduled check ran (`StartAsync`).
    pub started: bool,
    /// Rows "Install what is missing" still has to start, in order.
    missing_chain: Vec<&'static str>,
    /// The component a Remove question is about.
    pub removing: Option<&'static str>,
    /// QA: what counts as installed (and ours), instead of the disk.
    pub qa_installed: Option<Vec<&'static str>>,
    /// QA: the personal folder to name on screen, instead of the real one.
    pub display_profile: Option<std::path::PathBuf>,
}

impl Default for Components {
    fn default() -> Self {
        Components {
            rows: components::catalog()
                .iter()
                .map(|c| Row::new(c.id))
                .collect(),
            window: None,
            checking: false,
            started: false,
            missing_chain: Vec::new(),
            removing: None,
            qa_installed: None,
            display_profile: None,
        }
    }
}

impl Components {
    pub fn row_mut(&mut self, id: &str) -> Option<&mut Row> {
        self.rows.iter_mut().find(|r| r.id.eq_ignore_ascii_case(id))
    }

    pub fn any_busy(&self) -> bool {
        self.rows.iter().any(Row::is_busy)
    }
}

/// What a click asked for, carried out after drawing.
#[derive(Clone, Debug, PartialEq)]
enum Ask {
    Wanted(&'static str, bool),
    Install(&'static str),
    Update(&'static str),
    Cancel(&'static str),
    Remove(&'static str),
    InstallMissing,
    AutoUpdate(bool),
    CheckNow,
    Close,
}

// ------------------------------------------------------------------ what is here

fn installed(state: &AppState, component: &Component) -> bool {
    match &state.components.qa_installed {
        Some(list) => list.contains(&component.id),
        None => components::is_installed(&state.env.profile, component),
    }
}

fn managed(state: &AppState, component: &Component) -> bool {
    match &state.components.qa_installed {
        Some(list) => list.contains(&component.id),
        None => components::is_managed(&state.env.profile, component),
    }
}

fn shown_profile(state: &AppState) -> std::path::PathBuf {
    state
        .components
        .display_profile
        .clone()
        .unwrap_or_else(|| state.env.profile.clone())
}

// ------------------------------------------------------------------ texts per row

fn details(component: &Component) -> String {
    format(
        t(s::Components_Details),
        &[
            Arg::from(local_size(component.download_size, false, 2)),
            Arg::from(component.source),
        ],
    )
}

fn version_text(row: &Row, installed: bool) -> String {
    if !installed {
        return String::new();
    }
    if !row.installed_version.is_empty() {
        return format(
            t(s::Components_Version),
            &[Arg::from(row.installed_version.as_str())],
        );
    }
    if row.version_asked {
        t(s::Components_VersionUnknown).to_string()
    } else {
        String::new()
    }
}

fn update_note(row: &Row, component: &Component, installed: bool) -> String {
    if !component.tracks_updates() || !installed {
        return String::new();
    }
    let latest = updates::latest_known(component);
    if tool_release::is_newer(&latest, &row.installed_version) {
        format(t(s::Components_UpdateAvailable), &[Arg::from(latest)])
    } else {
        String::new()
    }
}

fn status(row: &Row, installed: bool, wanted: bool) -> String {
    if !row.message.is_empty() {
        row.message.clone()
    } else if installed {
        t(s::Components_Installed).to_string()
    } else if wanted {
        t(s::Components_NotInstalled).to_string()
    } else {
        t(s::Components_Skipped).to_string()
    }
}

fn describe_install(result: &InstallResult) -> String {
    let with = |key, message: &str| format(t(key), &[Arg::from(message)]);
    match result.outcome {
        InstallOutcome::Installed | InstallOutcome::AlreadyInstalled => String::new(),
        InstallOutcome::Cancelled => t(s::Components_Cancelled).to_string(),
        InstallOutcome::Offline => t(s::Components_ErrorOffline).to_string(),
        InstallOutcome::ChecksumFailed => t(s::Components_ErrorChecksum).to_string(),
        InstallOutcome::NoSpace => with(s::Components_ErrorNoSpace, &result.message),
        InstallOutcome::DownloadFailed => with(s::Components_ErrorDownload, &result.message),
        InstallOutcome::Refused => with(s::Components_ErrorRefused, &result.message),
        InstallOutcome::InstallFailed => with(s::Components_ErrorFailed, &result.message),
    }
}

fn describe_update(result: &UpdateResult) -> String {
    let with = |key, message: &str| format(t(key), &[Arg::from(message)]);
    match result.outcome {
        UpdateOutcome::Updated => with(s::Components_Updated, &result.version),
        UpdateOutcome::AlreadyCurrent => t(s::Components_UpToDate).to_string(),
        UpdateOutcome::CheckFailed => t(s::Components_UpdateCheckFailed).to_string(),
        UpdateOutcome::NotOurs => with(s::Components_UpdateNotOurs, &result.detail),
        UpdateOutcome::Cancelled => t(s::Components_Cancelled).to_string(),
        UpdateOutcome::VersionUnknown => t(s::Components_VersionUnknown).to_string(),
        UpdateOutcome::NotTracked => String::new(),
        UpdateOutcome::InstallFailed => with(s::Components_UpdateFailed, &result.detail),
    }
}

/// `LastCheckText`: local time, date and hour (`"g"`).
fn last_check_text(state: &AppState) -> String {
    match component_settings::last_update_check(&state.config) {
        None => t(s::Components_NeverChecked).to_string(),
        Some(at) => format(
            t(s::Components_LastCheck),
            &[Arg::from(modlight_core::culture::local_short(&at))],
        ),
    }
}

// ------------------------------------------------------------------ drawing

/// `Button.component-action`: 32 tall, 14 px padding, at least 96 wide; its label is
/// the button's own string content, in Text.
fn action_look(colors: &Colors) -> Look {
    let mut look = Look::button(colors).height(32.0).padding(14.0);
    look.min_width = 96.0;
    look.string_text = Some(colors.text());
    look
}

fn text_block(
    ui: &Ui,
    text: &str,
    size: f32,
    weight: Weight,
    color: egui::Color32,
    width: f32,
) -> Arc<egui::Galley> {
    layout(ui, wrapped(text, size, weight, color, width.max(20.0)))
}

/// The whole list into `ui`, `width` wide (the settings page and the window's scroller).
fn view(
    ui: &mut Ui,
    state: &mut AppState,
    width: f32,
    headline: &str,
    required: Option<&str>,
) -> Vec<Ask> {
    let colors = state.colors.clone();
    let mut asks = Vec::new();
    ui.spacing_mut().item_spacing.y = 0.0;
    for row in state.components.rows.iter_mut() {
        row.take_live();
    }

    // Headline, only when a feature sent the person here.
    if !headline.is_empty() {
        let g = text_block(ui, headline, 16.0, Weight::SemiBold, colors.text(), width);
        let (r, _) = ui.allocate_exact_size(vec2(width, g.size().y), Sense::hover());
        ui.painter().galley(r.min, g, colors.text());
        ui.add_space(10.0);
    }
    let intro = text_block(
        ui,
        t(s::Components_Intro),
        12.0,
        Weight::Normal,
        colors.text_secondary(),
        width,
    );
    let (r, _) = ui.allocate_exact_size(vec2(width, intro.size().y), Sense::hover());
    ui.painter().galley(r.min, intro, colors.text_secondary());
    ui.add_space(10.0);

    // The one that is in the way goes first.
    let mut order: Vec<usize> = (0..state.components.rows.len()).collect();
    if let Some(required) = required {
        order.sort_by_key(|&i| !state.components.rows[i].id.eq_ignore_ascii_case(required));
    }
    let look = action_look(&colors);
    for index in order {
        let row = state.components.rows[index].clone();
        let component = row.component();
        let installed = installed(state, component);
        let managed = managed(state, component);
        let mut wanted = component_settings::is_wanted(&state.config, component);
        let busy = row.is_busy();
        let can_install = !busy && !installed;
        let can_update =
            !busy && !row.updating && component.tracks_updates() && installed && managed;
        let can_remove = !busy && installed && managed;
        let is_required = required.is_some_and(|r| r.eq_ignore_ascii_case(component.id));

        // Buttons column width.
        let labels: Vec<(&str, Ask)> = [
            (can_install, t(s::Components_Install), Ask::Install(row.id)),
            (can_update, t(s::Components_Update), Ask::Update(row.id)),
            (busy, t(s::Components_Cancel), Ask::Cancel(row.id)),
            (can_remove, t(s::Components_Remove), Ask::Remove(row.id)),
        ]
        .into_iter()
        .filter(|(shown, _, _)| *shown)
        .map(|(_, label, ask)| (label, ask))
        .collect();
        let buttons_w = labels
            .iter()
            .map(|(label, _)| button_width(ui, &look, Content::label(label)))
            .fold(0.0_f32, f32::max);
        let buttons_h = if labels.is_empty() {
            0.0
        } else {
            labels.len() as f32 * 32.0 + (labels.len() - 1) as f32 * 6.0
        };
        let inner_w = width - 24.0 - 2.0;
        let text_w = (inner_w
            - 20.0
            - 12.0
            - if labels.is_empty() {
                0.0
            } else {
                buttons_w + 12.0
            })
        .max(80.0);

        let name = text_block(
            ui,
            component.display_name(),
            14.0,
            Weight::SemiBold,
            colors.text(),
            text_w,
        );
        let purpose = text_block(
            ui,
            component.purpose_text(),
            13.0,
            Weight::Normal,
            colors.text(),
            text_w,
        );
        let detail = text_block(
            ui,
            &details(component),
            12.0,
            Weight::Normal,
            colors.text_secondary(),
            text_w,
        );
        let version = version_text(&row, installed);
        let version_g = (!version.is_empty()).then(|| {
            text_block(
                ui,
                &version,
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                text_w,
            )
        });
        let note = update_note(&row, component, installed);
        let note_g = (!note.is_empty()).then(|| {
            text_block(
                ui,
                &note,
                12.0,
                Weight::SemiBold,
                colors.accent_start(),
                text_w,
            )
        });
        let status_color = if row.failed {
            colors.cancel_border()
        } else {
            colors.text_secondary()
        };
        let status_g = text_block(
            ui,
            &status(&row, installed, wanted),
            12.0,
            Weight::Normal,
            status_color,
            text_w,
        );
        let progress_g = (busy && !row.progress_text.is_empty()).then(|| {
            text_block(
                ui,
                &row.progress_text,
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                text_w,
            )
        });

        let mut stack: Vec<(Arc<egui::Galley>, egui::Color32)> = vec![
            (name, colors.text()),
            (purpose, colors.text()),
            (detail, colors.text_secondary()),
        ];
        if let Some(g) = version_g {
            stack.push((g, colors.text_secondary()));
        }
        if let Some(g) = note_g {
            stack.push((g, colors.accent_start()));
        }
        stack.push((status_g, status_color));
        let mut text_h: f32 =
            stack.iter().map(|(g, _)| g.size().y).sum::<f32>() + 2.0 * (stack.len() as f32 - 1.0);
        if busy {
            text_h += 2.0 + 4.0 + 6.0;
            if let Some(g) = &progress_g {
                text_h += 2.0 + g.size().y;
            }
        }
        let card_h = 12.0 + text_h.max(buttons_h).max(20.0) + 12.0 + 2.0;
        let (card, _) = ui.allocate_exact_size(vec2(width, card_h), Sense::hover());
        let painter = ui.painter().clone();
        painter.rect_filled(card, 8.0, colors.background_panel());
        painter.rect_stroke(
            card,
            8.0,
            Stroke::new(
                1.0,
                if is_required {
                    colors.accent_start()
                } else {
                    colors.border()
                },
            ),
            StrokeKind::Inside,
        );
        let inner = card.shrink(13.0);
        // The tick: "should Modlight fetch this". Unticking deletes nothing.
        let mut tick_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    pos2(inner.left(), inner.top() - 2.0 - 6.0),
                    vec2(20.0, 32.0),
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Min)),
        );
        let response = checkbox(&mut tick_ui, &colors, &mut wanted, None, true)
            .on_hover_text(t(s::Components_Wanted));
        if response.changed() {
            asks.push(Ask::Wanted(row.id, wanted));
        }
        let mut y = inner.top();
        let left = inner.left() + 20.0 + 12.0;
        for (galley, color) in stack {
            let h = galley.size().y;
            painter.galley(pos2(left, y), galley, color);
            y += h + 2.0;
        }
        if busy {
            y += 4.0;
            let bar = Rect::from_min_size(pos2(left, y), vec2(text_w, 6.0));
            let fraction = (!row.indeterminate()).then_some(row.progress_value / 100.0);
            paint_progress(ui, &colors, bar, fraction);
            y += 6.0 + 2.0;
            if let Some(g) = progress_g {
                painter.galley(pos2(left, y), g, colors.text_secondary());
            }
        }
        let mut by = inner.top();
        let bx = inner.right() - buttons_w;
        for (label, ask) in labels {
            let rect = Rect::from_min_size(pos2(bx, by), vec2(buttons_w, 32.0));
            if button_here(ui, &colors, &look, Content::label(label), true, rect).clicked() {
                asks.push(ask);
            }
            by += 32.0 + 6.0;
        }
        ui.add_space(8.0);
    }

    // The folder line, and "Install what is missing".
    let folder = shown_profile(state).join(components::TOOLS_FOLDER);
    let anything_missing = state.components.rows.iter().any(|row| {
        let component = row.component();
        !row.is_busy()
            && !installed(state, component)
            && component_settings::is_wanted(&state.config, component)
    });
    let any_busy = state.components.any_busy();
    let label = t(s::Components_InstallMissing);
    let missing_w = if anything_missing {
        button_width(ui, &look, Content::label(label)) + 8.0
    } else {
        0.0
    };
    let folder_g = text_block(
        ui,
        &format(
            t(s::Components_Folder),
            &[Arg::from(folder.display().to_string())],
        ),
        12.0,
        Weight::Normal,
        colors.text_secondary(),
        width - missing_w,
    );
    let line_h = folder_g
        .size()
        .y
        .max(if anything_missing { 32.0 } else { 0.0 });
    let (line, _) = ui.allocate_exact_size(vec2(width, line_h), Sense::hover());
    let gh = folder_g.size().y;
    ui.painter().galley(
        pos2(line.left(), line.center().y - gh / 2.0),
        folder_g,
        colors.text_secondary(),
    );
    if anything_missing {
        let w = missing_w - 8.0;
        let rect = Rect::from_min_size(
            pos2(line.right() - w, line.center().y - 16.0),
            vec2(w, 32.0),
        );
        if button_here(ui, &colors, &look, Content::label(label), !any_busy, rect).clicked() {
            asks.push(Ask::InstallMissing);
        }
    }
    ui.add_space(10.0);

    // Automatic updates: only when something follows a publisher (yt-dlp).
    if components::catalog().iter().any(Component::tracks_updates) {
        let note = text_block(
            ui,
            t(s::Components_AutoUpdateNote),
            12.0,
            Weight::Normal,
            colors.text_secondary(),
            width - 26.0,
        );
        let check_label = t(s::Components_CheckNow);
        let check_w = button_width(ui, &look, Content::label(check_label));
        let last = text_block(
            ui,
            &last_check_text(state),
            12.0,
            Weight::Normal,
            colors.text_secondary(),
            width - 26.0 - check_w - 8.0,
        );
        let card_h = 13.0 + 32.0 + 8.0 + note.size().y + 8.0 + last.size().y.max(32.0) + 13.0;
        let (card, _) = ui.allocate_exact_size(vec2(width, card_h), Sense::hover());
        let painter = ui.painter().clone();
        painter.rect_filled(card, 8.0, colors.background_panel());
        painter.rect_stroke(
            card,
            8.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        let inner = card.shrink(13.0);
        let mut auto = component_settings::auto_update(&state.config);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(inner.min, vec2(inner.width(), 32.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        if checkbox(
            &mut child,
            &colors,
            &mut auto,
            Some(t(s::Components_AutoUpdate)),
            true,
        )
        .changed()
        {
            asks.push(Ask::AutoUpdate(auto));
        }
        let mut y = inner.top() + 32.0 + 8.0;
        let nh = note.size().y;
        painter.galley(pos2(inner.left(), y), note, colors.text_secondary());
        y += nh + 8.0;
        let row_h = last.size().y.max(32.0);
        let lh = last.size().y;
        painter.galley(
            pos2(inner.left(), y + (row_h - lh) / 2.0),
            last,
            colors.text_secondary(),
        );
        let can_check = !state.components.checking && !any_busy;
        let rect = Rect::from_min_size(
            pos2(inner.right() - check_w, y + (row_h - 32.0) / 2.0),
            vec2(check_w, 32.0),
        );
        if button_here(
            ui,
            &colors,
            &look,
            Content::label(check_label),
            can_check,
            rect,
        )
        .clicked()
        {
            asks.push(Ask::CheckNow);
        }
    }
    asks
}

/// Settings -> Components: the list under the page's own header.
pub fn settings_page(ui: &mut Ui, state: &mut AppState, _colors: &Colors) {
    start(state);
    let width = ui.available_width();
    let asks = view(ui, state, width, "", None);
    handle(state, asks, false);
}

/// The window's content: the list in a scroller (20,18,20,10), the footer with Close.
/// Returns true when the window asked to close.
pub fn window_ui(ui: &mut Ui, state: &mut AppState) -> bool {
    let colors = state.colors.clone();
    start(state);
    let full = ui.max_rect();
    ui.painter()
        .rect_filled(full, 0.0, colors.background_main());
    let footer_h = 12.0 + 32.0 + 12.0;
    let body = Rect::from_min_max(full.min, pos2(full.right(), full.bottom() - footer_h));
    let (headline, required) = state
        .components
        .window
        .as_ref()
        .map(|w| (w.headline.clone(), w.required.clone()))
        .unwrap_or_default();
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(body);
    let mut asks = Vec::new();
    egui::ScrollArea::vertical()
        .id_salt("components-window")
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.vertical(|ui| {
                    let width = (body.width() - 40.0).max(200.0);
                    ui.set_width(width);
                    asks = view(ui, state, width, &headline, required.as_deref());
                });
            });
            ui.add_space(10.0);
        });
    let bar = crate::dialogs::footer_at(
        ui,
        &colors,
        Rect::from_min_max(pos2(full.left(), full.bottom() - footer_h), full.max),
    );
    let mut look = Look::button(&colors).height(32.0).padding(16.0);
    look.min_width = 96.0;
    look.string_text = Some(colors.text());
    let label = t(s::Components_Close);
    let w = button_width(ui, &look, Content::label(label));
    let rect = Rect::from_min_size(
        pos2(bar.right() - 20.0 - w, bar.top() + 12.0),
        vec2(w, 32.0),
    );
    if button_here(ui, &colors, &look, Content::label(label), true, rect).clicked() {
        asks.push(Ask::Close);
    }
    if state.components.rows.iter().any(Row::is_busy) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
    }
    let close = asks.contains(&Ask::Close);
    handle(state, asks, true);
    close
}

// ------------------------------------------------------------------ what clicks do

/// `ComponentsWindowIntegration.Require`: when the component is here, `None`; otherwise
/// the window opens (or comes forward) with the headline "<feature> needs <component>"
/// and `then` runs when it closes.
pub fn require(
    state: &mut AppState,
    id: &str,
    feature: modlight_core::l10n::Key,
    then: AfterClose,
) -> bool {
    let Some(component) = components::find(id) else {
        return false;
    };
    if components::is_installed(&state.env.profile, component) {
        return true;
    }
    let headline = format(
        t(s::Components_RequiredHeadline),
        &[Arg::from(t(feature)), Arg::from(component.display_name())],
    );
    match state.components.window.as_mut() {
        // One window: a second request means "let me see it".
        Some(window) => window.then.push(then),
        None => {
            state.components.window = Some(ComponentsWindow {
                required: Some(component.id.to_string()),
                headline,
                then: vec![then],
            });
        }
    }
    false
}

/// `ShowAll`: the full list in the window, with an optional headline.
pub fn show_all(state: &mut AppState, headline: Option<&str>) {
    if state.components.window.is_none() {
        state.components.window = Some(ComponentsWindow {
            required: None,
            headline: headline.unwrap_or("").to_string(),
            then: Vec::new(),
        });
    }
}

/// The window closed: what it started stops (the part file stays), and the feature
/// that opened it carries on.
pub fn window_closed(state: &mut AppState) {
    let Some(window) = state.components.window.take() else {
        return;
    };
    for row in state.components.rows.iter_mut() {
        if row.from_window
            && let Some(cancel) = &row.cancel
        {
            cancel.cancel();
        }
    }
    state.components.missing_chain.clear();
    let satisfied = window
        .required
        .as_deref()
        .is_none_or(|id| components::installed_path(&state.env.profile, id).is_some());
    for then in window.then {
        match then {
            AfterClose::Recording(output, method, settings) => {
                if satisfied {
                    state.start_screen_recording(output, method, *settings);
                }
            }
            then => crate::download_tab::after_components(state, then, satisfied),
        }
    }
}

/// `StartAsync`: read what is installed and, if it is time, ask what is published.
fn start(state: &mut AppState) {
    if state.components.started || state.mode == crate::app::Mode::Shot {
        return;
    }
    state.components.started = true;
    let profile = state.env.profile.clone();
    let network = state.mode == crate::app::Mode::Normal;
    let plan = scheduled_plan(state);
    state.rt.spawn(async move {
        let mut versions = Vec::new();
        for component in components::catalog() {
            if components::is_installed(&profile, component) {
                versions.push((
                    component.id,
                    updates::installed_version(&profile, component).await,
                ));
            }
        }
        let report = if network {
            Some(updates::run_scheduled(&profile, &plan).await)
        } else {
            None
        };
        move |state: &mut AppState, _ctx: &egui::Context| {
            for (id, version) in versions {
                if let Some(row) = state.components.row_mut(id) {
                    row.installed_version = version;
                    row.version_asked = true;
                }
            }
            if let Some(report) = report {
                scheduled_done(state, &report);
            }
        }
    });
}

fn scheduled_plan(state: &AppState) -> updates::ScheduledPlan {
    updates::ScheduledPlan {
        auto_update: component_settings::auto_update(&state.config),
        last_check: component_settings::last_update_check(&state.config),
        wanted: components::catalog()
            .iter()
            .filter(|c| component_settings::is_wanted(&state.config, c))
            .map(|c| c.id.to_string())
            .collect(),
    }
}

/// A scheduled check finished: stamp it if one completed, re-read the versions.
fn scheduled_done(state: &mut AppState, report: &updates::ScheduledReport) {
    if report.checked_any {
        component_settings::set_last_update_check(&mut state.config, chrono::Utc::now());
        state.save_config();
    }
    if report
        .results
        .iter()
        .any(|(_, outcome, _)| *outcome == UpdateOutcome::Updated)
    {
        state.downloads.refresh_tools_soon = true;
    }
    for (id, outcome, version) in &report.results {
        if *outcome == UpdateOutcome::Updated
            && let Some(row) = state.components.row_mut(id)
        {
            row.installed_version = version.clone();
            row.version_asked = true;
        }
    }
}

/// Startup (`ModlightComponents.OnStartup`): carry the settings forward, tidy what a
/// crash left, and schedule the daily yt-dlp check 45 seconds after launch. There is no
/// installer on a Mac, so the first-run window (`IsFirstRunAfterSetup`) never opens.
pub fn on_startup(state: &mut AppState) {
    if components::migrate(&mut state.config) {
        state.save_config();
    }
    let profile = state.env.profile.clone();
    install::sweep_staging(&profile);
    for component in components::catalog() {
        install::sweep_retired(&profile, component);
    }
    if state.mode != crate::app::Mode::Normal {
        return;
    }
    let plan = scheduled_plan(state);
    state.rt.spawn(async move {
        tokio::time::sleep(updates::STARTUP_DELAY).await;
        let report = updates::run_scheduled(&profile, &plan).await;
        move |state: &mut AppState, _ctx: &egui::Context| scheduled_done(state, &report)
    });
}

fn handle(state: &mut AppState, asks: Vec<Ask>, from_window: bool) {
    for ask in asks {
        match ask {
            Ask::Wanted(id, wanted) => {
                if let Some(component) = components::find(id)
                    && component_settings::set_wanted(&mut state.config, component, wanted)
                {
                    state.save_config();
                }
            }
            Ask::Install(id) => {
                // Pressing Install is changing one's mind about the tick.
                if let Some(component) = components::find(id)
                    && component_settings::set_wanted(&mut state.config, component, true)
                {
                    state.save_config();
                }
                start_install(state, id, from_window);
            }
            Ask::Update(id) => start_update(state, id, from_window),
            Ask::Cancel(id) => {
                if let Some(row) = state.components.row_mut(id)
                    && let Some(cancel) = &row.cancel
                {
                    cancel.cancel();
                }
            }
            Ask::Remove(id) => ask_remove(state, id),
            Ask::InstallMissing => {
                let profile = state.env.profile.clone();
                let missing: Vec<&'static str> = components::catalog()
                    .iter()
                    .filter(|c| {
                        component_settings::is_wanted(&state.config, c)
                            && !components::is_installed(&profile, c)
                    })
                    .map(|c| c.id)
                    .collect();
                state.components.missing_chain = missing;
                next_missing(state, from_window);
            }
            Ask::AutoUpdate(value) => {
                component_settings::set_auto_update(&mut state.config, value);
                state.save_config();
            }
            Ask::CheckNow => check_now(state),
            Ask::Close => {}
        }
    }
}

fn next_missing(state: &mut AppState, from_window: bool) {
    while !state.components.missing_chain.is_empty() {
        let id = state.components.missing_chain.remove(0);
        let profile = state.env.profile.clone();
        let busy = state.components.row_mut(id).is_some_and(|r| r.is_busy());
        if busy || components::installed_path(&profile, id).is_some() {
            continue;
        }
        start_install(state, id, from_window);
        return;
    }
}

/// `ComponentRowViewModel.InstallAsync`.
pub fn start_install(state: &mut AppState, id: &str, from_window: bool) {
    let profile = state.env.profile.clone();
    let Some(component) = components::find(id) else {
        return;
    };
    let waker = state.rt.waker();
    let Some(row) = state.components.row_mut(id) else {
        return;
    };
    if row.is_busy() || components::is_installed(&profile, component) {
        return;
    }
    let cancel = CancellationToken::new();
    row.cancel = Some(cancel.clone());
    row.from_window = from_window;
    row.failed = false;
    row.message.clear();
    row.progress_value = 0.0;
    row.progress_text.clear();
    row.set_stage(Stage::Queued);
    let live = row.live.clone();
    let sink: install::ProgressSink = Arc::new(move |value: InstallProgress| {
        if let Ok(mut slot) = live.lock() {
            *slot = Some(value);
        }
        waker();
    });
    state.rt.spawn(async move {
        let result = install::install(&profile, component, Some(sink), &cancel).await;
        move |state: &mut AppState, _ctx: &egui::Context| install_done(state, component.id, result)
    });
}

fn install_done(state: &mut AppState, id: &'static str, result: InstallResult) {
    let from_window = if let Some(row) = state.components.row_mut(id) {
        row.cancel = None;
        row.message = describe_install(&result);
        row.failed = !result.success() && result.outcome != InstallOutcome::Cancelled;
        row.progress_text.clear();
        row.stage = Stage::Idle;
        if let Ok(mut live) = row.live.lock() {
            *live = None;
        }
        row.from_window
    } else {
        false
    };
    if result.success() {
        // A fresh binary: ask it which build it is.
        load_version(state, id);
        state.downloads.refresh_tools_soon = true;
    }
    if result.outcome == InstallOutcome::Cancelled {
        state.components.missing_chain.clear();
    } else {
        next_missing(state, from_window);
    }
}

fn load_version(state: &mut AppState, id: &'static str) {
    let profile = state.env.profile.clone();
    let Some(component) = components::find(id) else {
        return;
    };
    state.rt.spawn(async move {
        let version = updates::installed_version(&profile, component).await;
        move |state: &mut AppState, _ctx: &egui::Context| {
            if let Some(row) = state.components.row_mut(id) {
                row.installed_version = version;
                row.version_asked = true;
            }
        }
    });
}

/// `ComponentRowViewModel.UpdateAsync`: the newest published build, asked for by a
/// person (`only_when_newer: false`).
pub fn start_update(state: &mut AppState, id: &str, from_window: bool) {
    let profile = state.env.profile.clone();
    let Some(component) = components::find(id) else {
        return;
    };
    if !updates::can_update(&profile, component) {
        return;
    }
    let waker = state.rt.waker();
    let Some(row) = state.components.row_mut(id) else {
        return;
    };
    if row.is_busy() || row.updating {
        return;
    }
    let cancel = CancellationToken::new();
    row.cancel = Some(cancel.clone());
    row.from_window = from_window;
    row.updating = true;
    row.failed = false;
    row.message = t(s::Components_Checking).to_string();
    row.progress_value = 0.0;
    row.progress_text.clear();
    let live = row.live.clone();
    let sink: install::ProgressSink = Arc::new(move |value: InstallProgress| {
        if let Ok(mut slot) = live.lock() {
            *slot = Some(value);
        }
        waker();
    });
    state.rt.spawn(async move {
        let result = updates::update(&profile, component, Some(sink), false, &cancel).await;
        move |state: &mut AppState, _ctx: &egui::Context| {
            if let Some(row) = state.components.row_mut(component.id) {
                row.updating = false;
                row.cancel = None;
                row.stage = Stage::Idle;
                row.progress_text.clear();
                row.message = describe_update(&result);
                row.failed = matches!(
                    result.outcome,
                    UpdateOutcome::InstallFailed | UpdateOutcome::CheckFailed
                );
                if result.installed() {
                    row.installed_version = result.version.clone();
                    row.version_asked = true;
                }
            }
            if result.installed() {
                state.downloads.refresh_tools_soon = true;
            }
        }
    });
}

/// "Check now": ask whatever the schedule says, then re-read every version.
fn check_now(state: &mut AppState) {
    if state.components.checking {
        return;
    }
    state.components.checking = true;
    let profile = state.env.profile.clone();
    state.rt.spawn(async move {
        let mut versions = Vec::new();
        for component in components::catalog() {
            if component.tracks_updates() {
                let _ = updates::check(component).await;
            }
            if components::is_installed(&profile, component) {
                versions.push((
                    component.id,
                    updates::installed_version(&profile, component).await,
                ));
            }
        }
        move |state: &mut AppState, _ctx: &egui::Context| {
            state.components.checking = false;
            for (id, version) in versions {
                if let Some(row) = state.components.row_mut(id) {
                    row.installed_version = version;
                    row.version_asked = true;
                }
            }
        }
    });
}

/// Remove asks first: the largest of these is a gigabyte.
fn ask_remove(state: &mut AppState, id: &'static str) {
    let Some(component) = components::find(id) else {
        return;
    };
    let question = format(
        t(s::Components_RemoveConfirm),
        &[
            Arg::from(component.display_name()),
            Arg::from(
                components::folder_for(&shown_profile(state), component)
                    .display()
                    .to_string(),
            ),
        ],
    );
    let mut confirm = crate::dialogs::confirm::Confirm::new(
        "",
        &question,
        crate::dialogs::confirm::ConfirmAction::RemoveComponent(id.to_string()),
    );
    confirm.title = modlight_core::title();
    state.windows.confirm = Some(confirm);
}

/// The Remove question was answered Yes.
pub fn remove_confirmed(state: &mut AppState, id: &str) {
    let profile = state.env.profile.clone();
    let Some(component) = components::find(id) else {
        return;
    };
    if !components::is_managed(&profile, component) {
        return;
    }
    let removed = install::remove(&profile, component);
    if let Some(row) = state.components.row_mut(id) {
        row.message = if removed {
            String::new()
        } else {
            t(s::Components_RemoveFailed).to_string()
        };
        row.failed = !removed;
        row.installed_version.clear();
        row.version_asked = false;
    }
    state.downloads.refresh_tools_soon = true;
}

/// QA: a row as an install looks part-way through.
pub fn qa_progress(row: &mut Row, progress: InstallProgress) {
    row.cancel = Some(CancellationToken::new());
    row.report(progress);
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
