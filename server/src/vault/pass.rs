//! The background pass: encrypts every file of an account switching
//! encryption on, decrypts every file of one switching it off, deletes what an
//! interrupted step left behind. Resumable at any point (docs/ENCRYPTION.md
//! section 7).
//!
//! One file per step:
//!
//! 1. register the NEW object's key in `vault_cleanup` (so a crash while it is
//!    being written leaves a key the next pass deletes);
//! 2. write the new object (encrypted, or decrypted) and check its size;
//! 3. one transaction: point the row at the new object (only if it still
//!    points at the old one, in the old state), unregister the new key,
//!    register the old one;
//! 4. delete the old object (and the upload's derived cache), unregister it.
//!
//! A reader always finds a row whose `storage_key` and `encryption` agree.
//! Each account is worked on under a PostgreSQL advisory lock, so two server
//! processes never step on each other.

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures_util::FutureExt;
use rand::RngCore;
use tokio::sync::Notify;
use uuid::Uuid;

use super::VaultState;
use super::cipher::{CipherHeader, decrypt_stream};
use super::keys::{AccountSecret, unbox_for_job};
use super::store;
use super::view::object_cipher;
use super::write::Sealer;
use crate::db::models::{Columns, FileVault, Upload};
use crate::media::paths::suffix_key;
use crate::state::AppState;
use crate::storage::StorageDriver;

/// How often the pass looks for work when nobody kicked it.
pub const TICK: Duration = Duration::from_secs(30);

fn notify() -> &'static Notify {
    static N: OnceLock<Notify> = OnceLock::new();
    N.get_or_init(Notify::new)
}

/// Wake the pass now (switched on or off, a plaintext row in an "on" vault).
pub fn kick() {
    notify().notify_one();
}

/// The worker: once at boot (auditing "on" vaults too), then whenever kicked
/// and every [`TICK`].
pub async fn worker(state: AppState) {
    let mut audit = true;
    loop {
        match run_once(&state, audit).await {
            Ok(n) if n > 0 => tracing::info!("[vault] pass finished {n} file(s)"),
            Ok(_) => {}
            Err(err) => tracing::error!("[vault] pass failed: {err:#}"),
        }
        audit = false;
        tokio::select! {
            _ = notify().notified() => {}
            _ = tokio::time::sleep(TICK) => {}
        }
    }
}

/// Stops a pass in tests (and on shutdown) between two files.
#[derive(Clone, Default)]
pub struct Stop(Arc<std::sync::atomic::AtomicBool>);

