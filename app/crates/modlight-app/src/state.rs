//! The whole app's state: settings, the account, the skin, what the main window shows,
//! and which secondary windows are open. Drawing lives elsewhere; this is the model.

use std::path::PathBuf;
use std::time::Instant;

use chrono::{DateTime, Utc};
use modlight_api::{AccountInfo, Api, Credential};
use modlight_core::account::{AccountStore, AdminFlag};
use modlight_core::config::AppConfig;
use modlight_core::culture::{Culture, set_culture};
use modlight_core::l10n::{self, Language};
use modlight_core::sync::{SyncSchedule, SyncState};
use modlight_core::theme::{ResolvedTheme, ThemeOptions, resolve};

use crate::dialogs::Windows;
use crate::media::Media;
use crate::myspace::details::Details;
use crate::myspace::model::MySpace;
use crate::runtime::Runtime;
use crate::strip::AccountStrip;
use crate::theme::Colors;

/// The tabs of the main window (`MainWindowTab`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    MySpace,
    Upload,
    Download,
    Create,
    Tools,
    Admin,
}

/// Where the app runs and what the system said at start.
#[derive(Clone, Debug)]
pub struct Env {
    pub profile: PathBuf,
    /// QA mode: no Keychain, no tray, no system prompts; a fixed clock when set.
    pub qa: bool,
    pub frozen_now: Option<DateTime<Utc>>,
    pub machine: String,
    pub system_language: String,
    pub system_locale: String,
    pub reduce_motion: bool,
    /// The system appearance, when known (egui reports it each frame in the real app).
    pub system_dark: Option<bool>,
}

impl Env {
    pub fn now(&self) -> DateTime<Utc> {
        self.frozen_now.unwrap_or_else(Utc::now)
    }
}

/// One notice in the window's corner (`UploadToastViewModel`).
#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub icon: char,
    pub title: String,
    pub name: String,
    pub detail: String,
    pub link: String,
    pub failed: bool,
    pub expires: f64,
    pub copied_until: f64,
}

/// Where completed work tells the person what happened.
pub const MAX_TOASTS: usize = 4;
pub const DONE_TOAST_SECONDS: f64 = 6.0;
pub const FAILED_TOAST_SECONDS: f64 = 14.0;

pub struct AppState {
    pub env: Env,
    pub rt: Runtime,
    pub api: Api,
    pub config: AppConfig,
    pub account: AccountStore,
    pub admin: AdminFlag,
    pub options: ThemeOptions,
    pub theme: ResolvedTheme,
    pub colors: Colors,
    pub language: Language,
    pub tab: Tab,
    pub strip: AccountStrip,
    pub space: MySpace,
    pub details: Details,
    pub media: Media,
    pub windows: Windows,
    pub toasts: Vec<Toast>,
    pub toast_counter: u64,
    pub sync: SyncState,
    pub schedule: SyncSchedule,
    pub sync_running: bool,
    /// The library side panel (`ModlightLibraryPanel`), and the two sections' folds.
    pub library_panel: bool,
    pub folders_section: bool,
    pub files_section: bool,
    pub config_dirty: bool,
    /// The clock of the UI (seconds, egui's time), kept for timers set outside a frame.
    pub time: f64,
    /// Things the window needs the platform to do this frame (copy, open, close...).
    pub effects: Vec<Effect>,
    pub started: Instant,
    /// Exit was asked for (tray, menu).
    pub quit_requested: bool,
    /// Commands raised while drawing, run after the frame.
    pub pending: Vec<crate::commands::Command>,
    /// When the library panel last started to slide (UI seconds).
    pub panel_moved_at: f64,
    /// A drag of cards towards a folder.
    pub drag: crate::myspace::drag::Drag,
    pub emptying_trash: bool,
    pub scanning_duplicates: bool,
    /// The card range last reported as on screen.
    pub visible_range: Option<(usize, usize)>,
    /// A new listing: the grid scrolls back to the top.
    pub scroll_to_top: bool,
    /// Files dragged in from Finder are over the window.
    pub drop_hover: bool,
    /// The details drawer was the last thing clicked: arrows and Escape go to it.
    pub drawer_has_keys: bool,
    /// How the app was started (set by the window).
    pub mode: crate::app::Mode,
    /// Tasks, the offline queue and the upload windows.
    pub uploads: crate::uploads::Uploads,
    /// The Upload tab's rows.
    pub upload_tab: crate::upload_tab::UploadTab,
    /// The queue as last read, and when (UI seconds).
    pub queue_snapshot: Vec<modlight_core::upload::QueueItem>,
    pub queue_snapshot_at: f64,
    /// What "the clipboard" holds in QA and headless runs (the real one is never read).
    pub qa_clipboard: Option<modlight_core::tasks::upload_manager::ClipboardContent>,
    /// Captures, hotkeys, recording, auto capture.
    pub captures: crate::capture_flow::Captures,
    /// The Server tab's disk report.
    pub server_tab: crate::admin_tab::AdminTab,
    /// No settings file existed when the app started (the first-run window opens).
    pub first_launch: bool,
    /// The scrolling capture window.
    pub scrolling: Option<crate::scrolling::ScrollingWindow>,
    /// The Download tab (phase 3, media).
    pub downloads: crate::download_tab::DownloadTab,
    /// The components list and its window (phase 3, media).
    pub components: crate::components_window::Components,
    /// The app's own update check and its windows (phase 3, media).
    pub updates: crate::update_window::Updates,
    /// The tool windows (phase 5, tools).
    pub tools: crate::tools::Tools,
}

