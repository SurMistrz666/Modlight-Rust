//! `FirstTimeConfigWindow` and `ImageEditorSelectorWindow`.
//!
//! First run: upstream's installer opened it; the port has no installer, so it opens on
//! the first launch of the app bundle or of the installed copy (no
//! `ApplicationConfig.json` yet). On Windows it offers upstream's choices: Start with
//! Windows, "Upload with Modlight" in Explorer's context menu, Modlight in "Send to". On a
//! Mac they become the login item and a note on the Finder service, which macOS lists
//! under Services by itself.
//!
//! The image editor selector opens the first time the image editor is asked for
//! (`ShowImageEditorSelector`); the choice is stored and the editor opens (the legacy
//! editor is not ported, so both choices open the same one).

use egui::{Align, Layout, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use serde_json::json;

use crate::platform::integration::StartupState;
use modlight_core::icons;
#[cfg(not(windows))]
use modlight_core::l10n::mac;
use modlight_core::l10n::{Key, s, t};

use crate::state::AppState;
use crate::theme::{Colors, Weight};
use crate::widgets::{Content, Look, button, button_at, layout, paint_icon, wrapped};

pub const FIRST_RUN_SIZE: (f32, f32) = (600.0, 390.0);
pub const SELECTOR_WIDTH: f32 = 580.0;

pub struct FirstRunWindow {
    pub startup: StartupState,
    /// Windows: Modlight's entries in Explorer's menus.
    pub explorer: crate::app_pages::ExplorerMenus,
    pub status: String,
}

/// The subtitle and the footer note: upstream's on Windows, the Mac's own elsewhere.
#[cfg(windows)]
const SUBTITLE: Key = s::FirstTimeConfigWindow_Subtitle;
#[cfg(not(windows))]
const SUBTITLE: Key = mac::FirstRunSubtitle;
#[cfg(windows)]
const OPTIONS_LATER: Key = s::FirstTimeConfigWindow_OptionsLater;
#[cfg(not(windows))]
const OPTIONS_LATER: Key = mac::FirstRunOptionsLater;

#[derive(Default)]
pub struct EditorSelector {
    pub content_height: f32,
}

impl AppState {
    /// `FirstTimeConfigWindowIntegration.Show`.
    pub fn show_first_run(&mut self) {
        let real = !self.env.qa && self.mode == crate::app::Mode::Normal;
        let startup = if real {
            crate::platform::integration::startup_state()
        } else {
            StartupState::NotAvailable
        };
        #[cfg(windows)]
        let explorer = if real {
            crate::app_pages::ExplorerMenus {
                context_menu: crate::platform::integration::context_menu_enabled(),
                send_to: crate::platform::integration::send_to_enabled(),
            }
        } else {
            Default::default()
        };
        #[cfg(not(windows))]
        let explorer = Default::default();
        self.windows.first_run = Some(FirstRunWindow {
            startup,
            explorer,
            status: String::new(),
        });
    }

    /// `TaskHelpers.OpenImageEditor`: the selector first, when it has not been answered.
    pub fn open_image_editor(&mut self) {
        let settings = self.default_task_settings();
        let show = self
            .config
            .get("DefaultTaskSettings")
            .and_then(|d| d.get("ToolsSettings"))
            .and_then(|t| t.get("ShowImageEditorSelector"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        let _ = settings;
        if show {
            self.windows.editor_selector = Some(EditorSelector::default());
        } else {
            self.open_image_editor_window();
        }
    }

    /// The selector was answered (`None`: closed without a choice).
    pub fn editor_selector_finished(&mut self, legacy: Option<bool>) {
        self.windows.editor_selector = None;
        if let Some(legacy) = legacy {
            self.set_task_member(
                crate::task_pages::TaskTarget::Defaults,
                &["ToolsSettings", "UseLegacyImageEditor"],
                json!(legacy),
            );
            self.set_task_member(
                crate::task_pages::TaskTarget::Defaults,
                &["ToolsSettings", "ShowImageEditorSelector"],
                json!(false),
            );
        }
        // The legacy editor (Greenshot's WinForms one) is not ported: either choice opens
        // the editor (MACOS.md).
        self.open_image_editor_window();
    }
}

/// What the first-run window asked for.
pub enum FirstRunAsk {
    StartAtLogin(bool),
    /// Windows: "Upload with Modlight" in Explorer's context menu.
    #[cfg(windows)]
    ContextMenu(bool),
    /// Windows: Modlight in Explorer's "Send to" menu.
    #[cfg(windows)]
    SendTo(bool),
    Ok,
}

pub fn first_run_ui(ui: &mut Ui, window: &FirstRunWindow, colors: &Colors) -> Vec<FirstRunAsk> {
    let mut asks = Vec::new();
    let full = ui.max_rect();
    ui.painter()
        .rect_filled(full, 0.0, colors.background_main());
    let inner = full.shrink(22.0);
    let painter = ui.painter().clone();
    // The heading: the gear (28, accent), the title (24 SemiBold, accent), the subtitle.
    let title = layout(
        ui,
        wrapped(
            t(s::FirstTimeConfigWindow_Heading),
            24.0,
            Weight::SemiBold,
            colors.accent_start(),
            inner.width() - 40.0,
        ),
    );
    let subtitle = layout(
        ui,
        wrapped(
            t(SUBTITLE),
            12.0,
            Weight::Normal,
            colors.text_secondary(),
            inner.width() - 40.0,
        ),
    );
    let head_h = title.size().y + 2.0 + subtitle.size().y;
    paint_icon(
        ui,
        &painter,
        pos2(inner.left() + 14.0, inner.top() + head_h / 2.0),
        icons::SETTINGS_2,
        28.0,
        colors.accent_start(),
    );
    let title_h = title.size().y;
    painter.galley(
        pos2(inner.left() + 28.0 + 11.0, inner.top()),
        title,
        colors.accent_start(),
    );
    painter.galley(
        pos2(inner.left() + 28.0 + 11.0, inner.top() + title_h + 2.0),
        subtitle,
        colors.text_secondary(),
    );
    // The options.
    let body_top = inner.top() + head_h + 14.0;
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(inner.left(), body_top),
                pos2(inner.right(), inner.bottom() - 36.0 - 14.0),
            ))
            .layout(Layout::top_down(Align::Min)),
    );
    child.spacing_mut().item_spacing.y = 8.0;
    let width = inner.width();
    egui::Frame::new()
        .fill(colors.background_panel())
        .stroke(Stroke::new(1.0, colors.border()))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(&mut child, |ui| {
            ui.set_width(width - 30.0);
            ui.spacing_mut().item_spacing.y = 7.0;
            options(ui, window, colors, &mut asks);
        });
    if !window.status.is_empty() {
        egui::Frame::new()
            .fill(colors.control_background())
            .stroke(Stroke::new(1.0, colors.cancel_border()))
            .corner_radius(5)
            .inner_margin(egui::Margin::symmetric(10, 7))
            .show(&mut child, |ui| {
                ui.set_width(width - 22.0);
                crate::widgets::text_wrapped(
                    ui,
                    &window.status,
                    13.0,
                    Weight::Normal,
                    colors.text(),
                );
            });
    }
    // The footer: the note and OK (92 by 36).
    let ok_rect = Rect::from_min_size(
        pos2(inner.right() - 92.0, inner.bottom() - 36.0),
        vec2(92.0, 36.0),
    );
    let note = layout(
        ui,
        wrapped(
            t(OPTIONS_LATER),
            12.0,
            Weight::Normal,
            colors.text_secondary(),
            inner.width() - 92.0 - 12.0,
        ),
    );
    painter.galley(
        pos2(inner.left(), ok_rect.center().y - note.size().y / 2.0),
        note,
        colors.text_secondary(),
    );
    let look = Look {
        string_text: Some(colors.text()),
        ..Look::button(colors).height(36.0)
    };
    if button_at(
        ui,
        colors,
        &look,
        Content::label(t(s::FirstTimeConfigWindow_OK)),
        true,
        ok_rect,
    )
    .clicked()
    {
        asks.push(FirstRunAsk::Ok);
    }
    asks
}

