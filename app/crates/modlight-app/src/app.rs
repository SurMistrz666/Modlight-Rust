//! The eframe application: one frame of the main window, the secondary windows as
//! immediate viewports, background completions, and the platform effects.

use egui::{Rect, Ui, Vec2, ViewportBuilder, ViewportCommand, ViewportId, pos2, vec2};
use modlight_core::l10n::{s, t};

use crate::dialogs::{confirm, duplicates, folder_delete, link_settings, prompt};
use crate::notification::NotificationOutcome;
use crate::state::{AppState, Effect};

/// The main window's size (`MainWindow.axaml`).
pub const MAIN_SIZE: Vec2 = Vec2::new(1040.0, 720.0);
pub const MAIN_MIN_SIZE: Vec2 = Vec2::new(880.0, 540.0);

/// How the app was started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The real app: tray, Keychain, windows.
    Normal,
    /// `--qa-profile` with a window a person drives: no tray, no Keychain.
    Qa,
    /// Automated end-to-end runs: talks to a server like `Qa`, but opens no window,
    /// touches no clipboard and opens no browser; tests drive the dialogs' state.
    Headless,
    /// A headless render: nothing is started, nothing talks to a server.
    Shot,
}

pub struct ModlightApp {
    pub state: AppState,
    pub mode: Mode,
    logo: Option<egui::TextureHandle>,
    tray: Option<crate::tray::Tray>,
    clipboard: Option<arboard::Clipboard>,
    allow_close: bool,
    focused: bool,
    system_dark: Option<bool>,
    /// The last thing copied, for headless checks.
    pub copied: Vec<String>,
    tray_checked_at: f64,
    /// The window title last given to the system.
    window_title: String,
}

/// The app icon (`ShareX/Modlight_Icon.ico`), decoded at its largest.
pub fn app_icon() -> Option<image::RgbaImage> {
    let bytes = include_bytes!("../../../assets/upstream/icons/Modlight_Icon.ico");
    image::load_from_memory_with_format(bytes, image::ImageFormat::Ico)
        .ok()
        .map(|i| i.to_rgba8())
}

impl ModlightApp {
    pub fn new(cc: &eframe::CreationContext<'_>, mut state: AppState, mode: Mode) -> ModlightApp {
        let ctx = &cc.egui_ctx;
        state.mode = mode;
        ctx.set_fonts(crate::theme::fonts());
        state.rt.attach(ctx);
        crate::theme::apply_style(ctx, &state.colors);
        let logo = app_icon().map(|icon| {
            let small =
                image::imageops::resize(&icon, 34, 34, image::imageops::FilterType::Lanczos3);
            let image = egui::ColorImage::from_rgba_unmultiplied([34, 34], small.as_raw());
            ctx.load_texture("title-logo", image, egui::TextureOptions::LINEAR)
        });
        let mut app = ModlightApp {
            state,
            mode,
            logo,
            tray: None,
            clipboard: None,
            allow_close: false,
            focused: true,
            system_dark: None,
            copied: Vec::new(),
            tray_checked_at: f64::NEG_INFINITY,
            window_title: modlight_core::title(),
        };
        if mode != Mode::Shot {
            app.start(ctx);
        }
        if matches!(mode, Mode::Normal | Mode::Qa) {
            modlight_macos::keys::install();
            modlight_core::tasks::manager::set_fullscreen_probe(
                modlight_macos::system::active_window_fullscreen,
            );
        }
        if mode == Mode::Normal {
            modlight_macos::integration::install_services_provider();
        }
        if mode == Mode::Normal && app.state.config.show_tray() {
            app.tray = crate::tray::Tray::new(ctx, &app.state);
        }
        app
    }

    /// What the window does once when it opens: the account, the library, the skin.
    fn start(&mut self, ctx: &egui::Context) {
        let state = &mut self.state;
        state.schedule.start(std::time::Instant::now());
        state.refresh_account_strip();
        state.request_avatar(ctx);
        state.space_refresh_account();
        state.sync_skin();
        state.start_uploads();
        state.start_hotkeys();
    }