/// A request to the platform layer, handled after the frame is drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    OpenUrl(String),
    CopyText(String),
    CopyImage(std::sync::Arc<image::RgbaImage>),
    CopyFiles(Vec<PathBuf>),
    ClearClipboard,
    /// Show a file in Finder.
    Reveal(PathBuf),
    /// Open a file or folder with its app.
    OpenFile(PathBuf),
    PlaySound(
        modlight_core::tasks::host::NotificationSound,
        Option<String>,
    ),
    Quit,
}

impl AppState {
    pub fn new(env: Env, config: AppConfig, account: AccountStore) -> AppState {
        let options = config.theme_options();
        let theme = resolve(&options, env.system_dark, None);
        let colors = Colors::new(&theme);
        let language = config.language().resolve(&env.system_language);
        l10n::set_language(language);
        set_culture(Culture::for_locale(&env.system_locale));
        let space = MySpace {
            page_size: modlight_core::library::pager::clamp_page_size(config.page_size()),
            ..MySpace::default()
        };
        let rt = Runtime::new();
        let env_profile = env.profile.clone();
        let env_qa = env.qa;
        let api = Api::new(env!("CARGO_PKG_VERSION")).expect("the HTTP client starts");
        let uploads = crate::uploads::Uploads::new(
            api.clone(),
            rt.handle(),
            &env.profile,
            modlight_core::tasks::settings::AppUploadSettings::from_config(&config),
            rt.waker(),
        );
        AppState {
            library_panel: config.library_panel(),
            folders_section: config.folders_section(),
            files_section: config.files_section(),
            env,
            rt,
            api,
            config,
            account,
            admin: AdminFlag::default(),
            options,
            theme,
            colors,
            language,
            tab: Tab::MySpace,
            strip: AccountStrip::default(),
            space,
            details: Details::default(),
            media: Media::default(),
            windows: Windows::default(),
            toasts: Vec::new(),
            toast_counter: 0,
            sync: SyncState::default(),
            schedule: SyncSchedule::default(),
            sync_running: false,
            config_dirty: false,
            time: 0.0,
            effects: Vec::new(),
            started: Instant::now(),
            quit_requested: false,
            pending: Vec::new(),
            panel_moved_at: f64::NEG_INFINITY,
            drag: Default::default(),
            emptying_trash: false,
            scanning_duplicates: false,
            visible_range: None,
            scroll_to_top: false,
            drop_hover: false,
            drawer_has_keys: false,
            mode: crate::app::Mode::Normal,
            uploads,
            upload_tab: crate::upload_tab::UploadTab::default(),
            queue_snapshot: Vec::new(),
            queue_snapshot_at: f64::NEG_INFINITY,
            qa_clipboard: None,
            captures: crate::capture_flow::Captures::new(&env_profile, env_qa),
            server_tab: Default::default(),
            first_launch: false,
            scrolling: None,
            downloads: Default::default(),
            components: Default::default(),
            updates: Default::default(),
            tools: Default::default(),
        }
    }

    pub fn credential(&self) -> Option<Credential> {
        self.account.credential()
    }

