//! The rows of a file vault: `file_vaults`, `vault_grants`, `vault_cleanup`
//! (migration 0016). Raw SQL like the rest of the crate.

use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use super::VaultState;
use crate::db::models::{Columns, FileVault, VaultGrant};
use crate::db::types::JsDate;

/// Grants an account keeps per kind; the oldest of a kind goes first.
pub const MAX_OWNER_GRANTS: i64 = 20;
pub const MAX_VISITOR_GRANTS: i64 = 200;

/// `vault_grants.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantKind {
    /// The account's own browser (the panel).
    Owner,
    /// Somebody who opened a link and typed the password.
    Visitor,
    /// A bearer client (the desktop app).
    App,
}

impl GrantKind {
    pub fn as_str(self) -> &'static str {
        match self {
            GrantKind::Owner => "owner",
            GrantKind::Visitor => "visitor",
            GrantKind::App => "app",
        }
    }

    fn cap(self) -> i64 {
        match self {
            GrantKind::Visitor => MAX_VISITOR_GRANTS,
            _ => MAX_OWNER_GRANTS,
        }
    }
}

pub async fn load(db: impl PgExecutor<'_>, user_id: &str) -> sqlx::Result<Option<FileVault>> {
    let sql = format!(r#"select {} from {} where "user_id" = $1 limit 1"#, FileVault::COLUMNS, FileVault::TABLE);
    sqlx::query_as(&sql).bind(user_id).fetch_optional(db).await
}

/// The row under a lock that keeps its state from changing until the
/// caller's transaction ends (`FOR SHARE`: writers of uploads use it).
pub async fn load_shared(db: impl PgExecutor<'_>, user_id: &str) -> sqlx::Result<Option<FileVault>> {
    let sql =
        format!(r#"select {} from {} where "user_id" = $1 limit 1 for share"#, FileVault::COLUMNS, FileVault::TABLE);
    sqlx::query_as(&sql).bind(user_id).fetch_optional(db).await
}

pub fn state_of(vault: &FileVault) -> Option<VaultState> {
    VaultState::parse(&vault.state)
}

/// What the new vault row is made of.
pub struct NewVault<'a> {
    pub user_id: &'a str,
    pub public_key: &'a [u8],
    pub public_key_mac: &'a [u8],
    pub wrapped_key: &'a [u8],
    pub kdf_salt: &'a [u8],
    pub params: super::keys::KdfParams,
}

/// Create the vault in state `encrypting`; false when the account has one.
pub async fn insert(db: &PgPool, v: &NewVault<'_>) -> sqlx::Result<bool> {
    let made: Option<String> = sqlx::query_scalar(
        r#"insert into "file_vaults" ("user_id", "state", "public_key", "public_key_mac", "wrapped_key", "kdf_salt",
                                      "kdf_memory_kib", "kdf_iterations", "kdf_parallelism")
           values ($1, 'encrypting', $2, $3, $4, $5, $6, $7, $8)
           on conflict ("user_id") do nothing returning "user_id""#,
    )
    .bind(v.user_id)
    .bind(v.public_key)
    .bind(v.public_key_mac)
    .bind(v.wrapped_key)
    .bind(v.kdf_salt)
    .bind(v.params.memory_kib as i32)
    .bind(v.params.iterations as i32)
    .bind(v.params.parallelism as i32)
    .fetch_optional(db)
    .await?;
    Ok(made.is_some())
}

/// Files of the account (trash included), and how many of them the pass has
/// already brought to the vault's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
pub struct Progress {
    pub done: i64,
    pub total: i64,
}

pub async fn progress(db: &PgPool, user_id: &str, state: VaultState) -> sqlx::Result<Progress> {
    let (encrypted, total): (i64, i64) = sqlx::query_as(
        r#"select count(*) filter (where "encryption" is not null), count(*) from "uploads" where "owner_id" = $1"#,
    )
    .bind(user_id)
    .fetch_one(db)
    .await?;
    Ok(match state {
        VaultState::Decrypting => Progress { done: total - encrypted, total },
        _ => Progress { done: encrypted, total },
    })
}

