//! Port of `app/api/uploads/route.ts`: the caller's own uploads over HTTP
//! (what a desktop client draws "My Space" from, and the right-click menu it
//! edits them with).
//!
//! - `GET`: one page of the library (or the trash), through the SAME
//!   `list_uploads` the dashboard renders from.
//! - `PATCH`: move, publish, vanity code, expiry, view cap, password. Every
//!   field is validated before anything is written; an absent key means "leave
//!   alone", an explicit `null` means "clear".
//! - `DELETE`: to the trash (default), out of it (`restore`) or for good
//!   (`permanent`), each mode with its own view of the trash.
//!
//! Token first, session second (`api_caller`); never anything but the
//! caller's own rows, an administrator included.

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::get;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::api_caller::resolve_api_owner;
use crate::api_json::{PRIVATE_NO_STORE, json_private, read_json_object, supplied};
use crate::api_token::{TokenCapability, UserRow};
use crate::auth_helpers::Sessions;
use crate::data::upload_query::parse_upload_query;
use crate::data::uploads::{FileItem, FolderFilter, ListParams, list_uploads, to_file_items};
use crate::db::models::{Columns, Upload};
use crate::error::{AppError, AppResult};
use crate::folders::{FolderError, move_uploads_to_folder_for};
use crate::media::actions::{DeletableMedia, delete_media_hard, restore_media, soft_delete_media};
use crate::media::mime::{MediaKind, media_kind};
use crate::media::motion_preview::{
    AV_MAX_INPUT_BYTES, MAX_TRANSFORM_BYTES, MOTION_PARAM, MotionCandidate, can_motion_preview, clamp_motion_edge,
};
use crate::media::urls::{LinkOwner, media_urls};
use crate::state::AppState;
use crate::upload::folder_target::{FolderTarget, FolderTargetError, ROOT_FOLDER, is_uuid, resolve_folder_target};
use crate::uploads::{
    UploadActor, UploadError, UploadIssue, actionable_uploads, set_expiry_for, set_max_views_for, set_password_for,
    set_published_for, set_vanity_for,
};

/// Thumbnail edge asked for in `thumbUrl` (one of the serving route's sizes).
const THUMB_PX: u32 = 480;

/// The still size through the clamp a moving preview gets.
fn motion_px() -> u32 {
    clamp_motion_edge(Some(THUMB_PX)).unwrap_or(THUMB_PX)
}

/// Ceiling on how many ids one write may name.
pub const MAX_IDS: usize = 500;

/// Ceiling on a PATCH body (500 quoted uuids fit comfortably).
pub const PATCH_BODY_MAX: usize = 64 * 1024;

/// Password changes allowed per account, and the window they are counted over.
const PASSWORD_LIMIT: u32 = 30;
const PASSWORD_WINDOW_MS: i64 = 60_000;

/// Ceiling on a DELETE body. Upstream reads it with `req.json()` and no limit;
/// 1 MiB is fifty times what 500 ids need (see the handoff's deviations).
const DELETE_BODY_MAX: usize = 1024 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/uploads", get(list).patch(patch).delete(delete))
}

fn json(status: StatusCode, body: &impl Serialize) -> Response {
    json_private(status, body)
}

fn message(status: StatusCode, text: &str) -> Response {
    json(status, &json!({ "message": text }))
}

/// `canThumbnail(f)`: exactly the conditions under which `/raw?w=` returns a
/// tile rather than the original.
fn can_thumbnail(f: &FileItem) -> bool {
    match media_kind(Some(&f.mime_type), Some(&f.ext)) {
        MediaKind::Video => f.size <= AV_MAX_INPUT_BYTES,
        MediaKind::Image => f.size <= MAX_TRANSFORM_BYTES,
        _ => false,
    }
}

