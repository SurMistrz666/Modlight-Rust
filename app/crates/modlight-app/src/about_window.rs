//! `AboutWindow` and `DebugLogWindow`.
//!
//! About: the logo and name on the left, the product title, the build (its commit and
//! when it was built, `modlight_core::build_info`), the update state from the server the
//! app is signed in to (with "Check now"; `update_window`) and the copyright; the credits
//! on the right. Upstream's Links section points at ShareX's own website, stores and
//! social accounts (getsharex.com and friends, which PORT.md leaves out); the port keeps
//! the team and the project page as the credit and links the Debug log there, which
//! upstream's menus no longer reach. The logo's clicker game is not ported.

use egui::{Align, Layout, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use modlight_core::icons;
use modlight_core::l10n::{Arg, format, port, s, t};
use modlight_core::update::{self, UpdateOutcome};

use crate::state::AppState;
use crate::theme::{Colors, Weight};
use crate::widgets::{Content, Look, button, layout, paint_icon, trimmed, wrapped};

pub const ABOUT_WIDTH: f32 = 920.0;
pub const ABOUT_HEIGHT: f32 = 540.0;
pub const ABOUT_MIN: (f32, f32) = (650.0, 500.0);
pub const LOG_WIDTH: f32 = 850.0;
pub const LOG_HEIGHT: f32 = 600.0;
pub const LOG_MIN: (f32, f32) = (600.0, 420.0);

/// What the About window says about updates.
#[derive(Clone, Debug, PartialEq)]
pub enum AboutUpdate {
    /// Nothing is said (a QA window, a headless render).
    Hidden,
    /// Signed out: updates come from the server one is signed in to.
    SignedOut,
    Checking,
    Failed,
    UpToDate,
    Available(Box<update::ReleaseManifest>),
}

pub struct AboutWindow {
    pub update: AboutUpdate,
    /// "Build: 1a2b3c4d, 2026-10-27 14:03 UTC".
    pub build_line: String,
    /// The logo, and whether it is the Graphite one.
    logo: Option<(bool, egui::TextureHandle)>,
}

/// What About says about updates now: the last check of `state.updates` (`update_window`).
pub fn status_of(state: &AppState) -> AboutUpdate {
    if state.mode != crate::app::Mode::Normal {
        return AboutUpdate::Hidden;
    }
    if state.credential().is_none() {
        return AboutUpdate::SignedOut;
    }
    if state.updates.checking {
        return AboutUpdate::Checking;
    }
    match &state.updates.last_outcome {
        Some(UpdateOutcome::UpdateAvailable(manifest)) => {
            AboutUpdate::Available(Box::new(manifest.clone()))
        }
        Some(UpdateOutcome::UpToDate) => AboutUpdate::UpToDate,
        Some(UpdateOutcome::CheckFailed(_)) => AboutUpdate::Failed,
        None => AboutUpdate::Checking,
    }
}

/// The Debug log window's state.
#[derive(Default)]
pub struct DebugLogWindow {
    /// The folder shown as the startup path (QA shows a fixed one).
    pub startup_path: String,
    pub text: String,
    pub seen: u64,
    pub confirm_upload: bool,
    pub scroll_to_end: bool,
}

impl AppState {
    /// `AboutWindowIntegration.Show`: the window, and a check of the server's Downloads
    /// (as upstream checks when About opens).
    pub fn show_about(&mut self) {
        if self.windows.about.is_some() {
            return;
        }
        let build = crate::update_window::this_build(self);
        let build_line = format(
            t(port::BuildLine),
            &[Arg::from(modlight_core::build_info::build_text(
                &build.build,
                build.built,
            ))],
        );
        if self.mode == crate::app::Mode::Normal {
            crate::update_window::check_quietly(self);
        }
        self.windows.about = Some(AboutWindow {
            update: status_of(self),
            build_line,
            logo: None,
        });
    }

    /// What a click in About (the window, or the Information page under the Graphite
    /// skin) asked for.
    pub fn about_ask(&mut self, ask: AboutAsk) {
        match ask {
            AboutAsk::Open(url) => self.open_url(&url),
            AboutAsk::Update(manifest) => {
                crate::update_window::offer(self, UpdateOutcome::UpdateAvailable(*manifest))
            }
            AboutAsk::CheckNow => crate::update_window::check_quietly(self),
            AboutAsk::DebugLog => self.show_debug_log(),
        }
    }

    /// `DebugLogWindowIntegration.Show`.
    pub fn show_debug_log(&mut self) {
        if self.windows.debug_log.is_none() {
            self.windows.debug_log = Some(DebugLogWindow {
                startup_path: startup_path(),
                text: modlight_core::debug_log::text(),
                seen: modlight_core::debug_log::count(),
                confirm_upload: false,
                scroll_to_end: true,
            });
        }
    }
}

/// `AppDomain.CurrentDomain.BaseDirectory`: the folder of the running binary.
pub fn startup_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.display().to_string()))
        .unwrap_or_default()
}

