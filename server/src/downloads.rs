//! Not upstream: Downloads (PL "Do pobrania"), the desktop app's builds for
//! macOS and Windows (docs/SKELETON.md section 9, item 45; README "Downloads").
//!
//! Build machines publish a build with `RELEASE_PUBLISH_TOKEN` (an
//! administrator may too, with a session or a `full` API token), in chunks
//! small enough for a proxy that caps a request at 100 MB; every signed-in
//! account lists and downloads the builds, and the desktop app reads the same
//! list with its own API token to find updates. The newest [`KEEP`] builds of
//! each platform and architecture stay: publishing one more deletes the oldest,
//! row and file.
//!
//! Where things live:
//! - metadata: the `releases` table (migration 0015), one row per build;
//! - bytes: the storage driver, at `releases/<id>/<filename>` (local or S3);
//! - an unfinished upload: local disk, `<UPLOAD_TMP_PATH>/releases/<upload>/`
//!   (`upload.json` plus one file per chunk). `finish` joins the chunks in
//!   order while hashing them and hands the result to the driver
//!   (`ingest_local_file`: a rename on the local driver, an upload on S3).
//!   An upload nobody touched for a day is swept ([`sweep_staged_uploads`]).
//!
//! The HTTP side (who may call what, status codes) is
//! `routes::api::downloads`; this module holds the rules and the work.

use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::io::AsyncWriteExt;

use crate::api_json::{PRIVATE_NO_STORE, json_response};
use crate::config::{Config, js_trim};
use crate::db::models::{Columns, JsDate, Release};
use crate::storage::StorageDriver;

/// Platforms, in the order the list answers them.
pub const PLATFORMS: [&str; 2] = ["macos", "windows"];
/// Architectures a build may be for.
pub const ARCHES: [&str; 2] = ["arm64", "x64"];
/// Builds kept per platform and architecture.
pub const KEEP: i64 = 5;
/// The largest chunk `PUT .../upload/{upload}/{n}` takes: half of the 100 MB a
/// Cloudflare-proxied request may carry.
pub const CHUNK_SIZE: u64 = 50 * 1024 * 1024;
/// The largest build accepted.
pub const MAX_BUILD_SIZE: u64 = 4 * 1024 * 1024 * 1024;
/// The highest chunk number (a chunk may be smaller than [`CHUNK_SIZE`], so
/// the count is not fixed by the size; this only bounds the file names).
pub const MAX_CHUNK_INDEX: u32 = 99_999;
/// An unfinished upload is dropped this long after it was last touched; a
/// finished one keeps its answer for a repeated `finish` as long.
pub const UPLOAD_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Unfinished uploads allowed at once.
pub const MAX_PENDING_UPLOADS: usize = 16;
/// `uploader` of a build published with `RELEASE_PUBLISH_TOKEN`.
pub const TOKEN_UPLOADER: &str = "publish-token";
/// Longest change note, in characters.
pub const NOTE_MAX_CHARS: usize = 2000;
/// Longest file name, version and build.
pub const FILENAME_MAX: usize = 128;
pub const VERSION_MAX: usize = 64;
pub const BUILD_MAX: usize = 64;

/// The storage prefix every build lives under.
pub const STORAGE_PREFIX: &str = "releases";

// ── Records ─────────────────────────────────────────────────────────────────

/// One build as the API, the page and the desktop app see it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Build {
    pub id: String,
    pub platform: String,
    pub arch: String,
    pub version: String,
    pub build: String,
    /// ISO 8601 (`toISOString()`), when the build machine built it.
    pub built: String,
    /// ISO 8601, when the server accepted it.
    pub published: String,
    pub size: i64,
    pub sha256: String,
    pub filename: String,
    pub uploader: String,
    pub note: String,
    /// `/downloads/{id}/{filename}`, relative to the server.
    pub url: String,
}

impl From<Release> for Build {
    fn from(r: Release) -> Build {
        Build {
            url: download_path(&r.id, &r.filename),
            built: r.built.to_iso_string(),
            published: r.published.to_iso_string(),
            id: r.id,
            platform: r.platform,
            arch: r.arch,
            version: r.version,
            build: r.build,
            size: r.size,
            sha256: r.sha256,
            filename: r.filename,
            uploader: r.uploader,
            note: r.note,
        }
    }
}

/// The builds of one platform, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlatformBuilds {
    pub platform: String,
    pub builds: Vec<Build>,
}

