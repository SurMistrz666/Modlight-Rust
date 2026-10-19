//! Port of the `@tus/utils` 0.7.0 models the tus server and its stores share:
//! `Upload`, `Metadata` (parse / stringify), `DataStore` and `ERRORS`.
//!
//! `@tus/server` 2.4.1 (the protocol), `@tus/file-store` 2.1.0 and
//! `@tus/s3-store` 2.0.4 are the libraries `lib/upload/tus.ts` builds on; their
//! wire behaviour is part of what clients (Uppy in the panel, the desktop
//! apps' resumable uploader) depend on, so they are ported next to it in
//! [`super`]. The sources read for this port are the exact versions upstream's
//! `pnpm-lock.yaml` pins, taken from the reference image.

use std::borrow::Cow;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::storage::ByteStream;

/// The extensions a store may declare (`DataStore.extensions`).
pub const EXT_CREATION: &str = "creation";
pub const EXT_CREATION_WITH_UPLOAD: &str = "creation-with-upload";
pub const EXT_CREATION_DEFER_LENGTH: &str = "creation-defer-length";
pub const EXT_TERMINATION: &str = "termination";
pub const EXT_EXPIRATION: &str = "expiration";

/// A tus refusal: `{ status_code, body }` (the objects in `ERRORS`, and what
/// `onUploadCreate` / `onUploadFinish` throw).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TusError {
    pub status_code: u16,
    pub body: Cow<'static, str>,
}

impl TusError {
    pub const fn fixed(status_code: u16, body: &'static str) -> TusError {
        TusError { status_code, body: Cow::Borrowed(body) }
    }

    pub fn new(status_code: u16, body: impl Into<String>) -> TusError {
        TusError { status_code, body: Cow::Owned(body.into()) }
    }
}

impl std::fmt::Display for TusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.status_code, self.body.trim_end())
    }
}

impl std::error::Error for TusError {}

/// `ERRORS` of `@tus/utils/constants`, with their exact bodies.
pub mod errors {
    use super::TusError;

    pub const MISSING_OFFSET: TusError = TusError::fixed(403, "Upload-Offset header required\n");
    pub const ABORTED: TusError = TusError::fixed(400, "Request aborted due to lock acquired");
    pub const INVALID_TERMINATION: TusError = TusError::fixed(400, "Cannot terminate an already completed upload");
    pub const ERR_LOCK_TIMEOUT: TusError = TusError::fixed(500, "failed to acquire lock before timeout");
    pub const INVALID_CONTENT_TYPE: TusError = TusError::fixed(403, "Content-Type header required\n");
    pub const FILE_NOT_FOUND: TusError = TusError::fixed(404, "The file for this url was not found\n");
    pub const INVALID_OFFSET: TusError = TusError::fixed(409, "Upload-Offset conflict\n");
    pub const FILE_NO_LONGER_EXISTS: TusError = TusError::fixed(410, "The file for this url no longer exists\n");
    pub const ERR_SIZE_EXCEEDED: TusError = TusError::fixed(413, "upload's size exceeded\n");
    pub const ERR_MAX_SIZE_EXCEEDED: TusError = TusError::fixed(413, "Maximum size exceeded\n");
    pub const INVALID_LENGTH: TusError = TusError::fixed(400, "Upload-Length or Upload-Defer-Length header required\n");
    pub const INVALID_METADATA: TusError = TusError::fixed(
        400,
        "Upload-Metadata is invalid. It MUST consist of one or more comma-separated key-value pairs. The key and value MUST be separated by a space. The key MUST NOT contain spaces and commas and MUST NOT be empty. The key SHOULD be ASCII encoded and the value MUST be Base64 encoded. All keys MUST be unique",
    );
    pub const UNKNOWN_ERROR: TusError = TusError::fixed(500, "Something went wrong with that request\n");
    pub const FILE_WRITE_ERROR: TusError = TusError::fixed(500, "Something went wrong receiving the file\n");
    pub const UNSUPPORTED_CONCATENATION_EXTENSION: TusError = TusError::fixed(
        501,
        "Concatenation extension is not (yet) supported. Disable parallel uploads in the tus client.\n",
    );
    pub const UNSUPPORTED_CREATION_DEFER_LENGTH_EXTENSION: TusError =
        TusError::fixed(501, "creation-defer-length extension is not (yet) supported.\n");
    pub const UNSUPPORTED_EXPIRATION_EXTENSION: TusError =
        TusError::fixed(501, "expiration extension is not (yet) supported.\n");
}

