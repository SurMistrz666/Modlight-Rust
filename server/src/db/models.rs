//! Rust rows for EVERY table the drizzle migrations create, with the exact
//! column names and types (`lib/db/schema/*.ts` + `drizzle/*.sql`).
//!
//! This is the shared vocabulary of the port. Conventions:
//! - field names are the SQL column names, so `#[derive(sqlx::FromRow)]` maps
//!   `SELECT *` (or [`Columns::COLUMNS`]) without renames;
//! - serde renames to camelCase, which is what drizzle's `$inferSelect` objects
//!   looked like in JSON (`userCode`, `usedQuota`, `createdAt`);
//! - every `timestamp` / `timestamptz` column is a [`JsDate`] (UTC; ISO string
//!   in JSON, `{"$date"}` in RPC and page payloads);
//! - `bigint` columns declared `{ mode: "number" }` upstream are `i64` and
//!   serialize as JSON numbers, exactly as drizzle returned them;
//! - nullable columns are `Option<_>` (and serialize as `null`).
//!
//! Field order follows the column order in the database (initial CREATE TABLE
//! order, then each `ADD COLUMN` in migration order), which is also what
//! `SELECT *` returns.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

pub use super::types::JsDate;

/// Table name (quoted where needed) and the full column list in DB order.
pub trait Columns {
    const TABLE: &'static str;
    const COLUMNS: &'static str;
}

// ── Better Auth tables (lib/db/schema/auth-schema.ts) ───────────────────────

/// `user`: accounts. Better Auth core fields, the admin and twoFactor plugin
/// fields, and Modlight's own columns.
#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    pub name: String,
    pub email: String,
    pub email_verified: bool,
    /// Holds a storage key, not a URL (see lib/avatar.ts).
    pub image: Option<String>,
    pub created_at: JsDate,
    pub updated_at: JsDate,
    /// "admin" | "user" (admin plugin). NULL counts as "user".
    pub role: Option<String>,
    pub banned: Option<bool>,
    pub ban_reason: Option<String>,
    pub ban_expires: Option<JsDate>,
    pub two_factor_enabled: Option<bool>,
    /// The user's "tag": the first segment of every public URL.
    pub user_code: Option<String>,
    pub naming_scheme: Option<String>,
    pub naming_pattern: Option<String>,
    pub public_profile: Option<bool>,
    /// Bytes. bigint, mode number.
    pub used_quota: Option<i64>,
    /// Bytes; -1 = unlimited. bigint, mode number.
    pub max_quota: Option<i64>,
    pub hide_uploads: Option<bool>,
    pub copy_raw: Option<bool>,
    /// Column default 'graphite' since migration 0014 ('discord' from 0009); sign-up
    /// writes "modern", which resolves to the default skin.
    pub theme: Option<String>,
    /// 'pl' | 'en' | NULL (= by browser).
    pub locale: Option<String>,
    /// Login name (0001), unique.
    pub username: Option<String>,
    pub skip_tag_in_url: Option<bool>,
    pub allow_root_short_links: Option<bool>,
    /// FK folders.id ON DELETE SET NULL (added by hand in 0002).
    pub default_folder_id: Option<Uuid>,
    pub naming_word_count: Option<i32>,
    pub naming_random_length: Option<i32>,
    pub preferred_domain: Option<String>,
    pub default_password_hash: Option<String>,
    pub default_password_version: i32,
    pub background_key: Option<String>,
    pub background_dim: i32,
}

impl Columns for User {
    const TABLE: &'static str = "\"user\"";
    const COLUMNS: &'static str = "\"id\", \"name\", \"email\", \"email_verified\", \"image\", \"created_at\", \"updated_at\", \"role\", \"banned\", \"ban_reason\", \"ban_expires\", \"two_factor_enabled\", \"user_code\", \"naming_scheme\", \"naming_pattern\", \"public_profile\", \"used_quota\", \"max_quota\", \"hide_uploads\", \"copy_raw\", \"theme\", \"locale\", \"username\", \"skip_tag_in_url\", \"allow_root_short_links\", \"default_folder_id\", \"naming_word_count\", \"naming_random_length\", \"preferred_domain\", \"default_password_hash\", \"default_password_version\", \"background_key\", \"background_dim\"";
}