/// `GET /api/downloads`, and the props of the Downloads page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadsList {
    pub platforms: Vec<PlatformBuilds>,
    pub keep: i64,
    /// The caller may delete builds and publish with their session.
    pub admin: bool,
    /// Whether `RELEASE_PUBLISH_TOKEN` is set; `null` for anyone but an administrator.
    pub publish_token: Option<bool>,
}

/// Where a build is downloaded.
pub fn download_path(id: &str, filename: &str) -> String {
    format!("/downloads/{id}/{filename}")
}

/// The storage key of a build's file.
pub fn storage_key(id: &str, filename: &str) -> String {
    format!("{STORAGE_PREFIX}/{id}/{filename}")
}

/// Everything stored for one build.
pub fn storage_prefix(id: &str) -> String {
    format!("{STORAGE_PREFIX}/{id}/")
}

/// A refusal: the status, a stable `code` for programs and an `error` for people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl Refusal {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Refusal {
        Refusal { status, code, message: message.into() }
    }

    pub fn bad(code: &'static str, message: impl Into<String>) -> Refusal {
        Refusal::new(StatusCode::BAD_REQUEST, code, message)
    }

    /// `{"error", "code"}`, never cached.
    pub fn into_response(self) -> Response {
        json_response(self.status, &json!({ "error": self.message, "code": self.code }), Some(PRIVATE_NO_STORE))
    }
}

// ── Names and ids ───────────────────────────────────────────────────────────

/// A fresh build or upload id: 16 random bytes as 32 lower-case hex characters.
pub fn new_id() -> String {
    hex::encode(rand::random::<[u8; 16]>())
}

/// The shape [`new_id`] produces (build ids and upload ids alike).
pub fn is_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn plain(s: &str, extra: &[u8]) -> bool {
    s.bytes().all(|b| b.is_ascii_alphanumeric() || extra.contains(&b))
}

/// A file name that goes into a URL path, a storage key and a
/// `Content-Disposition` header as it is: ASCII letters, digits, `.`, `_`,
/// `+` and `-`, so no path separator, no `..`, not starting with a dot.
pub fn valid_filename(name: &str) -> bool {
    (1..=FILENAME_MAX).contains(&name.len()) && !name.starts_with('.') && !name.contains("..") && plain(name, b"._+-")
}

pub fn valid_version(version: &str) -> bool {
    (1..=VERSION_MAX).contains(&version.len()) && plain(version, b"._+-")
}

/// A build number or tag; empty is allowed.
pub fn valid_build(build: &str) -> bool {
    build.len() <= BUILD_MAX && plain(build, b"._+-")
}

/// A change note: several lines and tabs allowed, no other control characters.
pub fn valid_note(note: &str) -> bool {
    note.chars().count() <= NOTE_MAX_CHARS && !note.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
}

/// `macos` / `windows`, case and spacing forgiven.
pub fn normalize_platform(raw: &str) -> Option<&'static str> {
    let lower = js_trim(raw).to_ascii_lowercase();
    PLATFORMS.into_iter().find(|p| *p == lower)
}

/// `arm64` / `x64`, also under the names build tools print (`aarch64`,
/// `x86_64`, `amd64`). Empty means the platform's usual one.
pub fn normalize_arch(raw: &str, platform: &str) -> Option<&'static str> {
    match js_trim(raw).to_ascii_lowercase().as_str() {
        "" => Some(if platform == "macos" { "arm64" } else { "x64" }),
        "arm64" | "aarch64" => Some("arm64"),
        "x64" | "x86_64" | "x86-64" | "amd64" => Some("x64"),
        _ => None,
    }
}

// ── Starting an upload ──────────────────────────────────────────────────────

/// A validated `POST /api/downloads/upload` body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRequest {
    pub platform: String,
    pub arch: String,
    pub version: String,
    pub build: String,
    pub built: JsDate,
    pub size: u64,
    pub sha256: String,
    pub filename: String,
    pub note: String,
}

/// The earliest `built` accepted (anything before is a unit mix-up).
const BUILT_MIN_MS: i64 = 1_420_070_400_000; // 2015-01-01
/// How far in the future `built` may be (clocks disagree a little).
const BUILT_SKEW_MS: i64 = 24 * 60 * 60 * 1000;

/// A string field: `Ok(None)` when absent or `null`, `Err` for another type.
fn text<'a>(body: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, ()> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(()),
    }
}