/// What a store or a hook can fail with: a tus refusal (status + body, sent
/// as is) or anything else, which the server answers like `@tus/server`'s
/// `onError`: 500 with `UNKNOWN_ERROR.body + error.message + "\n"`.
#[derive(Debug)]
pub enum StoreError {
    Tus(TusError),
    Other(anyhow::Error),
}

impl From<TusError> for StoreError {
    fn from(e: TusError) -> Self {
        StoreError::Tus(e)
    }
}

impl From<anyhow::Error> for StoreError {
    fn from(e: anyhow::Error) -> Self {
        StoreError::Other(e)
    }
}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Other(e.into())
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Tus(e) => write!(f, "{e}"),
            StoreError::Other(e) => write!(f, "{e:#}"),
        }
    }
}

impl StoreError {
    /// The status and body `@tus/server` answers this error with.
    pub fn status_and_body(&self) -> (u16, String) {
        match self {
            StoreError::Tus(e) => (e.status_code, e.body.to_string()),
            StoreError::Other(e) => (500, format!("{}{}\n", errors::UNKNOWN_ERROR.body, e)),
        }
    }
}

/// Upload-Metadata: keys in the client's order, values decoded (`null` for a
/// key sent without a value).
pub type Metadata = IndexMap<String, Option<String>>;

/// Where a store keeps an upload's bytes (`Upload.storage`):
/// `{type: "file", path}` for the file store, `{type: "s3", path, bucket}`
/// for the S3 store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadStorage {
    #[serde(rename = "type")]
    pub kind: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket: Option<String>,
}

/// `Upload` of `@tus/utils`. Field order is the class's, which is the order
/// `JSON.stringify` writes the file store's `<id>.json` record in, so a record
/// written by the TypeScript server and one written here are interchangeable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Upload {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
    /// `None` while the length is deferred (`sizeIsDeferred`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default)]
    pub offset: u64,
    /// ISO string (`new Date().toISOString()` at construction).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creation_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<UploadStorage>,
}

impl Upload {
    /// `new Upload({ id, size, offset, metadata })`: stamps `creation_date`.
    pub fn new(id: String, size: Option<u64>, offset: u64, metadata: Option<Metadata>) -> Upload {
        Upload {
            id,
            metadata,
            size,
            offset,
            creation_date: Some(crate::db::types::JsDate::now().to_iso_string()),
            storage: None,
        }
    }

    pub fn size_is_deferred(&self) -> bool {
        self.size.is_none()
    }

    /// A metadata value (`upload.metadata?.[key]`), `None` when absent or null.
    pub fn meta(&self, key: &str) -> Option<&str> {
        self.metadata.as_ref().and_then(|m| m.get(key)).and_then(|v| v.as_deref())
    }

    /// `creation_date` as epoch milliseconds (`new Date(creation_date).getTime()`).
    pub fn creation_millis(&self) -> Option<i64> {
        self.creation_date.as_deref().and_then(crate::db::types::JsDate::parse_iso).map(|d| d.millis())
    }
}

/// A tus datastore (`DataStore` of `@tus/utils`).
pub trait DataStore: Send + Sync {
    /// The protocol extensions this store implements, in the order they are
    /// advertised in `Tus-Extension`.
    fn extensions(&self) -> &[&'static str];

    fn has_extension(&self, extension: &str) -> bool {
        self.extensions().contains(&extension)
    }

    /// POST: create the (empty) upload. Returns it with `storage` filled in.
    fn create(&self, upload: Upload) -> BoxFuture<'_, Result<Upload, StoreError>>;

    /// DELETE (termination) and cleanup.
    fn remove<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), StoreError>>;

    /// PATCH: append `body` at `offset`; returns the new offset. A body that
    /// ends early (client gone) is NOT an error: what arrived is kept, so the
    /// client can resume from it.
    fn write<'a>(&'a self, body: ByteStream, id: &'a str, offset: u64) -> BoxFuture<'a, Result<u64, StoreError>>;

    /// HEAD / PATCH: the upload with its current offset.
    fn get_upload<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Upload, StoreError>>;

    /// PATCH with `Upload-Length` after a deferred creation.
    fn declare_upload_length<'a>(&'a self, id: &'a str, upload_length: u64) -> BoxFuture<'a, Result<(), StoreError>>;

    /// The expiration extension: delete unfinished uploads past their expiry.
    fn delete_expired(&self) -> BoxFuture<'_, Result<u64, StoreError>>;

    /// Milliseconds an unfinished upload lives (0 = forever).
    fn get_expiration(&self) -> u64;

    /// The file store's scratch directory (`FileStore.directory`), for the
    /// orphaned-record sweep of lib/upload/tus.ts. `None` for other stores.
    fn directory(&self) -> Option<&Path> {
        None
    }