// ── Grants ──────────────────────────────────────────────────────────────────

pub struct NewGrant<'a> {
    pub id: Uuid,
    pub user_id: &'a str,
    pub kind: GrantKind,
    pub generation: i32,
    pub wrapped_key: &'a [u8],
    pub label: Option<&'a str>,
    pub ip: Option<&'a str>,
    pub expires_at: JsDate,
}

/// Store a grant and keep the account within its per-kind cap.
pub async fn insert_grant(db: &PgPool, g: &NewGrant<'_>) -> sqlx::Result<()> {
    let label = g.label.map(|l| l.chars().take(200).collect::<String>());
    let ip = g.ip.map(|i| i.chars().take(64).collect::<String>());
    sqlx::query(
        r#"insert into "vault_grants" ("id", "user_id", "kind", "generation", "wrapped_key", "label", "ip", "expires_at")
           values ($1, $2, $3, $4, $5, $6, $7, $8)"#,
    )
    .bind(g.id)
    .bind(g.user_id)
    .bind(g.kind.as_str())
    .bind(g.generation)
    .bind(g.wrapped_key)
    .bind(label)
    .bind(ip)
    .bind(g.expires_at)
    .execute(db)
    .await?;
    sqlx::query(
        r#"delete from "vault_grants" where "id" in (
             select "id" from "vault_grants" where "user_id" = $1 and "kind" = $2
             order by "created_at" desc, "id" offset $3)"#,
    )
    .bind(g.user_id)
    .bind(g.kind.as_str())
    .bind(g.kind.cap())
    .execute(db)
    .await?;
    Ok(())
}

/// A live grant of `user_id` of this generation, or None.
pub async fn live_grant(db: &PgPool, id: Uuid, user_id: &str, generation: i32) -> sqlx::Result<Option<VaultGrant>> {
    let sql = format!(
        r#"select {} from {} where "id" = $1 and "user_id" = $2 and "generation" = $3 and "expires_at" > now() limit 1"#,
        VaultGrant::COLUMNS,
        VaultGrant::TABLE
    );
    sqlx::query_as(&sql).bind(id).bind(user_id).bind(generation).fetch_optional(db).await
}

/// A grant by id alone (the app's header names no account).
pub async fn live_grant_by_id(db: &PgPool, id: Uuid) -> sqlx::Result<Option<VaultGrant>> {
    let sql = format!(
        r#"select {} from {} where "id" = $1 and "expires_at" > now() limit 1"#,
        VaultGrant::COLUMNS,
        VaultGrant::TABLE
    );
    sqlx::query_as(&sql).bind(id).fetch_optional(db).await
}

/// Note that a grant was used, at most every five minutes (never awaited).
pub fn touch_grant(db: &PgPool, id: Uuid) {
    let db = db.clone();
    tokio::spawn(async move {
        let _ = sqlx::query(
            r#"update "vault_grants" set "last_used_at" = now()
               where "id" = $1 and ("last_used_at" is null or "last_used_at" < now() - interval '5 minutes')"#,
        )
        .bind(id)
        .execute(&db)
        .await;
    });
}

pub async fn list_grants(db: &PgPool, user_id: &str) -> sqlx::Result<Vec<VaultGrant>> {
    let sql = format!(
        r#"select {} from {} where "user_id" = $1 and "expires_at" > now() order by "created_at" desc"#,
        VaultGrant::COLUMNS,
        VaultGrant::TABLE
    );
    sqlx::query_as(&sql).bind(user_id).fetch_all(db).await
}

