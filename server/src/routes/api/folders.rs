//! Port of `app/api/folders/route.ts`: the caller's folders over HTTP, read
//! (`GET`) and changed (`POST` create, `PATCH` rename/move/publish, `DELETE`).
//!
//! Exists for the desktop companions, which can name a target folder on an
//! upload and need the ids, and which cannot invoke the panel's server
//! actions. None of the rules live here: `crate::folders` holds them (depth
//! ceiling, slug uniqueness, cycle prevention, ownership, the two delete
//! modes) and the panel's actions call the same functions. This file owns the
//! HTTP shape: who is calling, what a body may contain, and which status each
//! refusal becomes.
//!
//! Token first, session second (`api_caller`); the answer is scoped to one
//! account and never says anything about another.

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;
use serde::Serialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::api_caller::resolve_api_caller;
use crate::api_json::{JSON_BODY_MAX, PRIVATE_NO_STORE, json_private, read_json_object, supplied};
use crate::api_token::TokenCapability;
use crate::auth_helpers::Sessions;
use crate::data::folders::{folder_stats, list_user_folders, unfiled_stats};
use crate::error::{AppError, AppResult};
use crate::folders::{
    FolderError, FolderIssue, FolderOpError, FolderRecord, create_folder_for, delete_folder_for, get_owned_folder,
    move_folder_for, rename_folder_for, set_folder_visibility_for,
};
use crate::state::AppState;
use crate::upload::folder_target::ROOT_FOLDER;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/folders", get(list).post(create).patch(patch).delete(delete))
}

fn message(status: StatusCode, text: &str) -> Response {
    json_private(status, &json!({ "message": text }))
}

/// How each refusal reaches the wire. `not-found` is one 404 for an unknown
/// id, another account's id and a malformed id alike (no oracle); `too-deep`
/// and `cycle` are 409: well formed, refused by the current shape of the tree.
fn status_of(issue: FolderIssue) -> StatusCode {
    match issue {
        FolderIssue::NotFound => StatusCode::NOT_FOUND,
        FolderIssue::InvalidName => StatusCode::BAD_REQUEST,
        FolderIssue::TooDeep | FolderIssue::Cycle => StatusCode::CONFLICT,
    }
}

fn refuse(err: &FolderOpError) -> Response {
    json_private(status_of(err.reason), &json!({ "message": err.message, "reason": err.reason.as_str() }))
}

/// A refusal becomes its response; anything else is a 500.
fn settle(err: FolderError) -> AppResult<Response> {
    match err {
        FolderError::Op(e) => Ok(refuse(&e)),
        FolderError::Other(e) => Err(AppError::Internal(e)),
    }
}

/// One folder in the shape GET's `folders[]` entries use: same keys, same
/// order, so a client draws one tile from either answer.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FolderView {
    id: Uuid,
    name: String,
    slug: String,
    parent_id: Option<Uuid>,
    is_public: bool,
    /// DIRECT contents only, as in the listing.
    file_count: i64,
    size: i64,
    sort_order: i32,
}

async fn folder_view(state: &AppState, owner_id: &str, f: FolderRecord) -> AppResult<FolderView> {
    let stats = folder_stats(&state.db, owner_id, &f.id.to_string()).await?;
    Ok(FolderView {
        id: f.id,
        name: f.name,
        slug: f.slug,
        parent_id: f.parent_id,
        is_public: f.is_public,
        file_count: stats.count,
        size: stats.size,
        sort_order: f.sort_order,
    })
}