    /// `configstore.delete(id)`: drop the upload's record only (file store).
    fn delete_record<'a>(&'a self, _id: &'a str) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async { Ok(()) })
    }
}

// ── Metadata (`@tus/utils/models/Metadata`) ─────────────────────────────────

/// `validateKey`: non-empty, ASCII, no space, no comma.
fn validate_key(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| (c as u32) <= 127 && c != ' ' && c != ',')
}

/// `validateValue`: length a multiple of 4 and `/^[\d+/A-Za-z]*={0,2}$/`.
fn validate_value(value: &str) -> bool {
    if !value.encode_utf16().count().is_multiple_of(4) {
        return false;
    }
    let body = value.trim_end_matches('=');
    let pad = value.len() - body.len();
    pad <= 2 && body.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}

/// `Buffer.from(value, "base64").toString("utf8")`: lenient decoding (stops
/// at the first padding), invalid UTF-8 replaced.
fn decode_base64_lossy(value: &str) -> String {
    let clean: String = value.chars().take_while(|c| *c != '=').collect();
    let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(clean.as_bytes())
        .or_else(|_| {
            // A dangling single character carries no full byte; Node drops it.
            let n = clean.len() - clean.len() % 4;
            let keep = if clean.len() % 4 == 1 { n } else { clean.len() };
            base64::engine::general_purpose::STANDARD_NO_PAD.decode(&clean.as_bytes()[..keep])
        })
        .unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// "Metadata string is not valid" (`Metadata.parse` threw).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidMetadata;

/// `Metadata.parse(str)`.
pub fn parse_metadata(s: &str) -> Result<Metadata, InvalidMetadata> {
    let mut meta = Metadata::new();
    if crate::config::js_trim(s).is_empty() {
        return Err(InvalidMetadata);
    }
    for pair in s.split(',') {
        let tokens: Vec<&str> = pair.split(' ').collect();
        let key = tokens[0];
        let ok = match tokens.len() {
            1 => validate_key(key),
            2 => validate_key(key) && validate_value(tokens[1]),
            _ => false,
        };
        if !ok || meta.contains_key(key) {
            return Err(InvalidMetadata);
        }
        let value = match tokens.get(1) {
            Some(v) if !v.is_empty() => Some(decode_base64_lossy(v)),
            _ => None,
        };
        meta.insert(key.to_string(), value);
    }
    Ok(meta)
}

/// `Metadata.stringify(metadata)`.
pub fn stringify_metadata(metadata: &Metadata) -> String {
    metadata
        .iter()
        .map(|(k, v)| match v {
            None => k.clone(),
            Some(v) => format!("{k} {}", STANDARD.encode(v.as_bytes())),
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_round_trip_like_tus_utils() {
        let m = parse_metadata("filename d29ybGRfZG9taW5hdGlvbl9wbGFuLnBkZg==,is_confidential").unwrap();
        assert_eq!(m.get("filename").unwrap().as_deref(), Some("world_domination_plan.pdf"));
        assert_eq!(m.get("is_confidential").unwrap(), &None);
        assert_eq!(stringify_metadata(&m), "filename d29ybGRfZG9taW5hdGlvbl9wbGFuLnBkZg==,is_confidential");
    }

    #[test]
    fn metadata_refusals() {
        for bad in ["", "  ", "a b c", "k notbase64!", "k abc", ",", "a,a", "a b,", "k\u{e9} YQ=="] {
            assert!(parse_metadata(bad).is_err(), "{bad:?} should be refused");
        }
        // An empty value after the space is a valid (null) entry.
        assert_eq!(parse_metadata("k ").unwrap().get("k").unwrap(), &None);
        assert_eq!(parse_metadata("folderId cm9vdA==").unwrap().get("folderId").unwrap().as_deref(), Some("root"));
    }

    #[test]
    fn upload_record_json_matches_the_file_store() {
        let mut meta = Metadata::new();
        meta.insert("filename".into(), Some("a.png".into()));
        let u = Upload {
            id: "abc".into(),
            metadata: Some(meta),
            size: Some(10),
            offset: 0,
            creation_date: Some("2026-10-19T10:00:00.000Z".into()),
            storage: None,
        };
        assert_eq!(
            serde_json::to_string(&u).unwrap(),
            r#"{"id":"abc","metadata":{"filename":"a.png"},"size":10,"offset":0,"creation_date":"2026-10-19T10:00:00.000Z"}"#
        );
        let back: Upload = serde_json::from_str(&serde_json::to_string(&u).unwrap()).unwrap();
        assert_eq!(back, u);
        assert!(!u.size_is_deferred());
    }
}