/// Revoke one grant of `user_id`; false when there was none.
pub async fn delete_grant(db: &PgPool, id: Uuid, user_id: &str) -> sqlx::Result<bool> {
    let n = sqlx::query(r#"delete from "vault_grants" where "id" = $1 and "user_id" = $2"#)
        .bind(id)
        .bind(user_id)
        .execute(db)
        .await?
        .rows_affected();
    Ok(n > 0)
}

pub async fn delete_all_grants(db: impl PgExecutor<'_>, user_id: &str) -> sqlx::Result<u64> {
    Ok(sqlx::query(r#"delete from "vault_grants" where "user_id" = $1"#)
        .bind(user_id)
        .execute(db)
        .await?
        .rows_affected())
}

pub async fn delete_expired_grants(db: &PgPool) -> sqlx::Result<u64> {
    Ok(sqlx::query(r#"delete from "vault_grants" where "expires_at" <= now()"#).execute(db).await?.rows_affected())
}

// ── Cleanup ─────────────────────────────────────────────────────────────────

pub async fn add_cleanup(db: impl PgExecutor<'_>, storage_key: &str, user_id: Option<&str>) -> sqlx::Result<()> {
    sqlx::query(
        r#"insert into "vault_cleanup" ("storage_key", "user_id") values ($1, $2) on conflict ("storage_key") do nothing"#,
    )
    .bind(storage_key)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn remove_cleanup(db: impl PgExecutor<'_>, storage_key: &str) -> sqlx::Result<()> {
    sqlx::query(r#"delete from "vault_cleanup" where "storage_key" = $1"#).bind(storage_key).execute(db).await?;
    Ok(())
}

/// Keys left behind by an earlier pass for `user_id`.
pub async fn cleanup_for(db: &PgPool, user_id: &str) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar(r#"select "storage_key" from "vault_cleanup" where "user_id" = $1 order by "created_at""#)
        .bind(user_id)
        .fetch_all(db)
        .await
}

/// Keys left behind for accounts no pass will visit again (no vault any
/// more), older than `minutes`.
pub async fn stale_cleanup(db: &PgPool, minutes: i64) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar(
        r#"select c."storage_key" from "vault_cleanup" c
           where c."created_at" < now() - make_interval(mins => $1::int)
             and not exists (select 1 from "file_vaults" v where v."user_id" = c."user_id")
           order by c."created_at" limit 500"#,
    )
    .bind(minutes)
    .fetch_all(db)
    .await
}

/// Is any object of this key still referenced by a row? (Never delete one
/// that is.)
pub async fn is_referenced(db: impl PgExecutor<'_>, storage_key: &str) -> sqlx::Result<bool> {
    let hit: Option<Uuid> =
        sqlx::query_scalar(r#"select "id" from "uploads" where "storage_key" = $1 or "thumb_key" = $1 limit 1"#)
            .bind(storage_key)
            .fetch_optional(db)
            .await?;
    Ok(hit.is_some())
}

/// Every account with a vault, and its state.
pub async fn all_vaults(db: &PgPool) -> sqlx::Result<Vec<(String, String)>> {
    sqlx::query_as(r#"select "user_id", "state" from "file_vaults" order by "created_at""#).fetch_all(db).await
}

/// Vaults the pass has work in: encrypting, decrypting, or with objects left
/// to delete. With `audit_on`, also every "on" vault holding a plaintext row
/// (a writer that raced with switching on puts its vault back to
/// `encrypting` itself; this sweep, run at boot, is the safety net).
pub async fn vaults_with_work(db: &PgPool, audit_on: bool) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar(
        r#"select v."user_id" from "file_vaults" v
           where v."state" <> 'on'
              or exists (select 1 from "vault_cleanup" c where c."user_id" = v."user_id")
              or ($1 and exists (select 1 from "uploads" u where u."owner_id" = v."user_id" and u."encryption" is null))
           order by v."created_at""#,
    )
    .bind(audit_on)
    .fetch_all(db)
    .await
}

/// The owner ids among `ids` that have a vault, and its state (admin lists).
pub async fn states_for(db: &PgPool, ids: &[String]) -> sqlx::Result<std::collections::HashMap<String, String>> {
    if ids.is_empty() {
        return Ok(Default::default());
    }
    let rows: Vec<(String, String)> =
        sqlx::query_as(r#"select "user_id", "state" from "file_vaults" where "user_id" = any($1)"#)
            .bind(ids)
            .fetch_all(db)
            .await?;
    Ok(rows.into_iter().collect())
}