/// A Mac's options: the login item and the note on the Finder service.
#[cfg(not(windows))]
fn options(ui: &mut Ui, window: &FirstRunWindow, colors: &Colors, asks: &mut Vec<FirstRunAsk>) {
    let (label, enabled) = match window.startup {
        StartupState::DisabledByUser => (t(mac::StartAtLoginDisabledByUser), false),
        StartupState::RequiresApproval => (t(mac::StartAtLoginRequiresApproval), true),
        StartupState::NotAvailable => (t(mac::StartAtLoginNotAvailable), false),
        StartupState::Enabled | StartupState::Disabled => (t(mac::StartAtLogin), true),
    };
    let mut on = matches!(
        window.startup,
        StartupState::Enabled | StartupState::RequiresApproval
    );
    if crate::widgets::checkbox_weighted(ui, colors, &mut on, Some(label), enabled, Weight::Normal)
        .changed()
    {
        asks.push(FirstRunAsk::StartAtLogin(on));
    }
    crate::widgets::text_wrapped(
        ui,
        t(mac::FinderServiceHint),
        12.0,
        Weight::Normal,
        colors.text_secondary(),
    );
}

/// Windows: upstream's three options (`FirstTimeConfigViewModel`).
#[cfg(windows)]
fn options(ui: &mut Ui, window: &FirstRunWindow, colors: &Colors, asks: &mut Vec<FirstRunAsk>) {
    let (label, enabled, mut on) = crate::app_pages::start_with_windows(window.startup, true, None);
    if crate::widgets::checkbox_weighted(ui, colors, &mut on, Some(&label), enabled, Weight::Normal)
        .changed()
    {
        asks.push(FirstRunAsk::StartAtLogin(on));
    }
    let mut context_menu = window.explorer.context_menu;
    if crate::widgets::checkbox_weighted(
        ui,
        colors,
        &mut context_menu,
        Some(t(s::FirstTimeConfigWindow_ShellContextMenu)),
        true,
        Weight::Normal,
    )
    .changed()
    {
        asks.push(FirstRunAsk::ContextMenu(context_menu));
    }
    let mut send_to = window.explorer.send_to;
    if crate::widgets::checkbox_weighted(
        ui,
        colors,
        &mut send_to,
        Some(t(s::FirstTimeConfigWindow_SendToMenu)),
        true,
        Weight::Normal,
    )
    .changed()
    {
        asks.push(FirstRunAsk::SendTo(send_to));
    }
}

