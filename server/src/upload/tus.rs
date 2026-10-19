//! Port of `lib/upload/tus.ts`: resumable web uploads on `/api/files/*`
//! (Uppy in the panel, the desktop apps' resumable uploader), on the tus
//! protocol server of [`super::tus_server`].
//!
//! Authenticated by an API token first and the browser session second; a
//! PRESENT but invalid token is a refusal, never a fall-through to the
//! session. Beyond creation nothing re-authenticates: possession of the
//! upload URL minted here IS the capability for the PATCHes that follow.
//! Quota, free space and the drop target are checked at creation, before a
//! single byte is stored.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::Response;
use futures_util::future::BoxFuture;
use sqlx::PgPool;

use super::finalize::{FinalizeError, FinalizeInput, finalize_upload, remaining_quota};
use super::folder_target::{FOLDER_KEY, FolderTarget, ROOT_FOLDER, resolve_folder_from_metadata};
use super::tus_server::file_store::FileStore;
use super::tus_server::server::{FinishPatch, ServerOptions, TusHooks, TusRequest, TusServer};
use super::tus_server::store::{DataStore, Metadata, StoreError, TusError, Upload};
use crate::api_scopes::TokenCapability;
use crate::api_token::{TokenAuth, UserRow, authenticate_api_token, bearer_from_request, token_refusal};
use crate::auth::session::{RefreshPolicy, find_user, get_session};
use crate::db::types::JsDate;
use crate::state::AppState;

/// How long an abandoned upload is kept before the sweeper reclaims it. Long
/// enough that "my laptop slept overnight" still resumes.
pub const TUS_EXPIRATION_MS: u64 = 24 * 60 * 60 * 1000;
pub const TUS_EXPIRATION: Duration = Duration::from_millis(TUS_EXPIRATION_MS);

/// How often the server runs the sweep.
pub const TUS_CLEANUP_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Grace period before an orphaned record is reaped. See [`reap_orphaned_records`].
const ORPHAN_GRACE_MS: i64 = 5 * 60 * 1000;

/// Metadata key carrying the folder a web upload should land in.
pub const FOLDER_META_KEY: &str = FOLDER_KEY;
/// Sentinel meaning "no folder", as opposed to an absent key.
pub const ROOT_FOLDER_META: &str = ROOT_FOLDER;

/// The one tus server of the process (its lock table and store).
static SERVER: OnceLock<Arc<TusServer>> = OnceLock::new();

/// The tus server, built on first use from the application's configuration.
pub fn tus_server(state: &AppState) -> Arc<TusServer> {
    SERVER.get_or_init(|| Arc::new(create_tus_server(state))).clone()
}

pub fn routes() -> Router<AppState> {
    Router::new().fallback(handle)
}

/// Every request whose pathname starts with `/api/files` (server.ts).
pub async fn handle(State(state): State<AppState>, req: axum::extract::Request) -> Response {
    tus_server(&state).handle(req).await
}

/// `makeDatastore()`: the file store for the local driver, the S3 store
/// otherwise.
fn make_datastore(state: &AppState) -> Arc<dyn DataStore> {
    if state.config.storage.driver == "s3" {
        if let Some(store) = s3_datastore() {
            return store;
        }
        tracing::error!("[tus] the S3 tus store is not available in this build; falling back to the file store");
    }
    Arc::new(FileStore::new(&state.config.uploads.tus_tmp_path, TUS_EXPIRATION_MS))
}

/// The S3 store (`@tus/s3-store`), once ported; `None` until then.
fn s3_datastore() -> Option<Arc<dyn DataStore>> {
    None
}

/// `createTusServer()`
pub fn create_tus_server(state: &AppState) -> TusServer {
    let hard_max = if state.config.uploads.max_size > 0 { state.config.uploads.max_size as u64 } else { 0 };
    let datastore = make_datastore(state);
    TusServer::new(
        datastore.clone(),
        ServerOptions {
            path: "/api/files".to_string(),
            respect_forwarded_headers: state.config.trust_proxy,
            lock_drain_timeout: Duration::from_millis(3000),
            lock_timeout: Duration::from_secs(30),
        },
        Arc::new(ModlightHooks { state: state.clone(), hard_max, datastore }),
    )
}

/// Translate "bytes this user may still store" into tus's maxSize convention,
/// where **0 means unlimited**. A user exactly at their quota must not
/// collapse to 0, so the floor is 1 byte.
pub fn tus_max_size(used: Option<i64>, max: Option<i64>, hard_max: u64) -> u64 {
    let Some(left) = remaining_quota(used, max) else {
        return hard_max;
    };
    let left = left.max(0) as u64;
    let capped = if hard_max > 0 { left.min(hard_max) } else { left };
    capped.max(1)
}

