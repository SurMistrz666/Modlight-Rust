//! Modlight server, Rust port.
//!
//! The upstream TypeScript tree (`upstream/Modlight`) is the specification.
//! Every module here names the upstream file it ports in its first doc line,
//! so the two can be read side by side (PORT.md section 5).
//!
//! Start with `docs/SKELETON.md` for how routes, actions, page loaders and
//! background jobs are added.

pub mod actions;
pub mod admin_users;
pub mod api_caller;
pub mod api_json;
pub mod api_scopes;
pub mod api_token;
pub mod audit;
pub mod auth;
pub mod auth_helpers;
pub mod authorize;
pub mod avatar;
pub mod background;
pub mod client_settings;
pub mod config;
pub mod credentials;
pub mod data;
pub mod db;
pub mod error;
pub mod format;
pub mod http;
pub mod i18n;
pub mod ids;
pub mod invites;
pub mod jobs;
pub mod js_json;
pub mod mail;
pub mod mail_settings;
pub mod mail_templates;
pub mod maintenance;
pub mod media;
pub mod naming;
pub mod pages;
pub mod password;
pub mod paths;
pub mod quota;
pub mod registration;
pub mod releases;
pub mod releases_core;
pub mod routes;
pub mod rpc;
pub mod runtime;
pub mod secret_box;
pub mod settings;
pub mod state;
pub mod storage;
pub mod theme_api;
pub mod theme_cookies;
pub mod themes;
pub mod upload;

pub use config::Config;
pub use error::{AppError, AppResult};
pub use state::AppState;
