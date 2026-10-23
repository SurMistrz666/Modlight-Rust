//! Modlight App for macOS and Windows: the window, My space, sign-in, notifications,
//! settings and theme sync, and the tray. Everything that draws lives here; the protocol
//! is in `modlight-api`, the rules and texts in `modlight-core`, the platform in
//! `modlight-macos` or `modlight-windows` (both reached as [`platform`]).

/// The system's glue this build uses: the same modules and functions on either system.
#[cfg(not(windows))]
pub use modlight_macos as platform;
#[cfg(windows)]
pub use modlight_windows as platform;

pub mod about_window;
pub mod admin_tab;
pub mod app;
pub mod app_pages;
pub mod capture_flow;
pub mod capture_windows;
pub mod commands;
pub mod controls;
pub mod dialogs;
pub mod editor_window;
pub mod ffmpeg_window;
pub mod first_run;
pub mod hotkey_page;
pub mod media;
pub mod menus;
pub mod myspace;
pub mod notification;
#[cfg(windows)]
pub mod personal_folder;
pub mod qa;
pub mod qa_capture;
pub mod qa_editor;
pub mod qa_uploads;
pub mod recording_ui;
pub mod region_overlay;
pub mod runtime;
pub mod scrolling;
pub mod settings;
pub mod settings_kit;
pub mod shell;
pub mod signin;
pub mod state;
pub mod strip;
pub mod sync;
pub mod task_editors;
pub mod task_pages;
pub mod theme;
pub mod tray;
pub mod upload_tab;
pub mod uploads;
pub mod widgets;
#[cfg(windows)]
pub mod win32;
// Phase 3, media: components, the Download tab, the app update.
pub mod components_window;
pub mod download_tab;
pub mod media_windows;
pub mod qa_media;
pub mod qa_settings;
pub mod update_window;
// Phase 5, tools: the tool windows, pinned pictures, the screen colour picker, history.
pub mod tools;