    pub fn is_admin(&self) -> bool {
        self.account
            .credential()
            .map(|_| self.account.account_key())
            .is_some_and(|key| self.admin.is_admin(Some(&key)))
    }

    /// Re-resolve the skin after the options, the system appearance or the language moved.
    pub fn refresh_theme(&mut self, ctx: Option<&egui::Context>) {
        self.theme = resolve(&self.options, self.env.system_dark, None);
        self.colors = Colors::new(&self.theme);
        if let Some(ctx) = ctx {
            crate::theme::apply_style(ctx, &self.colors);
        }
    }

    pub fn set_language_setting(&mut self, setting: l10n::LanguageSetting) {
        self.config.set_language(setting);
        let language = setting.resolve(&self.env.system_language);
        let changed = language != self.language;
        self.language = language;
        l10n::set_language(self.language);
        // Windows: the Explorer menu entry is registered in the interface language; a
        // new language registers it again.
        #[cfg(windows)]
        if changed
            && self.mode == crate::app::Mode::Normal
            && !self.env.qa
            && crate::platform::integration::context_menu_enabled()
            && let Err(error) = crate::platform::integration::set_context_menu(true)
        {
            log::warn!("Explorer integration: {error}");
        }
        #[cfg(not(windows))]
        let _ = changed;
        self.space.pager.refresh_texts();
        self.space.update_count();
        self.save_config();
    }

    /// Persist the settings, and tell sync a change may need sending.
    pub fn save_config(&mut self) {
        self.config.set_theme_options(&self.options);
        self.config.set_library_panel(self.library_panel);
        self.config.set_folders_section(self.folders_section);
        self.config.set_files_section(self.files_section);
        self.config.set_page_size(self.space.page_size);
        self.config
            .set_last_folder_id(self.space.current_folder.as_deref());
        if let Err(error) = self.config.save() {
            log::warn!("Modlight: settings could not be saved: {error}");
        }
        self.schedule.nudge(Instant::now());
    }

    pub fn open_url(&mut self, url: &str) {
        if !url.is_empty() {
            self.effects.push(Effect::OpenUrl(url.to_string()));
        }
    }

    pub fn copy_text(&mut self, text: &str) {
        if !text.is_empty() {
            self.effects.push(Effect::CopyText(text.to_string()));
        }
    }

    /// A notice in the corner (`MainWindow.Notice`).
    pub fn notice(&mut self, icon: char, title: &str, detail: &str, failed: bool) {
        self.toast_counter += 1;
        let life = if failed {
            FAILED_TOAST_SECONDS
        } else {
            DONE_TOAST_SECONDS
        };
        self.toasts.insert(
            0,
            Toast {
                id: self.toast_counter,
                icon,
                title: title.to_string(),
                name: String::new(),
                detail: detail.to_string(),
                link: String::new(),
                failed,
                expires: self.time + life,
                copied_until: 0.0,
            },
        );
        self.toasts.truncate(MAX_TOASTS);
    }

    /// A finished upload in the corner, with its link to copy or open.
    pub fn toast_upload(&mut self, title: &str, name: &str, detail_or_link: &str, failed: bool) {
        self.toast_counter += 1;
        let life = if failed {
            FAILED_TOAST_SECONDS
        } else {
            DONE_TOAST_SECONDS
        };
        let (icon, link) = if failed {
            (modlight_core::icons::TRIANGLE_ALERT, String::new())
        } else {
            (
                modlight_core::icons::CIRCLE_CHECK,
                detail_or_link.to_string(),
            )
        };
        self.toasts.insert(
            0,
            Toast {
                id: self.toast_counter,
                icon,
                title: title.to_string(),
                name: name.to_string(),
                detail: detail_or_link.to_string(),
                link,
                failed,
                expires: self.time + life,
                copied_until: 0.0,
            },
        );
        self.toasts.truncate(MAX_TOASTS);
    }

    /// Ask for a command to run after this frame.
    pub fn push(&mut self, command: crate::commands::Command) {
        self.pending.push(command);
    }

    /// Something answered with what `/api/me` says: keep the identity, the role, the quota.
    pub fn note_account(&mut self, host: &str, info: &AccountInfo) {
        self.admin.record(host, info);
        self.account.adopt(info);
    }

    /// The token was refused: the role it carried goes with it.
    pub fn note_token_refused(&mut self) {
        self.admin.forget();
    }
}
