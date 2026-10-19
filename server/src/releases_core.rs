//! Port of `lib/releases-core.ts`: the release channel of the companion app,
//! without the `server-only` guard.
//!
//! Upstream split it from `lib/releases.ts` for one concrete reason: the
//! publish script is bundled by esbuild with `--packages=external`, and
//! `server-only` resolves only inside Next's bundler. The rules live here and
//! [`crate::releases`] is the guarded shim over them, as upstream.
//!
//! Published builds of the companion app.
//!
//! The binary is NOT a new kind of object: it is an ordinary upload in an
//! administrator's library, and this module only records WHICH upload is the
//! current release. Everything a download needs (streaming, Range requests,
//! ETags, the storage driver, the quota it occupies) therefore already works,
//! and a release channel costs one settings row instead of a subsystem.
//!
//! The integrity hash is not computed here either. `uploads.checksum` is the
//! sha256 the upload path already calculated while the bytes streamed past, so
//! the manifest can promise it for free, which happens to be exactly what the
//! companion's updater verifies before it swaps a binary in.
//!
//! The database work is behind [`ReleaseDb`] so the upstream unit tests (which
//! mock `@/lib/db` and `@/lib/settings`) port as they are; [`PgReleaseDb`] is
//! the real one.

use serde::Serialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::{Config, is_js_whitespace, js_trim};
use crate::db::types::JsDate;
use crate::media::urls::{raw_url, tagless_raw_url, user_base_url};
use crate::settings::keys::RELEASE_APP;

/// What the settings row stores. Deliberately small: a pointer plus prose.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoredRelease {
    pub version: String,
    /// `uploads.code` of the binary. The row is the source of truth for size/hash.
    pub code: String,
    /// `JSON.stringify` drops an `undefined` key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(rename = "publishedAt")]
    pub published_at: String,
}

/// What a client (or the download page) is told.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseManifest {
    pub name: String,
    pub version: String,
    /// Absolute URL of the binary itself.
    pub url: String,
    pub filename: String,
    pub size: i64,
    /// sha256, lower-case hex. Null only for files uploaded before checksums.
    pub sha256: Option<String>,
    pub notes: Option<String>,
    pub published_at: String,
}

pub const APP_NAME: &str = "Modlight App";

/// The largest value a component of `System.Version` (an Int32) can hold.
const INT32_MAX: f64 = 2_147_483_647.0;

/// A version string, split into numbers, by the desktop app's own rules.
///
/// Deliberately a re-statement of `ModlightRelease.TryParseVersion` in the
/// client: a leading `v` is allowed, everything else must be digits and dots,
/// and `System.Version` needs at least `Major.Minor`, so the plain "8" that
/// looks perfectly reasonable in a form is a version NO installed copy can read,
/// and a channel published with it answers `CheckFailed` forever while the panel
/// cheerfully reports a release.
fn parse_version(raw: &str) -> Option<Vec<u64>> {
    let value = js_trim(raw);
    let value = value.strip_prefix(['v', 'V']).unwrap_or(value);
    // /^\d+(\.\d+){1,3}$/ (JavaScript's `\d` is ASCII only).
    let parts: Vec<&str> = value.split('.').collect();
    if !(2..=4).contains(&parts.len()) || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let mut out = Vec::with_capacity(4);
    for part in parts {
        // `Number(part)`; each component of System.Version is an Int32 and must
        // be non-negative (`Number.isSafeInteger(n) && n >= 0 && n <= 2147483647`).
        let n: f64 = part.parse().unwrap_or(f64::NAN);
        if !n.is_finite() || n.fract() != 0.0 || !(0.0..=INT32_MAX).contains(&n) {
            return None;
        }
        out.push(n as u64);
    }
    Some(out)
}

/// The same version in one comparable shape, so 1.2, 1.2.0 and 1.2.0.0 are one thing.
fn version_key(raw: &str) -> Option<String> {
    let mut parts = parse_version(raw)?;
    while parts.len() < 4 {
        parts.push(0);
    }
    Some(parts.iter().map(u64::to_string).collect::<Vec<_>>().join("."))
}

/// The version as it will be stored and shown, or `None` if no client could read it.
///
/// The `v` is dropped rather than kept: every installed copy strips it before
/// comparing, so keeping it would leave the manifest saying one thing and every
/// updater on earth acting on another.
pub fn normalize_release_version(raw: &str) -> Option<String> {
    parse_version(raw)?;
    let value = js_trim(raw);
    Some(value.strip_prefix(['v', 'V']).unwrap_or(value).to_string())
}

/// The version the installer's own file name claims, or `None` when it claims none.
///
/// This is the only fact about the binary that is free. Inno writes the name from
/// `OutputBaseFilename={#MyAppName}-{#MyAppVersion}-setup-{#Platform}`, and
/// `{#MyAppVersion}` is read out of the built exe's ProductVersion, so the name
/// carries the number that the app will report about itself after installing.
/// Which is exactly the number the typed field has to agree with, and the reason
/// a typo there is not cosmetic: the app compares what it IS against what the
/// channel OFFERS, so an offer of 0.7.1 backed by a 0.7.0 binary is an update
/// that installs and then asks to be installed again, at every launch and every
/// hour after that, with nothing anywhere that ever says "you already have this".
pub fn version_from_file_name(filename: &str) -> Option<String> {
    // filename.replace(/\.[^.]*$/, ""): the last dot and everything after it.
    let base = match filename.rfind('.') {
        Some(dot) => &filename[..dot],
        None => filename,
    };
    // base.split(/[-_\s]+/): splitting on single separators only adds empty
    // tokens between consecutive ones, and an empty token is never a version.
    base.split(|c: char| c == '-' || c == '_' || is_js_whitespace(c)).find_map(normalize_release_version)
}

