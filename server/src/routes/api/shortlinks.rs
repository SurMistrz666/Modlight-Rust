//! Port of `app/api/shortlinks/route.ts`: the caller's short links over HTTP
//! (shortening the clipboard is a hotkey in the desktop client, and a hotkey
//! cannot invoke a server action).
//!
//! Minting goes through `shortlinks::create_short_link_for`, the same core the
//! panel's action uses: its target normalisation refuses `javascript:` and
//! `data:` URLs, and that guard existing in only one of two paths is how a
//! shortener becomes an XSS vector on its own domain.
//!
//! Every answer is a JSON body; a refusal carries `ok: false` and a `reason`
//! (the contract), plus a `message` only for a MALFORMED request.

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::get;
use serde::Serialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::api_caller::{ApiAuth, ApiCaller, resolve_api_caller};
use crate::api_json::{JSON_BODY_MAX, json_private, read_json_object};
use crate::api_token::TokenCapability;
use crate::auth_helpers::Sessions;
use crate::db::types::JsDate;
use crate::error::{AppError, AppResult};
use crate::shortlinks::{
    CreateShortLinkInput, CreateShortLinkResult, ShortLinkIssue, create_short_link_for, short_link_url,
};
use crate::state::AppState;

/// Creations allowed per account, and the window they are counted over.
const CREATE_LIMIT: u32 = 60;
const CREATE_WINDOW_MS: i64 = 60_000;

/// scrypt's first step is linear in the password: an unbounded one is CPU the
/// caller gets to spend for us.
const PASSWORD_MAX: usize = 1024;

/// Every field is bounded (4 KiB target, 256-char code, 1 KiB password).
const BODY_MAX: usize = JSON_BODY_MAX;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/shortlinks", get(list).post(create))
}