/// GET: every folder of the caller with its direct file count and bytes (the
/// panel's own grouped scan, not N+1), plus the "outside any folder" shelf.
async fn list(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, _) = req.into_parts();
    let (caller, _) = resolve_api_caller(&state, &session, &parts.headers, TokenCapability::Read)
        .await?
        .or_respond(Some(PRIVATE_NO_STORE))?;
    let rows = list_user_folders(&state.db, &caller.id).await?;
    let unfiled = unfiled_stats(&state.db, &caller.id).await?;
    let folders: Vec<FolderView> = rows
        .into_iter()
        .map(|f| FolderView {
            id: f.id,
            name: f.name,
            slug: f.slug,
            parent_id: f.parent_id,
            is_public: f.is_public,
            // DIRECT contents only: the client already holds every folder and
            // its parent, so rolling the subtree up is one loop there.
            file_count: f.count,
            size: f.size,
            sort_order: f.sort_order,
        })
        .collect();
    Ok(json_private(
        StatusCode::OK,
        &json!({
            // The literal a client sends back as `folderId` to mean "no folder".
            "rootValue": ROOT_FOLDER,
            "unfiled": { "fileCount": unfiled.count, "size": unfiled.size },
            "folders": folders,
        }),
    ))
}

/// The id a write names, or `None` (a 400). Only the SHAPE is judged here:
/// existence and ownership stay with `crate::folders` (one 404).
fn id_of(body: &Map<String, Value>) -> Option<String> {
    match body.get("id") {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// A parent id from a body: a folder id, or `None` for the root. The
/// `rootValue` sentinel and `""` are accepted alongside JSON `null`.
/// `Err(())` for any other type.
#[allow(clippy::result_unit_err)]
pub fn parent_of(raw: &Value) -> Result<Option<String>, ()> {
    match raw {
        Value::Null => Ok(None),
        Value::String(s) if s == ROOT_FOLDER || s.is_empty() => Ok(None),
        Value::String(s) => Ok(Some(s.clone())),
        _ => Err(()),
    }
}

/// POST: create a folder. Body `{ name, parentId? }`.
async fn create(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, body) = req.into_parts();
    // Resolved before the body is read, so an unauthenticated flood costs nothing.
    let (caller, _) = resolve_api_caller(&state, &session, &parts.headers, TokenCapability::Write)
        .await?
        .or_respond(Some(PRIVATE_NO_STORE))?;
    let body = match read_json_object(&parts.headers, body, JSON_BODY_MAX).await {
        Ok(b) => b,
        Err(e) => return Ok(message(e.status, &e.message)),
    };

    if !body.get("name").is_some_and(Value::is_string) {
        return Ok(message(StatusCode::BAD_REQUEST, "`name` is required and must be a string."));
    }
    let mut parent_id: Option<String> = None;
    if supplied(&body, "parentId") {
        match parent_of(&body["parentId"]) {
            Ok(p) => parent_id = p,
            Err(()) => return Ok(message(StatusCode::BAD_REQUEST, "`parentId` must be a string or null.")),
        }
    }

    let folder = match create_folder_for(&state.db, &caller.id, body.get("name"), parent_id.as_deref()).await {
        Ok(f) => f,
        Err(e) => return settle(e),
    };
    let view = folder_view(&state, &caller.id, folder).await?;
    Ok(json_private(StatusCode::CREATED, &json!({ "message": "OK", "folder": view })))
}

/// PATCH: rename, move, or publish/unpublish. Body `{ id, name?, parentId?,
/// isPublic? }`. An absent field is left alone; `parentId: null` is "move to
/// the root". The rename runs first so the move re-derives the slug from the
/// NEW name in the NEW parent.
async fn patch(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, body) = req.into_parts();
    let (caller, _) = resolve_api_caller(&state, &session, &parts.headers, TokenCapability::Write)
        .await?
        .or_respond(Some(PRIVATE_NO_STORE))?;
    let body = match read_json_object(&parts.headers, body, JSON_BODY_MAX).await {
        Ok(b) => b,
        Err(e) => return Ok(message(e.status, &e.message)),
    };

    let Some(id) = id_of(&body) else {
        return Ok(message(StatusCode::BAD_REQUEST, "`id` is required and must be a string."));
    };
    let wants_name = supplied(&body, "name");
    let wants_parent = supplied(&body, "parentId");
    let wants_public = supplied(&body, "isPublic");

    if !wants_name && !wants_parent && !wants_public {
        return Ok(message(StatusCode::BAD_REQUEST, "Nothing to change: send `name`, `parentId` or `isPublic`."));
    }
    if wants_name && !body["name"].is_string() {
        return Ok(message(StatusCode::BAD_REQUEST, "`name` must be a string."));
    }
    if wants_public && !body["isPublic"].is_boolean() {
        return Ok(message(StatusCode::BAD_REQUEST, "`isPublic` must be a boolean."));
    }
    let mut parent_id: Option<String> = None;
    if wants_parent {
        match parent_of(&body["parentId"]) {
            Ok(p) => parent_id = p,
            Err(()) => return Ok(message(StatusCode::BAD_REQUEST, "`parentId` must be a string or null.")),
        }
    }

    let outcome: Result<FolderRecord, FolderError> = async {
        // The owner-scoped read first: missing, foreign or not a uuid is one
        // 404 before any field is looked at.
        let mut folder = get_owned_folder(&state.db, &caller.id, &id).await?;
        if wants_name {
            folder = rename_folder_for(&state.db, &caller.id, &id, body.get("name")).await?;
        }
        if wants_parent {
            folder = move_folder_for(&state.db, &caller.id, &id, parent_id.as_deref()).await?;
        }
        if wants_public {
            let public = body["isPublic"].as_bool().unwrap_or(false);
            folder = set_folder_visibility_for(&state.db, &caller.id, &id, public).await?;
        }
        Ok(folder)
    }
    .await;
    let folder = match outcome {
        Ok(f) => f,
        Err(e) => return settle(e),
    };
    let view = folder_view(&state, &caller.id, folder).await?;
    Ok(json_private(StatusCode::OK, &json!({ "message": "OK", "folder": view })))
}

