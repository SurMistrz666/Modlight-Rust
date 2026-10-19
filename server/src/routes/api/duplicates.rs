//! Port of `app/api/duplicates/route.ts`: retroactive duplicate detection over
//! HTTP, for the desktop client (a server action is unreachable from it).
//!
//! Nothing here re-implements the rules: `scan_duplicates`,
//! `dismiss_checksums` and `restore_checksums` are the functions the panel's
//! actions call, so the two surfaces cannot disagree about what a duplicate
//! is. TRASHING duplicates has no verb here on purpose: it is
//! `DELETE /api/uploads` with `mode: "trash"`.

use std::net::SocketAddr;

use axum::Router;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::api_caller::resolve_api_owner;
use crate::api_json::{PRIVATE_NO_STORE, json_private, read_json_object};
use crate::api_token::TokenCapability;
use crate::audit::record_audit;
use crate::auth_helpers::Sessions;
use crate::data::duplicates::{
    DismissalInput, REMINDER_MIN_BYTES, REMINDER_MIN_REDUNDANT, dismiss_checksums, reminder_due, restore_checksums,
    scan_duplicates,
};
use crate::error::{AppError, AppResult};
use crate::routes::api::uploads_check::issue_message;
use crate::state::AppState;
use crate::upload::duplicate_check::parse_checksums;

/// Ceiling on a POST body, sized FROM `MAX_CHECKSUMS`: one `keep` entry is
/// ~110 bytes once quoted, so a legal 500-set request is ~55 KB.
pub const BODY_MAX: usize = 96 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/duplicates", get(scan).post(decide))
}

/// GET: scan this account's library for duplicate sets. Read-only, so a
/// `read` token may SEE the duplication; removing it is a write elsewhere.
async fn scan(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, _) = req.into_parts();
    let (owner, _) = resolve_api_owner(&state, &session, &parts.headers, TokenCapability::Read)
        .await?
        .or_respond(Some(PRIVATE_NO_STORE))?;

    let scan = scan_duplicates(&state.db, &state.config, &owner.id).await?;
    let due = reminder_due(&scan.active);
    let Value::Object(mut payload) = serde_json::to_value(&scan).map_err(AppError::internal)? else {
        return Err(AppError::internal(anyhow::anyhow!("a scan serialises as an object")));
    };
    // Whether the account is over a reminder threshold RIGHT NOW (computed
    // from `active`), and the thresholds themselves: the numbers belong to the
    // server, so a client never nags at old figures.
    payload.insert("due".into(), json!(due));
    payload.insert("thresholds".into(), json!({ "redundant": REMINDER_MIN_REDUNDANT, "bytes": REMINDER_MIN_BYTES }));
    Ok(json_private(StatusCode::OK, &Value::Object(payload)))
}

/// `Number(value)` for a value JSON can carry (`undefined` when absent).
pub fn js_to_number(value: Option<&Value>) -> f64 {
    match value {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => crate::config::js_number(s),
        Some(Value::Array(items)) => crate::config::js_number(&js_array_string(items)),
        Some(Value::Object(_)) => f64::NAN,
    }
}