/// `apiItem(f, owner, trashed)`: one row in the shape a desktop grid draws a
/// tile from. A trashed row gets no links (they would all 404).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiItem {
    id: Uuid,
    code: String,
    filename: String,
    ext: String,
    mime_type: String,
    kind: &'static str,
    size: i64,
    width: Option<i32>,
    height: Option<i32>,
    blurhash: Option<String>,
    published: bool,
    views: i32,
    downloads: i32,
    created_at: String,
    folder_id: Option<Uuid>,
    expires_at: Option<String>,
    max_views: Option<i32>,
    has_password: bool,
    url: Option<String>,
    raw_url: Option<String>,
    thumb_url: Option<String>,
    motion_thumb_url: Option<String>,
    copy_url: Option<String>,
}

pub fn api_item(state: &AppState, f: FileItem, owner: &UserRow, trashed: bool) -> ApiItem {
    let links = (!trashed).then(|| media_urls(&state.config, &LinkOwner::of_user(owner), &f.code, &f.ext));
    let thumb_url =
        links.as_ref().filter(|_| can_thumbnail(&f)).map(|l| format!("{}?w={THUMB_PX}&h={THUMB_PX}", l.raw_url));
    let candidate =
        MotionCandidate { mime_type: Some(&f.mime_type), ext: Some(&f.ext), size: f.size, frames: f.frames };
    let motion_thumb_url = links.as_ref().filter(|_| can_motion_preview(candidate)).map(|l| {
        let px = motion_px();
        format!("{}?w={px}&h={px}&{MOTION_PARAM}=1", l.raw_url)
    });
    let copy_url = links.as_ref().map(|l| if owner.copy_raw == Some(true) { l.raw_url.clone() } else { l.url.clone() });
    ApiItem {
        id: f.id,
        code: f.code,
        filename: f.filename,
        ext: f.ext,
        mime_type: f.mime_type,
        kind: f.kind,
        size: f.size,
        width: f.width,
        height: f.height,
        blurhash: f.blurhash,
        published: f.published,
        views: f.views,
        downloads: f.downloads,
        created_at: f.created_at,
        folder_id: f.folder_id,
        expires_at: f.expires_at,
        max_views: f.max_views,
        has_password: f.has_password,
        url: links.as_ref().map(|l| l.url.clone()),
        raw_url: links.as_ref().map(|l| l.raw_url.clone()),
        thumb_url,
        motion_thumb_url,
        copy_url,
    }
}

/// GET: the caller's own uploads, paged.
async fn list(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, _) = req.into_parts();
    let (owner, _) = resolve_api_owner(&state, &session, &parts.headers, TokenCapability::Read)
        .await?
        .or_respond(Some(PRIVATE_NO_STORE))?;

    let params: Vec<(String, String)> =
        form_urlencoded::parse(parts.uri.query().unwrap_or("").as_bytes()).into_owned().collect();
    let q = parse_upload_query(&params);

    // Same gate as both upload paths: one 404 for "not yours", "gone" and
    // "not a uuid".
    let folder = match resolve_folder_target(&state.db, &owner.id, q.folder.as_deref()).await {
        Ok(FolderTarget::Default) => FolderFilter::Any,
        Ok(FolderTarget::Root) => FolderFilter::Unfiled,
        Ok(FolderTarget::Folder(id)) => FolderFilter::Folder(id.to_string()),
        Err(FolderTargetError::Access(_)) => return Ok(message(StatusCode::NOT_FOUND, "Folder not found.")),
        Err(FolderTargetError::Db(e)) => return Err(e.into()),
    };

    let result = list_uploads(
        &state.db,
        &ListParams {
            user_id: owner.id.clone(),
            page: Some(q.page as f64),
            per_page: Some(q.per_page),
            search: q.search.clone(),
            sort: Some(q.sort.clone()),
            kind: q.kind.clone(),
            trashed: q.trashed,
            folder,
            ..Default::default()
        },
    )
    .await?;

    // Folder ids only: clients resolve the names from /api/folders.
    let items: Vec<ApiItem> = to_file_items(&result.items, &Default::default())
        .into_iter()
        .map(|f| api_item(&state, f, &owner, q.trashed))
        .collect();

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Page {
        page: i64,
        per_page: i64,
        total: i64,
        root_value: &'static str,
        items: Vec<ApiItem>,
    }
    Ok(json(
        StatusCode::OK,
        &Page { page: result.page, per_page: result.per_page, total: result.total, root_value: ROOT_FOLDER, items },
    ))
}