/// The app's logo as a texture `side` pixels square: the Graphite one or upstream's.
pub(crate) fn logo_texture(ctx: &egui::Context, graphite: bool, side: u32) -> egui::TextureHandle {
    let image = crate::app::window_logo(graphite)
        .map(|icon| {
            image::imageops::resize(&icon, side, side, image::imageops::FilterType::Lanczos3)
        })
        .unwrap_or_else(|| image::RgbaImage::new(1, 1));
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    );
    ctx.load_texture("about-logo", color, egui::TextureOptions::LINEAR)
}

/// One part of the credits: its title and its rows (name, what the link shows, what a
/// click asks for).
pub(crate) type Credits = (&'static str, Vec<(String, &'static str, AboutAsk)>);

/// The credits About shows: the team, and the project page with the Debug log.
pub(crate) fn credits() -> [Credits; 2] {
    [
        (
            t(s::AboutWindow_Team),
            vec![
                (
                    "Jaex".to_string(),
                    "https://github.com/Jaex",
                    AboutAsk::Open("https://github.com/Jaex".into()),
                ),
                (
                    "McoreD".to_string(),
                    "https://github.com/McoreD",
                    AboutAsk::Open("https://github.com/McoreD".into()),
                ),
            ],
        ),
        (
            t(s::AboutWindow_Links),
            vec![
                (
                    t(s::AboutWindow_ProjectPage).to_string(),
                    "https://github.com/ShareX/ShareX",
                    AboutAsk::Open("https://github.com/ShareX/ShareX".into()),
                ),
                (
                    t(s::DebugLogWindow_Title).replace("Modlight - ", ""),
                    "Modlight.log",
                    AboutAsk::DebugLog,
                ),
            ],
        ),
    ]
}

/// A text link: the accent, underlined under the pointer. Whether it was clicked.
pub(crate) fn link(ui: &mut Ui, colors: &Colors, text: &str) -> bool {
    let galley = layout(
        ui,
        crate::widgets::job(text, 13.0, Weight::Normal, colors.accent_start()),
    );
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::click());
    let color = if response.hovered() {
        colors.accent_end()
    } else {
        colors.accent_start()
    };
    if response.hovered() {
        ui.painter()
            .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, color));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().galley(rect.min, galley, color);
    response.clicked()
}

/// What a click in the About window asks for.
pub enum AboutAsk {
    Open(String),
    Update(Box<update::ReleaseManifest>),
    /// "Check now".
    CheckNow,
    DebugLog,
}

