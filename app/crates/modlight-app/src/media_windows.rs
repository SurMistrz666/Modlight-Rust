//! Phase 3 media's hooks into the window: what starts with the app, what runs every
//! frame, and the native windows of the components list and the app update. The main
//! window calls [`on_start`] once and [`viewports`] every frame (real and QA windows
//! only); everything else lives in `components_window`, `download_tab` and
//! `update_window`.

use egui::{Rect, Vec2, ViewportBuilder, ViewportCommand, ViewportId, pos2, vec2};
use modlight_core::l10n::{helpers, s, t};

use crate::state::AppState;

/// Once, when the window opens (after `start`).
pub fn on_start(state: &mut AppState) {
    crate::components_window::on_startup(state);
    crate::update_window::on_startup(state);
}

/// A window centred on the screen (`WindowStartupLocation="CenterScreen"`).
fn builder(title: &str, size: Vec2, resizable: bool) -> ViewportBuilder {
    let mut builder = ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(size)
        .with_resizable(resizable)
        .with_minimize_button(false)
        .with_maximize_button(resizable);
    if let Some((x, y, w, h)) = modlight_macos::system::visible_frame() {
        let area = Rect::from_min_size(pos2(x, y), vec2(w, h));
        builder = builder.with_position(pos2(
            area.center().x - size.x / 2.0,
            (area.center().y - size.y / 2.0).max(area.top()),
        ));
    }
    builder
}

/// Every frame of the real and the QA window.
pub fn viewports(ctx: &egui::Context, state: &mut AppState) {
    crate::download_tab::tick(state);

    if state.components.window.is_some() {
        let builder = builder(
            t(s::Components_Title),
            vec2(
                crate::components_window::WIDTH,
                crate::components_window::HEIGHT,
            ),
            true,
        )
        .with_min_inner_size(vec2(
            crate::components_window::MIN_WIDTH,
            crate::components_window::MIN_HEIGHT,
        ));
        let (asked, closed) = ctx.show_viewport_immediate(
            ViewportId::from_hash_of("components"),
            builder,
            |ui, _| {
                let close = ui.ctx().input(|i| i.viewport().close_requested());
                (crate::components_window::window_ui(ui, state), close)
            },
        );
        if asked || closed {
            crate::components_window::window_closed(state);
        }
    }

    if state.updates.message.is_some() {
        let height = state.updates.message_height;
        let size = vec2(crate::update_window::MESSAGE_WIDTH, height);
        let (answer, closed) = ctx.show_viewport_immediate(
            ViewportId::from_hash_of("update-message"),
            builder(
                t(helpers::UpdateMessageBox_UpdateMessageBox_update_is_available),
                size,
                false,
            ),
            |ui, _| {
                let close = ui.ctx().input(|i| i.viewport().close_requested());
                let current = ui
                    .ctx()
                    .input(|i| i.viewport().inner_rect.map(|r| r.size()));
                let answer = crate::update_window::message_ui(ui, state);
                let wanted = vec2(
                    crate::update_window::MESSAGE_WIDTH,
                    state.updates.message_height,
                );
                if current.is_some_and(|c| (c.y - wanted.y).abs() > 0.5) {
                    ui.ctx()
                        .send_viewport_cmd(ViewportCommand::InnerSize(wanted));
                }
                (answer, close)
            },
        );
        match answer {
            Some(yes) => crate::update_window::message_answered(state, yes),
            // Closing the window is a No.
            None if closed => crate::update_window::message_answered(state, false),
            None => {}
        }
    }

    if state.updates.downloader.is_some() {
        let size = vec2(
            crate::update_window::DOWNLOADER_WIDTH,
            crate::update_window::DOWNLOADER_HEIGHT,
        );
        let (asked, closed) = ctx.show_viewport_immediate(
            ViewportId::from_hash_of("update-downloader"),
            builder(t(helpers::DownloaderWindow_Title), size, false),
            |ui, _| {
                let close = ui.ctx().input(|i| i.viewport().close_requested());
                (crate::update_window::downloader_ui(ui, state), close)
            },
        );
        if asked || closed {
            crate::update_window::downloader_closed(state);
        }
    }
}
