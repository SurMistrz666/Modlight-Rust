//! The Modlight server protocol, as the desktop app speaks it.
//!
//! A port of the network half of the Windows client (`ShareX/Modlight/*.cs`, the
//! uploader and the URL shortener): the same endpoints, headers, bodies, timeouts and
//! refusal handling, with no GUI and no settings. Every call takes a [`Credential`] and
//! sends its token in the `x-api-key` header only; redirects are never followed.

pub mod account;
pub mod client;
pub mod error;
pub mod extras;
pub mod host;
pub mod library;
pub mod models;
pub mod signin;

pub use client::{API_KEY_HEADER, Api, Credential, REQUEST_TIMEOUT};
pub use error::{ApiError, ApiResult};
pub use extras::{
    DISK_PATH, DISK_TIMEOUT, DUPLICATE_SCAN_TIMEOUT, SETTINGS_CLIENT, SettingsAnswer,
    THEME_TIMEOUT, UploadAnswer, is_worth_queuing, server_text,
};
pub use host::{
    escape_data_string, is_plausible_token, rewrite_to_host, same_origin, try_normalize_host,
};
pub use library::{
    MAX_DELETE_IDS, MAX_MOTION_BYTES, MAX_THUMBNAIL_BYTES, ROOT_FOLDER, THUMBNAIL_TIMEOUT,
    UploadQuery,
};
pub use models::*;
pub use signin::{SignInFailure, SignedIn, display_name};