/// `user_code_aliases`: historical tags that keep resolving forever.
#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserCodeAlias {
    pub code: String,
    pub user_id: String,
    pub created_at: JsDate,
}

impl Columns for UserCodeAlias {
    const TABLE: &'static str = "\"user_code_aliases\"";
    const COLUMNS: &'static str = "\"code\", \"user_id\", \"created_at\"";
}

/// `session`: Better Auth sessions (cookie token -> user).
#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub expires_at: JsDate,
    pub token: String,
    pub created_at: JsDate,
    pub updated_at: JsDate,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub user_id: String,
    /// admin plugin: the admin impersonating, if any.
    pub impersonated_by: Option<String>,
}

impl Columns for Session {
    const TABLE: &'static str = "\"session\"";
    const COLUMNS: &'static str = "\"id\", \"expires_at\", \"token\", \"created_at\", \"updated_at\", \"ip_address\", \"user_agent\", \"user_id\", \"impersonated_by\"";
}

/// `account`: credentials per provider. Email+password rows have
/// `provider_id = 'credential'` and the password hash in `password`.
#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub account_id: String,
    pub provider_id: String,
    pub user_id: String,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub access_token_expires_at: Option<JsDate>,
    pub refresh_token_expires_at: Option<JsDate>,
    pub scope: Option<String>,
    /// Better Auth scrypt hash, or a legacy bcrypt `$2a/$2b/$2y` hash.
    pub password: Option<String>,
    pub created_at: JsDate,
    pub updated_at: JsDate,
}

impl Columns for Account {
    const TABLE: &'static str = "\"account\"";
    const COLUMNS: &'static str = "\"id\", \"account_id\", \"provider_id\", \"user_id\", \"access_token\", \"refresh_token\", \"id_token\", \"access_token_expires_at\", \"refresh_token_expires_at\", \"scope\", \"password\", \"created_at\", \"updated_at\"";
}

/// `verification`: reset-password and other one-off tokens.
#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Verification {
    pub id: String,
    pub identifier: String,
    pub value: String,
    pub expires_at: JsDate,
    pub created_at: JsDate,
    pub updated_at: JsDate,
}

impl Columns for Verification {
    const TABLE: &'static str = "\"verification\"";
    const COLUMNS: &'static str = "\"id\", \"identifier\", \"value\", \"expires_at\", \"created_at\", \"updated_at\"";
}

/// `two_factor`: encrypted TOTP secret and backup codes (twoFactor plugin).
#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TwoFactor {
    pub id: String,
    pub secret: String,
    pub backup_codes: String,
    pub user_id: String,
    pub verified: Option<bool>,
}

impl Columns for TwoFactor {
    const TABLE: &'static str = "\"two_factor\"";
    const COLUMNS: &'static str = "\"id\", \"secret\", \"backup_codes\", \"user_id\", \"verified\"";
}

// ── App tables (lib/db/schema/app-schema.ts) ────────────────────────────────

/// `uploads`: one row per stored file. URL shape: /{userCode}/{code}.{ext}.
#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Upload {
    pub id: Uuid,
    /// NULL for a file that outlived its account ("orphaned").
    pub owner_id: Option<String>,
    pub code: String,
    pub filename: String,
    pub ext: String,
    /// 'file' | 'paste'
    pub kind: String,
    pub language: Option<String>,
    pub mime_type: String,
    pub size: i64,
    pub storage_key: String,
    /// sha256 hex, lower case.
    pub checksum: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub blurhash: Option<String>,
    pub thumb_key: Option<String>,
    pub published: bool,
    pub delete_token: String,
    pub views: i32,
    pub downloads: i32,
    pub last_accessed_at: Option<JsDate>,
    pub expires_at: Option<JsDate>,
    pub max_views: Option<i32>,
    pub password_hash: Option<String>,
    pub folder_id: Option<Uuid>,
    pub deleted_at: Option<JsDate>,
    pub created_at: JsDate,
    pub updated_at: JsDate,
    /// Generation of the owner's default password this file was stamped with.
    pub password_version: Option<i32>,
    pub frames: Option<i32>,
    pub converted_at: Option<JsDate>,
    pub original_ext: Option<String>,
    pub duration_seconds: Option<f64>,
    pub fps: Option<f64>,
}