/// The selector; `Some(legacy)` when a choice was made.
pub fn selector_ui(ui: &mut Ui, window: &mut EditorSelector, colors: &Colors) -> Option<bool> {
    let mut choice = None;
    let full = ui.max_rect();
    ui.painter()
        .rect_filled(full, 0.0, colors.background_main());
    let painter = ui.painter().clone();
    // The band: a 40 square with the image icon, the heading and the subtitle.
    let band = Rect::from_min_size(full.min, vec2(full.width(), 12.0 + 40.0 + 12.0));
    painter.rect_filled(band, 0.0, colors.background_toolbar());
    let square = Rect::from_min_size(
        pos2(band.left() + 18.0, band.top() + 12.0),
        vec2(40.0, 40.0),
    );
    painter.rect_filled(square, 6.0, colors.background_panel());
    paint_icon(
        ui,
        &painter,
        square.center(),
        icons::IMAGE,
        20.0,
        colors.accent_start(),
    );
    let heading = layout(
        ui,
        crate::widgets::job(
            t(s::ImageEditorSelectorWindow_Heading),
            19.0,
            Weight::SemiBold,
            colors.accent_start(),
        ),
    );
    let subtitle = layout(
        ui,
        crate::widgets::job(
            t(s::ImageEditorSelectorWindow_Subtitle),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
        ),
    );
    let text_h = heading.size().y + 1.0 + subtitle.size().y;
    let top = square.center().y - text_h / 2.0;
    let heading_h = heading.size().y;
    painter.galley(
        pos2(square.right() + 10.0, top),
        heading,
        colors.accent_start(),
    );
    painter.galley(
        pos2(square.right() + 10.0, top + heading_h + 1.0),
        subtitle,
        colors.text_secondary(),
    );
    // The two choices, 104 tall, 10 apart.
    let row = Rect::from_min_size(
        pos2(full.left() + 18.0, band.bottom() + 14.0),
        vec2(full.width() - 36.0, 104.0),
    );
    let half = (row.width() - 10.0) / 2.0;
    let choices = [
        (
            Rect::from_min_size(row.min, vec2(half, 104.0)),
            icons::WAND_SPARKLES,
            t(s::ImageEditorSelectorWindow_ModernEditor),
            Some(t(s::ImageEditorSelectorWindow_Recommended)),
            t(s::ImageEditorSelectorWindow_ModernDescription),
            false,
        ),
        (
            Rect::from_min_size(pos2(row.left() + half + 10.0, row.top()), vec2(half, 104.0)),
            icons::PAINTBRUSH,
            t(s::ImageEditorSelectorWindow_LegacyEditor),
            None,
            t(s::ImageEditorSelectorWindow_LegacyDescription),
            true,
        ),
    ];
    for (index, (rect, glyph, title, badge, description, legacy)) in choices.into_iter().enumerate()
    {
        let response = ui.interact(rect, ui.id().with(("editor-choice", index)), Sense::click());
        let (fill, border) = if response.hovered() {
            (colors.control_background_hover(), colors.accent_start())
        } else {
            (colors.background_panel(), colors.border())
        };
        painter.rect_filled(rect, 6.0, fill);
        painter.rect_stroke(rect, 6.0, Stroke::new(1.0, border), StrokeKind::Inside);
        let inner = rect.shrink2(vec2(14.0, 12.0));
        paint_icon(
            ui,
            &painter,
            pos2(inner.left() + 12.0, inner.top() + 12.0),
            glyph,
            24.0,
            colors.accent_start(),
        );
        let text_left = inner.left() + 24.0 + 12.0;
        let text_w = inner.right() - text_left;
        let mut y = inner.top();
        let title = layout(
            ui,
            crate::widgets::job(title, 15.0, Weight::SemiBold, colors.text()),
        );
        let title_h = title.size().y;
        painter.galley(pos2(text_left, y), title, colors.text());
        y += title_h + 4.0;
        if let Some(badge) = badge {
            let badge = layout(
                ui,
                crate::widgets::job(badge, 11.0, Weight::Normal, colors.accent_start()),
            );
            let badge_h = badge.size().y;
            painter.galley(pos2(text_left, y), badge, colors.accent_start());
            y += badge_h + 4.0;
        }
        let description = layout(
            ui,
            wrapped(
                description,
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                text_w,
            ),
        );
        painter.galley(pos2(text_left, y), description, colors.text_secondary());
        if response.clicked() {
            choice = Some(legacy);
        }
    }
    // The note.
    let note = layout(
        ui,
        wrapped(
            t(s::ImageEditorSelectorWindow_ChangeLater),
            12.0,
            Weight::Normal,
            colors.text_secondary(),
            row.width() - 20.0,
        ),
    );
    let note_rect = Rect::from_min_size(
        pos2(row.left(), row.bottom() + 12.0),
        vec2(row.width(), note.size().y + 16.0),
    );
    painter.rect_filled(note_rect, 5.0, colors.background_panel());
    painter.galley(
        note_rect.min + vec2(10.0, 8.0),
        note,
        colors.text_secondary(),
    );
    window.content_height = note_rect.bottom() - full.top() + 16.0;
    let _ = button;
    choice
}