pub fn about_ui(ui: &mut Ui, window: &mut AboutWindow, colors: &Colors) -> Vec<AboutAsk> {
    let mut asks = Vec::new();
    let full = ui.max_rect();
    ui.painter()
        .rect_filled(full, 0.0, colors.background_main());
    let panel = Rect::from_min_size(full.min, vec2(300.0, full.height()));
    let painter = ui.painter().clone();
    painter.rect_filled(panel, 0.0, colors.background_panel());
    let inner = panel.shrink(24.0);

    // The logo (160 square, 50 down inside a 240 by 330 box) and the name under it.
    if window
        .logo
        .as_ref()
        .is_some_and(|(graphite, _)| *graphite != colors.graphite)
    {
        window.logo = None;
    }
    let (_, logo) = window.logo.get_or_insert_with(|| {
        (
            colors.graphite,
            logo_texture(ui.ctx(), colors.graphite, 320),
        )
    });
    let box_rect = Rect::from_min_size(
        pos2(inner.center().x - 120.0, inner.top()),
        vec2(240.0, 330.0),
    );
    let logo_rect = Rect::from_min_size(
        pos2(box_rect.center().x - 80.0, box_rect.top() + 50.0),
        vec2(160.0, 160.0),
    );
    painter.image(
        logo.id(),
        logo_rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
    let brand = layout(
        ui,
        crate::widgets::job("Modlight", 50.0, Weight::SemiBold, colors.text()),
    );
    painter.galley(
        pos2(
            box_rect.center().x - brand.size().x / 2.0,
            box_rect.top() + 225.0,
        ),
        brand,
        colors.text(),
    );
    let mut y = box_rect.bottom() + 14.0;
    let product = layout(ui, {
        let mut job = wrapped(
            &modlight_core::title(),
            21.0,
            Weight::SemiBold,
            colors.text(),
            inner.width(),
        );
        job.halign = Align::Center;
        job
    });
    painter.galley(pos2(inner.center().x, y), product.clone(), colors.text());
    y += product.size().y + 4.0;
    // The build: its commit and when it was made.
    let build = layout(ui, {
        let mut job = wrapped(
            &window.build_line,
            12.0,
            Weight::Normal,
            colors.text_secondary(),
            inner.width(),
        );
        job.halign = Align::Center;
        job
    });
    painter.galley(
        pos2(inner.center().x, y),
        build.clone(),
        colors.text_secondary(),
    );
    y += build.size().y + 12.0;
    // The update state, centred.
    let status = match &window.update {
        AboutUpdate::Hidden => None,
        AboutUpdate::SignedOut => Some(t(port::UpdateSignedOut)),
        AboutUpdate::Checking => Some(t(s::AboutWindow_CheckingForUpdates)),
        AboutUpdate::Failed => Some(t(s::AboutWindow_UpdateCheckFailed)),
        AboutUpdate::UpToDate => Some(t(s::AboutWindow_UpToDate)),
        AboutUpdate::Available(_) => None,
    };
    if window.update == AboutUpdate::Checking {
        let bar = Rect::from_center_size(pos2(inner.center().x, y + 1.5), vec2(180.0, 3.0));
        crate::widgets::paint_progress(ui, colors, bar, None);
        y += 3.0 + 6.0;
    }
    if let Some(status) = status {
        let galley = layout(ui, {
            let mut job = wrapped(status, 13.0, Weight::Normal, colors.text(), inner.width());
            job.halign = Align::Center;
            job
        });
        let height = galley.size().y;
        painter.galley(pos2(inner.center().x, y), galley, colors.text());
        y += height + 4.0;
    }
    // "Check now" under a finished check.
    if matches!(window.update, AboutUpdate::UpToDate | AboutUpdate::Failed) {
        let text = t(port::CheckNow);
        let width = layout(
            ui,
            crate::widgets::job(text, 13.0, Weight::Normal, colors.text()),
        )
        .size()
        .x;
        let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_size(
            pos2(inner.center().x - width / 2.0, y),
            vec2(width + 2.0, 20.0),
        )));
        if link(&mut child, colors, text) {
            asks.push(AboutAsk::CheckNow);
        }
    }
    if let AboutUpdate::Available(manifest) = &window.update {
        let text = t(s::AboutWindow_NewVersionAvailable);
        let width = layout(
            ui,
            crate::widgets::job(text, 13.0, Weight::Normal, colors.text()),
        )
        .size()
        .x;
        let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_size(
            pos2(inner.center().x - width / 2.0, y),
            vec2(width + 2.0, 20.0),
        )));
        if link(&mut child, colors, text) {
            asks.push(AboutAsk::Update(manifest.clone()));
        }
    }
    let copyright = layout(ui, {
        let mut job = wrapped(
            t(s::AboutWindow_Copyright),
            12.0,
            Weight::Normal,
            colors.text_secondary(),
            inner.width(),
        );
        job.halign = Align::Center;
        job
    });
    painter.galley(
        pos2(inner.center().x, inner.bottom() - copyright.size().y),
        copyright,
        colors.text_secondary(),
    );

    // The credits, centred in the rest of the window.
    let right = Rect::from_min_max(
        pos2(panel.right() + 22.0, full.top() + 14.0),
        full.max - vec2(14.0, 14.0),
    );
    let sections = credits();
    let row_h = 13.0 * 1.25 + 4.0;
    let content_h: f32 = sections
        .iter()
        .map(|(_, rows)| 8.0 + 22.0 + 4.0 + rows.len() as f32 * row_h + 14.0)
        .sum();
    let content_w = 180.0 + 260.0;
    let origin = pos2(
        right.center().x - content_w / 2.0,
        (right.center().y - content_h / 2.0).max(right.top()),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_size(
                origin,
                vec2(content_w, content_h + 20.0),
            ))
            .layout(Layout::top_down(Align::Min)),
    );
    for (title, rows) in sections {
        child.add_space(8.0);
        crate::widgets::text(&mut child, title, 17.0, Weight::SemiBold, colors.text());
        child.add_space(4.0);
        for (label, display, ask) in rows {
            child.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(180.0, row_h), Sense::hover());
                let g = layout(
                    ui,
                    trimmed(
                        &label,
                        13.0,
                        Weight::Normal,
                        colors.text_secondary(),
                        176.0,
                        1,
                    ),
                );
                ui.painter().galley(
                    pos2(r.left(), r.center().y - g.size().y / 2.0),
                    g,
                    colors.text_secondary(),
                );
                if link(ui, colors, display) {
                    asks.push(ask);
                }
            });
        }
        child.add_space(14.0);
    }
    asks
}

