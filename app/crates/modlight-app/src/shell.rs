//! The main window (`MainWindow.axaml`): the title band, the tab strip with storage,
//! account and settings at its right end, the tab contents, the completion notices in
//! the corner and the drop overlay.

use egui::{Color32, Id, Order, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{Key, s, t};

use crate::commands::Command;
use crate::menus::Entry;
use crate::state::{AppState, Tab};
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Look, button_at, button_width, icon_galley, job, layout, paint_icon, paint_progress,
    trimmed, wrapped,
};

pub const TITLE_HEIGHT: f32 = 34.0;
/// The tab strip row: 6 above, 38 of content, the 1 px rule.
pub const STRIP_HEIGHT: f32 = 45.0;
/// Where the window's own buttons sit on a Mac; the logo and title start after them.
pub const TRAFFIC_LIGHTS_WIDTH: f32 = 78.0;

/// Draw the whole window into `ui`.
pub fn ui(ui: &mut Ui, state: &mut AppState, logo: Option<&egui::TextureHandle>) {
    let full = ui.max_rect();
    let colors = state.colors.clone();
    ui.painter()
        .rect_filled(full, 0.0, colors.background_main());

    let title = Rect::from_min_size(full.min, vec2(full.width(), TITLE_HEIGHT));
    title_band(ui, state, title, logo);
    let strip = Rect::from_min_size(
        pos2(full.left(), title.bottom()),
        vec2(full.width(), STRIP_HEIGHT),
    );
    tab_strip(ui, state, strip);
    let content = Rect::from_min_max(pos2(full.left(), strip.bottom()), full.max);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(content)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(content);
    match state.tab {
        Tab::MySpace => crate::myspace::view::ui(&mut child, state),
        Tab::Upload => crate::upload_tab::ui(&mut child, state, content),
        Tab::Download => crate::download_tab::ui(&mut child, state, content),
        Tab::Create => capture_tab(&mut child, state),
        Tab::Tools => tools_tab(&mut child, state),
        Tab::Admin => crate::admin_tab::ui(&mut child, state, content),
    }
    toasts(ui, state, full);
    drop_overlay(ui, state, full);
}

// ---------------------------------------------------------------------- title band