/// `UPLOAD_STATUS`: how each core refusal reaches the wire.
fn upload_status(reason: UploadIssue) -> StatusCode {
    match reason {
        UploadIssue::NotFound => StatusCode::NOT_FOUND,
        UploadIssue::InvalidCode | UploadIssue::CodeTooLong => StatusCode::BAD_REQUEST,
        UploadIssue::CodeTaken | UploadIssue::CodeReserved => StatusCode::CONFLICT,
        UploadIssue::InvalidDate | UploadIssue::InvalidViews | UploadIssue::PasswordTooLong => StatusCode::BAD_REQUEST,
    }
}

/// PATCH: change files. Body `{ ids: [...] }` plus any of `folderId`,
/// `published`, `code` (one id only), `expiresAt`, `maxViews`, `password`.
async fn patch(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, body) = req.into_parts();
    // Before the body, so an unauthenticated caller cannot make us buffer one.
    let (owner, _) = resolve_api_owner(&state, &session, &parts.headers, TokenCapability::Write)
        .await?
        .or_respond(Some(PRIVATE_NO_STORE))?;

    let body = match read_json_object(&parts.headers, body, PATCH_BODY_MAX).await {
        Ok(b) => b,
        Err(e) => return Ok(message(e.status, &e.message)),
    };

    let ids: Vec<Value> = match body.get("ids") {
        Some(Value::Array(list)) => {
            list.iter().filter(|v| v.as_str().is_some_and(|s| !s.is_empty())).cloned().collect()
        }
        _ => Vec::new(),
    };
    if ids.is_empty() {
        return Ok(message(StatusCode::BAD_REQUEST, "No ids given."));
    }
    if ids.len() > MAX_IDS {
        return Ok(message(StatusCode::BAD_REQUEST, &format!("Too many ids (max {MAX_IDS}).")));
    }

    let wants_folder = supplied(&body, "folderId");
    let wants_published = supplied(&body, "published");
    let wants_code = supplied(&body, "code");
    let wants_expiry = supplied(&body, "expiresAt");
    let wants_max_views = supplied(&body, "maxViews");
    let wants_password = supplied(&body, "password");

    if !(wants_folder || wants_published || wants_code || wants_expiry || wants_max_views || wants_password) {
        return Ok(message(
            StatusCode::BAD_REQUEST,
            "Nothing to change: send `folderId`, `published`, `code`, `expiresAt`, `maxViews` or `password`.",
        ));
    }

    // ── shapes, all of them, before a single write ──
    let mut folder_id: Option<String> = None;
    if wants_folder {
        match &body["folderId"] {
            Value::Null => {}
            Value::String(s) if s.is_empty() || s == ROOT_FOLDER => {}
            Value::String(s) => folder_id = Some(s.clone()),
            _ => return Ok(message(StatusCode::BAD_REQUEST, "`folderId` must be a string or null.")),
        }
    }
    if wants_published && !body["published"].is_boolean() {
        return Ok(message(StatusCode::BAD_REQUEST, "`published` must be a boolean."));
    }
    if wants_code {
        if !body["code"].is_string() {
            return Ok(message(StatusCode::BAD_REQUEST, "`code` must be a string."));
        }
        if ids.len() > 1 {
            return Ok(json(
                StatusCode::BAD_REQUEST,
                &json!({
                    "message": "`code` sets one file's URL, so it cannot be sent with several ids.",
                    "reason": "code-single-id",
                }),
            ));
        }
    }
    if wants_expiry && !(body["expiresAt"].is_null() || body["expiresAt"].is_string()) {
        return Ok(message(StatusCode::BAD_REQUEST, "`expiresAt` must be an ISO date string or null."));
    }
    if wants_max_views && !(body["maxViews"].is_null() || body["maxViews"].is_number()) {
        return Ok(message(StatusCode::BAD_REQUEST, "`maxViews` must be a number or null."));
    }
    if wants_password && !(body["password"].is_null() || body["password"].is_string()) {
        return Ok(message(StatusCode::BAD_REQUEST, "`password` must be a string or null."));
    }

    // Hashing runs scrypt: priced per account. Clearing is free.
    if wants_password && body["password"].as_str().is_some_and(|p| !p.is_empty()) {
        let gate = crate::password::check_rate_limit(
            &format!("upload-password:{}", owner.id),
            PASSWORD_LIMIT,
            PASSWORD_WINDOW_MS,
        );
        if !gate.allowed {
            let mut res = json(
                StatusCode::TOO_MANY_REQUESTS,
                &json!({ "message": "Too many password changes. Try again shortly.", "reason": "rate-limited" }),
            );
            let secs = (gate.retry_after_ms as f64 / 1000.0).ceil() as i64;
            if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
                res.headers_mut().insert("retry-after", v);
            }
            return Ok(res);
        }
    }

    // ── writes ──
    let actor =
        UploadActor { user_id: owner.id.clone(), email: Some(owner.email.clone()), is_admin: false, ip: String::new() };
    if let Err(resp) = apply_patch(
        &state,
        &owner,
        &actor,
        &ids,
        &body,
        folder_id.as_deref(),
        Wants {
            folder: wants_folder,
            published: wants_published,
            code: wants_code,
            expiry: wants_expiry,
            max_views: wants_max_views,
            password: wants_password,
        },
    )
    .await?
    {
        return Ok(resp);
    }

    // ── what the client should now draw ──
    let rows = actionable_uploads(&state.db, &actor, &ids).await?;
    let items: Vec<ApiItem> = to_file_items(&rows, &Default::default())
        .into_iter()
        .zip(&rows)
        .map(|(f, row)| api_item(&state, f, &owner, row.deleted_at.is_some()))
        .collect();

    let mut payload = Map::new();
    payload.insert("message".into(), json!("OK"));
    payload.insert("requested".into(), json!(ids.len()));
    payload.insert("affected".into(), json!(rows.len()));
    payload.insert("items".into(), serde_json::to_value(&items).map_err(AppError::internal)?);
    if wants_folder {
        payload.insert("folderId".into(), json!(folder_id));
    }
    Ok(json(StatusCode::OK, &Value::Object(payload)))
}