/// Two version strings that mean the same thing to an installed client.
///
/// `System.Version` pads whatever it is not told, so 0.7, 0.7.0 and 0.7.0.0 are
/// one version while string comparison calls them three. Exported because the
/// publish script runs the same name-versus-version check BEFORE it uploads
/// anything, and a second spelling of this comparison would eventually disagree
/// with this one.
pub fn same_release_version(a: &str, b: &str) -> bool {
    match version_key(a) {
        Some(left) => version_key(b).as_deref() == Some(left.as_str()),
        None => false,
    }
}

/// A sha256 in exactly the shape `uploads.checksum` stores and the manifest
/// promises: 64 lower-case hex characters (`typeof value === "string"`, so
/// `None` is refused).
///
/// Not a formatting nicety. The desktop updater refuses an update whose manifest
/// carries no digest (fail-closed, and correctly so), so "there is a digest" is
/// the difference between a release that installs itself everywhere and one that
/// no installed copy will ever accept, which is what two published releases
/// turned out to be.
pub fn is_release_digest(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.len() == 64 && v.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// The name a release is published under.
///
/// `finalizeUpload` records whatever name the caller hands it, so a caller that
/// hands over the path it happened to stage the build at publishes a release
/// called `rel.exe`. That is not only ugly on the download page:
/// `version_from_file_name` is the one free second opinion on the typed version
/// number, and a temp name silences it, so the mistyped-version failure this
/// module goes to some length to catch simply stops being catchable.
///
/// Returns `None` when nothing usable survives, so the caller refuses instead of
/// inventing a name.
pub fn release_file_name(source_path: &str, override_name: Option<&str>) -> Option<String> {
    let raw = js_trim(override_name.unwrap_or(source_path));
    if raw.is_empty() {
        return None;
    }
    // Separators are stripped rather than rejected: the default IS a path, and an
    // override typed with one is a slip rather than an attack. It still leaves
    // here as a bare name, because it ends up in a database column and in a
    // Content-Disposition header.
    let name = raw.split(['\\', '/']).rfind(|s| !s.is_empty()).unwrap_or("");
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    // Control characters would travel into that header; nothing legitimate has them.
    if name.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}') {
        return None;
    }
    // `name.length` counts UTF-16 units.
    if name.encode_utf16().count() > 255 { None } else { Some(name.to_string()) }
}

/// The stored pointer, or `None` for anything that is not one.
fn parse(raw: Option<&str>) -> Option<StoredRelease> {
    let raw = raw.filter(|r| !r.is_empty())?;
    let value: Value = serde_json::from_str(raw).ok()?;
    // A non-object has no `version` property: `typeof undefined !== "string"`.
    let object = value.as_object()?;
    let version = object.get("version")?.as_str()?;
    let code = object.get("code")?.as_str()?;
    if version.is_empty() || code.is_empty() {
        return None;
    }
    Some(StoredRelease {
        version: version.to_string(),
        code: code.to_string(),
        notes: object.get("notes").and_then(Value::as_str).map(str::to_string),
        published_at: object.get("publishedAt").and_then(Value::as_str).unwrap_or("").to_string(),
    })
}

/// JavaScript truthiness of a nullable text column (`""` is falsy).
fn truthy(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.is_empty())
}

// ── Database ────────────────────────────────────────────────────────────────

/// The upload `publishRelease` inspects.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct PublishRow {
    pub id: Uuid,
    pub published: bool,
    pub password_hash: Option<String>,
    pub filename: String,
}

/// The upload `currentRelease` resolves the pointer against.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct ManifestRow {
    pub code: String,
    pub ext: String,
    pub filename: String,
    pub size: i64,
    pub checksum: Option<String>,
    pub published: bool,
    pub owner_id: Option<String>,
    pub expires_at: Option<JsDate>,
    pub password_hash: Option<String>,
}

/// The owner columns the download URL is built from.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct OwnerRow {
    pub user_code: Option<String>,
    pub preferred_domain: Option<String>,
    pub skip_tag_in_url: Option<bool>,
}

/// The statements the release channel issues.
pub trait ReleaseDb {
    /// `getSetting(key)`
    fn get_setting(&self, key: &str) -> impl Future<Output = anyhow::Result<Option<String>>> + Send;
    /// `setSetting(key, value)`
    fn set_setting(&self, key: &str, value: Option<&str>) -> impl Future<Output = anyhow::Result<()>> + Send;
    /// `SELECT id, published, password_hash, filename FROM uploads WHERE code = $1
    /// AND deleted_at IS NULL LIMIT 1`
    fn publish_row(&self, code: &str) -> impl Future<Output = anyhow::Result<Option<PublishRow>>> + Send;
    /// `SELECT code, ext, filename, size, checksum, published, owner_id,
    /// expires_at, password_hash FROM uploads WHERE code = $1 AND deleted_at IS
    /// NULL LIMIT 1`
    fn manifest_row(&self, code: &str) -> impl Future<Output = anyhow::Result<Option<ManifestRow>>> + Send;
    /// `SELECT user_code, preferred_domain, skip_tag_in_url FROM "user" WHERE id = $1 LIMIT 1`
    fn owner(&self, id: &str) -> impl Future<Output = anyhow::Result<Option<OwnerRow>>> + Send;
    /// `SELECT checksum FROM uploads WHERE code = $1 AND deleted_at IS NULL
    /// LIMIT 1`: `None` for no row, `Some(None)` for a NULL checksum.
    fn checksum_row(&self, code: &str) -> impl Future<Output = anyhow::Result<Option<Option<String>>>> + Send;
}