    /// Everything that is not drawing, before the frame.
    fn before(&mut self, ctx: &egui::Context) {
        let state = &mut self.state;
        state.time = ctx.input(|i| i.time);
        for completion in state.rt.drain() {
            completion(state, ctx);
        }
        if self.mode != Mode::Shot {
            state.pump_uploads(ctx);
            state.pump_hotkeys();
            state.tick_captures(ctx);
        }
        // The system appearance, when following it.
        let system = ctx
            .input(|i| i.raw.system_theme)
            .map(|theme| theme == egui::Theme::Dark);
        if system.is_some() && system != self.system_dark {
            self.system_dark = system;
            state.env.system_dark = system;
            state.refresh_theme(Some(ctx));
        }
        // Coming back to the window refreshes the storage reading (at most every 20 s).
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if focused && !self.focused && self.mode != Mode::Shot {
            state.refresh_account_strip();
        }
        self.focused = focused;
        // Files from Finder: the overlay while they hover, an upload into the open
        // folder when they land.
        let (hovering, dropped) =
            ctx.input(|i| (!i.raw.hovered_files.is_empty(), i.raw.dropped_files.clone()));
        state.drop_hover = hovering && state.account.is_signed_in();
        let paths: Vec<std::path::PathBuf> = dropped
            .iter()
            .map(|f| f.path().to_path_buf())
            .filter(|p| !p.as_os_str().is_empty())
            .collect();
        if !paths.is_empty() && self.mode != Mode::Shot {
            // `DragDropUpload`: the default settings; the open folder is the ambient
            // target the task picks up.
            state.drop_on_window(paths);
        }
        let modal = state.windows.modal_open() || state.uploads.modal_open();
        if state.tab == crate::state::Tab::MySpace && !modal {
            state.my_space_keys(ctx);
        }
        // Cmd+V outside a text field uploads the clipboard (`ClipboardUploadMainWindow`);
        // upstream's window keys answer on My space only. The flag is read every frame
        // so a press on another tab does not fire later.
        let pasted = if modlight_macos::keys::monitoring() {
            modlight_macos::keys::take_command_v()
        } else {
            ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))))
        };
        let main_focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if pasted
            && main_focused
            && state.tab == crate::state::Tab::MySpace
            && !modal
            && !ctx.egui_wants_keyboard_input()
            && self.mode != Mode::Shot
        {
            state.push(crate::commands::Command::ClipboardUpload);
        }
        state.details_tick(ctx);
        state.tick_motion(ctx);
        if self.mode != Mode::Shot {
            state.tick_settings_sync(ctx);
        }
        if let Some(tray) = &mut self.tray {
            // The menu is rebuilt from the settings at most twice a second.
            if self.state.time - self.tray_checked_at >= 0.5 {
                self.tray_checked_at = self.state.time;
                tray.update(&self.state);
            }
            // `UpdateTrayIcon` with the average progress, when the setting asks for it.
            let enabled = self.state.app_upload_settings().tray_icon_progress_enabled;
            let progress = match self.state.uploads.tasks.average_progress() {
                Some(progress) if enabled => progress as i32,
                _ => -1,
            };
            tray.show_progress(progress);
            for command in tray.commands() {
                self.state.push(command);
            }
        }
    }

    /// The main window's close button: hide to the tray, or quit (`OnClosing`).
    fn handle_close(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.allow_close {
            return;
        }
        if self.tray.is_some() && self.state.config.show_tray() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            if self.state.config.first_time_minimize_to_tray() {
                self.state.config.set_first_time_minimize_to_tray(false);
                self.state.save_config();
                self.state.show_notification_tip(
                    t(s::ShareXIsMinimizedToTheSystemTray),
                    "Modlight",
                    Some(8000.0),
                );
            }
            return;
        }
        self.state.save_config();
        self.allow_close = true;
    }

    /// After drawing: commands, then the platform effects.
    fn after(&mut self, ctx: &egui::Context) {
        let pending = std::mem::take(&mut self.state.pending);
        for command in pending {
            self.state.run(command, ctx);
        }
        for effect in std::mem::take(&mut self.state.effects) {
            self.perform(effect);
        }
        if self.state.quit_requested {
            self.state.quit_requested = false;
            self.state.save_config();
            self.allow_close = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    /// One platform effect. Only the real app opens, reveals or plays anything; the QA
    /// window also writes the clipboard (writing asks no permission), headless runs
    /// only record what they would have done.
    fn perform(&mut self, effect: Effect) {
        let real = self.mode == Mode::Normal;
        let writes_clipboard = matches!(self.mode, Mode::Normal | Mode::Qa);
        match effect {
            Effect::OpenUrl(url) => {
                if real {
                    modlight_macos::system::open_url(&url);
                } else {
                    log::info!("QA mode: not opening {url}");
                }
            }
            Effect::CopyText(text) => {
                self.copied.push(text.clone());
                if writes_clipboard
                    && let Some(clipboard) = self.clipboard()
                    && let Err(error) = clipboard.set_text(text)
                {
                    log::warn!("Modlight: the clipboard refused the text: {error}");
                }
            }
            Effect::CopyImage(image) => {
                self.copied
                    .push(format!("<image {}x{}>", image.width(), image.height()));
                if writes_clipboard && let Some(clipboard) = self.clipboard() {
                    let data = arboard::ImageData {
                        width: image.width() as usize,
                        height: image.height() as usize,
                        bytes: std::borrow::Cow::Borrowed(image.as_raw()),
                    };
                    if let Err(error) = clipboard.set_image(data) {
                        log::warn!("Modlight: the clipboard refused the image: {error}");
                    }
                }
            }
            Effect::CopyFiles(files) => {
                self.copied
                    .extend(files.iter().map(|f| f.display().to_string()));
                if writes_clipboard
                    && let Some(clipboard) = self.clipboard()
                    && let Err(error) = clipboard.set().file_list(&files)
                {
                    log::warn!("Modlight: the clipboard refused the files: {error}");
                }
            }
            Effect::ClearClipboard => {
                if writes_clipboard && let Some(clipboard) = self.clipboard() {
                    let _ = clipboard.clear();
                }
            }
            Effect::Reveal(path) => {
                if real {
                    modlight_macos::system::reveal_in_finder(&path);
                } else {
                    log::info!("QA mode: not revealing {}", path.display());
                }
            }
            Effect::OpenFile(path) => {
                if real {
                    modlight_macos::system::open_path(&path);
                } else {
                    log::info!("QA mode: not opening {}", path.display());
                }
            }
            Effect::PlaySound(sound, custom) => {
                if real {
                    AppState::play_sound(sound, custom);
                }
            }
            Effect::Quit => self.state.quit_requested = true,
        }
    }

    fn clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(clipboard) => self.clipboard = Some(clipboard),
                Err(error) => log::warn!("Modlight: the clipboard cannot be opened: {error}"),
            }
        }
        self.clipboard.as_mut()
    }

    /// The secondary windows, each its own native window.
    fn viewports(&mut self, ctx: &egui::Context) {
        let main = ctx.input(|i| i.viewport().outer_rect);
        let state = &mut self.state;
        let colors = state.colors.clone();

        if state.windows.sign_in.is_some() {
            let height = state
                .windows
                .sign_in
                .as_ref()
                .map(|w| w.content_height)
                .unwrap_or(300.0);
            let (action, close) = dialog(
                ctx,
                "sign-in",
                t(s::SignInWindow_Title),
                vec2(crate::signin::WIDTH, height),
                Placement::owner(main),
                false,
                |ui| {
                    state.sign_in_opened();
                    crate::signin::ui(ui, state)
                },
            );
            if let Some(action) = action {
                state.sign_in_action(action);
            }
            if close {
                state.sign_in_action(crate::signin::SignInAction::Close);
            }
        }

        if state.windows.settings.is_some() {
            let size = vec2(crate::settings::WIDTH, crate::settings::HEIGHT);
            let builder_min = vec2(crate::settings::MIN_WIDTH, crate::settings::MIN_HEIGHT);
            let (asks, close) = dialog_resizable(
                ctx,
                "settings",
                t(s::SettingsWindow_WindowTitle),
                size,
                builder_min,
                Placement::owner(main),
                |ui| crate::settings::ui(ui, state),
            );
            crate::settings::handle(state, asks);
            if close {
                state.close_settings();
            }
        }

        if let Some(window) = state.windows.prompt.as_mut() {
            let title = window.heading.clone();
            let (outcome, close) = dialog(
                ctx,
                "prompt",
                &title,
                vec2(prompt::WIDTH, window.content_height),
                Placement::owner(main),
                true,
                |ui| window.ui(ui, &colors),
            );
            if let Some(outcome) = outcome {
                state.prompt_finished(outcome);
            } else if close {
                state.prompt_finished(prompt::PromptOutcome::Cancelled);
            }
        }

        if let Some(window) = state.windows.folder_delete.as_mut() {
            let now = state.time;
            let (choice, close) = dialog(
                ctx,
                "folder-delete",
                t(s::MySpace_DeleteFolderTitle),
                vec2(folder_delete::WIDTH, window.content_height),
                Placement::owner(main),
                true,
                |ui| window.ui(ui, &colors, now),
            );
            if let Some(choice) = choice {
                state.folder_delete_finished(choice);
            } else if close {
                state.folder_delete_finished(folder_delete::FolderDeleteChoice::Cancel);
            }
        }

        if let Some(window) = state.windows.link_settings.as_mut() {
            let now = state.env.now();
            let (outcome, close) = dialog(
                ctx,
                "link-settings",
                t(s::MySpace_LinkSettings),
                vec2(link_settings::WIDTH, window.content_height),
                Placement::owner(main),
                true,
                |ui| window.ui(ui, &colors, now),
            );
            if let Some(outcome) = outcome {
                state.link_settings_finished(outcome);
            } else if close {
                state.link_settings_finished(link_settings::LinkSettingsOutcome::Cancel);
            }
        }

        if let Some(window) = state.windows.duplicates.as_mut() {
            let now = state.env.now();
            let (action, close) = dialog_resizable(
                ctx,
                "duplicates",
                t(s::MySpace_DuplicatesTitle),
                vec2(duplicates::WIDTH, duplicates::HEIGHT),
                vec2(560.0, 420.0),
                Placement::owner(main),
                |ui| window.ui(ui, &colors, now),
            );
            if let Some(action) = action {
                state.duplicates_action(action);
            } else if close {
                state.duplicates_action(duplicates::DuplicatesAction::Close);
            }
        }

        if let Some(window) = state.windows.confirm.as_mut() {
            let title = window.title.clone();
            let (outcome, close) = dialog(
                ctx,
                "confirm",
                &title,
                vec2(confirm::WIDTH, window.content_height),
                Placement::owner(main),
                true,
                |ui| window.ui(ui, &colors),
            );
            if let Some(outcome) = outcome {
                state.confirm_finished(outcome);
            } else if close {
                state.confirm_finished(confirm::ConfirmOutcome::No);
            }
        }

        self.upload_viewports(ctx, main);
        self.capture_viewports(ctx, main);

        if self.state.windows.notification.is_some() {
            self.notification(ctx);
        }
        if self.state.uploads.windows.drop_window.is_some() {
            self.drop_window(ctx);
        }
    }

    /// The upload flow's windows (`TaskHelpers`, `UploadManager`, the task questions).
    fn upload_viewports(&mut self, ctx: &egui::Context, main: Option<Rect>) {
        use crate::dialogs::{
            after_upload, before_upload, clipboard_upload, file_exists, message, paste,
            shorten_url, text_upload, upload_confirm, url_upload,
        };
        let state = &mut self.state;
        let colors = state.colors.clone();

        if let Some(window) = state.uploads.windows.messages.front_mut() {
            let title = window.title.clone();
            let (closed, close) = dialog(
                ctx,
                "message",
                &title,
                vec2(message::WIDTH, window.content_height),
                Placement::screen(false),
                true,
                |ui| window.ui(ui, &colors).then_some(()),
            );
            if closed.is_some() || close {
                state.uploads.windows.messages.pop_front();
            }
        }

        if let Some((window, _)) = state.uploads.windows.multi_upload.as_mut() {
            let title = window.title();
            let (outcome, close) = dialog(
                ctx,
                "multi-upload",
                title,
                vec2(upload_confirm::WIDTH, window.content_height),
                Placement::screen(true),
                true,
                |ui| window.ui(ui, &colors),
            );
            if let Some(outcome) = outcome {
                state.multi_upload_finished(outcome);
            } else if close {
                state.multi_upload_finished(upload_confirm::UploadConfirmOutcome {
                    go_on: false,
                    dont_show_again: false,
                });
            }
        }

        if let Some(window) = state.uploads.windows.large_file.as_mut() {
            let title = window.title();
            let (outcome, close) = dialog(
                ctx,
                "large-file",
                title,
                vec2(upload_confirm::WIDTH, window.content_height),
                Placement::screen(true),
                true,
                |ui| window.ui(ui, &colors),
            );
            if let Some(outcome) = outcome {
                state.large_file_finished(outcome);
            } else if close {
                state.large_file_finished(upload_confirm::UploadConfirmOutcome {
                    go_on: false,
                    dont_show_again: false,
                });
            }
        }

        if let Some(window) = state.uploads.windows.file_exists.as_mut() {
            let (outcome, close) = dialog(
                ctx,
                "file-exists",
                t(s::FileExistWindow_Title),
                vec2(file_exists::WIDTH, window.content_height),
                Placement::screen(true),
                true,
                |ui| window.ui(ui, &colors),
            );
            if let Some(path) = outcome {
                state.file_exists_finished(path);
            } else if close {
                let path = window.closed();
                state.file_exists_finished(path);
            }
        }

        if let Some(window) = state.uploads.windows.before_upload.as_mut() {
            let (outcome, close) = dialog_resizable(
                ctx,
                "before-upload",
                t(s::BeforeUploadWindow_Title),
                vec2(before_upload::WIDTH, before_upload::HEIGHT),
                vec2(before_upload::MIN_WIDTH, before_upload::MIN_HEIGHT),
                Placement::screen(true),
                |ui| window.ui(ui, &colors),
            );
            if let Some(outcome) = outcome {
                state.before_upload_finished(outcome);
            } else if close {
                state.before_upload_finished(before_upload::BeforeUploadOutcome::Cancel);
            }
        }

        let now = state.time;
        let mut actions = Vec::new();
        let mut previews = Vec::new();
        for window in state.uploads.windows.after_uploads.iter_mut() {
            if let Some(url) = window.wants_remote_preview() {
                previews.push((window.key, url));
            }
            let title = window.title();
            let key = window.key;
            let (action, close) = dialog_resizable(
                ctx,
                &format!("after-upload-{key}"),
                &title,
                vec2(after_upload::WIDTH, after_upload::HEIGHT),
                vec2(after_upload::MIN_WIDTH, after_upload::MIN_HEIGHT),
                Placement::screen(false),
                |ui| window.ui(ui, &colors, now),
            );
            if let Some(action) = action {
                actions.push((key, action));
            } else if close {
                actions.push((key, after_upload::AfterUploadAction::Close));
            }
        }
        for (key, url) in previews {
            state.load_after_upload_preview(key, url);
        }
        for (key, action) in actions {
            state.after_upload_action(key, action);
        }

        if let Some(window) = state.uploads.windows.paste.as_mut() {
            let (outcome, close) = dialog_resizable(
                ctx,
                "paste",
                t(s::Paste_WindowTitle),
                vec2(paste::WIDTH, paste::HEIGHT),
                vec2(paste::MIN_WIDTH, paste::MIN_HEIGHT),
                Placement::owner(main),
                |ui| window.ui(ui, &colors),
            );
            if let Some(outcome) = outcome {
                state.paste_finished(outcome);
            } else if close {
                state.paste_finished(paste::PasteOutcome::Cancel);
            }
        }

        if let Some(window) = state.uploads.windows.text_upload.as_mut() {
            let (outcome, close) = dialog_resizable(
                ctx,
                "text-upload",
                text_upload::TextUpload::title(),
                vec2(text_upload::WIDTH, text_upload::HEIGHT),
                vec2(text_upload::MIN_WIDTH, text_upload::MIN_HEIGHT),
                Placement::screen(false),
                |ui| window.ui(ui, &colors),
            );
            if let Some(text) = outcome {
                state.text_upload_finished(text);
            } else if close {
                state.text_upload_finished(None);
            }
        }

        if let Some(window) = state.uploads.windows.shorten_url.as_mut() {
            let (outcome, close) = dialog(
                ctx,
                "shorten-url",
                t(s::ShortenURLWindow_Title),
                vec2(shorten_url::WIDTH, window.content_height),
                Placement::screen(false),
                false,
                |ui| window.ui(ui, &colors),
            );
            if let Some(request) = outcome {
                state.shorten_url_finished(request);
            } else if close {
                state.shorten_url_finished(None);
            }
        }

        if let Some(window) = state.uploads.windows.url_upload.as_mut() {
            let (outcome, close) = dialog(
                ctx,
                "url-upload",
                t(s::URLUploadWindow_Title),
                vec2(url_upload::WIDTH, window.content_height),
                Placement::screen(false),
                false,
                |ui| window.ui(ui, &colors),
            );
            if let Some(url) = outcome {
                state.url_upload_finished(url);
            } else if close {
                state.url_upload_finished(None);
            }
        }

        if let Some((window, _)) = state.uploads.windows.clipboard_upload.as_mut() {
            let (outcome, close) = dialog_resizable(
                ctx,
                "clipboard-upload",
                &clipboard_upload::ClipboardUpload::title(),
                vec2(clipboard_upload::WIDTH, clipboard_upload::HEIGHT),
                vec2(clipboard_upload::MIN_WIDTH, clipboard_upload::MIN_HEIGHT),
                Placement::screen(false),
                |ui| window.ui(ui, &colors),
            );
            if let Some(outcome) = outcome {
                state.clipboard_viewer_finished(outcome);
            } else if close {
                state.clipboard_viewer_finished(clipboard_upload::ClipboardOutcome::Cancel);
            }
        }
    }

    /// The drag-and-drop target: undecorated, topmost, at a corner of the screen.
    fn drop_window(&mut self, ctx: &egui::Context) {
        let state = &mut self.state;
        let Some(window) = state.uploads.windows.drop_window.as_mut() else {
            return;
        };
        let area = modlight_macos::system::visible_frame()
            .map(|(x, y, w, h)| Rect::from_min_size(pos2(x, y), vec2(w, h)))
            .unwrap_or_else(|| Rect::from_min_size(pos2(0.0, 25.0), vec2(1440.0, 850.0)));
        let builder = ViewportBuilder::default()
            .with_title(t(s::DragDropUploadWindow_DropHere))
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_inner_size(window.window_size())
            .with_position(window.position(area));
        let colors = state.colors.clone();
        let outcome = ctx.show_viewport_immediate(
            ViewportId::from_hash_of("drop-window"),
            builder,
            |ui, _| {
                let (hovering, dropped) = ui.ctx().input(|i| {
                    (
                        !i.raw.hovered_files.is_empty(),
                        i.raw
                            .dropped_files
                            .iter()
                            .map(|f| f.path().to_path_buf())
                            .filter(|p| !p.as_os_str().is_empty())
                            .collect::<Vec<_>>(),
                    )
                });
                let outcome = window.ui(ui, &colors, hovering, dropped);
                if outcome == Some(crate::dialogs::drop_window::DropOutcome::StartDrag) {
                    ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
                }
                outcome
            },
        );
        if let Some(outcome) = outcome {
            state.drop_window_outcome(outcome);
        }
    }

    fn notification(&mut self, ctx: &egui::Context) {
        let state = &mut self.state;
        let Some(notice) = state.windows.notification.as_mut() else {
            return;
        };
        let (opacity, done) = notice.tick(state.time);
        if done {
            state.windows.notification = None;
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        let area = modlight_macos::system::visible_frame()
            .map(|(x, y, w, h)| Rect::from_min_size(pos2(x, y), vec2(w, h)))
            .unwrap_or_else(|| Rect::from_min_size(pos2(0.0, 25.0), vec2(1440.0, 850.0)));
        let size = notice.window_size();
        let position = notice.position(area);
        let builder = ViewportBuilder::default()
            .with_title("Modlight")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_active(false)
            .with_inner_size(size)
            .with_position(position);
        let colors = state.colors.clone();
        let outcome = ctx.show_viewport_immediate(
            ViewportId::from_hash_of("notification"),
            builder,
            |ui, _| {
                let rect = ui.max_rect();
                ui.painter()
                    .rect_filled(rect.expand(2.0), 0.0, egui::Color32::TRANSPARENT);
                let outcome = notice.ui(ui, &colors, opacity);
                let wanted = notice.window_size();
                if (ui
                    .ctx()
                    .input(|i| i.viewport().inner_rect.map(|r| r.size()))
                    .unwrap_or(wanted)
                    - wanted)
                    .length()
                    > 0.5
                {
                    ui.ctx()
                        .send_viewport_cmd(ViewportCommand::InnerSize(wanted));
                }
                outcome
            },
        );
        match outcome {
            Some(NotificationOutcome::Close) => state.windows.notification = None,
            Some(NotificationOutcome::Run(action)) => state.notification_action(action, true),
            Some(NotificationOutcome::Button(action, dismiss)) => {
                state.notification_action(action, dismiss)
            }
            None => {}
        }
    }

    /// One frame of the whole app, drawn into `ui` (the main window).
    pub fn frame(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.before(&ctx);
        self.handle_close(&ctx);
        crate::shell::ui(ui, &mut self.state, self.logo.as_ref());
        self.state.update_drag(&ctx);
        self.state.paint_drag_ghost(&ctx);
        if self.state.windows.modal_open() || self.state.uploads.modal_open() {
            crate::shell::block_input(&ctx, ui.max_rect());
        }
        if matches!(self.mode, Mode::Normal | Mode::Qa) {
            self.viewports(&ctx);
            // `UpdateMainWindowTitle`: the system's title follows the progress too.
            let title = self.state.uploads.tasks.title();
            if title != self.window_title {
                ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
                self.window_title = title;
            }
        }
        self.after(&ctx);
    }
}