struct Wants {
    folder: bool,
    published: bool,
    code: bool,
    expiry: bool,
    max_views: bool,
    password: bool,
}

/// The writes, in upstream's order (the code last, so the rows read back
/// already answer on the new URL). A refusal comes back as the response.
async fn apply_patch(
    state: &AppState,
    owner: &UserRow,
    actor: &UploadActor,
    ids: &[Value],
    body: &Map<String, Value>,
    folder_id: Option<&str>,
    wants: Wants,
) -> AppResult<Result<(), Response>> {
    let db = &state.db;
    if wants.folder {
        match move_uploads_to_folder_for(db, &owner.id, ids, folder_id).await {
            Ok(_) => {}
            // An unknown or foreign TARGET folder: the same 404 GET gives.
            Err(FolderError::Op(e)) => {
                return Ok(Err(json(
                    StatusCode::NOT_FOUND,
                    &json!({ "message": e.message, "reason": e.reason.as_str() }),
                )));
            }
            Err(FolderError::Other(e)) => return Err(AppError::Internal(e)),
        }
    }
    let outcome: Result<(), UploadError> = async {
        if wants.published {
            set_published_for(db, actor, ids, body["published"].as_bool().unwrap_or(false)).await?;
        }
        if wants.expiry {
            set_expiry_for(db, actor, ids, Some(&body["expiresAt"])).await?;
        }
        if wants.max_views {
            set_max_views_for(db, actor, ids, Some(&body["maxViews"])).await?;
        }
        if wants.password {
            set_password_for(db, actor, ids, Some(&body["password"])).await?;
        }
        if wants.code {
            set_vanity_for(db, actor, ids.first(), Some(&body["code"])).await?;
        }
        Ok(())
    }
    .await;
    match outcome {
        Ok(()) => Ok(Ok(())),
        Err(UploadError::Op(e)) => {
            Ok(Err(json(upload_status(e.reason), &json!({ "message": e.message, "reason": e.reason.as_str() }))))
        }
        Err(UploadError::Other(e)) => Err(AppError::Internal(e)),
    }
}