/// The real statements.
#[derive(Clone, Copy)]
pub struct PgReleaseDb<'a>(pub &'a PgPool);

impl ReleaseDb for PgReleaseDb<'_> {
    async fn get_setting(&self, key: &str) -> anyhow::Result<Option<String>> {
        Ok(crate::settings::get_setting(self.0, key).await?)
    }

    async fn set_setting(&self, key: &str, value: Option<&str>) -> anyhow::Result<()> {
        Ok(crate::settings::set_setting(self.0, key, value).await?)
    }

    async fn publish_row(&self, code: &str) -> anyhow::Result<Option<PublishRow>> {
        Ok(sqlx::query_as::<_, PublishRow>(
            "SELECT \"id\", \"published\", \"password_hash\", \"filename\" FROM \"uploads\" WHERE (\"uploads\".\"code\" = $1 AND \"uploads\".\"deleted_at\" IS NULL) LIMIT 1",
        )
        .bind(code)
        .fetch_optional(self.0)
        .await?)
    }

    async fn manifest_row(&self, code: &str) -> anyhow::Result<Option<ManifestRow>> {
        Ok(sqlx::query_as::<_, ManifestRow>(
            "SELECT \"code\", \"ext\", \"filename\", \"size\", \"checksum\", \"published\", \"owner_id\", \"expires_at\", \"password_hash\" FROM \"uploads\" WHERE (\"uploads\".\"code\" = $1 AND \"uploads\".\"deleted_at\" IS NULL) LIMIT 1",
        )
        .bind(code)
        .fetch_optional(self.0)
        .await?)
    }

    async fn owner(&self, id: &str) -> anyhow::Result<Option<OwnerRow>> {
        Ok(sqlx::query_as::<_, OwnerRow>(
            "SELECT \"user_code\", \"preferred_domain\", \"skip_tag_in_url\" FROM \"user\" WHERE \"user\".\"id\" = $1 LIMIT 1",
        )
        .bind(id)
        .fetch_optional(self.0)
        .await?)
    }

    async fn checksum_row(&self, code: &str) -> anyhow::Result<Option<Option<String>>> {
        Ok(sqlx::query_scalar::<_, Option<String>>(
            "SELECT \"checksum\" FROM \"uploads\" WHERE (\"uploads\".\"code\" = $1 AND \"uploads\".\"deleted_at\" IS NULL) LIMIT 1",
        )
        .bind(code)
        .fetch_optional(self.0)
        .await?)
    }
}

// ── The channel ─────────────────────────────────────────────────────────────

/// The current release, resolved against the upload it names.
///
/// Returns `None` when nothing is published OR when the file behind it is gone
/// (deleted, trashed, or its account removed). That second case matters: a
/// manifest naming a file that 404s would send every installed copy of the app
/// into a failed-update loop, so "the pointer is dangling" has to read as "there
/// is no release", not as a release with a broken link.
pub async fn current_release_with<D: ReleaseDb>(db: &D, config: &Config) -> anyhow::Result<Option<ReleaseManifest>> {
    let Some(stored) = parse(db.get_setting(RELEASE_APP).await?.as_deref()) else {
        return Ok(None);
    };
    let Some(row) = db.manifest_row(&stored.code).await? else {
        return Ok(None);
    };
    let Some(owner_id) = row.owner_id.as_deref().filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    // A release has to be fetchable by a machine with no session and no password.
    if !row.published || truthy(row.password_hash.as_deref()) || row.expires_at.is_some() {
        return Ok(None);
    }
    let Some(owner) = db.owner(owner_id).await? else {
        return Ok(None);
    };

    let base = user_base_url(config, owner.preferred_domain.as_deref());
    let tag = owner.user_code.as_deref().unwrap_or("u");
    let url = if owner.skip_tag_in_url == Some(true) {
        tagless_raw_url(&row.code, &row.ext, &base)
    } else {
        raw_url(tag, &row.code, &row.ext, &base)
    };

    Ok(Some(ReleaseManifest {
        name: APP_NAME.to_string(),
        version: stored.version,
        url,
        filename: row.filename,
        size: row.size,
        sha256: row.checksum,
        notes: stored.notes,
        published_at: stored.published_at,
    }))
}

/// [`current_release_with`] against the database.
pub async fn current_release(db: &PgPool, config: &Config) -> anyhow::Result<Option<ReleaseManifest>> {
    current_release_with(&PgReleaseDb(db), config).await
}

/// `PublishResult`
#[derive(Debug, Clone, PartialEq)]
pub enum PublishResult {
    Ok(ReleaseManifest),
    NoVersion,
    BadVersion,
    NotFound,
    NotPublic,
    VersionMismatch { file_version: String, filename: String },
}

impl PublishResult {
    pub fn is_ok(&self) -> bool {
        matches!(self, PublishResult::Ok(_))
    }

    /// The `reason` of a refusal.
    pub fn reason(&self) -> Option<&'static str> {
        match self {
            PublishResult::Ok(_) => None,
            PublishResult::NoVersion => Some("no-version"),
            PublishResult::BadVersion => Some("bad-version"),
            PublishResult::NotFound => Some("not-found"),
            PublishResult::NotPublic => Some("not-public"),
            PublishResult::VersionMismatch { .. } => Some("version-mismatch"),
        }
    }

    /// The object upstream returns: `{ok: true, manifest}`, `{ok: false,
    /// reason}` or `{ok: false, reason: "version-mismatch", fileVersion, filename}`.
    pub fn to_json(&self) -> Value {
        match self {
            PublishResult::Ok(manifest) => json!({ "ok": true, "manifest": manifest }),
            PublishResult::VersionMismatch { file_version, filename } => json!({
                "ok": false,
                "reason": "version-mismatch",
                "fileVersion": file_version,
                "filename": filename,
            }),
            refused => json!({ "ok": false, "reason": refused.reason() }),
        }
    }
}