/// Validate and normalise the body that starts an upload.
pub fn parse_start(body: &Map<String, Value>, now: JsDate) -> Result<StartRequest, Refusal> {
    let platform = text(body, "platform")
        .ok()
        .flatten()
        .and_then(normalize_platform)
        .ok_or_else(|| Refusal::bad("invalid_platform", "platform must be \"macos\" or \"windows\"."))?;

    let arch = text(body, "arch")
        .ok()
        .map(|a| a.unwrap_or(""))
        .and_then(|a| normalize_arch(a, platform))
        .ok_or_else(|| Refusal::bad("invalid_arch", "arch must be \"arm64\" or \"x64\"."))?;

    let version = text(body, "version").ok().flatten().map(js_trim).filter(|v| valid_version(v)).ok_or_else(|| {
        Refusal::bad(
            "invalid_version",
            format!("version must be 1 to {VERSION_MAX} characters: letters, digits, dot, underscore, plus or hyphen."),
        )
    })?;

    // A build number may come as a number too.
    let build = match body.get("build") {
        None | Some(Value::Null) => Some(String::new()),
        Some(Value::String(s)) => Some(js_trim(s).to_string()),
        Some(Value::Number(n)) if n.is_u64() => Some(n.to_string()),
        Some(_) => None,
    }
    .filter(|b| valid_build(b))
    .ok_or_else(|| {
        Refusal::bad(
            "invalid_build",
            format!("build must be at most {BUILD_MAX} characters: letters, digits, dot, underscore, plus or hyphen."),
        )
    })?;

    // An ISO 8601 date-time, or milliseconds since 1970; absent: now.
    let built = match body.get("built") {
        None | Some(Value::Null) => Some(now),
        Some(Value::String(s)) if js_trim(s).is_empty() => Some(now),
        Some(Value::String(s)) => JsDate::parse_iso(js_trim(s)),
        Some(Value::Number(n)) => n.as_i64().and_then(JsDate::from_millis),
        Some(_) => None,
    }
    .filter(|d| d.millis() >= BUILT_MIN_MS && d.millis() <= now.millis() + BUILT_SKEW_MS)
    .ok_or_else(|| {
        Refusal::bad(
            "invalid_built",
            "built must be an ISO 8601 date-time (or milliseconds since 1970), not in the future.",
        )
    })?;

    let size = body.get("size").and_then(Value::as_u64).filter(|s| *s > 0).ok_or_else(|| {
        Refusal::bad("invalid_size", "size must be the file's size in bytes, a whole number above zero.")
    })?;
    if size > MAX_BUILD_SIZE {
        return Err(Refusal::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_large",
            format!("A build may be at most {} GiB.", MAX_BUILD_SIZE >> 30),
        ));
    }

    let sha256 = text(body, "sha256")
        .ok()
        .flatten()
        .map(|s| js_trim(s).to_ascii_lowercase())
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| Refusal::bad("invalid_sha256", "sha256 must be the file's SHA-256 as 64 hex digits."))?;

    let filename = text(body, "filename").ok().flatten().map(js_trim).filter(|f| valid_filename(f)).ok_or_else(|| {
        Refusal::bad(
            "invalid_filename",
            format!(
                "filename must be 1 to {FILENAME_MAX} characters: letters, digits, dot, underscore, plus or hyphen, not starting with a dot (no paths)."
            ),
        )
    })?;

    let note = match text(body, "note") {
        Ok(None) => Some(String::new()),
        Ok(Some(n)) => Some(js_trim(&n.replace("\r\n", "\n")).to_string()),
        Err(()) => None,
    }
    .filter(|n| valid_note(n))
    .ok_or_else(|| {
        Refusal::bad(
            "invalid_note",
            format!("note must be text of at most {NOTE_MAX_CHARS} characters, without control characters other than new lines."),
        )
    })?;

    Ok(StartRequest {
        platform: platform.to_string(),
        arch: arch.to_string(),
        version: version.to_string(),
        build,
        built,
        size,
        sha256,
        filename: filename.to_string(),
        note,
    })
}

// ── Unfinished uploads (local disk) ─────────────────────────────────────────

/// `upload.json`: what the start promised, and who started it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedUpload {
    #[serde(flatten)]
    pub request: StartRequest,
    pub uploader: String,
    pub started: JsDate,
}

const RECORD: &str = "upload.json";
const DONE: &str = "done";
const ASSEMBLED: &str = "assembled";
const CHUNK_PREFIX: &str = "chunk-";

/// Where unfinished uploads are kept: `<UPLOAD_TMP_PATH>/releases`.
pub fn staging_root(config: &Config) -> PathBuf {
    PathBuf::from(&config.uploads.tmp_path).join(STORAGE_PREFIX)
}

/// The folder of one upload (`upload` must be an [`is_id`] id).
pub fn upload_dir(config: &Config, upload: &str) -> PathBuf {
    staging_root(config).join(upload)
}

