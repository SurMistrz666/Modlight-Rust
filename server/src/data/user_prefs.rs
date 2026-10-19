//! Port of `lib/data/user-prefs.ts`: per-user URL/upload preferences.
//!
//! These columns are deliberately NOT exposed through Better Auth's
//! `additionalFields`, so `session.user` does not carry them: read them here
//! instead. Keeping them out of the session also means a change takes effect
//! on the next request instead of waiting for the session cookie to be
//! refreshed.

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::background::DIM_DEFAULT;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserPrefs {
    pub id: String,
    pub role: Option<String>,
    /// Current tag = first URL segment.
    pub user_code: Option<String>,
    /// Admin-only: generate links without the tag segment.
    pub skip_tag_in_url: bool,
    /// May create short links at the root of the shortener domain.
    pub allow_root_short_links: bool,
    /// Folder new uploads land in (None = none).
    pub default_folder_id: Option<Uuid>,
    /// Hostname this user's links are generated with (None = deployment
    /// default). Resolve it through `user_base_url()`: the stored value is only
    /// honoured when it is still one of `config.appDomains`.
    pub preferred_domain: Option<String>,
    /// Storage key of the account's wallpaper (None = none).
    pub background_key: Option<String>,
    /// Veil strength over the wallpaper, 0-100.
    pub background_dim: i32,
}

type PrefsRow = (
    String,
    Option<String>,
    Option<String>,
    Option<bool>,
    Option<bool>,
    Option<Uuid>,
    Option<String>,
    Option<String>,
    Option<i32>,
);

pub async fn get_user_prefs(db: &PgPool, user_id: &str) -> sqlx::Result<Option<UserPrefs>> {
    let row: Option<PrefsRow> = sqlx::query_as(
        r#"SELECT "id", "role", "user_code", "skip_tag_in_url", "allow_root_short_links", "default_folder_id",
                  "preferred_domain", "background_key", "background_dim"
           FROM "user" WHERE "id" = $1 LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| UserPrefs {
        id: r.0,
        role: r.1,
        user_code: r.2,
        skip_tag_in_url: r.3.unwrap_or(false),
        allow_root_short_links: r.4.unwrap_or(false),
        default_folder_id: r.5,
        preferred_domain: r.6,
        background_key: r.7,
        background_dim: r.8.unwrap_or(DIM_DEFAULT),
    }))
}

/// A default password is set (there is no separate on/off flag), and its
/// generation counter, bumped on every change and stamped onto new uploads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultPasswordPrefs {
    pub enabled: bool,
    pub version: i32,
}

pub async fn get_default_password_prefs(db: &PgPool, user_id: &str) -> sqlx::Result<DefaultPasswordPrefs> {
    let row: Option<(Option<String>, Option<i32>)> = sqlx::query_as(
        r#"SELECT "default_password_hash", "default_password_version" FROM "user" WHERE "id" = $1 LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    let (hash, version) = row.unwrap_or((None, None));
    Ok(DefaultPasswordPrefs { enabled: hash.is_some_and(|h| !h.is_empty()), version: version.unwrap_or(0) })
}

/// How many of the owner's files (trashed ones included) carry the default
/// password: the number the "apply to existing files" choice is about.
pub async fn count_files_on_default_password(db: &PgPool, user_id: &str) -> sqlx::Result<i64> {
    let (n,): (i64,) =
        sqlx::query_as(r#"SELECT count(*) FROM "uploads" WHERE "owner_id" = $1 AND "password_version" IS NOT NULL"#)
            .bind(user_id)
            .fetch_one(db)
            .await?;
    Ok(n)
}

/// What a new upload is stamped with: the owner's default hash and the
/// generation it came from, or nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultPasswordStamp {
    pub password_hash: Option<String>,
    pub password_version: Option<i32>,
}

pub async fn default_password_stamp(db: &PgPool, user_id: &str) -> sqlx::Result<DefaultPasswordStamp> {
    let row: Option<(Option<String>, Option<i32>)> = sqlx::query_as(
        r#"SELECT "default_password_hash", "default_password_version" FROM "user" WHERE "id" = $1 LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    match row {
        Some((Some(hash), version)) if !hash.is_empty() => {
            Ok(DefaultPasswordStamp { password_hash: Some(hash), password_version: Some(version.unwrap_or(0)) })
        }
        _ => Ok(DefaultPasswordStamp { password_hash: None, password_version: None }),
    }
}

/// The owner's current default-password generation, or None without one.
pub async fn get_owner_password_version(db: &PgPool, owner_id: &str) -> sqlx::Result<Option<i32>> {
    Ok(default_password_stamp(db, owner_id).await?.password_version)
}

/// The account's default folder, but only while it still exists and still
/// belongs to the account.
pub async fn get_default_folder_id(db: &PgPool, user_id: &str) -> sqlx::Result<Option<Uuid>> {
    let Some(prefs) = get_user_prefs(db, user_id).await? else {
        return Ok(None);
    };
    let Some(folder) = prefs.default_folder_id else {
        return Ok(None);
    };
    let row: Option<(Uuid,)> =
        sqlx::query_as(r#"SELECT "id" FROM "folders" WHERE "id" = $1 AND "owner_id" = $2 LIMIT 1"#)
            .bind(folder)
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    Ok(row.map(|r| r.0))
}

/// Admins always may; anyone else only with `allow_root_short_links`.
pub fn may_create_root_short_link(role: Option<&str>, allow_root_short_links: bool) -> bool {
    role == Some("admin") || allow_root_short_links
}

/// The tag of the owner of a live upload with this code, when that owner opted
/// out of the tag segment (`skip_tag_in_url`): what a tagless link resolves to.
pub async fn resolve_tagless_owner_tag(db: &PgPool, code: &str) -> sqlx::Result<Option<String>> {
    let row: Option<(Option<String>, Option<bool>)> = sqlx::query_as(
        r#"SELECT u."user_code", u."skip_tag_in_url" FROM "uploads" up
           INNER JOIN "user" u ON u."id" = up."owner_id"
           WHERE up."code" = $1 AND up."deleted_at" IS NULL LIMIT 1"#,
    )
    .bind(code)
    .fetch_optional(db)
    .await?;
    match row {
        Some((Some(tag), Some(true))) if !tag.is_empty() => Ok(Some(tag)),
        _ => Ok(None),
    }
}
