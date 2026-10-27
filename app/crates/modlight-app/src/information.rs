//! The Information page of the Graphite skin's side panel: About, in the main window.
//! What the About window shows (the logo, the name and the version, the copyright, the
//! team and the links, the Debug log among them) and an Updates card over the app's own
//! update check, used as About uses it: asked when the page opens and again on "Check
//! now" (only the real app, and only with a release channel), an update found offered
//! through the update question of `update_window`.

use egui::{Align, Layout, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{helpers, mac, s, t};

use crate::about_window::{AboutAsk, AboutUpdate, credits, link, logo_texture};
use crate::state::AppState;
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Look, button_at, button_width, job, layout, paint_icon, trimmed, wrapped,
};

/// The widest the page's column gets (a settings page's).
const MAX_WIDTH: f32 = 820.0;
/// The cards' label column.
const LABEL_WIDTH: f32 = 200.0;
/// A card's line.
const LINE: f32 = 40.0;

/// The page's state while it is shown.
pub struct InformationPage {
    /// What the Updates card says, read each frame from the shared update state.
    /// `Hidden`: not the real app (QA), which never asks.
    pub update: AboutUpdate,
    /// The logo, and whether it is the Graphite one.
    logo: Option<(bool, egui::TextureHandle)>,
}

impl InformationPage {
    pub fn new(update: AboutUpdate) -> InformationPage {
        InformationPage { update, logo: None }
    }
}

impl AppState {
    /// The side panel's About row (`ShowAbout` under the Graphite skin): the page in the
    /// window, and a check for an update as the About window makes when it opens.
    pub fn show_information(&mut self) {
        // The settings pages give way, as they do to a tab.
        if self.windows.settings.as_ref().is_some_and(|w| w.embedded) {
            self.close_settings();
        }
        self.side.reveal = true;
        if self.side.information.is_some() {
            return;
        }
        let status = crate::about_window::status_of(self);
        self.side.information = Some(InformationPage::new(status));
        self.check_for_update_now();
    }

    /// "Check now": the signed-in server's Downloads, as the About window asks it; the
    /// answer lands in the shared update state the page reads.
    pub fn check_for_update_now(&mut self) {
        if self.mode != crate::app::Mode::Normal {
            return;
        }
        crate::update_window::check_quietly(self);
    }
}

/// The page, in the content area `rect`.
pub fn ui(ui: &mut Ui, state: &mut AppState, rect: Rect) {
    let colors = state.colors.clone();
    let status = crate::about_window::status_of(state);
    let Some(page) = state.side.information.as_mut() else {
        return;
    };
    page.update = status;
    if page
        .logo
        .as_ref()
        .is_some_and(|(graphite, _)| *graphite != colors.graphite)
    {
        page.logo = None;
    }
    let (_, logo) = page.logo.get_or_insert_with(|| {
        (
            colors.graphite,
            logo_texture(ui.ctx(), colors.graphite, 160),
        )
    });
    let logo = logo.clone();
    let update = page.update.clone();
    // As About: an update is offered unless its question is up or a download runs.
    let can_offer = state.updates.message.is_none() && state.updates.downloader.is_none();
    let mut asks = Vec::new();
    let mut check_now = false;
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    child.set_clip_rect(rect);
    egui::ScrollArea::vertical()
        .id_salt("information-page")
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let width = (rect.width() - 56.0).clamp(300.0, MAX_WIDTH);
            ui.add_space(28.0);
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    hero(ui, &colors, &logo, width);
                    ui.add_space(24.0);
                    card(
                        ui,
                        &colors,
                        width,
                        t(s::ApplicationSettingsWindow_Updates),
                        |ui, width| {
                            updates(
                                ui,
                                &colors,
                                width,
                                &update,
                                can_offer,
                                &mut asks,
                                &mut check_now,
                            )
                        },
                    );
                    for (title, rows) in credits() {
                        card(ui, &colors, width, title, |ui, width| {
                            for (index, (label, display, ask)) in rows.into_iter().enumerate() {
                                if index > 0 {
                                    rule(ui, &colors, width);
                                }
                                if link_line(ui, &colors, width, &label, display) {
                                    asks.push(ask);
                                }
                            }
                        });
                    }
                });
            });
            ui.add_space(28.0);
        });
    if check_now {
        state.check_for_update_now();
    }
    for ask in asks {
        state.about_ask(ask);
    }
}