/// How each refusal reaches the wire. `no-tag` is 422, not 409: a client
/// reading 409 as "taken, try another" would loop, and no fresh code helps an
/// account that has no tag yet.
fn status_of(reason: ShortLinkIssue) -> StatusCode {
    match reason {
        ShortLinkIssue::InvalidUrl | ShortLinkIssue::InvalidCode | ShortLinkIssue::CodeTooLong => {
            StatusCode::BAD_REQUEST
        }
        ShortLinkIssue::CodeTaken => StatusCode::CONFLICT,
        ShortLinkIssue::RootNotAllowed => StatusCode::FORBIDDEN,
        ShortLinkIssue::NoTag => StatusCode::UNPROCESSABLE_ENTITY,
        ShortLinkIssue::Error => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// `{ ok: false, reason, message? }`
fn refuse(reason: ShortLinkIssue, message: Option<&str>, status: Option<StatusCode>) -> Response {
    let mut body = Map::new();
    body.insert("ok".into(), json!(false));
    body.insert("reason".into(), json!(reason.as_str()));
    if let Some(m) = message.filter(|m| !m.is_empty()) {
        body.insert("message".into(), json!(m));
    }
    json_private(status.unwrap_or_else(|| status_of(reason)), &Value::Object(body))
}

/// The caller, or `{ ok: false, ...refusal }` with the refusal's status.
fn caller_or_refusal(auth: ApiAuth<ApiCaller>) -> AppResult<ApiCaller> {
    match auth {
        ApiAuth::Ok { caller, .. } => Ok(caller),
        ApiAuth::Refused { status, body } => {
            let mut out = Map::new();
            out.insert("ok".into(), json!(false));
            if let Value::Object(fields) = body {
                out.extend(fields);
            }
            Err(AppError::Response(Box::new(json_private(status, &Value::Object(out)))))
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LinkItem {
    id: Uuid,
    code: String,
    url: String,
    target: String,
    clicks: i32,
    has_password: bool,
    last_used_at: Option<String>,
    expires_at: Option<String>,
    created_at: String,
}

/// GET: the caller's own short links, newest first.
async fn list(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, _) = req.into_parts();
    let auth = resolve_api_caller(&state, &session, &parts.headers, TokenCapability::Read).await?;
    let owner = caller_or_refusal(auth)?;
    // `has_password` asked as a boolean in SQL so the hash never leaves
    // Postgres at all.
    type Row = (Uuid, String, String, i32, bool, Option<JsDate>, Option<JsDate>, JsDate);
    let rows: Vec<Row> = sqlx::query_as(
        r#"select "id", "code", "target", "clicks", "password_hash" is not null, "last_used_at", "expires_at", "created_at"
           from "short_links" where "short_links"."owner_id" = $1 order by "short_links"."created_at" desc"#,
    )
    .bind(&owner.id)
    .fetch_all(&state.db)
    .await?;
    let links: Vec<LinkItem> = rows
        .into_iter()
        .map(|(id, code, target, clicks, has_password, last_used_at, expires_at, created_at)| LinkItem {
            id,
            // Absolute, built the way the panel builds it: a client never has
            // to know whether there is a separate shortener domain.
            url: short_link_url(&state.config, &code),
            code,
            target,
            clicks,
            has_password,
            last_used_at: last_used_at.map(|d| d.to_iso_string()),
            expires_at: expires_at.map(|d| d.to_iso_string()),
            created_at: created_at.to_iso_string(),
        })
        .collect();
    Ok(json_private(StatusCode::OK, &json!({ "links": links })))
}

/// POST: mint a short link. Body `{ target, customCode?, expiresInDays?,
/// password?, useUserCode? }`.
async fn create(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, body) = req.into_parts();
    let auth = resolve_api_caller(&state, &session, &parts.headers, TokenCapability::Write).await?;
    let owner = caller_or_refusal(auth)?;

    // Priced per call: a link may carry a password and hashing runs scrypt.
    // Keyed on the account (a token is an account; rotating IPs would sidestep
    // an address key).
    let gate =
        crate::password::check_rate_limit(&format!("shortlink-create:{}", owner.id), CREATE_LIMIT, CREATE_WINDOW_MS);
    if !gate.allowed {
        let mut res = json_private(
            StatusCode::TOO_MANY_REQUESTS,
            &json!({ "ok": false, "reason": "rate-limited", "message": "Too many short links. Try again shortly." }),
        );
        let secs = (gate.retry_after_ms as f64 / 1000.0).ceil() as i64;
        if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
            res.headers_mut().insert("retry-after", v);
        }
        return Ok(res);
    }

    // `readCreateInput`: the same Content-Type (CSRF control), size and shape
    // rules as `read_json_object`, each refusal as `reason: "error"`.
    let b = match read_json_object(&parts.headers, body, BODY_MAX).await {
        Ok(b) => b,
        Err(e) => return Ok(refuse(ShortLinkIssue::Error, Some(&e.message), Some(e.status))),
    };
    let input = match read_create_input(&b) {
        Ok(input) => input,
        Err((reason, message)) => return Ok(refuse(reason, Some(&message), Some(StatusCode::BAD_REQUEST))),
    };

    // Caught here rather than in the core: this endpoint's contract is that
    // EVERY answer is JSON with a `reason`.
    let result = match create_short_link_for(&state.db, &state.config, &owner.id, &input).await {
        Ok(r) => r,
        Err(err) => {
            tracing::error!(error = ?err, "short link creation failed");
            return Ok(refuse(ShortLinkIssue::Error, None, None));
        }
    };
    match result {
        CreateShortLinkResult::Refused(reason) => Ok(refuse(reason, None, None)),
        // No revalidation: there is no router cache behind a desktop client.
        ok @ CreateShortLinkResult::Ok { .. } => Ok(json_private(StatusCode::CREATED, &ok.to_json())),
    }
}

/// `undefined` or `null`: the field was not supplied.
fn absent(v: Option<&Value>) -> bool {
    matches!(v, None | Some(Value::Null))
}

/// Validate the body into the core's input. Every optional field is absent or
/// exactly its type: a wrong type is refused, never coerced or ignored (an
/// ignored `customCode` would hand back a random link the client believes is
/// the name it asked for). Value rules stay in the core.
fn read_create_input(b: &Map<String, Value>) -> Result<CreateShortLinkInput, (ShortLinkIssue, String)> {
    let malformed = |m: &str| Err((ShortLinkIssue::Error, m.to_string()));
    let Some(Value::String(target)) = b.get("target") else {
        return Err((ShortLinkIssue::InvalidUrl, "`target` is required and must be a string.".into()));
    };
    if !absent(b.get("customCode")) && !b["customCode"].is_string() {
        return Err((ShortLinkIssue::InvalidCode, "`customCode` must be a string.".into()));
    }
    if !absent(b.get("expiresInDays")) && !crate::api_json::is_js_number(&b["expiresInDays"]) {
        return malformed("`expiresInDays` must be a number of days.");
    }
    if !absent(b.get("password")) && !b["password"].is_string() {
        return malformed("`password` must be a string.");
    }
    if b.get("password").and_then(Value::as_str).is_some_and(|p| p.encode_utf16().count() > PASSWORD_MAX) {
        return malformed(&format!("`password` must be at most {PASSWORD_MAX} characters."));
    }
    if !absent(b.get("useUserCode")) && !b["useUserCode"].is_boolean() {
        return malformed("`useUserCode` must be a boolean.");
    }
    let keep = |key: &str, ok: fn(&Value) -> bool| b.get(key).filter(|v| ok(v)).cloned();
    Ok(CreateShortLinkInput {
        target: Value::String(target.clone()),
        custom_code: keep("customCode", Value::is_string),
        expires_in_days: keep("expiresInDays", crate::api_json::is_js_number),
        password: keep("password", Value::is_string),
        // Left out when absent so the core's own default (prefixed) applies.
        use_user_code: keep("useUserCode", Value::is_boolean),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(v: Value) -> Result<CreateShortLinkInput, (ShortLinkIssue, String)> {
        read_create_input(v.as_object().unwrap())
    }

    #[test]
    fn create_input_refuses_wrong_types() {
        let e = input(json!({})).unwrap_err();
        assert_eq!(e, (ShortLinkIssue::InvalidUrl, "`target` is required and must be a string.".into()));
        let e = input(json!({ "target": "x", "customCode": 5 })).unwrap_err();
        assert_eq!(e.0, ShortLinkIssue::InvalidCode);
        let e = input(json!({ "target": "x", "expiresInDays": "5" })).unwrap_err();
        assert_eq!(e, (ShortLinkIssue::Error, "`expiresInDays` must be a number of days.".into()));
        let e = input(json!({ "target": "x", "password": "p".repeat(1025) })).unwrap_err();
        assert_eq!(e.1, "`password` must be at most 1024 characters.");
        let e = input(json!({ "target": "x", "useUserCode": "no" })).unwrap_err();
        assert_eq!(e.1, "`useUserCode` must be a boolean.");
        let ok = input(json!({ "target": "x", "customCode": null, "expiresInDays": 3, "useUserCode": false })).unwrap();
        assert!(ok.custom_code.is_none());
        assert_eq!(ok.expires_in_days, Some(json!(3)));
        assert_eq!(ok.use_user_code, Some(json!(false)));
        assert!(input(json!({ "target": "x", "password": "p".repeat(1024) })).is_ok());
    }

    #[test]
    fn statuses() {
        assert_eq!(status_of(ShortLinkIssue::NoTag), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(status_of(ShortLinkIssue::RootNotAllowed), StatusCode::FORBIDDEN);
        assert_eq!(status_of(ShortLinkIssue::CodeTaken), StatusCode::CONFLICT);
        assert_eq!(status_of(ShortLinkIssue::Error), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