/// DELETE: remove a folder. Body `{ id, withContents?: boolean }`. Only an
/// explicit `true` empties the tree into the trash; omitting it gets the SAFE
/// mode (files fall back to "no folder", children re-parent).
async fn delete(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, body) = req.into_parts();
    let (caller, _) = resolve_api_caller(&state, &session, &parts.headers, TokenCapability::Write)
        .await?
        .or_respond(Some(PRIVATE_NO_STORE))?;
    let body = match read_json_object(&parts.headers, body, JSON_BODY_MAX).await {
        Ok(b) => b,
        Err(e) => return Ok(message(e.status, &e.message)),
    };

    let Some(id) = id_of(&body) else {
        return Ok(message(StatusCode::BAD_REQUEST, "`id` is required and must be a string."));
    };
    if supplied(&body, "withContents") && !body["withContents"].is_boolean() {
        return Ok(message(StatusCode::BAD_REQUEST, "`withContents` must be a boolean."));
    }
    // `== true`, not truthy: a stringified checkbox must not empty a tree.
    let with_contents = body.get("withContents") == Some(&Value::Bool(true));

    let storage = state.storage.get().ok();
    let result = match delete_folder_for(&state.db, storage.as_deref(), &caller.id, &id, with_contents).await {
        Ok(r) => r,
        Err(e) => return settle(e),
    };
    Ok(json_private(
        StatusCode::OK,
        &json!({
            "message": "OK",
            "withContents": with_contents,
            "folders": result.folders,
            "trashed": result.trashed,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parents() {
        assert_eq!(parent_of(&Value::Null), Ok(None));
        assert_eq!(parent_of(&json!(ROOT_FOLDER)), Ok(None));
        assert_eq!(parent_of(&json!("")), Ok(None));
        assert_eq!(parent_of(&json!("abc")), Ok(Some("abc".to_string())));
        assert_eq!(parent_of(&json!(7)), Err(()));
        assert_eq!(parent_of(&json!(false)), Err(()));
    }

    #[test]
    fn statuses() {
        assert_eq!(status_of(FolderIssue::NotFound), StatusCode::NOT_FOUND);
        assert_eq!(status_of(FolderIssue::InvalidName), StatusCode::BAD_REQUEST);
        assert_eq!(status_of(FolderIssue::TooDeep), StatusCode::CONFLICT);
        assert_eq!(status_of(FolderIssue::Cycle), StatusCode::CONFLICT);
    }
}