/// What a click in the Debug log asks for.
pub enum LogAsk {
    CopyAll(String),
    OpenLogFile,
    Upload(String),
    OpenUrl(String),
    OpenStartupFolder,
}

/// `DebugLogWindow`.
pub fn debug_log_ui(ui: &mut Ui, window: &mut DebugLogWindow, colors: &Colors) -> Vec<LogAsk> {
    let mut asks = Vec::new();
    // `MessageAdded`: new lines arrive while the window is open.
    let count = modlight_core::debug_log::count();
    if count != window.seen {
        window.text = modlight_core::debug_log::text();
        window.seen = count;
        window.scroll_to_end = true;
    }
    let full = ui.max_rect();
    ui.painter()
        .rect_filled(full, 0.0, colors.background_main());
    let area = Rect::from_min_max(
        pos2(full.left() + 12.0, full.top()),
        pos2(full.right() - 12.0, full.bottom() - 12.0),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(area)
            .layout(Layout::top_down(Align::Min)),
    );
    child.spacing_mut().item_spacing.y = 8.0;
    // The toolbar (`docked-top-toolbar`).
    let width = area.width();
    egui::Frame::new()
        .fill(colors.background_panel())
        .stroke(Stroke::new(1.0, colors.border()))
        .corner_radius(egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: 4,
            se: 4,
        })
        .inner_margin(8.0)
        .show(&mut child, |ui| {
            ui.set_width(width - 18.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let look = Look::button(colors)
                    .height(34.0)
                    .padding(10.0)
                    .icon(14.0, None)
                    .spacing(7.0);
                if button(
                    ui,
                    colors,
                    &look,
                    Content::both(icons::COPY, t(s::DebugLogWindow_CopyAll)),
                    true,
                )
                .clicked()
                {
                    asks.push(LogAsk::CopyAll(window.text.trim().to_string()));
                }
                let has_file = modlight_core::debug_log::file_path().is_some();
                if button(
                    ui,
                    colors,
                    &look,
                    Content::both(icons::FILE_TEXT, t(s::DebugLogWindow_OpenLogFile)),
                    has_file,
                )
                .clicked()
                {
                    asks.push(LogAsk::OpenLogFile);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(
                        ui,
                        colors,
                        &look,
                        Content::both(icons::UPLOAD, t(s::DebugLogWindow_UploadLog)),
                        true,
                    )
                    .clicked()
                        && !window.text.is_empty()
                    {
                        window.confirm_upload = true;
                    }
                });
            });
        });
    // The log (`tool-panel`, padding 1; the text 12 monospace with 9 padding).
    let footer_h = 20.0;
    let log_h = child.available_height() - footer_h - 8.0;
    let log_rect = Rect::from_min_size(child.cursor().min, vec2(width, log_h));
    child.allocate_rect(log_rect, Sense::hover());
    child
        .painter()
        .rect_filled(log_rect, 4.0, colors.background_panel());
    child.painter().rect_stroke(
        log_rect,
        4.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let mut log_ui = child.new_child(
        UiBuilder::new()
            .max_rect(log_rect.shrink(1.0))
            .layout(Layout::top_down(Align::Min)),
    );
    let scroll = egui::ScrollArea::both()
        .id_salt("debug-log")
        .auto_shrink([false, false])
        .stick_to_bottom(true);
    let mut text = window.text.clone();
    let output = scroll.show(&mut log_ui, |ui| {
        egui::TextEdit::multiline(&mut text)
            .id(egui::Id::new("debug-log-text"))
            .frame(egui::Frame::NONE.inner_margin(9))
            .font(egui::FontId::monospace(12.0))
            .text_color(colors.text())
            .code_editor()
            .desired_width(f32::INFINITY)
            .min_size(log_rect.size() - vec2(2.0, 2.0))
            .show(ui)
    });
    window.scroll_to_end = false;
    // A double click on an address opens it (`OnLogDoubleTapped`).
    let edit = output.inner;
    if edit.response.double_clicked()
        && let Some(range) = edit.cursor_range
        && let Some(url) = modlight_core::debug_log::url_at(&window.text, range.primary.index.0)
    {
        asks.push(LogAsk::OpenUrl(url));
    }
    // "Startup path:" and the folder the app runs from.
    child.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 7.0;
        crate::widgets::text(
            ui,
            t(s::DebugLogWindow_StartupPath),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
        );
        if link(ui, colors, &window.startup_path) {
            asks.push(LogAsk::OpenStartupFolder);
        }
    });
    if window.confirm_upload {
        upload_question(ui, colors, full, window, &mut asks);
    }
    let _ = paint_icon;
    asks
}