impl eframe::App for ModlightApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn on_exit(&mut self) {
        self.state.save_config();
    }
}

impl AppState {
    /// `TaskHelpers.ShowNotificationTip`.
    pub fn show_notification_tip(&mut self, text: &str, title: &str, duration_ms: Option<f64>) {
        let settings = self.config.toast();
        self.windows.notification = Some(crate::notification::Notification::tip(
            text,
            title,
            duration_ms,
            &settings,
            self.time,
        ));
    }
}

/// Where a dialog opens: centred on an area (the main window for `CenterOwner`, the
/// screen's visible frame for `CenterScreen`), and whether it stays on top (`Topmost`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placement {
    area: Option<Rect>,
    topmost: bool,
}

impl Placement {
    pub(crate) fn owner(main: Option<Rect>) -> Placement {
        Placement {
            area: main,
            topmost: false,
        }
    }

    pub(crate) fn screen(topmost: bool) -> Placement {
        Placement {
            area: modlight_macos::system::visible_frame()
                .map(|(x, y, w, h)| Rect::from_min_size(pos2(x, y), vec2(w, h))),
            topmost,
        }
    }
}

/// The builder of a dialog window centred where `place` says.
fn builder(title: &str, size: Vec2, place: Placement, resizable: bool) -> ViewportBuilder {
    let main = place.area;
    let mut builder = ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(size)
        .with_resizable(resizable)
        .with_minimize_button(false)
        .with_maximize_button(resizable);
    if place.topmost {
        builder = builder.with_always_on_top();
    }
    if let Some(main) = main {
        builder = builder.with_position(pos2(
            main.center().x - size.x / 2.0,
            (main.center().y - size.y / 2.0).max(main.top()),
        ));
    }
    builder
}