fn chunk_name(index: u32) -> String {
    format!("{CHUNK_PREFIX}{index}")
}

fn chunk_index(name: &str) -> Option<u32> {
    let digits = name.strip_prefix(CHUNK_PREFIX)?;
    if digits.is_empty() || digits.len() > 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if digits.len() > 1 && digits.starts_with('0') {
        return None;
    }
    digits.parse().ok().filter(|i| *i <= MAX_CHUNK_INDEX)
}

/// A chunk number from the URL: digits only, no leading zeros, at most
/// [`MAX_CHUNK_INDEX`].
pub fn parse_chunk_index(raw: &str) -> Option<u32> {
    chunk_index(&format!("{CHUNK_PREFIX}{raw}"))
}

/// Write `bytes` to `path` whole or not at all (temporary name, then rename).
async fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", &new_id()[..8]));
    let result = async {
        let mut file = tokio::fs::File::create(&tmp).await?;
        file.write_all(bytes).await?;
        file.flush().await?;
        drop(file);
        tokio::fs::rename(&tmp, path).await
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result
}

/// Start an upload: a new id and its folder with `upload.json`.
pub async fn begin_upload(config: &Config, request: StartRequest, uploader: &str) -> anyhow::Result<String> {
    let id = new_id();
    let dir = upload_dir(config, &id);
    tokio::fs::create_dir_all(&dir).await?;
    let record = StagedUpload { request, uploader: uploader.to_string(), started: JsDate::now() };
    write_atomic(&dir.join(RECORD), &serde_json::to_vec(&record)?).await?;
    Ok(id)
}

/// The record of an unfinished upload, `None` when there is none (never
/// started, finished, dropped or expired).
pub async fn staged_upload(config: &Config, upload: &str) -> anyhow::Result<Option<StagedUpload>> {
    if !is_id(upload) {
        return Ok(None);
    }
    match tokio::fs::read(upload_dir(config, upload).join(RECORD)).await {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// The build a finished upload became, kept for a repeated `finish` whose
/// first answer got lost.
pub async fn finished_build_id(config: &Config, upload: &str) -> Option<String> {
    if !is_id(upload) {
        return None;
    }
    let raw = tokio::fs::read_to_string(upload_dir(config, upload).join(DONE)).await.ok()?;
    let id = raw.trim();
    is_id(id).then(|| id.to_string())
}

/// The chunk files of an upload by number, with their sizes.
pub async fn chunk_sizes(dir: &Path) -> std::io::Result<BTreeMap<u32, u64>> {
    let mut out = BTreeMap::new();
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let Some(index) = name.to_str().and_then(chunk_index) else {
            continue;
        };
        if let Ok(meta) = entry.metadata().await
            && meta.is_file()
        {
            out.insert(index, meta.len());
        }
    }
    Ok(out)
}

/// Why a chunk was not kept.
#[derive(Debug)]
pub enum ChunkError {
    /// More bytes than `limit`.
    TooLarge,
    /// No bytes at all.
    Empty,
    /// The client hung up or sent a broken body.
    Body,
    Io(std::io::Error),
}

/// Stream a request body into chunk `index` of the upload in `dir`, refusing
/// it as soon as it passes `limit` bytes. Written under a temporary name and
/// renamed, so a chunk is there whole or not at all; sending a chunk again
/// replaces it.
pub async fn write_chunk(dir: &Path, index: u32, body: Body, limit: u64) -> Result<u64, ChunkError> {
    let tmp = dir.join(format!("{}.part-{}", chunk_name(index), &new_id()[..8]));
    let result = async {
        let mut file = tokio::fs::File::create(&tmp).await.map_err(ChunkError::Io)?;
        let mut written: u64 = 0;
        let mut stream = body.into_data_stream();
        while let Some(piece) = stream.next().await {
            let piece = piece.map_err(|_| ChunkError::Body)?;
            written += piece.len() as u64;
            if written > limit {
                return Err(ChunkError::TooLarge);
            }
            file.write_all(&piece).await.map_err(ChunkError::Io)?;
        }
        if written == 0 {
            return Err(ChunkError::Empty);
        }
        file.flush().await.map_err(ChunkError::Io)?;
        drop(file);
        tokio::fs::rename(&tmp, dir.join(chunk_name(index))).await.map_err(ChunkError::Io)?;
        Ok(written)
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result
}

/// The chunks of an upload joined into one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assembled {
    pub path: PathBuf,
    pub size: u64,
    /// Lower-case hex.
    pub sha256: String,
}

/// Join chunks 0, 1, 2, ... of the upload in `dir` into one file, hashing
/// them on the way. `Ok(Err(reason))` when the chunks do not form an
/// unbroken run from 0.
pub async fn assemble(dir: &Path) -> anyhow::Result<Result<Assembled, String>> {
    let chunks = chunk_sizes(dir).await?;
    if chunks.is_empty() {
        return Ok(Err("no chunk was received".to_string()));
    }
    let count = chunks.len() as u32;
    if let Some(missing) = (0..count).find(|i| !chunks.contains_key(i)) {
        return Ok(Err(format!("chunk {missing} is missing")));
    }
    let dir = dir.to_path_buf();
    let out = dir.join(ASSEMBLED);
    let path = out.clone();
    let (size, sha256) = tokio::task::spawn_blocking(move || -> std::io::Result<(u64, String)> {
        let mut hasher = Sha256::new();
        let mut writer = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&out)?);
        let mut buf = vec![0u8; 1 << 20];
        let mut size: u64 = 0;
        for index in 0..count {
            let mut chunk = std::fs::File::open(dir.join(chunk_name(index)))?;
            loop {
                let n = chunk.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                writer.write_all(&buf[..n])?;
                size += n as u64;
            }
        }
        let file = writer.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()?;
        Ok((size, hex::encode(hasher.finalize())))
    })
    .await??;
    Ok(Ok(Assembled { path, size, sha256 }))
}