/// `input.notes` as `publishRelease` sees it: `input.notes?.trim()` runs only
/// when the pointer is about to be written, so a value `.trim()` cannot be
/// called on throws THERE, after every refusal had its chance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseNotes<'a> {
    /// `undefined` or `null`.
    Absent,
    Text(&'a str),
    /// A number, boolean, object, ...: a TypeError upstream.
    NotAString,
}

/// The input of `publishRelease`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishInput<'a> {
    pub version: &'a str,
    pub code: &'a str,
    pub notes: ReleaseNotes<'a>,
    /// Publish even when the file name disagrees with the version.
    ///
    /// There are honest reasons for a disagreement (a hand-renamed file, an
    /// installer whose name carries no version at all), so this is a
    /// confirmation rather than a wall. It has to be asked for explicitly, once,
    /// per publish.
    pub allow_version_mismatch: bool,
}

/// Point the release channel at an upload.
///
/// Validated eagerly rather than at download time: an administrator who mistypes
/// a code should hear about it while they are looking at the form, not when the
/// first user's updater fails at three in the morning. That sentence was already
/// here and applied only to `code`; the version field, the other half of the same
/// form, was free text with a single check that it was not empty.
///
/// It is not free text. It is one half of a comparison whose other half is
/// compiled into the binary (`Directory.build.props` -> `AssemblyInformationalVersion`
/// -> `Application.ProductVersion`), and the app decides whether to offer an
/// update by comparing the two. Offering 0.7.1 for a binary that installs as
/// 0.7.0 therefore does not fail loudly: it succeeds, forever. So the typo is
/// caught here, where somebody can still fix it.
pub async fn publish_release_with<D: ReleaseDb>(
    db: &D,
    config: &Config,
    input: PublishInput<'_>,
) -> anyhow::Result<PublishResult> {
    let raw = js_trim(input.version);
    let code = js_trim(input.code);
    if raw.is_empty() || code.is_empty() {
        return Ok(PublishResult::NoVersion);
    }

    let Some(version) = normalize_release_version(raw) else {
        return Ok(PublishResult::BadVersion);
    };

    let Some(row) = db.publish_row(code).await? else {
        return Ok(PublishResult::NotFound);
    };
    if !row.published || truthy(row.password_hash.as_deref()) {
        return Ok(PublishResult::NotPublic);
    }

    // The binary's own name is the cheapest second opinion available, and the
    // live manifest carries one: Modlight-0.7.0-setup-x64.exe. Compared on the
    // numbers, so 0.7 and 0.7.0.0 do not read as a disagreement.
    if let Some(file_version) = version_from_file_name(&row.filename)
        && !input.allow_version_mismatch
        && !same_release_version(&file_version, &version)
    {
        return Ok(PublishResult::VersionMismatch { file_version, filename: row.filename });
    }

    let notes = match input.notes {
        ReleaseNotes::Absent => None,
        ReleaseNotes::Text(text) => Some(js_trim(text)).filter(|t| !t.is_empty()).map(str::to_string),
        ReleaseNotes::NotAString => anyhow::bail!("input.notes?.trim is not a function"),
    };
    let stored = StoredRelease { version, code: code.to_string(), notes, published_at: JsDate::now().to_iso_string() };
    db.set_setting(RELEASE_APP, Some(&serde_json::to_string(&stored)?)).await?;

    // currentRelease re-reads and re-validates; if it says no, the write was
    // pointless and the caller must not be told it worked.
    Ok(match current_release_with(db, config).await? {
        Some(manifest) => PublishResult::Ok(manifest),
        None => PublishResult::NotPublic,
    })
}

/// [`publish_release_with`] against the database.
pub async fn publish_release(db: &PgPool, config: &Config, input: PublishInput<'_>) -> anyhow::Result<PublishResult> {
    publish_release_with(&PgReleaseDb(db), config, input).await
}

/// `VerifiedPublishResult`
#[derive(Debug, Clone, PartialEq)]
pub enum VerifiedPublishResult {
    Published(PublishResult),
    NoDigest,
    DigestMismatch { stored: String, expected: String },
}

/// Publish, but only once the upload really carries the digest the publisher
/// hashed out of the bytes it handed over.
///
/// `publish_release` cannot ask this of itself. An administrator republishing a
/// file that predates checksums has a legitimate reason to move the pointer, and
/// the panel's own result union is closed, so a new refusal reason there would
/// break the form rather than the mistake. A script cutting a release has no such
/// excuse: it is holding the file, so it can hash it.
///
/// The gate runs BEFORE `publish_release`, so a refusal here writes nothing at
/// all and the channel keeps pointing at the previous, working release.
pub async fn publish_verified_release_with<D: ReleaseDb>(
    db: &D,
    config: &Config,
    input: PublishInput<'_>,
    expected_digest: &str,
) -> anyhow::Result<VerifiedPublishResult> {
    let code = js_trim(input.code);
    if js_trim(input.version).is_empty() || code.is_empty() {
        return Ok(VerifiedPublishResult::Published(PublishResult::NoVersion));
    }
    // Checked before the query, so a caller with nothing to compare cannot even
    // learn whether the code exists.
    if !is_release_digest(Some(expected_digest)) {
        return Ok(VerifiedPublishResult::NoDigest);
    }

    let Some(checksum) = db.checksum_row(code).await? else {
        return Ok(VerifiedPublishResult::Published(PublishResult::NotFound));
    };
    let Some(stored) = checksum.filter(|c| is_release_digest(Some(c))) else {
        return Ok(VerifiedPublishResult::NoDigest);
    };
    // Different bytes: the pointer would name a file this publisher never hashed,
    // and the manifest would promise a digest that belongs to something else.
    if stored != expected_digest {
        return Ok(VerifiedPublishResult::DigestMismatch { stored, expected: expected_digest.to_string() });
    }

    Ok(VerifiedPublishResult::Published(publish_release_with(db, config, input).await?))
}