/// A fixed-size dialog whose height follows its content. Returns what `add` returned
/// and whether its window was closed.
pub(crate) fn dialog<R>(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    size: Vec2,
    place: Placement,
    _modal: bool,
    mut add: impl FnMut(&mut Ui) -> Option<R>,
) -> (Option<R>, bool) {
    let size = vec2(size.x, size.y.max(80.0).round());
    let viewport = ViewportId::from_hash_of(("dialog", id));
    ctx.show_viewport_immediate(viewport, builder(title, size, place, false), |ui, _| {
        let close = ui.ctx().input(|i| i.viewport().close_requested());
        let current = ui
            .ctx()
            .input(|i| i.viewport().inner_rect.map(|r| r.size()));
        if let Some(current) = current
            && (current.y - size.y).abs() > 0.5
        {
            ui.ctx().send_viewport_cmd(ViewportCommand::InnerSize(size));
        }
        let result = add(ui);
        (result, close)
    })
}

/// A resizable window (settings, duplicates).
pub(crate) fn dialog_resizable<R>(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    size: Vec2,
    min: Vec2,
    place: Placement,
    mut add: impl FnMut(&mut Ui) -> R,
) -> (R, bool) {
    let viewport = ViewportId::from_hash_of(("dialog", id));
    let builder = builder(title, size, place, true).with_min_inner_size(min);
    ctx.show_viewport_immediate(viewport, builder, |ui, _| {
        let close = ui.ctx().input(|i| i.viewport().close_requested());
        (add(ui), close)
    })
}