/// Remember which build an upload became, and free its chunks.
pub async fn mark_finished(config: &Config, upload: &str, build_id: &str) -> std::io::Result<()> {
    let dir = upload_dir(config, upload);
    write_atomic(&dir.join(DONE), build_id.as_bytes()).await?;
    let mut entries = tokio::fs::read_dir(&dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_name() != DONE {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
    Ok(())
}

/// Forget an upload and everything it received.
pub async fn drop_upload(config: &Config, upload: &str) {
    if !is_id(upload) {
        return;
    }
    let dir = upload_dir(config, upload);
    if let Err(e) = tokio::fs::remove_dir_all(&dir).await
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("[downloads] could not remove {}: {e}", dir.display());
    }
}

/// Remove uploads nobody touched for [`UPLOAD_TTL`] (a chunk or the start
/// renames a file into the folder, which is what moves its mtime). Returns
/// how many unfinished uploads remain.
pub async fn sweep_staged_uploads(config: &Config, now: SystemTime) -> usize {
    let root = staging_root(config);
    let Ok(mut entries) = tokio::fs::read_dir(&root).await else {
        return 0;
    };
    let mut pending = 0;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|n| is_id(n)) else {
            continue;
        };
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let stale = meta.modified().ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age > UPLOAD_TTL);
        if stale {
            drop_upload(config, name).await;
        } else if tokio::fs::try_exists(entry.path().join(RECORD)).await.unwrap_or(false) {
            pending += 1;
        }
    }
    pending
}

/// One `finish` at a time per upload, in this process.
pub struct FinishGuard(String);

fn finishing() -> &'static Mutex<HashSet<String>> {
    static SET: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SET.get_or_init(Default::default)
}

impl FinishGuard {
    /// `None` while another `finish` of the same upload runs.
    pub fn acquire(upload: &str) -> Option<FinishGuard> {
        let mut set = finishing().lock().unwrap_or_else(|p| p.into_inner());
        set.insert(upload.to_string()).then(|| FinishGuard(upload.to_string()))
    }
}

impl Drop for FinishGuard {
    fn drop(&mut self) {
        finishing().lock().unwrap_or_else(|p| p.into_inner()).remove(&self.0);
    }
}

// ── The table ───────────────────────────────────────────────────────────────

/// The `published` time of a new build: now, but always later than the
/// previous one this process recorded, so "newest" (and with it which builds
/// [`KEEP`] drops) never rests on two builds sharing a millisecond.
pub fn publish_time() -> JsDate {
    use std::sync::atomic::{AtomicI64, Ordering};
    static LAST: AtomicI64 = AtomicI64::new(0);
    let now = JsDate::now().millis();
    let mut previous = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(previous + 1);
        match LAST.compare_exchange_weak(previous, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return JsDate::from_millis(next).unwrap_or_else(JsDate::now),
            Err(actual) => previous = actual,
        }
    }
}

/// Every build, newest first.
pub async fn all_releases(db: &PgPool) -> sqlx::Result<Vec<Release>> {
    let sql = format!("SELECT {} FROM {} ORDER BY \"published\" DESC, \"id\" DESC", Release::COLUMNS, Release::TABLE);
    sqlx::query_as(&sql).fetch_all(db).await
}

