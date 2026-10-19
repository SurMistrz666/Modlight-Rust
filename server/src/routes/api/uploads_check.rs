//! Port of `app/api/uploads/check/route.ts`: `POST /api/uploads/check`,
//! "before I send this, do I already have it?".
//!
//! A separate route rather than a verb on `/api/uploads`: a POST that creates
//! nothing under the URL where creating is the obvious meaning would lie about
//! the verb. POST rather than GET because 500 sha256 strings are ~33 KB of
//! query string, and a checksum in a URL is a checksum in an access log.
//!
//! Everything is scoped to the caller: a checksum is a global fact about
//! bytes, so an unscoped lookup would answer "does anybody on this server hold
//! this file". "No match" is reported as `unknown` rather than `new` whenever
//! the library holds files with no stored checksum (`duplicate_check`).

use std::collections::HashMap;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;
use indexmap::IndexSet;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::api_caller::resolve_api_owner;
use crate::api_json::{PRIVATE_NO_STORE, json_private, read_json_object};
use crate::api_token::TokenCapability;
use crate::auth_helpers::Sessions;
use crate::error::AppResult;
use crate::media::urls::{LinkOwner, media_urls};
use crate::state::AppState;
use crate::upload::duplicate_check::{
    CheckMatch, CheckResponse, CheckResult, ChecksumListIssue, MAX_CHECKSUMS, parse_checksums, status_for,
};
use crate::upload::duplicate_lookup::{ChecksumRow, count_unchecksummed, matches_by_checksum};

/// Ceiling on the request body, sized FROM `MAX_CHECKSUMS`: a legal
/// 500-checksum question is ~33 KB, and the 16 KiB default would refuse it
/// with a 413 before the count ceiling ever ran.
pub const CHECK_BODY_MAX: usize = 64 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/uploads/check", post(check))
}

/// `ISSUE_MESSAGE`
pub fn issue_message(issue: ChecksumListIssue) -> String {
    match issue {
        ChecksumListIssue::Empty => "No checksums given.".to_string(),
        ChecksumListIssue::Malformed => "Every entry must be a sha256 checksum (64 hex characters).".to_string(),
        ChecksumListIssue::TooMany => format!("Too many checksums (max {MAX_CHECKSUMS})."),
    }
}

async fn check(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, body) = req.into_parts();
    // Asking "do you already have these bytes" is a read of the library.
    // Before the body, so an unauthenticated caller cannot make us buffer one.
    let (owner, _) = resolve_api_owner(&state, &session, &parts.headers, TokenCapability::Read)
        .await?
        .or_respond(Some(PRIVATE_NO_STORE))?;

    let body = match read_json_object(&parts.headers, body, CHECK_BODY_MAX).await {
        Ok(b) => b,
        Err(e) => return Ok(json_private(e.status, &json!({ "message": e.message }))),
    };

    // Shape and ceiling BEFORE a query is built: an entry that is not a sha256
    // must never reach the database.
    let checksums = match parse_checksums(body.get("checksums")) {
        Ok(list) => list,
        Err(issue) => {
            return Ok(json_private(
                StatusCode::BAD_REQUEST,
                &json!({ "message": issue_message(issue), "reason": issue.as_str() }),
            ));
        }
    };

    let (matches, unchecksummed) = tokio::try_join!(
        matches_by_checksum(&state.db, &owner.id, &checksums),
        count_unchecksummed(&state.db, &owner.id),
    )?;
    let folder_names = names_for(&state.db, &owner.id, matches.by_checksum.values().flatten()).await?;

    // A library holding files this lookup cannot see is a library about which
    // "you do not have this" cannot be said.
    let blind = unchecksummed > 0 || matches.truncated;
    let link_owner = LinkOwner::of_user(&owner);
    let results: Vec<CheckResult> = checksums
        .iter()
        .map(|checksum| {
            let rows = matches.by_checksum.get(checksum).map(Vec::as_slice).unwrap_or(&[]);
            CheckResult {
                checksum: checksum.clone(),
                status: status_for(!rows.is_empty(), blind),
                matches: rows
                    .iter()
                    .map(|row| {
                        // Built from the OWNER's settings, as the listing route
                        // and ShareX do, so the same file offers the same string
                        // wherever it is shown.
                        let links = media_urls(&state.config, &link_owner, &row.code, &row.ext);
                        CheckMatch {
                            id: row.id.to_string(),
                            code: row.code.clone(),
                            filename: row.filename.clone(),
                            ext: row.ext.clone(),
                            mime_type: row.mime_type.clone(),
                            size: row.size,
                            created_at: row.created_at.to_iso_string(),
                            folder_id: row.folder_id.map(|f| f.to_string()),
                            folder_name: row.folder_id.and_then(|f| folder_names.get(&f).cloned()),
                            copy_url: if owner.copy_raw == Some(true) {
                                links.raw_url.clone()
                            } else {
                                links.url.clone()
                            },
                            url: links.url,
                            raw_url: links.raw_url,
                        }
                    })
                    .collect(),
            }
        })
        .collect();

    Ok(json_private(
        StatusCode::OK,
        &CheckResponse {
            message: "OK".into(),
            requested: checksums.len(),
            unchecksummed,
            truncated: matches.truncated,
            results,
        },
    ))
}

/// Names for the folders the matches live in: one scoped SELECT over a
/// handful of ids (not a join widening every row), with `owner_id` folded in
/// so a `folder_id` pointing outside this account yields no name.
async fn names_for<'a>(
    db: &PgPool,
    user_id: &str,
    rows: impl Iterator<Item = &'a ChecksumRow>,
) -> sqlx::Result<HashMap<Uuid, String>> {
    let ids: IndexSet<Uuid> = rows.filter_map(|r| r.folder_id).collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let ids: Vec<Uuid> = ids.into_iter().collect();
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        r#"select "id", "name" from "folders" where ("folders"."owner_id" = $1 and "folders"."id" = any($2))"#,
    )
    .bind(user_id)
    .bind(&ids)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().collect())
}
