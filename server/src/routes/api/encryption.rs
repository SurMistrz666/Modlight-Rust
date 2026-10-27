//! Not upstream: the file vault's routes (docs/ENCRYPTION.md section 6).
//!
//! | Route | Who | What |
//! |---|---|---|
//! | `GET /api/encryption` | the account (token or session) | `{state, unlocked, progress}` |
//! | `POST /api/encryption/unlock`, JSON | the account's API token | `{grant, expiresAt}` for `X-Modlight-Unlock` |
//! | `POST /api/encryption/unlock`, form | a browser on a raw link's unlock form | the grant cookie, `303` back |
//! | `POST /api/encryption/lock` | an API client | revokes the grant it sends |
//!
//! The file password is never logged and never echoed.

use std::net::SocketAddr;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use axum::routing::{get, post};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::api_caller::resolve_api_owner;
use crate::api_json::{PRIVATE_NO_STORE, json_private, read_json_object};
use crate::api_token::TokenCapability;
use crate::auth_helpers::{RequestSession, Sessions};
use crate::error::AppResult;
use crate::http::set_cookie::ResponseCookies;
use crate::state::AppState;
use crate::vault::access::{Access, access_with, parse_grant};
use crate::vault::gate::{GateError, gate_document, safe_next};
use crate::vault::store::{self, Progress};
use crate::vault::unlock::{self, Asker, UnlockError, grant_cookie};
use crate::vault::{UNLOCK_HEADER, VaultState};

/// A password and a label: small.
const BODY_MAX: usize = 4 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/encryption", get(status))
        .route("/api/encryption/unlock", post(unlock_route))
        .route("/api/encryption/lock", post(lock))
}

fn peer_ip(state: &AppState, req_headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    crate::password::client_ip(req_headers, peer, state.config.trust_proxy)
}

/// `GET /api/encryption`
async fn status(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, _) = req.into_parts();
    let (owner, _) = resolve_api_owner(&state, &session, &parts.headers, TokenCapability::Read)
        .await?
        .or_respond(Some(PRIVATE_NO_STORE))?;
    let vault = store::load(&state.db, &owner.id).await?;
    let Some(vault) = vault else {
        return Ok(json_private(
            StatusCode::OK,
            &json!({ "state": "off", "unlocked": false, "progress": Progress::default() }),
        ));
    };
    let vault_state = store::state_of(&vault).unwrap_or(VaultState::On);
    let access = access_with(&state, &parts.headers, Some(&vault)).await?;
    let unlocked = vault_state.is_gated() && matches!(access, Access::Unlocked { grant: Some(_), .. });
    let progress = store::progress(&state.db, &owner.id, vault_state).await?;
    Ok(json_private(
        StatusCode::OK,
        &json!({ "state": vault_state.as_str(), "unlocked": unlocked, "progress": progress }),
    ))
}

fn api_error(status: StatusCode, error: &str) -> Response {
    json_private(status, &json!({ "error": error }))
}

fn rate_limited(retry_after_ms: i64) -> Response {
    let seconds = ((retry_after_ms.max(0) + 999) / 1000).max(1);
    let mut res =
        json_private(StatusCode::TOO_MANY_REQUESTS, &json!({ "error": "rate_limited", "retryAfter": seconds }));
    if let Ok(v) = HeaderValue::from_str(&seconds.to_string()) {
        res.headers_mut().insert(header::RETRY_AFTER, v);
    }
    res
}

/// `POST /api/encryption/unlock`
async fn unlock_route(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, body) = req.into_parts();
    let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    let cookies = parts.extensions.get::<ResponseCookies>().cloned().unwrap_or_default();
    let ip = peer_ip(&state, &parts.headers, peer);
    let content_type = parts.headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
    if content_type.to_ascii_lowercase().starts_with("application/x-www-form-urlencoded") {
        return form_unlock(&state, &session, &parts.headers, body, &ip, &cookies).await;
    }
    app_unlock(&state, &session, &parts.headers, body, &ip).await
}