fn upload_question(
    ui: &mut Ui,
    colors: &Colors,
    full: Rect,
    window: &mut DebugLogWindow,
    asks: &mut Vec<LogAsk>,
) {
    let ctx = ui.ctx().clone();
    egui::Area::new(egui::Id::new("debug-log-upload"))
        .order(egui::Order::Foreground)
        .fixed_pos(full.min)
        .show(&ctx, |ui| {
            let (veil, _) = ui.allocate_exact_size(full.size(), Sense::click_and_drag());
            ui.painter().rect_filled(veil, 0.0, colors.overlay_modal());
            let inner_w = 440.0 - 36.0;
            let heading = layout(
                ui,
                wrapped(
                    t(s::DebugLogWindow_UploadPrompt),
                    15.0,
                    Weight::SemiBold,
                    colors.text(),
                    inner_w - 30.0,
                ),
            );
            let text = layout(
                ui,
                wrapped(
                    t(s::MainForm_UploadDebugLogWarning),
                    13.0,
                    Weight::Normal,
                    colors.text(),
                    inner_w,
                ),
            );
            let head_h = heading.size().y.max(20.0);
            let height = 18.0 + head_h + 12.0 + text.size().y + 12.0 + 40.0 + 18.0;
            let panel = Rect::from_center_size(veil.center(), vec2(440.0, height));
            ui.painter()
                .rect_filled(panel, 4.0, colors.background_panel());
            ui.painter().rect_stroke(
                panel,
                4.0,
                Stroke::new(1.0, colors.border()),
                StrokeKind::Inside,
            );
            let inner = panel.shrink(18.0);
            paint_icon(
                ui,
                ui.painter(),
                pos2(inner.left() + 10.0, inner.top() + head_h / 2.0),
                icons::TRIANGLE_ALERT,
                20.0,
                colors.cancel_border(),
            );
            ui.painter().galley(
                pos2(inner.left() + 30.0, inner.top()),
                heading,
                colors.text(),
            );
            let text_top = inner.top() + head_h + 12.0;
            let text_h = text.size().y;
            ui.painter()
                .galley(pos2(inner.left(), text_top), text, colors.text());
            let row = Rect::from_min_max(pos2(inner.left(), text_top + text_h + 12.0), inner.max);
            let mut child = ui.new_child(
                UiBuilder::new()
                    .max_rect(row)
                    .layout(Layout::right_to_left(Align::Center)),
            );
            child.spacing_mut().item_spacing.x = 8.0;
            let look = Look {
                string_text: Some(colors.text()),
                ..Look::button(colors)
            };
            if button(
                &mut child,
                colors,
                &look,
                Content::label(t(s::DebugLogWindow_Upload)),
                true,
            )
            .clicked()
            {
                window.confirm_upload = false;
                asks.push(LogAsk::Upload(window.text.clone()));
            }
            if button(
                &mut child,
                colors,
                &look,
                Content::label(t(s::DebugLogWindow_Cancel)),
                true,
            )
            .clicked()
            {
                window.confirm_upload = false;
            }
        });
}