/// Who is uploading: `Ok(user)` or a refusal (status, plain-text body).
pub enum TusAuth {
    Ok(Box<UserRow>),
    Refused(u16, String),
}

const SIGN_IN: &str = "Sign in or send a valid API token.";

/// `ownerFromRequest(req)`: an API token first, the browser session second.
pub async fn owner_from_request(state: &AppState, headers: &axum::http::HeaderMap) -> anyhow::Result<TusAuth> {
    if let Some(token) = bearer_from_request(headers) {
        let auth = authenticate_api_token(&state.db, Some(&token), TokenCapability::Upload).await?;
        return Ok(match auth {
            TokenAuth::Ok { user, .. } => TusAuth::Ok(Box::new(user)),
            other => {
                let (status, body) = token_refusal(&other, SIGN_IN).unwrap_or_default();
                // tus answers with a plain-text body: the sentence carries the explanation.
                let message = body.get("message").and_then(|m| m.as_str()).unwrap_or("Refused.").to_string();
                TusAuth::Refused(status.as_u16(), message)
            }
        });
    }
    let lookup = get_session(&state.db, &state.auth_cookies, headers, RefreshPolicy::DbOnly).await?;
    let user = match lookup.session {
        Some(s) => find_user(&state.db, &s.user.id).await?,
        None => None,
    };
    let Some(user) = user else {
        return Ok(TusAuth::Refused(401, SIGN_IN.to_string()));
    };
    // A session is the account itself and carries no scope; only the ban applies.
    if user.banned == Some(true) {
        return Ok(TusAuth::Refused(401, "Account disabled.".to_string()));
    }
    Ok(TusAuth::Ok(Box::new(user)))
}

struct ModlightHooks {
    state: AppState,
    hard_max: u64,
    datastore: Arc<dyn DataStore>,
}

fn refusal(status: u16, body: &str) -> StoreError {
    StoreError::Tus(TusError::new(status, body))
}

impl TusHooks for ModlightHooks {
    /// Quota as a cap on the DECLARED length, at creation only (PATCH derives
    /// its ceiling from the length validated here).
    fn max_size<'a>(&'a self, req: &'a TusRequest) -> BoxFuture<'a, u64> {
        Box::pin(async move {
            if req.method != axum::http::Method::POST {
                return self.hard_max;
            }
            match owner_from_request(&self.state, &req.headers).await {
                Ok(TusAuth::Ok(user)) => tus_max_size(user.used_quota, user.max_quota, self.hard_max),
                _ => self.hard_max, // onUploadCreate refuses it anyway
            }
        })
    }