impl Columns for Upload {
    const TABLE: &'static str = "\"uploads\"";
    const COLUMNS: &'static str = "\"id\", \"owner_id\", \"code\", \"filename\", \"ext\", \"kind\", \"language\", \"mime_type\", \"size\", \"storage_key\", \"checksum\", \"width\", \"height\", \"blurhash\", \"thumb_key\", \"published\", \"delete_token\", \"views\", \"downloads\", \"last_accessed_at\", \"expires_at\", \"max_views\", \"password_hash\", \"folder_id\", \"deleted_at\", \"created_at\", \"updated_at\", \"password_version\", \"frames\", \"converted_at\", \"original_ext\", \"duration_seconds\", \"fps\"";
}

/// `folders`: the single organisational unit (nesting via `parent_id`).
#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: Uuid,
    pub owner_id: String,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub is_public: bool,
    pub cover_upload_id: Option<Uuid>,
    pub sort_order: i32,
    pub created_at: JsDate,
}

impl Columns for Folder {
    const TABLE: &'static str = "\"folders\"";
    const COLUMNS: &'static str = "\"id\", \"owner_id\", \"parent_id\", \"name\", \"slug\", \"description\", \"is_public\", \"cover_upload_id\", \"sort_order\", \"created_at\"";
}

/// `api_tokens`: per-user tokens for ShareX / API (stored as sha256 hex).
#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiToken {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    pub token_hash: String,
    /// Last six characters of the token, for display.
    pub preview: String,
    pub last_used_at: Option<JsDate>,
    pub revoked_at: Option<JsDate>,
    pub created_at: JsDate,
    /// 'upload' | 'read' | 'write' | 'full' (CHECK constraint, no default).
    pub scope: String,
}

impl Columns for ApiToken {
    const TABLE: &'static str = "\"api_tokens\"";
    const COLUMNS: &'static str = "\"id\", \"user_id\", \"name\", \"token_hash\", \"preview\", \"last_used_at\", \"revoked_at\", \"created_at\", \"scope\"";
}

/// `settings`: global key/value runtime settings (admin-configurable).
#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setting {
    pub key: String,
    pub value: Option<String>,
}

impl Columns for Setting {
    const TABLE: &'static str = "\"settings\"";
    const COLUMNS: &'static str = "\"key\", \"value\"";
}

/// `audit_log`: activity log.
#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLog {
    pub id: Uuid,
    pub user_id: Option<String>,
    pub action: String,
    pub detail: Option<Value>,
    pub ip: Option<String>,
    pub created_at: JsDate,
}

impl Columns for AuditLog {
    const TABLE: &'static str = "\"audit_log\"";
    const COLUMNS: &'static str = "\"id\", \"user_id\", \"action\", \"detail\", \"ip\", \"created_at\"";
}

/// `short_links`: the built-in URL shortener.
#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortLink {
    pub id: Uuid,
    pub owner_id: String,
    pub code: String,
    pub target: String,
    pub password_hash: Option<String>,
    pub clicks: i32,
    pub last_used_at: Option<JsDate>,
    pub expires_at: Option<JsDate>,
    pub created_at: JsDate,
}

impl Columns for ShortLink {
    const TABLE: &'static str = "\"short_links\"";
    const COLUMNS: &'static str = "\"id\", \"owner_id\", \"code\", \"target\", \"password_hash\", \"clicks\", \"last_used_at\", \"expires_at\", \"created_at\"";
}