fn title_band(ui: &mut Ui, state: &mut AppState, rect: Rect, logo: Option<&egui::TextureHandle>) {
    let colors = &state.colors;
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, colors.background_toolbar());
    // Dragging the band moves the window; a double click zooms it, as on any Mac title bar.
    let band = ui.interact(rect, Id::new("title-band"), Sense::click_and_drag());
    if band.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if band.double_clicked() {
        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
    let mut x = rect.left() + TRAFFIC_LIGHTS_WIDTH;
    let cy = rect.center().y;
    if let Some(logo) = logo {
        let r = Rect::from_min_size(pos2(x, cy - 8.5), vec2(17.0, 17.0));
        painter.image(
            logo.id(),
            r,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    x += 17.0 + 9.0;
    // `UpdateMainWindowTitle`: the average progress while uploads run.
    let title = state.uploads.tasks.title();
    let galley = layout(
        ui,
        trimmed(
            &title,
            12.0,
            Weight::Normal,
            colors.text_secondary(),
            (rect.right() - 12.0 - x).max(10.0),
            1,
        ),
    );
    painter.galley(
        pos2(x, cy - galley.size().y / 2.0),
        galley,
        colors.text_secondary(),
    );
}

// ---------------------------------------------------------------------- tab strip

struct TabSpec {
    tab: Tab,
    icon: char,
    label: Key,
}

const TABS: [TabSpec; 6] = [
    TabSpec {
        tab: Tab::MySpace,
        icon: icons::CLOUD,
        label: s::MainWindow_TabMySpace,
    },
    TabSpec {
        tab: Tab::Upload,
        icon: icons::UPLOAD,
        label: s::MainWindow_TabUpload,
    },
    TabSpec {
        tab: Tab::Download,
        icon: icons::DOWNLOAD,
        label: s::MainWindow_TabDownload,
    },
    TabSpec {
        tab: Tab::Create,
        icon: icons::SPARKLES,
        label: s::MainWindow_TabCreate,
    },
    TabSpec {
        tab: Tab::Tools,
        icon: icons::WRENCH,
        label: s::MainWindow_TabTools,
    },
    TabSpec {
        tab: Tab::Admin,
        icon: icons::SERVER,
        label: s::MainWindow_TabAdmin,
    },
];

fn tab_strip(ui: &mut Ui, state: &mut AppState, rect: Rect) {
    let colors = state.colors.clone();
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, colors.background_toolbar());
    painter.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, colors.border()),
    );
    let grid = Rect::from_min_max(
        pos2(rect.left() + 10.0, rect.top() + 6.0),
        pos2(rect.right() - 10.0, rect.bottom() - 1.0),
    );

    // The tabs, bottom-aligned, 2 apart.
    let mut x = grid.left();
    let admin = state.is_admin();
    for spec in &TABS {
        if spec.tab == Tab::Admin && !admin {
            continue;
        }
        let selected = state.tab == spec.tab;
        let label = t(spec.label);
        let weight = if selected {
            Weight::SemiBold
        } else {
            Weight::Normal
        };
        let text = layout(ui, job(label, 13.0, weight, colors.text()));
        let icon = icon_galley(ui, spec.icon, 15.0, colors.text());
        let w = 14.0 + icon.size().x + 8.0 + text.size().x + 14.0;
        let r = Rect::from_min_size(pos2(x, grid.bottom() - 36.0), vec2(w, 36.0));
        let response = ui.interact(r, Id::new(("tab", spec.tab)), Sense::click());
        let hovered = response.hovered();
        let pressed = response.is_pointer_button_down_on();
        let fill = if selected || pressed {
            Some(colors.control_background_hover())
        } else if hovered {
            Some(colors.control_background())
        } else {
            None
        };
        if let Some(fill) = fill {
            painter.rect_filled(r, 10.0, fill);
        }
        let (icon_color, icon_alpha, text_alpha) = if selected {
            (colors.accent_start(), 1.0, 1.0)
        } else if hovered {
            (colors.text_secondary(), 0.9, 0.95)
        } else {
            (colors.text_secondary(), 0.7, 0.75)
        };
        let icon_size = if selected { 15.0 * 1.08 } else { 15.0 };
        let icon = icon_galley(ui, spec.icon, icon_size, icon_color);
        let cy = r.center().y;
        let icon_w = icon.size().x;
        let mut layer = painter.clone();
        layer.set_opacity(icon_alpha);
        let icon_x = r.left() + 14.0 + (15.0 - icon_w).min(0.0) / 2.0;
        layer.galley(pos2(icon_x, cy - icon.size().y / 2.0), icon, icon_color);
        let mut label_layer = painter.clone();
        label_layer.set_opacity(text_alpha);
        let text_size = text.size();
        label_layer.galley(
            pos2(r.left() + 14.0 + 15.0 + 8.0, cy - text_size.y / 2.0),
            text,
            colors.text(),
        );
        if response.clicked() {
            state.push(Command::SelectTab(spec.tab));
        }
        x += w + 2.0;
    }

    // The right end: storage, account, settings, 6 apart, centred over the bottom 6.
    let cy = grid.top() + 16.0;
    let mut right = grid.right();
    let settings = Rect::from_min_size(pos2(right - 34.0, cy - 16.0), vec2(34.0, 32.0));
    let look = Look::strip(&colors)
        .width(34.0)
        .padding(0.0)
        .icon(17.0, None);
    let response = button_at(
        ui,
        &colors,
        &look,
        Content::icon(icons::SETTINGS),
        true,
        settings,
    )
    .on_hover_text(t(s::MainWindow_Settings));
    if let Some(command) = crate::menus::popup_below(&colors, &response, || settings_menu(state)) {
        state.push(command);
    }
    right = settings.left() - 6.0;

    let account = account_button(ui, state, right, cy);
    right = account.left() - 6.0;
    if state.strip.has_storage {
        storage_chip(ui, state, right, cy);
    }
}