    /// Authenticate + quota-gate BEFORE any bytes are stored.
    fn on_upload_create<'a>(
        &'a self,
        req: &'a TusRequest,
        upload: &'a Upload,
    ) -> BoxFuture<'a, Result<Option<Metadata>, StoreError>> {
        Box::pin(async move {
            let owner = match owner_from_request(&self.state, &req.headers).await? {
                TusAuth::Ok(user) => user,
                TusAuth::Refused(status, body) => return Err(refusal(status, &body)),
            };
            // Refuse `Upload-Defer-Length: 1`: an unknown size cannot be quota-checked.
            let Some(size) = upload.size else {
                return Err(refusal(411, "Upload-Length is required."));
            };
            if remaining_quota(owner.used_quota, owner.max_quota).is_some_and(|left| size as i64 > left) {
                return Err(refusal(507, "Quota exceeded."));
            }
            // Room on the machine, checked here rather than at finalize.
            let config = &self.state.config;
            if !super::disk::has_room_for(config, size as f64, &config.uploads.tus_tmp_path).await {
                return Err(refusal(507, "Server is low on disk space; upload rejected."));
            }
            // Fail the drop before a single byte is stored if the folder it
            // names is not this user's.
            if resolve_folder_from_metadata(&self.state.db, &owner.id, upload.metadata.as_ref()).await.is_err() {
                return Err(refusal(403, "Folder not found."));
            }
            let mut meta = upload.metadata.clone().unwrap_or_default();
            meta.insert("userId".to_string(), Some(owner.id.clone()));
            Ok(Some(meta))
        })
    }

    /// Finalize into our storage + DB.
    fn on_upload_finish<'a>(
        &'a self,
        _req: &'a TusRequest,
        upload: &'a Upload,
    ) -> BoxFuture<'a, Result<FinishPatch, StoreError>> {
        Box::pin(async move {
            let state = &self.state;
            let Some(user_id) = upload.meta("userId").filter(|u| !u.is_empty()) else {
                return Err(refusal(401, "Missing user."));
            };
            let Some(owner) = find_user(&state.db, user_id).await? else {
                return Err(refusal(401, "Missing user."));
            };
            let filename = upload
                .meta("filename")
                .filter(|f| !f.is_empty())
                .or_else(|| upload.meta("name").filter(|f| !f.is_empty()))
                .unwrap_or("file")
                .to_string();
            let mime = upload
                .meta("filetype")
                .filter(|f| !f.is_empty())
                .or_else(|| upload.meta("type").filter(|f| !f.is_empty()))
                .unwrap_or("application/octet-stream")
                .to_string();
            let storage_info = upload.storage.clone();
            let is_local = storage_info.as_ref().is_some_and(|s| s.kind == "file");
            let path = storage_info.as_ref().map(|s| s.path.clone());
            let local_path = if is_local { path.as_ref().map(PathBuf::from) } else { None };
            let storage = state.storage.get()?;
            let result = finalize_upload(
                &state.db,
                &state.config,
                storage.as_ref(),
                FinalizeInput {
                    owner: &owner,
                    filename: &filename,
                    mime: &mime,
                    size: upload.size.unwrap_or(upload.offset) as i64,
                    head: None,
                    local_path: local_path.as_deref(),
                    existing_key: if is_local { None } else { path.as_deref() },
                    checksum: None,
                    custom_code: None,
                    folder_id: None,
                },
            )
            .await;
            let result = match result {
                Ok(r) => r,
                Err(e @ (FinalizeError::QuotaExceeded | FinalizeError::OutOfSpace)) => {
                    return Err(refusal(507, &e.to_string()));
                }
                Err(FinalizeError::CodeTaken) => {
                    return Err(StoreError::Other(anyhow::anyhow!("Custom url code already exists.")));
                }
                Err(FinalizeError::Other(e)) => return Err(StoreError::Other(e)),
            };
            // Drag-and-drop semantics: a file dropped while a folder is open
            // lands in THAT folder.
            apply_target_folder(&state.db, &owner.id, result.id, upload.metadata.as_ref()).await?;
            // FileStore still holds a sidecar .json describing a finished upload.
            if is_local {
                discard_tus_upload(self.datastore.as_ref(), &upload.id).await;
            }
            Ok(FinishPatch {
                status_code: Some(200),
                body: None,
                headers: vec![
                    ("X-Media-Url".to_string(), result.url.clone()),
                    ("X-Media-Raw-Url".to_string(), result.raw_url.clone()),
                    // The link to put on the clipboard, from the account's `copy_raw`.
                    ("X-Media-Copy-Url".to_string(), result.copy_url.clone()),
                ],
            })
        })
    }
}

/// Move a freshly finalized upload into the folder the drop targeted.
/// Ownership is re-checked and folded into the WHERE clause, so the worst a
/// race can do is leave the file where it was.
async fn apply_target_folder(
    db: &PgPool,
    owner_id: &str,
    upload_id: uuid::Uuid,
    metadata: Option<&Metadata>,
) -> anyhow::Result<()> {
    let target = match resolve_folder_from_metadata(db, owner_id, metadata).await {
        Ok(t) => t,
        Err(_) => return Ok(()), // vanished between create and finish
    };
    let folder = match target {
        FolderTarget::Default => return Ok(()),
        FolderTarget::Root => None,
        FolderTarget::Folder(id) => Some(id),
    };
    sqlx::query(
        r#"update "uploads" set "folder_id" = $1, "updated_at" = $2 where ("uploads"."id" = $3 and "uploads"."owner_id" = $4)"#,
    )
    .bind(folder)
    .bind(JsDate::now())
    .bind(upload_id)
    .bind(owner_id)
    .execute(db)
    .await?;
    Ok(())
}

/// Drop a finished upload's tus bookkeeping. `remove()` unlinks the payload
/// first and fails when finalize already moved it; then the record itself.
pub async fn discard_tus_upload(store: &dyn DataStore, id: &str) {
    if store.remove(id).await.is_ok() {
        return;
    }
    if let Err(err) = store.delete_record(id).await {
        tracing::warn!("[tus] could not drop upload record {id}: {err}");
    }
}