/// Withdraw the current release, so the page and the manifest report none.
pub async fn unpublish_release_with<D: ReleaseDb>(db: &D) -> anyhow::Result<()> {
    db.set_setting(RELEASE_APP, None).await
}

/// [`unpublish_release_with`] against the database.
pub async fn unpublish_release(db: &PgPool) -> anyhow::Result<()> {
    unpublish_release_with(&PgReleaseDb(db)).await
}

#[cfg(test)]
pub(crate) mod test_support {
    //! The fake `@/lib/db` + `@/lib/settings` of the upstream release tests:
    //! answers queued per statement, settings in a map, every query counted.
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub struct FakeReleaseDb {
        pub settings: Mutex<HashMap<String, Option<String>>>,
        pub publish_rows: Mutex<VecDeque<PublishRow>>,
        pub manifest_rows: Mutex<VecDeque<ManifestRow>>,
        pub owners: Mutex<VecDeque<OwnerRow>>,
        pub checksums: Mutex<VecDeque<Option<String>>>,
        pub queries: Mutex<u32>,
    }

    impl FakeReleaseDb {
        pub fn setting(&self, key: &str) -> Option<Option<String>> {
            self.settings.lock().unwrap().get(key).cloned()
        }

        fn counted(&self) {
            *self.queries.lock().unwrap() += 1;
        }

        /// The upload row `publishRelease` looks at, and then the two
        /// `currentRelease` reads (`queueHappyPath` upstream).
        pub fn queue_happy_path(&self, filename: &str, checksum: Option<&str>) {
            self.publish_rows.lock().unwrap().push_back(PublishRow {
                id: Uuid::nil(),
                published: true,
                password_hash: None,
                filename: filename.to_string(),
            });
            self.manifest_rows.lock().unwrap().push_back(ManifestRow {
                code: "saluy6".into(),
                ext: "exe".into(),
                filename: filename.to_string(),
                size: 82_317_446,
                checksum: checksum.map(str::to_string),
                published: true,
                owner_id: Some("admin".into()),
                expires_at: None,
                password_hash: None,
            });
            self.owners.lock().unwrap().push_back(OwnerRow {
                user_code: Some("tag".into()),
                preferred_domain: None,
                skip_tag_in_url: Some(false),
            });
        }
    }

