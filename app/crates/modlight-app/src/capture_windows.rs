//! The windows of captures and recording, each its own native window: the region
//! overlay (one per display), the recording frame and toolbar, the after-capture window,
//! the auto-capture window, the quick task menu and the scrolling capture window.

use egui::{Rect, ViewportBuilder, ViewportCommand, ViewportId, pos2, vec2};
use modlight_core::l10n::{capture as c, s, t};

use crate::app::{ModlightApp, Placement, dialog, dialog_resizable};
use crate::dialogs::after_capture::{self, AfterCaptureOutcome};
use crate::dialogs::auto_capture;

impl ModlightApp {
    pub(crate) fn capture_viewports(&mut self, ctx: &egui::Context, main: Option<Rect>) {
        self.overlay_viewports(ctx);
        self.recording_viewports(ctx);
        let state = &mut self.state;
        let colors = state.colors.clone();

        // After capture: one window per picture, oldest first.
        let mut finished = Vec::new();
        for (window, _) in state.captures.after_capture.iter_mut() {
            let key = window.key;
            let (outcome, close) = dialog_resizable(
                ctx,
                &format!("after-capture-{key}"),
                after_capture::AfterCapture::title(),
                vec2(after_capture::WIDTH, after_capture::HEIGHT),
                vec2(after_capture::MIN_WIDTH, after_capture::MIN_HEIGHT),
                Placement::screen(true),
                |ui| window.ui(ui, &colors),
            );
            if let Some(outcome) = outcome {
                finished.push((key, outcome));
            } else if close {
                finished.push((key, AfterCaptureOutcome::Cancel));
            }
        }
        for (key, outcome) in finished {
            state.after_capture_finished(key, outcome);
        }

        // Auto capture.
        if state.captures.auto_capture.window_open && !state.captures.auto_capture.hidden {
            let view = state.auto_capture_view();
            let now = state.time;
            let auto = &state.captures.auto_capture;
            let (action, close) = dialog(
                ctx,
                "auto-capture",
                t(s::AutoCaptureWindow_Title),
                vec2(auto_capture::WIDTH, auto_capture::HEIGHT),
                Placement::screen(false),
                false,
                |ui| auto_capture::ui(ui, &colors, auto, &view, now),
            );
            if let Some(action) = action {
                state.auto_capture_action(action);
            }
            if close {
                // `OnClosed`: closing stops it.
                state.stop_auto_capture();
                state.captures.auto_capture.window_open = false;
            }
        }

        // The quick task menu, at the pointer.
        if let Some((menu, _)) = state.captures.quick_menu.as_mut() {
            let at = *menu.at.get_or_insert_with(|| {
                let (x, y) = state.captures.source.cursor_position();
                pos2(x as f32 - 10.0, y as f32 - 10.0)
            });
            let size = ctx
                .data(|d| d.get_temp::<egui::Vec2>(egui::Id::new("quick-task-menu-size")))
                .unwrap_or(vec2(260.0, 360.0));
            let builder = ViewportBuilder::default()
                .with_title("Modlight")
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(false)
                .with_always_on_top()
                .with_taskbar(false)
                .with_position(at)
                .with_inner_size(size);
            let picked = ctx.show_viewport_immediate(
                ViewportId::from_hash_of("quick-task-menu"),
                builder,
                |ui, _| {
                    let (command, rect) = menu.ui(ui, &colors);
                    let wanted = rect.size() + vec2(2.0, 2.0);
                    if (wanted - size).length() > 0.5 {
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(egui::Id::new("quick-task-menu-size"), wanted)
                        });
                        ui.ctx()
                            .send_viewport_cmd(ViewportCommand::InnerSize(wanted));
                    }
                    let lost = ui.ctx().input(|i| {
                        i.viewport().focused == Some(false) || i.key_pressed(egui::Key::Escape)
                    });
                    (command, lost)
                },
            );
            match picked {
                (Some(command), _) => state.push(command),
                (None, true) => state.quick_menu_finished(None),
                _ => {}
            }
        }

        // Scrolling capture.
        if let Some(window) = state.scrolling.as_mut()
            && !window.minimized
        {
            let (action, close) = dialog_resizable(
                ctx,
                "scrolling-capture",
                crate::scrolling::ScrollingWindow::title(),
                vec2(crate::scrolling::WIDTH, crate::scrolling::HEIGHT),
                vec2(crate::scrolling::MIN_WIDTH, crate::scrolling::MIN_HEIGHT),
                Placement::screen(false),
                |ui| window.ui(ui, &colors),
            );
            if let Some(action) = action {
                crate::scrolling::action(state, action);
            }
            if close {
                if let Some(window) = state.scrolling.as_ref()
                    && window.capturing
                {
                    window.stop.store(true, std::sync::atomic::Ordering::SeqCst);
                } else {
                    state.scrolling = None;
                }
            }
        }
        // The frame around the area being scrolled (`ScrollingCaptureRegionWindow`).
        if let Some(region) = state.scrolling.as_ref().and_then(|w| w.region) {
            let frame = Rect::from_min_size(
                pos2(region.x as f32 - 1.0, region.y as f32 - 1.0),
                vec2(region.width as f32 + 2.0, region.height as f32 + 2.0),
            );
            let builder = ViewportBuilder::default()
                .with_title("Modlight - scrolling region")
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(false)
                .with_always_on_top()
                .with_mouse_passthrough(true)
                .with_taskbar(false)
                .with_position(frame.min)
                .with_inner_size(frame.size());
            ctx.show_viewport_immediate(
                ViewportId::from_hash_of("scrolling-region"),
                builder,
                |ui, _| {
                    let r = ui.max_rect().shrink(0.5);
                    ui.painter().rect_stroke(
                        r,
                        0.0,
                        egui::Stroke::new(1.0, egui::Color32::RED),
                        egui::StrokeKind::Middle,
                    );
                },
            );
        }

        // The quick task menu's editor.
        if let Some(editor) = state.captures.quick_task_editor.as_mut() {
            use crate::dialogs::quick_task_editor as e;
            let (outcome, close) = dialog_resizable(
                ctx,
                "quick-task-editor",
                e::QuickTaskEditor::title(),
                vec2(e::WIDTH, e::HEIGHT),
                vec2(e::MIN_WIDTH, e::MIN_HEIGHT),
                Placement::screen(false),
                |ui| editor.ui(ui, &colors),
            );
            state.save_quick_task_presets();
            if outcome.is_some() || close {
                state.captures.quick_task_editor = None;
            }
        }
        // One hotkey's settings: the settings shell drawn with that window in the
        // settings slot for the length of the frame.
        if state.windows.hotkey_task.is_some() {
            let hotkey = state.windows.hotkey_task.take();
            let main_settings = std::mem::replace(&mut state.windows.settings, hotkey);
            let title = state
                .windows
                .settings
                .as_ref()
                .map(|w| w.title.clone())
                .unwrap_or_default();
            let (asks, close) = dialog_resizable(
                ctx,
                "hotkey-task-settings",
                &title,
                vec2(crate::settings::WIDTH, crate::settings::HEIGHT),
                vec2(crate::settings::MIN_WIDTH, crate::settings::MIN_HEIGHT),
                Placement::owner(main),
                |ui| crate::settings::ui(ui, state),
            );
            let mut closing = close;
            let mut rest = Vec::new();
            for ask in asks {
                match ask {
                    crate::settings::SettingsAsk::Close => closing = true,
                    other => rest.push(other),
                }
            }
            crate::settings::handle(state, rest);
            let hotkey = std::mem::replace(&mut state.windows.settings, main_settings);
            state.windows.hotkey_task = hotkey;
            if closing {
                state.close_hotkey_task_settings();
            }
        }

        // First-time configuration (CenterScreen, fixed size).
        if let Some(window) = state.windows.first_run.as_mut() {
            use crate::first_run as f;
            let (asks, close) = dialog_resizable(
                ctx,
                "first-run",
                "Modlight",
                vec2(f::FIRST_RUN_SIZE.0, f::FIRST_RUN_SIZE.1),
                vec2(f::FIRST_RUN_SIZE.0, f::FIRST_RUN_SIZE.1),
                Placement::screen(true),
                |ui| f::first_run_ui(ui, window, &colors),
            );
            let mut finished = close;
            for ask in asks {
                match ask {
                    f::FirstRunAsk::Ok => finished = true,
                    f::FirstRunAsk::StartAtLogin(on) => {
                        if !state.env.qa
                            && let Some(window) = state.windows.first_run.as_mut()
                        {
                            match crate::platform::integration::set_startup(on) {
                                Ok(startup) => {
                                    window.startup = startup;
                                    window.status.clear();
                                }
                                Err(error) => {
                                    window.status = error;
                                    window.startup = crate::platform::integration::startup_state();
                                }
                            }
                        }
                    }
                    // `CreateShellContextMenuButton` / `CreateSendToMenuButton`, then the
                    // boxes show what Explorer has now.
                    #[cfg(windows)]
                    f::FirstRunAsk::ContextMenu(on) | f::FirstRunAsk::SendTo(on) => {
                        use crate::platform::integration as i;
                        if !state.env.qa
                            && let Some(window) = state.windows.first_run.as_mut()
                        {
                            let result = if matches!(ask, f::FirstRunAsk::SendTo(_)) {
                                i::set_send_to(on)
                            } else {
                                i::set_context_menu(on)
                            };
                            match result {
                                Ok(()) => window.status.clear(),
                                Err(error) => window.status = error,
                            }
                            window.explorer = crate::app_pages::ExplorerMenus {
                                context_menu: i::context_menu_enabled(),
                                send_to: i::send_to_enabled(),
                            };
                        }
                    }
                }
            }
            if finished {
                state.windows.first_run = None;
            }
        }

        // The image editor selector (CenterOwner, height of its content).
        if let Some(window) = state.windows.editor_selector.as_mut() {
            use crate::first_run as f;
            let height = window.content_height.max(220.0);
            let (choice, close) = dialog(
                ctx,
                "editor-selector",
                t(s::ImageEditorSelectorWindow_Title),
                vec2(f::SELECTOR_WIDTH, height),
                Placement::owner(main),
                true,
                |ui| f::selector_ui(ui, window, &colors),
            );
            if let Some(legacy) = choice {
                state.editor_selector_finished(Some(legacy));
            } else if close {
                state.editor_selector_finished(None);
            }
        }

        // About (CenterScreen), saying what the last update check found.
        if state.windows.about.is_some() {
            let status = crate::about_window::status_of(state);
            if let Some(window) = state.windows.about.as_mut()
                && window.update != status
                && status != crate::about_window::AboutUpdate::Hidden
            {
                window.update = status;
            }
        }
        if let Some(window) = state.windows.about.as_mut() {
            use crate::about_window as a;
            let (asks, close) = dialog_resizable(
                ctx,
                "about",
                t(s::AboutWindow_Title),
                vec2(a::ABOUT_WIDTH, a::ABOUT_HEIGHT),
                vec2(a::ABOUT_MIN.0, a::ABOUT_MIN.1),
                Placement::screen(true),
                |ui| a::about_ui(ui, window, &colors),
            );
            for ask in asks {
                match ask {
                    a::AboutAsk::Open(url) => state.open_url(&url),
                    a::AboutAsk::Update(manifest) => crate::update_window::offer(
                        state,
                        modlight_core::update::UpdateOutcome::UpdateAvailable(*manifest),
                    ),
                    a::AboutAsk::CheckNow => crate::update_window::check_quietly(state),
                    a::AboutAsk::DebugLog => state.show_debug_log(),
                }
            }
            if close {
                state.windows.about = None;
            }
        }

        // Debug log (CenterScreen).
        if let Some(window) = state.windows.debug_log.as_mut() {
            use crate::about_window as a;
            let (asks, close) = dialog_resizable(
                ctx,
                "debug-log",
                t(s::DebugLogWindow_Title),
                vec2(a::LOG_WIDTH, a::LOG_HEIGHT),
                vec2(a::LOG_MIN.0, a::LOG_MIN.1),
                Placement::screen(true),
                |ui| a::debug_log_ui(ui, window, &colors),
            );
            for ask in asks {
                match ask {
                    a::LogAsk::CopyAll(text) => state.copy_text(&text),
                    a::LogAsk::OpenLogFile => {
                        if let Some(path) = modlight_core::debug_log::file_path() {
                            state.effects.push(crate::state::Effect::OpenFile(path));
                        }
                    }
                    a::LogAsk::Upload(text) => {
                        // `UploadManager.UploadText`.
                        let settings = state.default_task_settings();
                        let task_ctx = state.uploads.ctx.clone();
                        modlight_core::tasks::upload_manager::upload_text(
                            &mut state.uploads.tasks,
                            &text,
                            &settings,
                            false,
                            &task_ctx,
                        );
                    }
                    a::LogAsk::OpenUrl(url) => state.open_url(&url),
                    a::LogAsk::OpenStartupFolder => {
                        if let Some(folder) = std::env::current_exe()
                            .ok()
                            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                        {
                            state.effects.push(crate::state::Effect::OpenFile(folder));
                        }
                    }
                }
            }
            if close {
                state.windows.debug_log = None;
            }
        }

        // The screen recording options (`FFmpegOptionsWindow`, CenterOwner).
        if let Some(window) = state.captures.ffmpeg_window.as_mut() {
            use crate::ffmpeg_window as f;
            let (asks, close) = dialog_resizable(
                ctx,
                "ffmpeg-options",
                t(c::FFmpegOptionsWindow_Title),
                vec2(f::WIDTH, f::HEIGHT),
                vec2(f::MIN_WIDTH, f::MIN_HEIGHT),
                Placement::owner(main),
                |ui| f::ui(ui, window, &colors),
            );
            for ask in asks {
                match ask {
                    f::FFmpegAsk::Refresh => state.refresh_ffmpeg_devices(),
                    f::FFmpegAsk::Browse => state.browse_for_ffmpeg(),
                }
            }
            if close {
                state.ffmpeg_options_closed();
            }
        }
    }

    /// The region capture overlay: one borderless window per display it covers.
    fn overlay_viewports(&mut self, ctx: &egui::Context) {
        let state = &mut self.state;
        let Some(overlay) = state.captures.overlay.as_mut() else {
            return;
        };
        let colors = state.colors.clone();
        let now = state.time;
        // The frozen desktop goes up with the main window's frame, before any display's
        // window draws (a texture first made inside one of them left it black).
        overlay.prepare_textures(ctx);
        let screens = overlay.screens();
        let pointer = overlay.session.to_screen(overlay.session.last_point);
        let mut region_tool = false;
        for screen in &screens {
            let title = format!(
                "Modlight - {} {}",
                t(c::BaseRegionForm_InitializeComponent_Region_capture),
                screen.id
            );
            let builder = ViewportBuilder::default()
                .with_title(title.clone())
                .with_decorations(false)
                .with_resizable(false)
                .with_always_on_top()
                .with_taskbar(false)
                .with_transparent(
                    overlay.session.request.kind
                        == modlight_core::capture::region::RegionKind::Transparent,
                )
                .with_position(pos2(screen.bounds.x as f32, screen.bounds.y as f32))
                .with_inner_size(vec2(
                    screen.bounds.width as f32,
                    screen.bounds.height as f32,
                ));
            let id = ViewportId::from_hash_of(("region-overlay", screen.id));
            // Per overlay: each capture opens new windows, raised and shown anew.
            let run = overlay.opened_at.to_bits();
            let raised_key = egui::Id::new(("region-overlay-raised", screen.id, run));
            let frames_key = egui::Id::new(("region-overlay-frames", screen.id, run));
            // The window stays hidden for its first frames, while it is raised over the
            // menu bar and the frozen desktop reaches it: shown at once it flashed dark.
            let frames = ctx.data(|d| d.get_temp::<u32>(frames_key).unwrap_or(0));
            let shown = frames >= 2;
            let builder = builder.with_visible(shown);
            ctx.show_viewport_immediate(id, builder, |ui, _| {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(frames_key, frames.saturating_add(1)));
                if !ui
                    .ctx()
                    .data(|d| d.get_temp::<bool>(raised_key).unwrap_or(false))
                    && crate::platform::windows::raise_overlay(&title)
                {
                    ui.ctx().data_mut(|d| d.insert_temp(raised_key, true));
                }
                if frames == 2 {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Focus);
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                overlay.input(ui, screen, now);
                overlay.ui(ui, &colors, screen);
                let here = screen.bounds.contains(pointer.0, pointer.1);
                if overlay.toolbar(ui, &colors, screen, here) {
                    region_tool = true;
                }
                if ui.ctx().input(|i| i.viewport().close_requested()) {
                    overlay.session.cancel();
                }
            });
        }
        if region_tool {
            overlay.session.region_tool_active = true;
        }
        ctx.request_repaint();
    }

    /// The recording's frame (the pointer goes through it) and its toolbar.
    fn recording_viewports(&mut self, ctx: &egui::Context) {
        let state = &mut self.state;
        let Some(recording) = state.captures.recording.as_mut() else {
            return;
        };
        let colors = state.colors.clone();
        let snapshot = recording.control.snapshot();
        // `Encoding`: the window hides while the second pass runs.
        if snapshot.state == modlight_core::recording::RecordState::Encoding || snapshot.finished {
            return;
        }
        let frame = recording.frame_rect();
        let builder = ViewportBuilder::default()
            .with_title(t(c::ScreenRecordWindow_Title))
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_always_on_top()
            .with_mouse_passthrough(true)
            .with_taskbar(false)
            .with_position(frame.min)
            .with_inner_size(frame.size());
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of("recording-frame"),
            builder,
            |ui, _| {
                ui.ctx()
                    .send_viewport_cmd(ViewportCommand::OuterPosition(frame.min));
                ui.ctx()
                    .send_viewport_cmd(ViewportCommand::InnerSize(frame.size()));
                recording.frame_ui(ui);
            },
        );
        let toolbar = recording.toolbar_rect();
        let builder = ViewportBuilder::default()
            .with_title(t(c::ScreenRecordWindow_Title))
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_active(recording.activate)
            .with_position(toolbar.min)
            .with_inner_size(toolbar.size());
        let action = ctx.show_viewport_immediate(
            ViewportId::from_hash_of("recording-toolbar"),
            builder,
            |ui, _| {
                ui.ctx()
                    .send_viewport_cmd(ViewportCommand::OuterPosition(toolbar.min));
                ui.ctx()
                    .send_viewport_cmd(ViewportCommand::InnerSize(toolbar.size()));
                recording.toolbar_ui(ui, &colors)
            },
        );
        if let Some(action) = action {
            state.recording_action(action);
        }
    }
}