/// One build by id.
pub async fn find_release(db: &PgPool, id: &str) -> sqlx::Result<Option<Release>> {
    if !is_id(id) {
        return Ok(None);
    }
    let sql = format!("SELECT {} FROM {} WHERE \"id\" = $1", Release::COLUMNS, Release::TABLE);
    sqlx::query_as(&sql).bind(id).fetch_optional(db).await
}

/// The list the API answers and the page shows.
pub async fn list(db: &PgPool, admin: bool, publish_token_set: bool) -> sqlx::Result<DownloadsList> {
    let mut by_platform: BTreeMap<String, Vec<Build>> = BTreeMap::new();
    for release in all_releases(db).await? {
        by_platform.entry(release.platform.clone()).or_default().push(Build::from(release));
    }
    Ok(DownloadsList {
        platforms: PLATFORMS
            .iter()
            .map(|p| PlatformBuilds { platform: p.to_string(), builds: by_platform.remove(*p).unwrap_or_default() })
            .collect(),
        keep: KEEP,
        admin,
        publish_token: admin.then_some(publish_token_set),
    })
}

/// Record a new build and drop every build of its platform and architecture
/// beyond the newest [`KEEP`], in one transaction serialised per platform and
/// architecture. Returns the dropped rows (their files are the caller's).
pub async fn insert_release(db: &PgPool, release: &Release) -> sqlx::Result<Vec<Release>> {
    let mut tx = db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("modlight:releases:{}:{}", release.platform, release.arch))
        .execute(&mut *tx)
        .await?;
    sqlx::query(&format!(
        "INSERT INTO {} ({}) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        Release::TABLE,
        Release::COLUMNS
    ))
    .bind(&release.id)
    .bind(&release.platform)
    .bind(&release.arch)
    .bind(&release.version)
    .bind(&release.build)
    .bind(release.built)
    .bind(release.published)
    .bind(release.size)
    .bind(&release.sha256)
    .bind(&release.filename)
    .bind(&release.uploader)
    .bind(&release.note)
    .execute(&mut *tx)
    .await?;
    let pruned: Vec<Release> = sqlx::query_as(&format!(
        "DELETE FROM {table} WHERE \"id\" IN (SELECT \"id\" FROM {table} WHERE \"platform\" = $1 AND \"arch\" = $2 \
         ORDER BY \"published\" DESC, \"id\" DESC OFFSET $3) RETURNING {columns}",
        table = Release::TABLE,
        columns = Release::COLUMNS
    ))
    .bind(&release.platform)
    .bind(&release.arch)
    .bind(KEEP)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(pruned)
}

/// Delete one build's row; `None` when there was none.
pub async fn delete_release(db: &PgPool, id: &str) -> sqlx::Result<Option<Release>> {
    if !is_id(id) {
        return Ok(None);
    }
    let sql = format!("DELETE FROM {} WHERE \"id\" = $1 RETURNING {}", Release::TABLE, Release::COLUMNS);
    sqlx::query_as(&sql).bind(id).fetch_optional(db).await
}

/// Delete a build's file (best effort: a leftover is logged, never an error
/// for the caller, whose row is already gone).
pub async fn delete_files(storage: &dyn StorageDriver, id: &str) {
    if !is_id(id) {
        return;
    }
    if let Err(e) = storage.delete_prefix(&storage_prefix(id)).await {
        tracing::warn!("[downloads] could not delete the files of build {id}: {e:#}");
    }
}

// ── Serving ─────────────────────────────────────────────────────────────────