/// Delete `.json` records whose payload is gone (the store's own list() never
/// sees them). The grace period guards a record mid-creation.
pub async fn reap_orphaned_records(directory: &Path) -> u64 {
    let Ok(mut dir) = tokio::fs::read_dir(directory).await else {
        return 0; // directory not created yet
    };
    let mut entries: Vec<String> = Vec::new();
    while let Ok(Some(e)) = dir.next_entry().await {
        entries.push(e.file_name().to_string_lossy().into_owned());
    }
    let payloads: std::collections::HashSet<&str> =
        entries.iter().filter(|e| !e.ends_with(".json")).map(String::as_str).collect();
    let cutoff = JsDate::now().millis() - ORPHAN_GRACE_MS;
    let mut removed = 0;
    for entry in &entries {
        let Some(stem) = entry.strip_suffix(".json") else {
            continue;
        };
        if payloads.contains(stem) {
            continue;
        }
        let path = directory.join(entry);
        let Ok(meta) = tokio::fs::metadata(&path).await else {
            continue;
        };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        if mtime > cutoff {
            continue;
        }
        if tokio::fs::remove_file(&path).await.is_ok() {
            removed += 1;
        }
    }
    removed
}

/// `sweepTusUploads(server)`: abandoned uploads plus records whose payload was
/// already adopted into storage. Returns how many were removed.
pub async fn sweep_tus_uploads(state: &AppState) -> anyhow::Result<u64> {
    let server = tus_server(state);
    sweep_store(server.datastore.as_ref()).await
}

pub async fn sweep_store(store: &dyn DataStore) -> anyhow::Result<u64> {
    let orphans = match store.directory() {
        Some(dir) => reap_orphaned_records(dir).await,
        None => 0,
    };
    if !store.has_extension(super::tus_server::store::EXT_EXPIRATION) {
        anyhow::bail!("expiration extension is not (yet) supported.");
    }
    let expired = store.delete_expired().await.map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(orphans + expired)
}

#[cfg(test)]
mod tests {
    //! tests/unit/upload-tus.test.ts, tests/unit/upload-quota.test.ts (tusMaxSize)
    use super::*;

    const GIB: i64 = 1024 * 1024 * 1024;

    #[test]
    fn tus_max_size_rules() {
        assert_eq!(tus_max_size(None, Some(-1), 0), 0);
        assert_eq!(tus_max_size(None, Some(-1), 5 * GIB as u64), 5 * GIB as u64);
        assert_eq!(tus_max_size(Some(15 * GIB), Some(20 * GIB), 0), 5 * GIB as u64);
        assert_eq!(tus_max_size(Some(15 * GIB), Some(20 * GIB), GIB as u64), GIB as u64);
        assert_eq!(tus_max_size(Some(19 * GIB), Some(20 * GIB), 5 * GIB as u64), GIB as u64);
        assert_eq!(tus_max_size(Some(20 * GIB), Some(20 * GIB), 0), 1);
        assert_eq!(tus_max_size(Some(30 * GIB), Some(20 * GIB), 5 * GIB as u64), 1);
        assert_eq!(TUS_EXPIRATION_MS, 24 * 60 * 60 * 1000);
    }

    async fn files(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let mut d = tokio::fs::read_dir(dir).await.unwrap();
        while let Some(e) = d.next_entry().await.unwrap() {
            out.push(e.file_name().to_string_lossy().into_owned());
        }
        out.sort();
        out
    }

    fn age(path: &Path) {
        let old = std::time::SystemTime::now() - Duration::from_secs(600);
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_modified(old).unwrap();
    }

    #[tokio::test]
    async fn discard_and_sweep() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path().to_str().unwrap(), TUS_EXPIRATION_MS);
        // Removes both the payload and its sidecar.
        store.create(Upload::new("full01".into(), Some(4), 0, None)).await.unwrap();
        discard_tus_upload(&store, "full01").await;
        assert!(files(dir.path()).await.is_empty());
        // Still drops the sidecar when finalize already moved the payload away.
        store.create(Upload::new("moved1".into(), Some(4), 0, None)).await.unwrap();
        std::fs::remove_file(dir.path().join("moved1")).unwrap();
        discard_tus_upload(&store, "moved1").await;
        assert!(files(dir.path()).await.is_empty());
        // A no-op for an id the store never knew about.
        discard_tus_upload(&store, "ghost").await;
        // Reaps records list() cannot see, respecting the grace period.
        store.create(Upload::new("orph01".into(), Some(4), 0, None)).await.unwrap();
        std::fs::remove_file(dir.path().join("orph01")).unwrap();
        assert_eq!(sweep_store(&store).await.unwrap(), 0);
        age(&dir.path().join("orph01.json"));
        assert_eq!(sweep_store(&store).await.unwrap(), 1);
        assert!(files(dir.path()).await.is_empty());
        // Leaves a healthy in-progress upload alone.
        store.create(Upload::new("live01".into(), Some(4), 0, None)).await.unwrap();
        assert_eq!(sweep_store(&store).await.unwrap(), 0);
        assert_eq!(files(dir.path()).await, vec!["live01", "live01.json"]);
    }
}