fn account_button(ui: &mut Ui, state: &mut AppState, right: f32, cy: f32) -> Rect {
    let colors = state.colors.clone();
    let signed_in = state.account.is_signed_in();
    let label = state.account_label();
    let text = layout(
        ui,
        trimmed(
            &label,
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            150.0,
            1,
        ),
    );
    let mark_w = if signed_in {
        22.0
    } else {
        icon_galley(ui, icons::LOG_IN, 16.0, colors.text_secondary())
            .size()
            .x
    };
    let w = 9.0 + mark_w + 7.0 + text.size().x + 9.0;
    let r = Rect::from_min_size(pos2(right - w, cy - 16.0), vec2(w, 32.0));
    let response = ui.interact(r, Id::new("account-button"), Sense::click());
    let painter = ui.painter().clone();
    if response.hovered() || response.is_pointer_button_down_on() {
        painter.rect_filled(r, 6.0, colors.control_background_hover());
    }
    let mut x = r.left() + 9.0;
    if signed_in {
        let circle = Rect::from_min_size(pos2(x, cy - 11.0), vec2(22.0, 22.0));
        let texture = state.media.avatar.texture().cloned();
        crate::signin::paint_avatar(
            ui,
            &colors,
            circle,
            texture.as_ref(),
            &state.account.display_name(),
            16.0,
            10.0,
        );
    } else {
        paint_icon(
            ui,
            &painter,
            pos2(x + mark_w / 2.0, cy),
            icons::LOG_IN,
            16.0,
            colors.text_secondary(),
        );
    }
    x += mark_w + 7.0;
    painter.galley(
        pos2(x, cy - text.size().y / 2.0),
        text,
        colors.text_secondary(),
    );
    let response = response.on_hover_text(state.account_tooltip());
    if !signed_in {
        if response.clicked() {
            state.push(Command::ShowSignIn);
        }
    } else if let Some(command) = crate::menus::popup_below(&colors, &response, account_menu) {
        state.push(command);
    }
    r
}