/// `String(array)`: elements joined by commas, null and undefined as "".
fn js_array_string(items: &[Value]) -> String {
    items
        .iter()
        .map(|v| match v {
            Value::Null => String::new(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => crate::i18n::format::js_number_to_string(n.as_f64().unwrap_or(f64::NAN)),
            Value::String(s) => s.clone(),
            Value::Array(inner) => js_array_string(inner),
            Value::Object(_) => "[object Object]".to_string(),
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// `whole(value)`: `Math.trunc(Number(value))` when finite and positive, else 0.
pub fn whole(value: Option<&Value>) -> f64 {
    let n = js_to_number(value).trunc();
    if n.is_finite() && n > 0.0 { n } else { 0.0 }
}

/// POST: decide what happens to a set the scan found.
/// `{ "keep": [{ "checksum", "copies", "bytes" }] }` stops counting sets;
/// `{ "countAgain": ["..."] }` undoes that. Exactly one of the two keys: both
/// at once have no obviously right ordering, so they are refused.
async fn decide(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, body) = req.into_parts();
    // Both verbs change what the account will be told in future: writes.
    // Before the body, so an unauthenticated caller cannot make us buffer one.
    let (owner, _) = resolve_api_owner(&state, &session, &parts.headers, TokenCapability::Write)
        .await?
        .or_respond(Some(PRIVATE_NO_STORE))?;
    let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    let ip = crate::password::client_ip(&parts.headers, peer, state.config.trust_proxy);

    let body: Map<String, Value> = match read_json_object(&parts.headers, body, BODY_MAX).await {
        Ok(b) => b,
        Err(e) => return Ok(json_private(e.status, &json!({ "message": e.message }))),
    };

    let wants_keep = body.get("keep").is_some_and(Value::is_array);
    let wants_count_again = body.get("countAgain").is_some_and(Value::is_array);
    if wants_keep == wants_count_again {
        return Ok(json_private(
            StatusCode::BAD_REQUEST,
            &json!({ "message": "Send exactly one of `keep` or `countAgain`.", "reason": "one-verb" }),
        ));
    }

    let refuse = |issue: crate::upload::duplicate_check::ChecksumListIssue| {
        json_private(StatusCode::BAD_REQUEST, &json!({ "message": issue_message(issue), "reason": issue.as_str() }))
    };

    if wants_count_again {
        let checksums = match parse_checksums(body.get("countAgain")) {
            Ok(c) => c,
            Err(issue) => return Ok(refuse(issue)),
        };
        // Ownership lives inside restore_checksums' WHERE clause: a digest
        // alone would let one account clear another's decision.
        let restored = restore_checksums(&state.db, &owner.id, &checksums).await?;
        record_audit(
            &state.db,
            &owner.id,
            "uploads.duplicates_counted_again",
            json!({ "count": restored, "requested": checksums.len(), "via": "api" }),
            &ip,
        )
        .await;
        return Ok(json_private(StatusCode::OK, &json!({ "message": "OK", "restored": restored })));
    }

    // `typeof entry === "object" && entry !== null`: arrays pass too (and
    // carry no checksum, which then reads as malformed).
    let list: Vec<&Value> = body["keep"]
        .as_array()
        .map(|a| a.iter().filter(|e| e.is_object() || e.is_array()).collect())
        .unwrap_or_default();

    // The same validator the upload-time check uses: the dismissals column is
    // a varchar, so an arbitrary string would be stored and match nothing for
    // ever.
    let asked: Vec<Value> = list.iter().map(|e| e.get("checksum").cloned().unwrap_or(Value::Null)).collect();
    let checksums = match parse_checksums(Some(&Value::Array(asked))) {
        Ok(c) => c,
        Err(issue) => return Ok(refuse(issue)),
    };

    // `parse_checksums` lower-cases and de-duplicates, so the copies/bytes
    // note is matched back by the NORMALISED digest, not by position.
    let mut by_checksum: IndexMap<String, DismissalInput> = IndexMap::new();
    for entry in &list {
        let checksum = match entry.get("checksum") {
            Some(Value::String(s)) => crate::config::js_trim(s).to_lowercase(),
            _ => String::new(),
        };
        if !checksum.is_empty() {
            let input = DismissalInput {
                checksum: checksum.clone(),
                copies: whole(entry.get("copies")),
                bytes: whole(entry.get("bytes")),
            };
            by_checksum.insert(checksum, input);
        }
    }
    let entries: Vec<DismissalInput> = checksums
        .iter()
        .map(|c| by_checksum.get(c).cloned().unwrap_or(DismissalInput { checksum: c.clone(), copies: 0.0, bytes: 0.0 }))
        .collect();

    let kept = dismiss_checksums(&state.db, &owner.id, &entries).await?;
    record_audit(
        &state.db,
        &owner.id,
        "uploads.duplicates_kept",
        json!({ "count": kept, "requested": checksums.len(), "via": "api" }),
        &ip,
    )
    .await;
    Ok(json_private(StatusCode::OK, &json!({ "message": "OK", "kept": kept, "checksums": checksums })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_like_javascript() {
        assert_eq!(whole(Some(&json!(4))), 4.0);
        assert_eq!(whole(Some(&json!(4.9))), 4.0);
        assert_eq!(whole(Some(&json!(-3))), 0.0);
        assert_eq!(whole(None), 0.0);
        assert_eq!(whole(Some(&json!(null))), 0.0);
        assert_eq!(whole(Some(&json!(true))), 1.0);
        assert_eq!(whole(Some(&json!(" 12 "))), 12.0);
        assert_eq!(whole(Some(&json!("0x10"))), 16.0);
        assert_eq!(whole(Some(&json!([7]))), 7.0);
        assert_eq!(whole(Some(&json!([1, 2]))), 0.0);
        assert_eq!(whole(Some(&json!({}))), 0.0);
        assert_eq!(whole(Some(&json!("abc"))), 0.0);
        assert!(js_to_number(Some(&json!([]))) == 0.0);
    }
}