/// The desktop app (any bearer client of the account's own token).
async fn app_unlock(
    state: &AppState,
    session: &RequestSession,
    headers: &HeaderMap,
    body: Body,
    ip: &str,
) -> AppResult<Response> {
    let (owner, _) =
        resolve_api_owner(state, session, headers, TokenCapability::Read).await?.or_respond(Some(PRIVATE_NO_STORE))?;
    let input = read_json_object(headers, body, BODY_MAX).await?;
    let password = match input.get("password") {
        Some(Value::String(p)) if !p.is_empty() => Zeroizing::new(p.clone()),
        _ => return Ok(api_error(StatusCode::BAD_REQUEST, "invalid_request")),
    };
    let label = match input.get("label") {
        Some(Value::String(l)) if !l.trim().is_empty() => Some(l.trim().chars().take(120).collect::<String>()),
        _ => Some("App".to_string()),
    };
    let Some(vault) = store::load(&state.db, &owner.id).await? else {
        return Ok(api_error(StatusCode::CONFLICT, "not_encrypted"));
    };
    match unlock::unlock(state, &vault, &password, Asker::App, ip, true, label.as_deref()).await? {
        Ok(granted) => {
            crate::audit::record_audit(
                &state.db,
                &owner.id,
                "vault.unlocked",
                unlock::audit_detail(Asker::App, Some(granted.id)),
                ip,
            )
            .await;
            Ok(json_private(
                StatusCode::OK,
                &json!({ "grant": granted.value.as_str(), "expiresAt": granted.expires_at.to_iso_string() }),
            ))
        }
        Err(UnlockError::Wrong) => {
            crate::audit::record_audit(&state.db, &owner.id, "vault.unlock_failed", json!({ "by": "app" }), ip).await;
            Ok(api_error(StatusCode::UNAUTHORIZED, "wrong_password"))
        }
        Err(UnlockError::NotEncrypted) => Ok(api_error(StatusCode::CONFLICT, "not_encrypted")),
        Err(UnlockError::RateLimited { retry_after_ms }) => Ok(rate_limited(retry_after_ms)),
    }
}

/// The unlock form of a raw link (vault::gate): `password`, `remember`,
/// `next` (the raw URL to go back to, same origin only).
async fn form_unlock(
    state: &AppState,
    session: &RequestSession,
    headers: &HeaderMap,
    body: Body,
    ip: &str,
    cookies: &ResponseCookies,
) -> AppResult<Response> {
    let locale = crate::i18n::request::request_locale(headers);
    // A form of this origin only (the page that renders it is ours).
    if headers.get(header::ORIGIN).is_some() && !crate::rpc::origin_matches(headers, state.config.trust_proxy) {
        return Ok(api_error(StatusCode::FORBIDDEN, "forbidden"));
    }
    let Ok(bytes) = to_bytes(body, BODY_MAX).await else {
        return Ok(api_error(StatusCode::PAYLOAD_TOO_LARGE, "too_large"));
    };
    let mut password = Zeroizing::new(String::new());
    let mut next = String::new();
    let mut remember = false;
    for (k, v) in form_urlencoded::parse(&bytes) {
        match k.as_ref() {
            "password" => *password = v.into_owned(),
            "next" => next = v.into_owned(),
            "remember" => remember = !v.is_empty(),
            _ => {}
        }
    }
    let Some(next) = safe_next(&next).map(str::to_string) else {
        return Ok(api_error(StatusCode::BAD_REQUEST, "invalid_request"));
    };
    let fail = |error: GateError| Ok(gate_document(state, locale, &next, Some(error)));

    let owner_id = crate::vault::access::owner_of_path(state, &next).await?;
    let vault = match owner_id.as_deref() {
        Some(o) => store::load(&state.db, o).await?,
        None => None,
    };
    let Some(vault) = vault.filter(|v| store::state_of(v).is_some_and(VaultState::is_gated)) else {
        let _ = unlock::admit_attempt("unknown", Asker::Visitor, ip);
        return fail(GateError::Wrong);
    };
    let viewer = session.get_or_none().await;
    let asker = if viewer.as_ref().is_some_and(|s| s.user.id == vault.user_id) { Asker::Owner } else { Asker::Visitor };
    let label =
        crate::data::sessions::describe_device(headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok())).browser;
    match unlock::unlock(state, &vault, &password, asker, ip, remember, label.as_deref()).await? {
        Ok(granted) => {
            cookies.push(grant_cookie(state, &vault.user_id, &granted));
            let mut res = Response::new(Body::empty());
            *res.status_mut() = StatusCode::SEE_OTHER;
            if let Ok(v) = HeaderValue::from_str(&next) {
                res.headers_mut().insert(header::LOCATION, v);
            }
            res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            Ok(res)
        }
        Err(UnlockError::RateLimited { retry_after_ms }) => {
            fail(GateError::RateLimited { minutes: ((retry_after_ms + 59_999) / 60_000).max(1) })
        }
        Err(_) => fail(GateError::Wrong),
    }
}

/// `POST /api/encryption/lock`: forget the grant this request sends.
async fn lock(State(state): State<AppState>, req: Request) -> AppResult<Response> {
    let (parts, _) = req.into_parts();
    for value in parts.headers.get_all(UNLOCK_HEADER) {
        if let Some((id, _)) = value.to_str().ok().and_then(parse_grant)
            && let Some(row) = store::live_grant_by_id(&state.db, id).await?
        {
            // Possession of the grant's id and secret is what may revoke it.
            let vault = store::load(&state.db, &row.user_id).await?;
            if let Some(vault) = vault
                && crate::vault::access::open_grant(&state, &vault, value.to_str().unwrap_or("")).await?.is_some()
            {
                store::delete_grant(&state.db, id, &row.user_id).await?;
            }
        }
    }
    let mut res = Response::new(Body::empty());
    *res.status_mut() = StatusCode::NO_CONTENT;
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(res)
}