/// The logo, the name with the version, and the copyright.
fn hero(ui: &mut Ui, colors: &Colors, logo: &egui::TextureHandle, width: f32) {
    let logo_side = 64.0;
    let text_left = logo_side + 18.0;
    let title = layout(
        ui,
        job(
            modlight_core::APP_NAME,
            24.0,
            Weight::SemiBold,
            colors.heading(),
        ),
    );
    let version = layout(
        ui,
        job(
            &modlight_core::version_text(),
            12.0,
            Weight::Medium,
            colors.text_secondary(),
        ),
    );
    let copyright = layout(
        ui,
        wrapped(
            t(s::AboutWindow_Copyright),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            width - text_left,
        ),
    );
    let block = title.size().y + 6.0 + copyright.size().y;
    let (r, _) = ui.allocate_exact_size(vec2(width, block.max(logo_side)), Sense::hover());
    let painter = ui.painter();
    painter.image(
        logo.id(),
        Rect::from_min_size(
            pos2(r.left(), r.center().y - logo_side / 2.0),
            vec2(logo_side, logo_side),
        ),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
    let x = r.left() + text_left;
    let top = r.center().y - block / 2.0;
    let (title_w, title_h) = (title.size().x, title.size().y);
    painter.galley(pos2(x, top), title, colors.heading());
    // The version on a chip after the name.
    let chip = Rect::from_min_size(
        pos2(
            x + title_w + 10.0,
            top + (title_h - version.size().y - 6.0) / 2.0,
        ),
        version.size() + vec2(14.0, 6.0),
    );
    painter.rect_filled(chip, 6.0, colors.control_background());
    painter.rect_stroke(
        chip,
        6.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    painter.galley(chip.min + vec2(7.0, 3.0), version, colors.text_secondary());
    painter.galley(
        pos2(x, top + title_h + 6.0),
        copyright,
        colors.text_secondary(),
    );
}

/// A card: its title over a rule, then what `add` puts in, `width` wide.
fn card(ui: &mut Ui, colors: &Colors, width: f32, title: &str, add: impl FnOnce(&mut Ui, f32)) {
    egui::Frame::new()
        .fill(colors.background_popup())
        .stroke(Stroke::new(1.0, colors.border()))
        .corner_radius(10)
        .show(ui, |ui| {
            let inner = width - 2.0;
            ui.set_width(inner);
            let galley = layout(
                ui,
                trimmed(
                    title,
                    14.0,
                    Weight::SemiBold,
                    colors.text(),
                    inner - 32.0,
                    1,
                ),
            );
            let (r, _) = ui.allocate_exact_size(vec2(inner, 44.0), Sense::hover());
            ui.painter().galley(
                pos2(r.left() + 16.0, r.center().y - galley.size().y / 2.0),
                galley,
                colors.text(),
            );
            rule(ui, colors, inner);
            add(ui, inner);
        });
    ui.add_space(14.0);
}

/// The thin line between a card's lines.
fn rule(ui: &mut Ui, colors: &Colors, width: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
    ui.painter()
        .hline(r.x_range(), r.center().y, Stroke::new(1.0, colors.border()));
}

/// A label, and its value in the second column; returns the value's column.
fn line(ui: &mut Ui, colors: &Colors, width: f32, label: &str) -> Rect {
    let (r, _) = ui.allocate_exact_size(vec2(width, LINE), Sense::hover());
    let galley = layout(
        ui,
        trimmed(
            label,
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            LABEL_WIDTH - 24.0,
            1,
        ),
    );
    ui.painter().galley(
        pos2(r.left() + 16.0, r.center().y - galley.size().y / 2.0),
        galley,
        colors.text_secondary(),
    );
    Rect::from_min_max(pos2(r.left() + 16.0 + LABEL_WIDTH, r.top()), r.max)
}

fn value_line(ui: &mut Ui, colors: &Colors, width: f32, label: &str, value: &str) {
    let column = line(ui, colors, width, label);
    let galley = layout(
        ui,
        trimmed(
            value,
            13.0,
            Weight::Normal,
            colors.text(),
            column.width() - 16.0,
            1,
        ),
    );
    ui.painter().galley(
        pos2(column.left(), column.center().y - galley.size().y / 2.0),
        galley,
        colors.text(),
    );
}

/// A label and a link; whether the link was clicked.
fn link_line(ui: &mut Ui, colors: &Colors, width: f32, label: &str, display: &str) -> bool {
    let column = line(ui, colors, width, label);
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(column)
            .layout(Layout::left_to_right(Align::Center)),
    );
    link(&mut child, colors, display)
}

/// The Updates card's lines: this version, the newer one when there is one, and what the
/// last check said with "Check now" (and "Download" for an update).
fn updates(
    ui: &mut Ui,
    colors: &Colors,
    width: f32,
    update: &AboutUpdate,
    can_offer: bool,
    asks: &mut Vec<AboutAsk>,
    check_now: &mut bool,
) {
    value_line(
        ui,
        colors,
        width,
        t(helpers::UpdateMessageBox_UpdateMessageBox_CurrentVersion),
        env!("CARGO_PKG_VERSION"),
    );
    if let AboutUpdate::Available(manifest) = update {
        rule(ui, colors, width);
        value_line(
            ui,
            colors,
            width,
            t(helpers::UpdateMessageBox_UpdateMessageBox_LatestVersion),
            &manifest.version_text,
        );
    }
    rule(ui, colors, width);
    let (glyph, glyph_color, text) = match update {
        AboutUpdate::Hidden => (
            Some(icons::INFO),
            colors.text_secondary(),
            t(mac::UpdatesNotChecked),
        ),
        AboutUpdate::SignedOut => (
            Some(icons::LOG_IN),
            colors.text_secondary(),
            t(modlight_core::l10n::port::UpdateSignedOut),
        ),
        // `Checking` draws a spinner instead of a glyph.
        AboutUpdate::Checking => (
            None,
            colors.text_secondary(),
            t(s::AboutWindow_CheckingForUpdates),
        ),
        AboutUpdate::Failed => (
            Some(icons::TRIANGLE_ALERT),
            colors.cancel_border(),
            t(s::AboutWindow_UpdateCheckFailed),
        ),
        AboutUpdate::UpToDate => (
            Some(icons::CIRCLE_CHECK),
            colors.continue_border(),
            t(s::AboutWindow_UpToDate),
        ),
        AboutUpdate::Available(_) => (
            Some(icons::CIRCLE_ARROW_DOWN),
            colors.accent_start(),
            t(s::AboutWindow_NewVersionAvailable),
        ),
    };
    // The buttons at the right end: "Check now" whenever a check can be made, "Download"
    // for an update found.
    let check = Look {
        spacing: 6.0,
        ..Look::button(colors).height(32.0).padding(12.0)
    };
    let download = Look::primary(colors).height(32.0);
    let check_label = t(s::Components_CheckNow);
    let download_label = t(helpers::DownloaderWindow_Download);
    let can_check = !matches!(update, AboutUpdate::Hidden | AboutUpdate::SignedOut);
    let available = matches!(update, AboutUpdate::Available(_));
    let check_w = if can_check {
        button_width(ui, &check, Content::label(check_label))
    } else {
        0.0
    };
    let download_w = if available {
        button_width(ui, &download, Content::label(download_label))
    } else {
        0.0
    };
    let buttons_w = check_w + download_w + if available { 8.0 } else { 0.0 };
    let text_left = 16.0 + 22.0 + 10.0;
    let text_w = (width - text_left - 16.0 - buttons_w - 16.0).max(80.0);
    // Without a release channel the line is a note, not a state.
    let text_color = if can_check {
        colors.text()
    } else {
        colors.text_secondary()
    };
    let galley = layout(ui, wrapped(text, 13.0, Weight::Normal, text_color, text_w));
    let (r, _) = ui.allocate_exact_size(
        vec2(width, (galley.size().y + 24.0).max(56.0)),
        Sense::hover(),
    );
    let cy = r.center().y;
    match glyph {
        Some(glyph) => paint_icon(
            ui,
            ui.painter(),
            pos2(r.left() + 16.0 + 11.0, cy),
            glyph,
            18.0,
            glyph_color,
        ),
        // Asking: a spinner in the icon's place.
        None => egui::Spinner::new()
            .size(16.0)
            .color(colors.accent_start())
            .paint_at(
                ui,
                Rect::from_center_size(pos2(r.left() + 16.0 + 11.0, cy), vec2(16.0, 16.0)),
            ),
    }
    ui.painter().galley(
        pos2(r.left() + text_left, cy - galley.size().y / 2.0),
        galley,
        text_color,
    );
    let mut right = r.right() - 16.0;
    if let AboutUpdate::Available(manifest) = update {
        let at = Rect::from_min_size(pos2(right - download_w, cy - 16.0), vec2(download_w, 32.0));
        if button_at(
            ui,
            colors,
            &download,
            Content::label(download_label),
            can_offer,
            at,
        )
        .clicked()
        {
            asks.push(AboutAsk::Update(manifest.clone()));
        }
        right = at.left() - 8.0;
    }
    if can_check {
        let at = Rect::from_min_size(pos2(right - check_w, cy - 16.0), vec2(check_w, 32.0));
        let checking = *update == AboutUpdate::Checking;
        if button_at(
            ui,
            colors,
            &check,
            Content::label(check_label),
            !checking,
            at,
        )
        .clicked()
        {
            *check_now = true;
        }
    }
}