fn storage_chip(ui: &mut Ui, state: &mut AppState, right: f32, cy: f32) {
    let colors = state.colors.clone();
    let strip = &state.strip;
    let text = layout(
        ui,
        trimmed(
            &strip.storage_text,
            11.0,
            Weight::Normal,
            colors.text_secondary(),
            150.0,
            1,
        ),
    );
    let glyph = if strip.has_storage_problem {
        icons::TRIANGLE_ALERT
    } else {
        icons::HARD_DRIVE
    };
    let glyph_color = if strip.has_storage_problem {
        colors.cancel_border()
    } else {
        colors.accent_start()
    };
    let icon = icon_galley(ui, glyph, 15.0, glyph_color);
    let column_w = if strip.has_storage_bar {
        text.size().x.max(112.0)
    } else {
        text.size().x
    };
    let column_h = text.size().y
        + if strip.has_storage_bar {
            4.0 + 3.0
        } else {
            0.0
        };
    let inner_h = column_h.max(icon.size().y);
    let w = 1.0 + 9.0 + icon.size().x + 8.0 + column_w + 9.0 + 1.0;
    let h = 1.0 + 4.0 + inner_h + 4.0 + 1.0;
    let r = Rect::from_min_size(pos2(right - w, cy - h / 2.0), vec2(w, h));
    let painter = ui.painter().clone();
    painter.rect_filled(r, 6.0, colors.control_background());
    painter.rect_stroke(
        r,
        6.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let x = r.left() + 10.0;
    let icon_w = icon.size().x;
    painter.galley(pos2(x, cy - icon.size().y / 2.0), icon, glyph_color);
    let tx = x + icon_w + 8.0;
    let top = cy - column_h / 2.0;
    let text_h = text.size().y;
    painter.galley(pos2(tx, top), text, colors.text_secondary());
    if strip.has_storage_bar {
        let bar = Rect::from_min_size(pos2(tx, top + text_h + 4.0), vec2(112.0, 3.0));
        paint_progress(ui, &colors, bar, Some(strip.storage_percent / 100.0));
    }
    let tooltip = strip.storage_tooltip.clone();
    let _ = ui
        .interact(r, Id::new("storage-chip"), Sense::hover())
        .on_hover_text(tooltip);
}

/// `MainMenuBuilder.BuildAccountMenu`.
pub fn account_menu() -> Vec<Entry> {
    vec![
        Entry::item(
            t(s::MainMenuBuilder_AccountSettings),
            icons::CIRCLE_USER,
            Command::ShowSettings(Some(crate::settings::ACCOUNT_SECTION)),
        ),
        Entry::item(
            t(s::MainMenuBuilder_ModlightAccount),
            icons::PLUG,
            Command::ShowSignIn,
        ),
        Entry::Separator,
        Entry::item(
            t(s::MainMenuBuilder_SignOut),
            icons::LOG_OUT,
            Command::SignOut,
        ),
    ]
}

/// `MainMenuBuilder.BuildSettingsMenu`; what arrives in a later phase says so.
pub fn settings_menu(state: &AppState) -> Vec<Entry> {
    let defaults = state.default_task_settings();
    vec![
        Entry::item(
            t(s::SettingsWindow_WindowTitle),
            icons::SETTINGS,
            Command::ShowSettings(None),
        ),
        Entry::item(
            t(s::MainMenuBuilder_AccountSettings),
            icons::CIRCLE_USER,
            Command::ShowSettings(Some(crate::settings::ACCOUNT_SECTION)),
        ),
        Entry::Separator,
        Entry::submenu(
            t(s::MainMenuBuilder_AfterCaptureTasks),
            icons::IMAGE_UP,
            true,
            crate::uploads::after_capture_menu(defaults.after_capture_job),
        ),
        Entry::submenu(
            t(s::MainMenuBuilder_AfterUploadTasks),
            icons::CLOUD_UPLOAD,
            !state.app_upload_settings().disable_upload,
            crate::uploads::after_upload_menu(defaults.after_upload_job),
        ),
        Entry::Separator,
        Entry::item(
            t(s::MainMenuBuilder_ScreenshotsFolder),
            icons::FOLDER_OPEN,
            Command::OpenScreenshotsFolder,
        ),
        Entry::item(t(s::MainMenuBuilder_About), icons::INFO, Command::NotYet),
    ]
}

// ---------------------------------------------------------------------- other tabs

/// The capture tab (`IsCreateSelected`): the hint, the notice when a hotkey could not be
/// registered, four large buttons with their shortcuts, and the full menu under them.
fn capture_tab(ui: &mut Ui, state: &mut AppState) {
    use modlight_core::capture::CaptureKind;
    use modlight_core::capture::region::RegionKind;
    use modlight_core::hotkeys::HotkeyType;
    use modlight_core::recording::ScreenRecordOutput;
    let colors = state.colors.clone();
    let rect = ui.max_rect();
    let painter = ui.painter().clone();
    let left = rect.left() + 18.0;
    let width = rect.width() - 36.0;
    let mut y = rect.top() + 18.0;
    let hint = layout(
        ui,
        wrapped(
            t(s::MainWindow_CaptureHint),
            11.0,
            Weight::Normal,
            colors.text_secondary(),
            width,
        ),
    );
    let hint_h = hint.size().y;
    let mut faint = painter.clone();
    faint.set_opacity(0.75);
    faint.galley(pos2(left, y), hint, colors.text_secondary());
    y += hint_h;

    // `Border.notice`: 10 px corners, 14,11 padding, the keys macOS kept.
    if let Some(problem) = crate::capture_flow::hotkey_problem(state) {
        y += 10.0;
        let fix = t(s::MainWindow_HotkeyProblemFix);
        let look = Look::quiet_link(&colors);
        let fix_w = button_width(ui, &look, Content::label(fix));
        let text_w = width - 28.0 - 16.0 - 10.0 - 10.0 - fix_w;
        let text = layout(
            ui,
            wrapped(
                &problem,
                13.0,
                Weight::SemiBold,
                colors.text_secondary(),
                text_w,
            ),
        );
        let height = (text.size().y.max(32.0)) + 22.0;
        let notice = Rect::from_min_size(pos2(left, y), vec2(width, height));
        painter.rect_filled(notice, 10.0, colors.control_background());
        painter.rect_stroke(
            notice,
            10.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        paint_icon(
            ui,
            &painter,
            pos2(notice.left() + 14.0 + 8.0, notice.center().y),
            icons::KEYBOARD_OFF,
            16.0,
            colors.text_secondary(),
        );
        painter.galley(
            pos2(
                notice.left() + 14.0 + 16.0 + 10.0,
                notice.center().y - text.size().y / 2.0,
            ),
            text,
            colors.text_secondary(),
        );
        let fix_rect = Rect::from_min_size(
            pos2(notice.right() - 14.0 - fix_w, notice.center().y - 16.0),
            vec2(fix_w, 32.0),
        );
        if button_at(ui, &colors, &look, Content::label(fix), true, fix_rect).clicked() {
            state.push(Command::ShowSettings(Some("hotkeys.list")));
        }
        y = notice.bottom();
    }
    y += 12.0;

    let tiles: [(char, Key, Key, HotkeyType, Command); 4] = [
        (
            icons::SCAN,
            s::MainWindow_CaptureRegionTitle,
            s::MainWindow_CaptureRegionBody,
            HotkeyType::RectangleRegion,
            Command::Capture(CaptureKind::Region(RegionKind::Default), true),
        ),
        (
            icons::VIDEO,
            s::MainWindow_CaptureRecordTitle,
            s::MainWindow_CaptureRecordBody,
            HotkeyType::ScreenRecorder,
            Command::StartRecording(
                ScreenRecordOutput::FFmpeg,
                crate::capture_flow::RecordStartMethod::Region,
            ),
        ),
        (
            icons::FILM,
            s::MainWindow_CaptureGifTitle,
            s::MainWindow_CaptureGifBody,
            HotkeyType::ScreenRecorderGIF,
            Command::StartRecording(
                ScreenRecordOutput::Gif,
                crate::capture_flow::RecordStartMethod::Region,
            ),
        ),
        (
            icons::CLIPBOARD,
            s::MainWindow_CaptureClipboardTitle,
            s::MainWindow_CaptureClipboardBody,
            HotkeyType::ClipboardUpload,
            Command::ClipboardUpload,
        ),
    ];
    let cell = width / 2.0;
    for (index, (glyph, title, body, job, command)) in tiles.into_iter().enumerate() {
        let col = index % 2;
        let row = index / 2;
        let r = Rect::from_min_size(
            pos2(left + col as f32 * cell, y + row as f32 * (86.0 + 12.0)),
            vec2(cell - 12.0, 86.0),
        );
        let response = ui.interact(r, Id::new(("capture-tile", index)), Sense::click());
        let hovered = response.hovered();
        let scale = if response.is_pointer_button_down_on() {
            0.97
        } else {
            1.0
        };
        let shown = Rect::from_center_size(r.center(), r.size() * scale);
        painter.rect_filled(
            shown,
            10.0,
            if hovered {
                colors.control_background()
            } else {
                colors.background_panel()
            },
        );
        painter.rect_stroke(
            shown,
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
        let badge_scale = if hovered { 1.06 } else { 1.0 };
        let badge = Rect::from_center_size(
            pos2(shown.left() + 16.0 + 23.0, shown.center().y),
            vec2(46.0, 46.0) * badge_scale,
        );
        painter.rect_filled(badge, 10.0, colors.control_background_hover());
        paint_icon(
            ui,
            &painter,
            badge.center(),
            glyph,
            22.0,
            colors.accent_start(),
        );
        // `capture-tile-key`: the registered shortcut, right of the text.
        let key = state.captures.hotkeys.shortcut_for(job);
        let key_galley = key.as_deref().map(|k| {
            layout(
                ui,
                trimmed(k, 11.0, Weight::SemiBold, colors.text_secondary(), 200.0, 1),
            )
        });
        let key_w = key_galley
            .as_ref()
            .map(|g| g.size().x + 14.0 + 14.0)
            .unwrap_or(0.0);
        let text_left = shown.left() + 16.0 + 46.0 + 14.0;
        let text_w = shown.right() - 16.0 - key_w - text_left;
        let title_g = layout(
            ui,
            wrapped(t(title), 15.0, Weight::SemiBold, colors.text(), text_w),
        );
        let body_g = layout(
            ui,
            wrapped(
                t(body),
                12.0,
                Weight::SemiBold,
                colors.text_secondary(),
                text_w,
            ),
        );
        let stack = title_g.size().y + 3.0 + body_g.size().y;
        let top = shown.center().y - stack / 2.0;
        let title_h = title_g.size().y;
        painter.galley(pos2(text_left, top), title_g, colors.text());
        painter.galley(
            pos2(text_left, top + title_h + 3.0),
            body_g,
            colors.text_secondary(),
        );
        if let Some(galley) = key_galley {
            let chip = Rect::from_min_size(
                pos2(
                    shown.right() - 16.0 - galley.size().x - 14.0,
                    shown.center().y - galley.size().y / 2.0 - 3.0,
                ),
                vec2(galley.size().x + 14.0, galley.size().y + 6.0),
            );
            painter.rect_filled(chip, 6.0, colors.control_background_hover());
            painter.galley(chip.min + vec2(7.0, 3.0), galley, colors.text_secondary());
        }
        if response.clicked() {
            state.push(command);
        }
    }
    let rows_bottom = y + 2.0 * (86.0 + 12.0);
    let look = Look::quiet_link(&colors).icon(15.0, None);
    let content = Content::both(icons::ELLIPSIS, t(s::MainWindow_MoreCaptureOptions));
    let w = button_width(ui, &look, content);
    let bottom = (rect.bottom() - 18.0 - 32.0).max(rows_bottom + 10.0);
    let more = button_at(
        ui,
        &colors,
        &look,
        content,
        true,
        Rect::from_min_size(pos2(left, bottom), vec2(w, 32.0)),
    );
    // `BuildContextMenu(BuildCaptureMenu())`, below the button, left edges aligned.
    let picked = egui::Popup::menu(&more)
        .frame(crate::menus::frame(&colors))
        .align(egui::RectAlign::BOTTOM_START)
        .gap(4.0)
        .show(|ui| {
            let list = crate::capture_flow::capture_menu(state, false);
            crate::menus::entries(ui, &colors, &list)
        })
        .and_then(|r| r.inner);
    if let Some(command) = picked {
        state.push(command);
    }
}

/// The tools tab: one card per tool of `BuildToolsMenu`.
fn tools_tab(ui: &mut Ui, state: &mut AppState) {
    let colors = state.colors.clone();
    let rect = ui.max_rect();
    let painter = ui.painter().clone();
    let hint = layout(
        ui,
        wrapped(
            t(s::MainWindow_ToolsHint),
            11.0,
            Weight::Normal,
            colors.text_secondary(),
            rect.width() - 32.0,
        ),
    );
    let hint_h = hint.size().y;
    let mut faint = painter.clone();
    faint.set_opacity(0.75);
    faint.galley(
        pos2(rect.left() + 16.0, rect.top() + 14.0),
        hint,
        colors.text_secondary(),
    );
    use modlight_core::hotkeys::HotkeyType;
    let jobs = [
        HotkeyType::ColorPicker,
        HotkeyType::ScreenColorPicker,
        HotkeyType::PinToScreen,
        HotkeyType::ImageEditor,
        HotkeyType::ImageEffects,
        HotkeyType::ImageViewer,
        HotkeyType::BackgroundRemover,
        HotkeyType::ImageCombiner,
        HotkeyType::VideoConverter,
        HotkeyType::OCR,
        HotkeyType::QRCode,
        HotkeyType::Metadata,
    ];
    let tools = [
        (icons::PALETTE, s::MainMenuBuilder_ColorPicker),
        (icons::PIPETTE, s::MainMenuBuilder_ScreenColorPicker),
        (icons::PIN, s::MainMenuBuilder_PinToScreenDialog),
        (icons::IMAGE, s::MainMenuBuilder_ImageEditor),
        (icons::WAND_SPARKLES, s::MainMenuBuilder_ImageEffects),
        (icons::EYE, s::MainMenuBuilder_ImageViewer),
        (icons::ERASER, s::MainMenuBuilder_BackgroundRemover),
        (icons::COMBINE, s::MainMenuBuilder_ImageCombiner),
        (icons::FILE_VIDEO, s::MainMenuBuilder_VideoConverter),
        (icons::SCAN_TEXT, s::MainMenuBuilder_OCR),
        (icons::QR_CODE, s::MainMenuBuilder_QRCode),
        (icons::TAGS, s::MainMenuBuilder_Metadata),
    ];
    let top = rect.top() + 14.0 + hint_h + 10.0;
    let available = rect.width() - 16.0 - 8.0;
    let per_row = ((available + 0.01) / (196.0 + 8.0)).floor().max(1.0) as usize;
    for (index, (glyph, label)) in tools.iter().enumerate() {
        let col = index % per_row;
        let row = index / per_row;
        let r = Rect::from_min_size(
            pos2(
                rect.left() + 16.0 + col as f32 * 204.0,
                top + row as f32 * 66.0,
            ),
            vec2(196.0, 58.0),
        );
        let look = Look::button(&colors).radius(8.0);
        let response = ui.interact(r, Id::new(("tool-card", index)), Sense::click());
        crate::widgets::paint_button(
            ui,
            &colors,
            &look,
            Content {
                icon: None,
                label: None,
            },
            true,
            r,
            &response,
        );
        paint_icon(
            ui,
            &painter,
            pos2(r.left() + 12.0 + 12.0, r.center().y),
            *glyph,
            19.0,
            colors.accent_start(),
        );
        let text = layout(
            ui,
            trimmed(
                t(*label),
                13.0,
                Weight::Normal,
                colors.text_secondary(),
                196.0 - 24.0 - 46.0,
                2,
            ),
        );
        painter.galley(
            pos2(
                r.left() + 12.0 + 24.0 + 10.0,
                r.center().y - text.size().y / 2.0,
            ),
            text,
            colors.text_secondary(),
        );
        if response.clicked() {
            state.push(Command::RunJob(jobs[index]));
        }
    }
}

// ---------------------------------------------------------------------- notices and drops

/// The completion notices, bottom right, newest on top (`Border.toast`).
fn toasts(ui: &mut Ui, state: &mut AppState, full: Rect) {
    state.toasts.retain(|toast| toast.expires > state.time);
    if state.toasts.is_empty() {
        return;
    }
    let colors = state.colors.clone();
    let ctx = ui.ctx().clone();
    let next = state
        .toasts
        .iter()
        .map(|t| t.expires)
        .fold(f64::INFINITY, f64::min);
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(
        (next - state.time).max(0.05),
    ));
    let width = 352.0;
    let text_w = width - 26.0 - 2.0 - 18.0 - 11.0 - 11.0;
    // Measure each, then stack them up from the corner.
    let mut heights = Vec::new();
    for toast in &state.toasts {
        let buttons = if toast.link.is_empty() { 1.0 } else { 3.0 };
        let w = text_w - (buttons * 28.0 + (buttons - 1.0) * 2.0);
        let line = |size: f32, weight: Weight| {
            ctx.fonts_mut(|f| f.row_height(&egui::FontId::new(size, weight.family())))
        };
        let detail = layout(
            ui,
            trimmed(
                &toast.detail,
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                w,
                2,
            ),
        );
        let detail_h = if toast.detail.is_empty() {
            line(12.0, Weight::Normal)
        } else {
            detail.size().y
        };
        let h = 11.0
            + line(13.0, Weight::SemiBold)
            + 2.0
            + line(12.0, Weight::Normal)
            + 2.0
            + detail_h
            + 11.0
            + 2.0;
        heights.push(h.max(28.0 + 24.0));
    }
    let mut bottom = full.bottom() - 18.0;
    let mut dismissed = Vec::new();
    let mut copy = Vec::new();
    let mut open = Vec::new();
    let now = state.time;
    for (toast, h) in state.toasts.iter().zip(heights).rev() {
        let rect = Rect::from_min_size(
            pos2(full.right() - 18.0 - width, bottom - h),
            vec2(width, h),
        );
        bottom = rect.top() - 8.0;
        egui::Area::new(Id::new(("toast", toast.id)))
            .order(Order::Foreground)
            .fixed_pos(rect.min)
            .constrain(false)
            .show(&ctx, |ui| {
                let painter = ui.painter().clone();
                painter.add(
                    egui::Shadow {
                        offset: [0, 6],
                        blur: 22,
                        spread: 0,
                        color: Color32::from_black_alpha(0x38),
                    }
                    .as_shape(rect, 10.0),
                );
                painter.rect_filled(rect, 10.0, colors.background_popup());
                painter.rect_stroke(
                    rect,
                    10.0,
                    Stroke::new(1.0, colors.border()),
                    StrokeKind::Inside,
                );
                let _ = ui.interact(rect, Id::new(("toast-surface", toast.id)), Sense::click());
                let inner = rect.shrink2(vec2(13.0 + 1.0, 11.0 + 1.0));
                let icon_color = if toast.failed {
                    colors.cancel_border()
                } else {
                    colors.continue_border()
                };
                let icon = icon_galley(ui, toast.icon, 18.0, icon_color);
                let icon_w = icon.size().x;
                painter.galley(pos2(inner.left(), inner.top() + 1.0), icon, icon_color);
                // Buttons, right, top-aligned.
                let mut bx = inner.right();
                let look = Look::icon_button(&colors, 28.0, 6.0, colors.control_background_hover())
                    .icon(14.0, None);
                let at =
                    |x: f32| Rect::from_min_size(pos2(x - 28.0, inner.top()), vec2(28.0, 28.0));
                if button_at(ui, &colors, &look, Content::icon(icons::X), true, at(bx))
                    .on_hover_text(t(s::MainWindow_ToastDismiss))
                    .clicked()
                {
                    dismissed.push(toast.id);
                }
                bx -= 30.0;
                if !toast.link.is_empty() {
                    if button_at(
                        ui,
                        &colors,
                        &look,
                        Content::icon(icons::EXTERNAL_LINK),
                        true,
                        at(bx),
                    )
                    .on_hover_text(t(s::MySpace_OpenInBrowser))
                    .clicked()
                    {
                        open.push(toast.link.clone());
                    }
                    bx -= 30.0;
                    let copied = toast.copied_until > now;
                    let (glyph, tip) = if copied {
                        (icons::CHECK, t(s::Upload_Copied))
                    } else {
                        (icons::COPY, t(s::Upload_CopyLink))
                    };
                    if button_at(ui, &colors, &look, Content::icon(glyph), true, at(bx))
                        .on_hover_text(tip)
                        .clicked()
                    {
                        copy.push(toast.id);
                    }
                    bx -= 30.0;
                }
                let text_left = inner.left() + icon_w + 11.0;
                let w = (bx + 28.0 - 11.0 - text_left).max(10.0);
                let mut y = inner.top();
                let title = layout(
                    ui,
                    trimmed(&toast.title, 13.0, Weight::SemiBold, colors.text(), w, 1),
                );
                let line = |size: f32, weight: Weight| {
                    ui.fonts_mut(|f| f.row_height(&egui::FontId::new(size, weight.family())))
                };
                painter.galley(pos2(text_left, y), title, colors.text());
                y += line(13.0, Weight::SemiBold) + 2.0;
                let name = layout(
                    ui,
                    trimmed(&toast.name, 12.0, Weight::Normal, colors.text(), w, 1),
                );
                painter.galley(pos2(text_left, y), name, colors.text());
                y += line(12.0, Weight::Normal) + 2.0;
                let detail = layout(
                    ui,
                    trimmed(
                        &toast.detail,
                        12.0,
                        Weight::Normal,
                        colors.text_secondary(),
                        w,
                        2,
                    ),
                );
                painter.galley(pos2(text_left, y), detail, colors.text_secondary());
            });
    }
    for id in dismissed {
        state.toasts.retain(|t| t.id != id);
    }
    for id in copy {
        if let Some(toast) = state.toasts.iter_mut().find(|t| t.id == id) {
            toast.copied_until = now + 2.0;
            let link = toast.link.clone();
            state.copy_text(&link);
        }
    }
    for link in open {
        state.open_url(&link);
    }
}

/// Files from Finder over the window (`DropOverlay`).
fn drop_overlay(ui: &mut Ui, state: &mut AppState, full: Rect) {
    if !state.drop_hover {
        return;
    }
    let colors = state.colors.clone();
    let ctx = ui.ctx().clone();
    egui::Area::new(Id::new("drop-overlay"))
        .order(Order::Foreground)
        .fixed_pos(full.min)
        .interactable(false)
        .show(&ctx, |ui| {
            let painter = ui.painter();
            painter.rect_filled(full, 0.0, colors.overlay_modal());
            let width = 440.0;
            let inner_w = width - 64.0;
            let title = layout(
                ui,
                wrapped(
                    t(s::MainWindow_ReleaseToUploadFiles),
                    22.0,
                    Weight::SemiBold,
                    colors.text(),
                    inner_w,
                ),
            );
            let body = layout(
                ui,
                wrapped(
                    t(s::MainWindow_FilesDroppedUploadDescription),
                    13.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    inner_w,
                ),
            );
            let icon = icon_galley(ui, icons::UPLOAD, 48.0, colors.accent_start());
            let h = 32.0 + icon.size().y + 12.0 + title.size().y + 12.0 + body.size().y + 32.0;
            let card = Rect::from_center_size(full.center(), vec2(width, h));
            painter.rect_filled(card, 12.0, colors.background_panel());
            painter.rect_stroke(
                card,
                12.0,
                Stroke::new(2.0, colors.accent_start()),
                StrokeKind::Inside,
            );
            let mut y = card.top() + 32.0;
            let icon_h = icon.size().y;
            painter.galley(
                pos2(card.center().x - icon.size().x / 2.0, y),
                icon,
                colors.accent_start(),
            );
            y += icon_h + 12.0;
            let title_h = title.size().y;
            painter.galley(
                pos2(card.center().x - title.size().x / 2.0, y),
                title,
                colors.text(),
            );
            y += title_h + 12.0;
            painter.galley(
                pos2(card.center().x - body.size().x / 2.0, y),
                body,
                colors.text_secondary(),
            );
        });
}

/// While a dialog is open the main window takes no input (`ShowDialog(owner)`).
pub fn block_input(ctx: &egui::Context, full: Rect) {
    egui::Area::new(Id::new("modal-block"))
        .order(Order::Foreground)
        .fixed_pos(full.min)
        .show(ctx, |ui| {
            let _ = ui.interact(
                full,
                Id::new("modal-block-surface"),
                Sense::click_and_drag(),
            );
        });
}

/// The colours a notice uses, by outcome, for callers outside this module.
pub fn notice_color(colors: &Colors, failed: bool) -> Color32 {
    if failed {
        colors.cancel_border()
    } else {
        colors.continue_border()
    }
}