impl Stop {
    pub fn stop(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    fn stopped(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// One run over every account with work. Returns the files moved.
pub async fn run_once(state: &AppState, audit_on: bool) -> anyhow::Result<u64> {
    run_with(state, audit_on, &Stop::default(), u64::MAX).await
}

/// [`run_once`] that stops after `limit` files or when `stop` is set.
pub async fn run_with(state: &AppState, audit_on: bool, stop: &Stop, limit: u64) -> anyhow::Result<u64> {
    let _ = store::delete_expired_grants(&state.db).await;
    sweep_stale_cleanup(state).await;
    let mut moved = 0u64;
    for user_id in store::vaults_with_work(&state.db, audit_on).await? {
        if stop.stopped() || moved >= limit {
            break;
        }
        match work_on(state, &user_id, stop, limit - moved).await {
            Ok(n) => moved += n,
            Err(err) => {
                tracing::error!("[vault] {user_id}: {err:#}");
                set_error(state, &user_id, Some(&format!("{err:#}"))).await;
            }
        }
    }
    Ok(moved)
}

async fn set_error(state: &AppState, user_id: &str, error: Option<&str>) {
    let error = error.map(|e| e.chars().take(500).collect::<String>());
    let _ = sqlx::query(r#"update "file_vaults" set "job_error" = $1 where "user_id" = $2"#)
        .bind(error)
        .bind(user_id)
        .execute(&state.db)
        .await;
}

fn lock_key(user_id: &str) -> String {
    format!("modlight-vault:{user_id}")
}

/// Everything for one account, under its advisory lock.
async fn work_on(state: &AppState, user_id: &str, stop: &Stop, limit: u64) -> anyhow::Result<u64> {
    let mut conn = state.db.acquire().await?;
    let locked: bool = sqlx::query_scalar("select pg_try_advisory_lock(hashtext($1))")
        .bind(lock_key(user_id))
        .fetch_one(&mut *conn)
        .await?;
    if !locked {
        return Ok(0);
    }
    // A panic must not leave the session-level lock on a pooled connection.
    let result = std::panic::AssertUnwindSafe(work_locked(state, user_id, stop, limit)).catch_unwind().await;
    let _ = sqlx::query("select pg_advisory_unlock(hashtext($1))").bind(lock_key(user_id)).execute(&mut *conn).await;
    match result {
        Ok(result) => result,
        Err(_) => anyhow::bail!("the pass panicked"),
    }
}

async fn work_locked(state: &AppState, user_id: &str, stop: &Stop, limit: u64) -> anyhow::Result<u64> {
    let storage = state.storage.get()?;
    // What an interrupted step of this account left behind. Nothing of ours is
    // in flight: we hold the account's lock.
    for key in store::cleanup_for(&state.db, user_id).await? {
        delete_unreferenced(state, storage.as_ref(), &key).await?;
    }
    let Some(vault) = store::load(&state.db, user_id).await? else {
        return Ok(0);
    };
    let Some(vault_state) = store::state_of(&vault) else {
        return Ok(0);
    };
    let mut moved = 0u64;
    let mut failed: HashSet<Uuid> = HashSet::new();
    let mut last_error: Option<String> = None;
    let mut queue: Vec<Upload> = Vec::new();
    match vault_state {
        VaultState::Encrypting | VaultState::On => {
            let sealer = Sealer::from_vault(&state.config.auth.secret, &vault)?;
            while moved < limit && !stop.stopped() {
                let Some(row) = next_row(state, user_id, false, &failed, &mut queue).await? else { break };
                match encrypt_row(state, storage.as_ref(), &sealer, &row).await {
                    Ok(()) => moved += 1,
                    Err(err) => {
                        tracing::warn!("[vault] could not encrypt {}: {err:#}", row.id);
                        last_error = Some(format!("{err:#}"));
                        failed.insert(row.id);
                    }
                }
            }
            if failed.is_empty() && !stop.stopped() && moved < limit {
                sqlx::query(
                    r#"update "file_vaults" set "state" = 'on', "job_error" = null, "updated_at" = now()
                       where "user_id" = $1 and "state" = 'encrypting'
                         and not exists (select 1 from "uploads" where "owner_id" = $1 and "encryption" is null)"#,
                )
                .bind(user_id)
                .execute(&state.db)
                .await?;
            }
        }
        VaultState::Decrypting => {
            let secret = job_secret(state, &vault)?;
            while moved < limit && !stop.stopped() {
                let Some(row) = next_row(state, user_id, true, &failed, &mut queue).await? else { break };
                match decrypt_row(state, storage.as_ref(), &secret, &row).await {
                    Ok(()) => moved += 1,
                    Err(err) => {
                        tracing::warn!("[vault] could not decrypt {}: {err:#}", row.id);
                        last_error = Some(format!("{err:#}"));
                        failed.insert(row.id);
                    }
                }
            }
            if failed.is_empty() && !stop.stopped() && moved < limit {
                let mut tx = state.db.begin().await?;
                let gone = sqlx::query(
                    r#"delete from "file_vaults" where "user_id" = $1 and "state" = 'decrypting'
                         and not exists (select 1 from "uploads" where "owner_id" = $1 and "encryption" is not null)"#,
                )
                .bind(user_id)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                if gone > 0 {
                    store::delete_all_grants(&mut *tx, user_id).await?;
                }
                tx.commit().await?;
            }
        }
    }
    if let Some(err) = last_error {
        set_error(state, user_id, Some(&err)).await;
    } else if moved > 0 {
        set_error(state, user_id, None).await;
    }
    Ok(moved)
}

fn job_secret(state: &AppState, vault: &FileVault) -> anyhow::Result<AccountSecret> {
    let boxed = vault.job_key.as_deref().ok_or_else(|| anyhow::anyhow!("decrypting without a key"))?;
    Ok(unbox_for_job(boxed, &state.config.auth.secret, &vault.public_key)?)
}

/// The next row to move: newest first, trash included, failures of this run
/// skipped.
/// Rows are read in batches of this many (each one is re-checked when its
/// row is switched, so a stale batch is harmless).
const BATCH: i64 = 32;

async fn next_row(
    state: &AppState,
    user_id: &str,
    encrypted: bool,
    skip: &HashSet<Uuid>,
    queue: &mut Vec<Upload>,
) -> anyhow::Result<Option<Upload>> {
    if queue.is_empty() {
        let skip: Vec<Uuid> = skip.iter().copied().collect();
        let sql = format!(
            r#"select {} from {} where "owner_id" = $1 and ("encryption" is not null) = $2 and not ("id" = any($3))
               order by "created_at" desc limit {BATCH}"#,
            Upload::COLUMNS,
            Upload::TABLE
        );
        let mut rows: Vec<Upload> =
            sqlx::query_as(&sql).bind(user_id).bind(encrypted).bind(&skip).fetch_all(&state.db).await?;
        rows.reverse();
        *queue = rows;
    }
    Ok(queue.pop())
}

/// A key next to `row`'s, unused by construction (48 random bits): the new
/// object never overwrites the old one, so a reader of either is safe.
pub fn fresh_key(row: &Upload) -> String {
    let dir = row.storage_key.rfind('/').map(|i| &row.storage_key[..=i]).unwrap_or("");
    let name = if row.ext.is_empty() { row.code.clone() } else { format!("{}.{}", row.code, row.ext) };
    let mut bytes = [0u8; 6];
    rand::thread_rng().fill_bytes(&mut bytes);
    suffix_key(&format!("{dir}{name}"), &hex::encode(bytes))
}

async fn encrypt_row(
    state: &AppState,
    storage: &dyn StorageDriver,
    sealer: &Sealer,
    row: &Upload,
) -> anyhow::Result<()> {
    let user_id = Some(sealer.user_id.as_str());
    let Some(stat) = storage.stat(&row.storage_key).await? else {
        anyhow::bail!("stored object is missing: {}", row.storage_key);
    };
    let new_key = fresh_key(row);
    store::add_cleanup(&state.db, &new_key, user_id).await?;
    let plain = storage.read_stream(&row.storage_key, None).await?;
    let header = match sealer.write_stream(storage, &new_key, plain, stat.size).await {
        Ok(h) => h,
        Err(err) => {
            let _ = store::remove_cleanup(&state.db, &new_key).await;
            return Err(err);
        }
    };
    let switched = switch_row(state, row, &new_key, Some(&header), user_id).await?;
    if !switched {
        // The row moved under us (deleted, converted): ours is the stray copy.
        let _ = storage.delete(&new_key).await;
        store::remove_cleanup(&state.db, &new_key).await?;
        return Ok(());
    }
    retire(state, storage, row).await
}

async fn decrypt_row(
    state: &AppState,
    storage: &dyn StorageDriver,
    secret: &AccountSecret,
    row: &Upload,
) -> anyhow::Result<()> {
    let header_text = row.encryption.clone().unwrap_or_default();
    let header = CipherHeader::parse(&header_text)?;
    let cipher = Arc::new(object_cipher(secret, &row.storage_key, &header_text)?);
    let user_id = row.owner_id.as_deref();
    let new_key = fresh_key(row);
    store::add_cleanup(&state.db, &new_key, user_id).await?;
    let sealed = storage.read_stream(&row.storage_key, None).await?;
    let written = storage.write(&new_key, decrypt_stream(cipher, sealed)).await;
    let verified = match written {
        Ok(()) => matches!(storage.stat(&new_key).await, Ok(Some(info)) if info.size == header.plain_size),
        Err(err) => {
            let _ = storage.delete(&new_key).await;
            let _ = store::remove_cleanup(&state.db, &new_key).await;
            return Err(err);
        }
    };
    if !verified {
        let _ = storage.delete(&new_key).await;
        let _ = store::remove_cleanup(&state.db, &new_key).await;
        anyhow::bail!("decrypted object did not arrive whole: {new_key}");
    }
    let switched = switch_row(state, row, &new_key, None, user_id).await?;
    if !switched {
        let _ = storage.delete(&new_key).await;
        store::remove_cleanup(&state.db, &new_key).await?;
        return Ok(());
    }
    retire(state, storage, row).await
}

/// Step 3: point the row at the new object, in one transaction with the
/// cleanup bookkeeping. False when the row is no longer what we read.
async fn switch_row(
    state: &AppState,
    row: &Upload,
    new_key: &str,
    header: Option<&str>,
    user_id: Option<&str>,
) -> anyhow::Result<bool> {
    let mut tx = state.db.begin().await?;
    // The blurhash is a picture of the content: never kept for ciphertext.
    let n = sqlx::query(
        r#"update "uploads" set "storage_key" = $1, "encryption" = $2,
             "blurhash" = case when $2::text is null then "blurhash" else null end
           where "id" = $3 and "storage_key" = $4 and "encryption" is not distinct from $5"#,
    )
    .bind(new_key)
    .bind(header)
    .bind(row.id)
    .bind(&row.storage_key)
    .bind(&row.encryption)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        tx.rollback().await?;
        return Ok(false);
    }
    store::remove_cleanup(&mut *tx, new_key).await?;
    store::add_cleanup(&mut *tx, &row.storage_key, user_id).await?;
    tx.commit().await?;
    Ok(true)
}

/// Step 4: the old object and everything derived from it.
async fn retire(state: &AppState, storage: &dyn StorageDriver, row: &Upload) -> anyhow::Result<()> {
    crate::media::derived_cache::purge_derived(storage, &row.id.to_string()).await;
    if let Some(thumb) = row.thumb_key.as_deref().filter(|t| !t.is_empty()) {
        let _ = storage.delete(thumb).await;
        let _ = sqlx::query(r#"update "uploads" set "thumb_key" = null where "id" = $1"#)
            .bind(row.id)
            .execute(&state.db)
            .await;
    }
    delete_unreferenced(state, storage, &row.storage_key).await
}

/// Delete a key from storage and from `vault_cleanup`, unless a row points at
/// it again (never delete live bytes).
async fn delete_unreferenced(state: &AppState, storage: &dyn StorageDriver, key: &str) -> anyhow::Result<()> {
    if !store::is_referenced(&state.db, key).await? {
        storage.delete(key).await?;
    }
    store::remove_cleanup(&state.db, key).await?;
    Ok(())
}

/// Objects registered for accounts that no longer have a vault (the account
/// was deleted, or switching off finished between two steps), after a grace
/// period so a step in progress elsewhere is never cut short.
async fn sweep_stale_cleanup(state: &AppState) {
    let Ok(storage) = state.storage.get() else { return };
    let Ok(keys) = store::stale_cleanup(&state.db, 30).await else { return };
    for key in keys {
        if let Err(err) = delete_unreferenced(state, storage.as_ref(), &key).await {
            tracing::warn!("[vault] cleanup of {key} failed: {err:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_keys_stay_beside_the_old_one_and_never_grow() {
        let row: Upload = serde_json::from_value(serde_json::json!({
            "id": "9d2e1a54-0000-4000-8000-000000000001", "ownerId": "u1", "code": "abc", "filename": "a.png",
            "ext": "png", "kind": "file", "language": null, "mimeType": "image/png", "size": 10,
            "storageKey": "Tag/abc-0011aabbccdd.png", "checksum": null, "width": null, "height": null,
            "blurhash": null, "thumbKey": null, "published": true, "deleteToken": "d", "views": 0, "downloads": 0,
            "lastAccessedAt": null, "expiresAt": null, "maxViews": null, "passwordHash": null, "folderId": null,
            "deletedAt": null, "createdAt": "2026-01-01T00:00:00.000Z", "updatedAt": "2026-01-01T00:00:00.000Z",
            "passwordVersion": null, "frames": null, "convertedAt": null, "originalExt": null,
            "durationSeconds": null, "fps": null
        }))
        .unwrap();
        let a = fresh_key(&row);
        let b = fresh_key(&row);
        assert_ne!(a, b);
        assert!(a.starts_with("Tag/abc-") && a.ends_with(".png") && a.len() == "Tag/abc-".len() + 12 + 4, "{a}");
        let bare = Upload { ext: String::new(), storage_key: "abc".into(), ..row };
        let k = fresh_key(&bare);
        assert!(k.starts_with("abc-") && !k.contains('/'), "{k}");
    }
}