    impl ReleaseDb for FakeReleaseDb {
        async fn get_setting(&self, key: &str) -> anyhow::Result<Option<String>> {
            Ok(self.settings.lock().unwrap().get(key).cloned().flatten())
        }
        async fn set_setting(&self, key: &str, value: Option<&str>) -> anyhow::Result<()> {
            self.settings.lock().unwrap().insert(key.to_string(), value.map(str::to_string));
            Ok(())
        }
        async fn publish_row(&self, _code: &str) -> anyhow::Result<Option<PublishRow>> {
            self.counted();
            Ok(self.publish_rows.lock().unwrap().pop_front())
        }
        async fn manifest_row(&self, _code: &str) -> anyhow::Result<Option<ManifestRow>> {
            self.counted();
            Ok(self.manifest_rows.lock().unwrap().pop_front())
        }
        async fn owner(&self, _id: &str) -> anyhow::Result<Option<OwnerRow>> {
            self.counted();
            Ok(self.owners.lock().unwrap().pop_front())
        }
        async fn checksum_row(&self, _code: &str) -> anyhow::Result<Option<Option<String>>> {
            self.counted();
            Ok(self.checksums.lock().unwrap().pop_front())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::test_support::FakeReleaseDb;
    use super::*;

    fn config() -> Config {
        let mut vars = HashMap::new();
        vars.insert("APP_URL".to_string(), "https://host.example.com".to_string());
        Config::from_env_map(&vars).unwrap()
    }

    fn input<'a>(version: &'a str, code: &'a str) -> PublishInput<'a> {
        PublishInput { version, code, notes: ReleaseNotes::Absent, allow_version_mismatch: false }
    }

    const INSTALLER: &str = "Modlight-0.7.0-setup-x64.exe";

    // ── tests/unit/release-version.test.ts ──

    #[test]
    fn normalize_release_version_accepts_what_system_version_parses() {
        // System.Version needs at least Major.Minor, so a bare number is a
        // version no installed client can parse.
        for good in ["0.7", "0.7.0", "0.7.0.1", "v0.7.0", " 1.2.3 ", "10.20.30.40"] {
            assert!(normalize_release_version(good).is_some(), "{good:?}");
        }
        for bad in ["", "8", "0.7.1-beta", "0.7.1+build", "seven", "0.7.0.1.2", "0..7", "-1.0"] {
            assert_eq!(normalize_release_version(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn drops_the_v_because_every_client_drops_it_before_comparing() {
        assert_eq!(normalize_release_version("v0.7.0").as_deref(), Some("0.7.0"));
        assert_eq!(normalize_release_version(" V1.2 ").as_deref(), Some("1.2"));
    }

    #[test]
    fn each_component_is_a_non_negative_int32() {
        assert_eq!(normalize_release_version("2147483647.0").as_deref(), Some("2147483647.0"));
        assert_eq!(normalize_release_version("2147483648.0"), None);
        assert_eq!(normalize_release_version("99999999999999999999.1"), None);
        // Leading zeros are digits: kept in the spelling, ignored in comparison.
        assert_eq!(normalize_release_version("01.2").as_deref(), Some("01.2"));
        assert!(same_release_version("01.2", "1.2.0.0"));
        // JavaScript's \d is ASCII only.
        assert_eq!(normalize_release_version("１.2"), None);
        assert_eq!(normalize_release_version("vv1.2"), None);
    }

    #[test]
    fn reads_the_version_inno_wrote_into_the_name() {
        assert_eq!(version_from_file_name(INSTALLER).as_deref(), Some("0.7.0"));
    }

    #[test]
    fn is_not_fooled_by_the_architecture_suffix() {
        assert_eq!(version_from_file_name("Modlight-1.2.3.4-setup-x64.exe").as_deref(), Some("1.2.3.4"));
    }

    #[test]
    fn says_nothing_when_the_name_carries_no_version() {
        assert_eq!(version_from_file_name("Modlight-setup.exe"), None);
        assert_eq!(version_from_file_name("installer.exe"), None);
        // Only the LAST extension goes: "app-1.2" is "app-1".
        assert_eq!(version_from_file_name("app-1.2"), None);
        assert_eq!(version_from_file_name("Modlight 0.7 setup.exe").as_deref(), Some("0.7"));
        assert_eq!(version_from_file_name("Modlight__v0.7.0--x64.exe").as_deref(), Some("0.7.0"));
    }

    #[test]
    fn same_version_pads_like_system_version() {
        assert!(same_release_version("0.7", "0.7.0.0"));
        assert!(same_release_version("v0.7.0", "0.7"));
        assert!(!same_release_version("0.7.1", "0.7.0"));
        assert!(!same_release_version("8", "8"));
    }

    #[tokio::test]
    async fn refuses_a_version_no_client_could_parse_before_touching_the_database() {
        let db = FakeReleaseDb::default();
        let result = publish_release_with(&db, &config(), input("0.7.1-beta", "saluy6")).await.unwrap();
        assert_eq!(result, PublishResult::BadVersion);
        assert_eq!(*db.queries.lock().unwrap(), 0);
        assert_eq!(db.setting(RELEASE_APP), None);
    }

    #[tokio::test]
    async fn still_refuses_an_empty_version_the_old_way() {
        let db = FakeReleaseDb::default();
        assert_eq!(
            publish_release_with(&db, &config(), input("  ", "saluy6")).await.unwrap(),
            PublishResult::NoVersion
        );
        assert_eq!(publish_release_with(&db, &config(), input("0.7", " ")).await.unwrap(), PublishResult::NoVersion);
    }

    #[tokio::test]
    async fn refuses_a_version_the_installers_own_name_contradicts_and_writes_nothing() {
        let db = FakeReleaseDb::default();
        db.publish_rows.lock().unwrap().push_back(PublishRow {
            id: Uuid::nil(),
            published: true,
            password_hash: None,
            filename: INSTALLER.into(),
        });
        let result = publish_release_with(&db, &config(), input("0.7.1", "saluy6")).await.unwrap();
        assert_eq!(result, PublishResult::VersionMismatch { file_version: "0.7.0".into(), filename: INSTALLER.into() });
        assert_eq!(
            result.to_json(),
            json!({ "ok": false, "reason": "version-mismatch", "fileVersion": "0.7.0", "filename": INSTALLER })
        );
        assert_eq!(db.setting(RELEASE_APP), None);
    }

    #[tokio::test]
    async fn publishes_the_same_mismatch_once_it_is_explicitly_confirmed() {
        let db = FakeReleaseDb::default();
        db.queue_happy_path(INSTALLER, Some(&"77".repeat(32)));
        let mut i = input("0.7.1", "saluy6");
        i.allow_version_mismatch = true;
        let result = publish_release_with(&db, &config(), i).await.unwrap();
        assert!(result.is_ok());
        assert!(db.setting(RELEASE_APP).flatten().unwrap().contains("\"version\":\"0.7.1\""));
    }

    #[tokio::test]
    async fn treats_0_7_and_0_7_0_0_as_one_version_rather_than_a_disagreement() {
        let db = FakeReleaseDb::default();
        db.queue_happy_path("Modlight-0.7-setup-x64.exe", None);
        assert!(publish_release_with(&db, &config(), input("0.7.0.0", "saluy6")).await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn says_nothing_about_the_name_when_the_name_says_nothing() {
        let db = FakeReleaseDb::default();
        db.queue_happy_path("Modlight-setup.exe", None);
        assert!(publish_release_with(&db, &config(), input("0.7.0", "saluy6")).await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn stores_the_version_without_its_v_so_the_manifest_matches_what_clients_compare() {
        let db = FakeReleaseDb::default();
        db.queue_happy_path(INSTALLER, None);
        let result = publish_release_with(&db, &config(), input("v0.7.0", "saluy6")).await.unwrap();
        let PublishResult::Ok(manifest) = result else { panic!("expected a release") };
        assert_eq!(manifest.version, "0.7.0");
        assert!(db.setting(RELEASE_APP).flatten().unwrap().contains("\"version\":\"0.7.0\""));
    }

    #[tokio::test]
    async fn the_stored_pointer_and_the_manifest() {
        let db = FakeReleaseDb::default();
        db.queue_happy_path(INSTALLER, Some(&"77".repeat(32)));
        let mut i = input(" 0.7.0 ", "  saluy6 ");
        i.notes = ReleaseNotes::Text("  Faster.  ");
        let PublishResult::Ok(manifest) = publish_release_with(&db, &config(), i).await.unwrap() else {
            panic!("expected a release")
        };
        let stored = db.setting(RELEASE_APP).flatten().unwrap();
        let value: Value = serde_json::from_str(&stored).unwrap();
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["version", "code", "notes", "publishedAt"]);
        assert_eq!(value["code"], "saluy6");
        assert_eq!(value["notes"], "Faster.");
        assert_eq!(manifest.url, "https://host.example.com/tag/saluy6/raw.exe");
        assert_eq!(manifest.notes.as_deref(), Some("Faster."));
        assert_eq!(manifest.sha256, Some("77".repeat(32)));
        assert_eq!(manifest.size, 82_317_446);
        assert_eq!(
            serde_json::to_value(&manifest).unwrap().as_object().unwrap().keys().collect::<Vec<_>>(),
            ["name", "version", "url", "filename", "size", "sha256", "notes", "publishedAt"]
        );
        // Blank notes are not stored at all (`notes?.trim() || undefined`).
        db.queue_happy_path(INSTALLER, None);
        let mut i = input("0.7.0", "saluy6");
        i.notes = ReleaseNotes::Text("   ");
        publish_release_with(&db, &config(), i).await.unwrap();
        assert!(!db.setting(RELEASE_APP).flatten().unwrap().contains("notes"));
    }

    #[tokio::test]
    async fn a_notes_value_that_is_not_a_string_throws_only_when_the_pointer_is_written() {
        let db = FakeReleaseDb::default();
        let mut i = input("0.7.0", "missing");
        i.notes = ReleaseNotes::NotAString;
        assert_eq!(publish_release_with(&db, &config(), i).await.unwrap(), PublishResult::NotFound);
        db.queue_happy_path(INSTALLER, None);
        assert!(publish_release_with(&db, &config(), i).await.is_err());
        assert_eq!(db.setting(RELEASE_APP), None);
    }

    #[tokio::test]
    async fn unpublished_or_password_protected_uploads_are_not_public() {
        let db = FakeReleaseDb::default();
        for (published, password) in [(false, None), (true, Some("scrypt$x$y"))] {
            db.publish_rows.lock().unwrap().push_back(PublishRow {
                id: Uuid::nil(),
                published,
                password_hash: password.map(str::to_string),
                filename: INSTALLER.into(),
            });
            assert_eq!(
                publish_release_with(&db, &config(), input("0.7.0", "saluy6")).await.unwrap(),
                PublishResult::NotPublic
            );
        }
        assert_eq!(db.setting(RELEASE_APP), None);
    }

    #[tokio::test]
    async fn an_expiring_upload_is_written_and_then_reported_not_public() {
        // currentRelease refuses what publishRelease let through: the pointer
        // stays written (upstream behaviour), the caller hears "not-public".
        let db = FakeReleaseDb::default();
        db.queue_happy_path(INSTALLER, None);
        db.manifest_rows.lock().unwrap()[0].expires_at = Some(JsDate::now());
        assert_eq!(
            publish_release_with(&db, &config(), input("0.7.0", "saluy6")).await.unwrap(),
            PublishResult::NotPublic
        );
        assert!(db.setting(RELEASE_APP).flatten().is_some());
    }

    // ── currentRelease ──

    fn stored(db: &FakeReleaseDb, value: &str) {
        db.settings.lock().unwrap().insert(RELEASE_APP.into(), Some(value.into()));
    }

    #[tokio::test]
    async fn no_pointer_or_a_malformed_one_is_no_release() {
        for raw in [
            "",
            "not json",
            "null",
            "[]",
            "5",
            "{\"version\":\"1.0\"}",
            "{\"version\":\"\",\"code\":\"a\"}",
            "{\"version\":1,\"code\":\"a\"}",
        ] {
            let db = FakeReleaseDb::default();
            stored(&db, raw);
            assert_eq!(current_release_with(&db, &config()).await.unwrap(), None, "{raw}");
            assert_eq!(*db.queries.lock().unwrap(), 0, "{raw}");
        }
        let db = FakeReleaseDb::default();
        assert_eq!(current_release_with(&db, &config()).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_dangling_pointer_is_no_release() {
        let pointer = "{\"version\":\"1.0\",\"code\":\"saluy6\"}";
        // No live upload with that code.
        let db = FakeReleaseDb::default();
        stored(&db, pointer);
        assert_eq!(current_release_with(&db, &config()).await.unwrap(), None);
        // An orphaned upload: its account was removed.
        let db = FakeReleaseDb::default();
        stored(&db, pointer);
        db.queue_happy_path(INSTALLER, None);
        db.manifest_rows.lock().unwrap()[0].owner_id = None;
        assert_eq!(current_release_with(&db, &config()).await.unwrap(), None);
        // The owner row is gone.
        let db = FakeReleaseDb::default();
        stored(&db, pointer);
        db.queue_happy_path(INSTALLER, None);
        db.owners.lock().unwrap().clear();
        assert_eq!(current_release_with(&db, &config()).await.unwrap(), None);
    }

    #[tokio::test]
    async fn stored_fields_pass_through_and_the_owner_shapes_the_url() {
        let db = FakeReleaseDb::default();
        stored(&db, "{\"version\":\"v9\",\"code\":\"saluy6\",\"notes\":5,\"publishedAt\":7}");
        db.queue_happy_path(INSTALLER, Some(""));
        {
            let mut owners = db.owners.lock().unwrap();
            owners[0] =
                OwnerRow { user_code: None, preferred_domain: Some("https://nope.test".into()), skip_tag_in_url: None };
        }
        let m = current_release_with(&db, &config()).await.unwrap().unwrap();
        // The stored version is not re-validated; non-string notes / date read
        // as null / "", an empty checksum stays "" (`?? null`).
        assert_eq!(m.version, "v9");
        assert_eq!(m.notes, None);
        assert_eq!(m.published_at, "");
        assert_eq!(m.sha256.as_deref(), Some(""));
        assert_eq!(m.url, "https://host.example.com/u/saluy6/raw.exe");

        stored(&db, "{\"version\":\"1.0\",\"code\":\"saluy6\",\"notes\":\"\"}");
        db.queue_happy_path(INSTALLER, None);
        db.manifest_rows.lock().unwrap()[0].ext = String::new();
        db.owners.lock().unwrap()[0].skip_tag_in_url = Some(true);
        let m = current_release_with(&db, &config()).await.unwrap().unwrap();
        assert_eq!(m.url, "https://host.example.com/saluy6/raw");
        assert_eq!(m.notes.as_deref(), Some(""));
    }

    // ── tests/unit/release-publish.test.ts (the gate; the script is not ported) ──

    fn digest() -> String {
        use sha2::{Digest, Sha256};
        let mut payload = b"MZ".to_vec();
        payload.extend(std::iter::repeat_n(7u8, 2048));
        hex::encode(Sha256::digest(&payload))
    }

    #[test]
    fn is_release_digest_accepts_exactly_what_uploads_checksum_stores() {
        let d = digest();
        assert!(is_release_digest(Some(&d)));
        for bad in [None, Some(String::new()), Some(d.to_uppercase()), Some(d[..63].to_string()), Some(format!("{d}0"))]
        {
            assert!(!is_release_digest(bad.as_deref()), "{bad:?}");
        }
        assert!(!is_release_digest(Some(&"z".repeat(64))));
        assert!(!is_release_digest(Some(&format!("{}\n", &d[..63]))));
    }

    #[tokio::test]
    async fn verified_publish_refuses_a_row_whose_checksum_is_null_and_writes_nothing() {
        let db = FakeReleaseDb::default();
        db.checksums.lock().unwrap().push_back(None);
        let result = publish_verified_release_with(&db, &config(), input("0.7.2", "rel123"), &digest()).await.unwrap();
        assert_eq!(result, VerifiedPublishResult::NoDigest);
        assert_eq!(db.setting(RELEASE_APP), None);
    }

    #[tokio::test]
    async fn verified_publish_refuses_before_the_lookup_when_the_caller_has_no_digest() {
        let db = FakeReleaseDb::default();
        let result = publish_verified_release_with(&db, &config(), input("0.7.2", "rel123"), "").await.unwrap();
        assert_eq!(result, VerifiedPublishResult::NoDigest);
        // Nothing was consumed: a caller with nothing to compare cannot even
        // learn whether the code exists.
        assert_eq!(*db.queries.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn verified_publish_refuses_when_the_stored_digest_belongs_to_different_bytes() {
        let db = FakeReleaseDb::default();
        let other = "a".repeat(64);
        db.checksums.lock().unwrap().push_back(Some(other.clone()));
        let result = publish_verified_release_with(&db, &config(), input("0.7.2", "rel123"), &digest()).await.unwrap();
        assert_eq!(result, VerifiedPublishResult::DigestMismatch { stored: other, expected: digest() });
        assert_eq!(db.setting(RELEASE_APP), None);
    }

    #[tokio::test]
    async fn verified_publish_publishes_when_the_row_carries_the_digest() {
        let db = FakeReleaseDb::default();
        db.checksums.lock().unwrap().push_back(Some(digest()));
        db.queue_happy_path("Modlight-0.7.2-setup-x64.exe", Some(&digest()));
        let result = publish_verified_release_with(&db, &config(), input("0.7.2", "rel123"), &digest()).await.unwrap();
        let VerifiedPublishResult::Published(PublishResult::Ok(manifest)) = result else {
            panic!("expected a release, got {result:?}")
        };
        assert_eq!(manifest.sha256, Some(digest()));
        assert_eq!(manifest.filename, "Modlight-0.7.2-setup-x64.exe");
    }

    #[test]
    fn release_file_name_defaults_to_the_basename_of_the_path() {
        assert_eq!(release_file_name(&format!("/tmp/{INSTALLER}"), None).as_deref(), Some(INSTALLER));
        assert_eq!(release_file_name(&format!("C:\\builds\\{INSTALLER}"), None).as_deref(), Some(INSTALLER));
    }

    #[test]
    fn release_file_name_lets_an_override_win() {
        assert_eq!(release_file_name("/tmp/rel.exe", Some(INSTALLER)).as_deref(), Some(INSTALLER));
        assert_eq!(release_file_name("/tmp/rel.exe", Some(&format!("dist/{INSTALLER}"))).as_deref(), Some(INSTALLER));
    }

    #[test]
    fn release_file_name_refuses_rather_than_inventing_a_name() {
        let long = format!("{}.exe", "n".repeat(256));
        for bad in ["", "   ", "/", ".", "..", "bad\nname.exe", long.as_str()] {
            assert_eq!(release_file_name("/tmp/rel.exe", Some(bad)), None, "{bad:?}");
        }
        // 255 UTF-16 units is still a name.
        let fits = format!("{}.exe", "n".repeat(251));
        assert_eq!(release_file_name("/tmp/rel.exe", Some(&fits)), Some(fits.clone()));
    }

    #[tokio::test]
    async fn unpublish_keeps_the_row_with_a_null_value() {
        let db = FakeReleaseDb::default();
        stored(&db, "{\"version\":\"1.0\",\"code\":\"a\"}");
        unpublish_release_with(&db).await.unwrap();
        assert_eq!(db.setting(RELEASE_APP), Some(None));
        assert_eq!(current_release_with(&db, &config()).await.unwrap(), None);
    }
}