/// `client_settings`: the desktop client's own configuration (opaque jsonb).
#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientSettings {
    pub id: Uuid,
    pub owner_id: String,
    pub client: String,
    /// Bumped by the server on every replace; 0 = nothing stored.
    pub version: i32,
    pub settings: Value,
    pub created_at: JsDate,
    pub updated_at: JsDate,
}

impl Columns for ClientSettings {
    const TABLE: &'static str = "\"client_settings\"";
    const COLUMNS: &'static str =
        "\"id\", \"owner_id\", \"client\", \"version\", \"settings\", \"created_at\", \"updated_at\"";
}

// ── lib/db/schema/duplicates-schema.ts ──────────────────────────────────────

/// `duplicate_dismissals`: "these copies are meant to exist". PK (owner_id, checksum).
#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateDismissal {
    pub owner_id: String,
    pub checksum: String,
    pub copies: i32,
    pub bytes: i64,
    pub created_at: JsDate,
}

impl Columns for DuplicateDismissal {
    const TABLE: &'static str = "\"duplicate_dismissals\"";
    const COLUMNS: &'static str = "\"owner_id\", \"checksum\", \"copies\", \"bytes\", \"created_at\"";
}

// ── Not upstream: Downloads (migration 0015, src/downloads.rs) ──────────────

/// `releases`: a build of the desktop app offered under Downloads. The bytes
/// are in storage at `releases/<id>/<filename>`.
#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    /// 32 lower-case hex characters.
    pub id: String,
    /// `macos` or `windows` (CHECK constraint).
    pub platform: String,
    /// `arm64` or `x64` (CHECK constraint).
    pub arch: String,
    pub version: String,
    /// May be empty.
    pub build: String,
    /// When the build machine built it.
    pub built: JsDate,
    /// When the server accepted it; "newest" means this.
    pub published: JsDate,
    pub size: i64,
    /// Lower-case hex.
    pub sha256: String,
    pub filename: String,
    /// Who published it: an administrator's name, or `publish-token`.
    pub uploader: String,
    /// May be empty.
    pub note: String,
}

impl Columns for Release {
    const TABLE: &'static str = "\"releases\"";
    const COLUMNS: &'static str = "\"id\", \"platform\", \"arch\", \"version\", \"build\", \"built\", \"published\", \"size\", \"sha256\", \"filename\", \"uploader\", \"note\"";
}

// ── drizzle's own bookkeeping ───────────────────────────────────────────────

/// `drizzle.__drizzle_migrations`: one row per applied migration.
#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DrizzleMigration {
    pub id: i32,
    /// sha256 hex of the SQL file.
    pub hash: String,
    /// The journal `when` of the migration (ms since epoch).
    pub created_at: Option<i64>,
}

impl Columns for DrizzleMigration {
    const TABLE: &'static str = "\"drizzle\".\"__drizzle_migrations\"";
    const COLUMNS: &'static str = "\"id\", \"hash\", \"created_at\"";
}

/// Every model's table and column list, for schema checks.
pub const ALL_TABLES: &[(&str, &str)] = &[
    (User::TABLE, User::COLUMNS),
    (UserCodeAlias::TABLE, UserCodeAlias::COLUMNS),
    (Session::TABLE, Session::COLUMNS),
    (Account::TABLE, Account::COLUMNS),
    (Verification::TABLE, Verification::COLUMNS),
    (TwoFactor::TABLE, TwoFactor::COLUMNS),
    (Upload::TABLE, Upload::COLUMNS),
    (Folder::TABLE, Folder::COLUMNS),
    (ApiToken::TABLE, ApiToken::COLUMNS),
    (Setting::TABLE, Setting::COLUMNS),
    (AuditLog::TABLE, AuditLog::COLUMNS),
    (ShortLink::TABLE, ShortLink::COLUMNS),
    (ClientSettings::TABLE, ClientSettings::COLUMNS),
    (DuplicateDismissal::TABLE, DuplicateDismissal::COLUMNS),
    (Release::TABLE, Release::COLUMNS),
    (DrizzleMigration::TABLE, DrizzleMigration::COLUMNS),
];
