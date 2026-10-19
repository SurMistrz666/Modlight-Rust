//! Modlight App for macOS: the window, My space, sign-in, notifications, settings and
//! theme sync, and the tray. Everything that draws lives here; the protocol is in
//! `modlight-api`, the rules and texts in `modlight-core`, the platform in
//! `modlight-macos`.

pub mod app;
pub mod commands;
pub mod dialogs;
pub mod media;
pub mod menus;
pub mod myspace;
pub mod notification;
pub mod qa;
pub mod qa_uploads;
pub mod runtime;
pub mod settings;
pub mod shell;
pub mod signin;
pub mod state;
pub mod strip;
pub mod sync;
pub mod theme;
pub mod tray;
pub mod upload_tab;
pub mod uploads;
pub mod widgets;