/// The `Content-Type` of a build file, by its extension.
pub fn content_type_for(filename: &str) -> &'static str {
    let ext = filename.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "zip" => "application/zip",
        "dmg" => "application/x-apple-diskimage",
        "gz" | "tgz" => "application/gzip",
        "exe" => "application/vnd.microsoft.portable-executable",
        "msi" => "application/x-msi",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn now() -> JsDate {
        JsDate::parse_iso("2026-10-27T12:00:00.000Z").unwrap()
    }

    fn valid() -> Value {
        json!({
            "platform": "macos",
            "arch": "arm64",
            "version": "0.9.3",
            "build": "45",
            "built": "2026-10-27T10:00:00Z",
            "size": 1234,
            "sha256": "AB".repeat(32),
            "filename": "Modlight-0.9.3-45-macos-arm64.zip",
            "note": "  Faster uploads.\r\nFixes.  ",
        })
    }

    #[test]
    fn ids_are_32_hex_characters() {
        let id = new_id();
        assert!(is_id(&id), "{id}");
        assert_ne!(new_id(), id);
        for bad in ["", "ABCDEF0123456789abcdef0123456789", "abc", &"g".repeat(32), &"a".repeat(33)] {
            assert!(!is_id(bad), "{bad}");
        }
    }

    #[test]
    fn file_names_are_plain() {
        for ok in [
            "Modlight-0.9.3-45-macos-arm64.zip",
            "Modlight-1.0.0--windows-x64.zip",
            "a",
            "Setup_x64+beta.exe",
            &"x".repeat(128),
        ] {
            assert!(valid_filename(ok), "{ok}");
        }
        for bad in [
            "",
            ".hidden.zip",
            "a/b.zip",
            "a\\b.zip",
            "..",
            "a..b.zip",
            "a b.zip",
            "ż.zip",
            "a\"b",
            "a%2fb",
            "a\nb",
            &"x".repeat(129),
        ] {
            assert!(!valid_filename(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_valid_start_is_normalised() {
        let s = parse_start(&body(valid()), now()).unwrap();
        assert_eq!(s.platform, "macos");
        assert_eq!(s.arch, "arm64");
        assert_eq!(s.version, "0.9.3");
        assert_eq!(s.build, "45");
        assert_eq!(s.built.to_iso_string(), "2026-10-27T10:00:00.000Z");
        assert_eq!(s.size, 1234);
        assert_eq!(s.sha256, "ab".repeat(32));
        assert_eq!(s.filename, "Modlight-0.9.3-45-macos-arm64.zip");
        assert_eq!(s.note, "Faster uploads.\nFixes.");
    }

    #[test]
    fn optional_fields_have_defaults() {
        let mut v = valid();
        let o = v.as_object_mut().unwrap();
        o.remove("arch");
        o.remove("build");
        o.remove("built");
        o.remove("note");
        o.insert("platform".into(), json!(" Windows "));
        let s = parse_start(&body(v), now()).unwrap();
        assert_eq!(
            (s.platform.as_str(), s.arch.as_str(), s.build.as_str(), s.note.as_str()),
            ("windows", "x64", "", "")
        );
        assert_eq!(s.built, now());
        // Build tools' spellings of the architecture, a numeric build, built in ms.
        let mut v = valid();
        v["arch"] = json!("aarch64");
        v["build"] = json!(45);
        v["built"] = json!(1_793_100_000_000_i64);
        let s = parse_start(&body(v), now()).unwrap();
        assert_eq!((s.arch.as_str(), s.build.as_str()), ("arm64", "45"));
        assert_eq!(s.built.millis(), 1_793_100_000_000);
        let mut v = valid();
        v["platform"] = json!("windows");
        v["arch"] = json!("x86_64");
        assert_eq!(parse_start(&body(v), now()).unwrap().arch, "x64");
    }

    #[test]
    fn every_bad_field_has_its_own_code() {
        let cases: Vec<(&str, Value, &str)> = vec![
            ("platform", json!("linux"), "invalid_platform"),
            ("platform", Value::Null, "invalid_platform"),
            ("platform", json!(1), "invalid_platform"),
            ("arch", json!("riscv"), "invalid_arch"),
            ("arch", json!(64), "invalid_arch"),
            ("version", json!(""), "invalid_version"),
            ("version", json!("1.0 beta"), "invalid_version"),
            ("version", json!("x".repeat(65)), "invalid_version"),
            ("build", json!("a/b"), "invalid_build"),
            ("build", json!(-1), "invalid_build"),
            ("build", json!(true), "invalid_build"),
            ("built", json!("yesterday"), "invalid_built"),
            ("built", json!("2030-01-01T00:00:00Z"), "invalid_built"),
            ("built", json!(1_000), "invalid_built"),
            ("built", json!(true), "invalid_built"),
            ("size", json!(0), "invalid_size"),
            ("size", json!(-5), "invalid_size"),
            ("size", json!(1.5), "invalid_size"),
            ("size", json!("1234"), "invalid_size"),
            ("size", json!(MAX_BUILD_SIZE + 1), "too_large"),
            ("sha256", json!("ab"), "invalid_sha256"),
            ("sha256", json!("zz".repeat(32)), "invalid_sha256"),
            ("filename", json!("../x.zip"), "invalid_filename"),
            ("filename", json!("dir/x.zip"), "invalid_filename"),
            ("filename", json!(".x.zip"), "invalid_filename"),
            ("note", json!("bell\u{7}"), "invalid_note"),
            ("note", json!("n".repeat(NOTE_MAX_CHARS + 1)), "invalid_note"),
            ("note", json!(["a"]), "invalid_note"),
        ];
        for (field, value, code) in cases {
            let mut v = valid();
            v[field] = value.clone();
            let err = parse_start(&body(v), now()).unwrap_err();
            assert_eq!(err.code, code, "{field} = {value}");
            let status = if code == "too_large" { StatusCode::PAYLOAD_TOO_LARGE } else { StatusCode::BAD_REQUEST };
            assert_eq!(err.status, status, "{field} = {value}");
        }
        for field in ["platform", "version", "size", "sha256", "filename"] {
            let mut v = valid();
            v.as_object_mut().unwrap().remove(field);
            assert!(parse_start(&body(v), now()).is_err(), "{field} is required");
        }
    }

    #[test]
    fn chunk_numbers() {
        assert_eq!(parse_chunk_index("0"), Some(0));
        assert_eq!(parse_chunk_index("17"), Some(17));
        assert_eq!(parse_chunk_index("99999"), Some(MAX_CHUNK_INDEX));
        for bad in ["", "-1", "01", "1.0", "100000", "x", " 1", "finish"] {
            assert_eq!(parse_chunk_index(bad), None, "{bad}");
        }
        assert_eq!(chunk_index("chunk-3"), Some(3));
        assert_eq!(chunk_index("chunk-3.part-abcdef12"), None);
        assert_eq!(chunk_index(RECORD), None);
    }

    #[test]
    fn keys_and_urls() {
        let id = "0123456789abcdef0123456789abcdef";
        assert_eq!(storage_key(id, "a.zip"), "releases/0123456789abcdef0123456789abcdef/a.zip");
        assert_eq!(storage_prefix(id), "releases/0123456789abcdef0123456789abcdef/");
        assert_eq!(download_path(id, "a.zip"), "/downloads/0123456789abcdef0123456789abcdef/a.zip");
        assert_eq!(content_type_for("Modlight-1-2-macos-arm64.ZIP"), "application/zip");
        assert_eq!(content_type_for("x.bin"), "application/octet-stream");
        assert_eq!(content_type_for("noext"), "application/octet-stream");
    }

    #[test]
    fn the_list_names_the_token_only_to_an_administrator() {
        let list = DownloadsList { platforms: vec![], keep: KEEP, admin: false, publish_token: None };
        assert_eq!(
            serde_json::to_value(&list).unwrap(),
            json!({"platforms": [], "keep": 5, "admin": false, "publishToken": null})
        );
    }

    #[tokio::test]
    async fn chunks_are_written_whole_and_joined_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert_eq!(write_chunk(d, 1, Body::from("world"), 10).await.unwrap(), 5);
        assert!(matches!(write_chunk(d, 2, Body::from("x".repeat(11)), 10).await, Err(ChunkError::TooLarge)));
        assert!(matches!(write_chunk(d, 2, Body::empty(), 10).await, Err(ChunkError::Empty)));
        // Neither refusal left a file behind.
        assert_eq!(chunk_sizes(d).await.unwrap().into_iter().collect::<Vec<_>>(), vec![(1, 5)]);
        assert_eq!(assemble(d).await.unwrap(), Err("chunk 0 is missing".to_string()));
        write_chunk(d, 0, Body::from("hello "), 10).await.unwrap();
        // Sending a chunk again replaces it.
        write_chunk(d, 0, Body::from("Hello "), 10).await.unwrap();
        let joined = assemble(d).await.unwrap().unwrap();
        assert_eq!(joined.size, 11);
        assert_eq!(joined.sha256, hex::encode(Sha256::digest(b"Hello world")));
        assert_eq!(std::fs::read(&joined.path).unwrap(), b"Hello world");
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(assemble(empty.path()).await.unwrap(), Err("no chunk was received".to_string()));
    }

    #[test]
    fn publish_times_only_go_forward() {
        let times: Vec<i64> = (0..100).map(|_| publish_time().millis()).collect();
        assert!(times.windows(2).all(|w| w[0] < w[1]));
        assert!((times[0] - JsDate::now().millis()).abs() < 1000);
    }

    #[test]
    fn one_finish_at_a_time() {
        let a = FinishGuard::acquire("u1").unwrap();
        assert!(FinishGuard::acquire("u1").is_none());
        let b = FinishGuard::acquire("u2");
        assert!(b.is_some());
        drop(a);
        assert!(FinishGuard::acquire("u1").is_some());
    }
}