/// The three things DELETE can be asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeleteMode {
    Trash,
    Permanent,
    Restore,
}

impl DeleteMode {
    fn as_str(self) -> &'static str {
        match self {
            DeleteMode::Trash => "trash",
            DeleteMode::Permanent => "permanent",
            DeleteMode::Restore => "restore",
        }
    }

    /// `reachableBy(mode)`: trash sees LIVE rows, restore TRASHED rows,
    /// permanent BOTH.
    fn reachable(self) -> &'static str {
        match self {
            DeleteMode::Trash => r#" and "uploads"."deleted_at" is null"#,
            DeleteMode::Restore => r#" and "uploads"."deleted_at" is not null"#,
            DeleteMode::Permanent => "",
        }
    }
}

/// `await req.json()`: the body decoded as UTF-8 (BOM dropped) and parsed.
async fn read_json_any(body: Body) -> Option<Value> {
    let mut buf: Vec<u8> = Vec::new();
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk.ok()?);
        if buf.len() > DELETE_BODY_MAX {
            return None;
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
    serde_json::from_str(text).ok()
}

/// DELETE: to the trash by default, permanently or back out on request.
async fn delete(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, body) = req.into_parts();
    let (owner, _) = resolve_api_owner(&state, &session, &parts.headers, TokenCapability::Write)
        .await?
        .or_respond(Some(PRIVATE_NO_STORE))?;

    let Some(input) = read_json_any(body).await else {
        return Ok(message(StatusCode::BAD_REQUEST, "Expected a JSON body."));
    };

    // Shape checked BEFORE the query: a non-uuid cannot be anybody's row.
    let ids: Vec<String> = match input.get("ids") {
        Some(Value::Array(list)) => {
            list.iter().filter_map(Value::as_str).filter(|s| is_uuid(s)).map(str::to_string).collect()
        }
        _ => Vec::new(),
    };
    if ids.is_empty() {
        return Ok(message(StatusCode::BAD_REQUEST, "No ids given."));
    }
    if ids.len() > MAX_IDS {
        return Ok(message(StatusCode::BAD_REQUEST, &format!("Too many ids (max {MAX_IDS}).")));
    }

    let mode = match input.get("mode").and_then(Value::as_str) {
        Some("permanent") => DeleteMode::Permanent,
        Some("restore") => DeleteMode::Restore,
        _ => DeleteMode::Trash,
    };

    // Ownership folded into the SELECT: another account's id is simply not
    // found.
    let uuids: Vec<Uuid> = ids.iter().filter_map(|s| Uuid::parse_str(s).ok()).collect();
    let sql = format!(
        r#"select {} from {} where ("uploads"."id" = any($1) and "uploads"."owner_id" = $2{})"#,
        Upload::COLUMNS,
        Upload::TABLE,
        mode.reachable()
    );
    let rows: Vec<Upload> = sqlx::query_as(&sql).bind(&uuids).bind(&owner.id).fetch_all(&state.db).await?;

    let mut done = 0usize;
    for row in &rows {
        match mode {
            DeleteMode::Permanent => {
                let storage = state.storage.get().map_err(AppError::Internal)?;
                delete_media_hard(&state.db, storage.as_ref(), &DeletableMedia::from(row)).await?
            }
            DeleteMode::Restore => restore_media(&state.db, row.id).await?,
            DeleteMode::Trash => {
                let storage = state.storage.get().map_err(AppError::Internal)?;
                soft_delete_media(&state.db, storage.as_ref(), row.id).await?
            }
        }
        done += 1;
    }

    Ok(json(
        StatusCode::OK,
        &json!({ "message": "OK", "mode": mode.as_str(), "requested": ids.len(), "affected": done }),
    ))
}
