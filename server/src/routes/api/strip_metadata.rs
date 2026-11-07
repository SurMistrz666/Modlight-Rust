//! Not upstream (SKELETON.md 9.47, CONTRACT.md section 11): the account's "Remove
//! metadata from files on upload" switch, for the desktop app and scripts.
//!
//! | Route | Capability | Answer |
//! |---|---|---|
//! | `GET /api/strip-metadata` | `read` | `{"enabled": bool}` |
//! | `PUT /api/strip-metadata`, JSON `{"enabled": bool}` | `write` | `{"enabled": bool}`, as stored |
//!
//! An API token first and the browser session second, like `/api/theme`; a
//! present but wrong token is a refusal. The switch only changes what happens
//! to the NEXT uploads: files already uploaded are never touched.

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::api_caller::{CallerDeps, LiveCallerDeps, UNAUTHORIZED_MESSAGE, resolve_api_caller_with};
use crate::api_json::{PRIVATE_NO_STORE, json_private, read_json_object};
use crate::api_token::TokenCapability;
use crate::auth_helpers::Sessions;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// `{"enabled": true}` and nothing much else.
const BODY_MAX: usize = 1024;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/strip-metadata", get(get_switch).put(put_switch))
}

/// The two statements the route issues (stubbed in tests).
pub trait SwitchStore {
    fn read(&self, user_id: &str) -> impl Future<Output = anyhow::Result<Option<bool>>> + Send;
    fn write(&self, user_id: &str, enabled: bool) -> impl Future<Output = anyhow::Result<Option<bool>>> + Send;
}

/// The real statements (`data::strip_metadata`).
pub struct PgSwitchStore<'a>(pub &'a PgPool);

impl SwitchStore for PgSwitchStore<'_> {
    async fn read(&self, user_id: &str) -> anyhow::Result<Option<bool>> {
        Ok(crate::data::strip_metadata::read_switch(self.0, user_id).await?)
    }

    async fn write(&self, user_id: &str, enabled: bool) -> anyhow::Result<Option<bool>> {
        Ok(crate::data::strip_metadata::write_switch(self.0, user_id, enabled).await?)
    }
}

fn answer(enabled: bool) -> Response {
    json_private(StatusCode::OK, &json!({ "enabled": enabled }))
}

/// No row: the account was deleted while the credential lived on.
fn unauthorized() -> Response {
    json_private(StatusCode::UNAUTHORIZED, &json!({ "message": UNAUTHORIZED_MESSAGE }))
}

pub async fn get_switch_with<C: CallerDeps, S: SwitchStore>(
    callers: &C,
    store: &S,
    headers: &HeaderMap,
) -> AppResult<Response> {
    let (caller, _) =
        resolve_api_caller_with(callers, headers, TokenCapability::Read).await?.or_respond(Some(PRIVATE_NO_STORE))?;
    Ok(match store.read(&caller.id).await? {
        Some(enabled) => answer(enabled),
        None => unauthorized(),
    })
}

pub async fn put_switch_with<C: CallerDeps, S: SwitchStore>(
    callers: &C,
    store: &S,
    headers: &HeaderMap,
    body: axum::body::Body,
) -> AppResult<Response> {
    // The credential decides before a byte of the body is read.
    let (caller, _) =
        resolve_api_caller_with(callers, headers, TokenCapability::Write).await?.or_respond(Some(PRIVATE_NO_STORE))?;
    let input = read_json_object(headers, body, BODY_MAX).await?;
    let Some(Value::Bool(enabled)) = input.get("enabled") else {
        return Ok(json_private(StatusCode::BAD_REQUEST, &json!({ "message": "enabled must be true or false." })));
    };
    Ok(match store.write(&caller.id, *enabled).await? {
        Some(stored) => answer(stored),
        None => unauthorized(),
    })
}

/// How Next answers a route handler that threw: a bare 500.
fn finish(result: AppResult<Response>) -> Response {
    match result {
        Ok(response) => response,
        Err(AppError::Internal(err)) => {
            tracing::error!(error = ?err, "GET/PUT /api/strip-metadata failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        Err(other) => other.into_response(),
    }
}

async fn get_switch(State(state): State<AppState>, Sessions(session): Sessions, headers: HeaderMap) -> Response {
    let callers = LiveCallerDeps { state: &state, session: &session };
    finish(get_switch_with(&callers, &PgSwitchStore(&state.db), &headers).await)
}

async fn put_switch(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let callers = LiveCallerDeps { state: &state, session: &session };
    finish(put_switch_with(&callers, &PgSwitchStore(&state.db), &parts.headers, body).await)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use axum::body::Body;
    use axum::http::{HeaderValue, header};
    use http_body_util::BodyExt;

    use super::*;
    use crate::api_token::test_support::user;
    use crate::api_token::{TokenAuth, TokenScope};
    use crate::routes::api::theme::test_support::{FakeCallers, session_of};

    #[derive(Default)]
    struct FakeStore {
        value: Mutex<Option<bool>>,
        written: Mutex<Vec<(String, bool)>>,
    }

    impl SwitchStore for FakeStore {
        async fn read(&self, _user_id: &str) -> anyhow::Result<Option<bool>> {
            Ok(*self.value.lock().unwrap())
        }
        async fn write(&self, user_id: &str, enabled: bool) -> anyhow::Result<Option<bool>> {
            self.written.lock().unwrap().push((user_id.to_string(), enabled));
            let mut value = self.value.lock().unwrap();
            if value.is_some() {
                *value = Some(enabled);
            }
            Ok(*value)
        }
    }

    fn store(value: Option<bool>) -> FakeStore {
        FakeStore { value: Mutex::new(value), ..Default::default() }
    }

    fn key(json: bool) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-api-key", HeaderValue::from_static("good"));
        if json {
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        }
        h
    }

    async fn text(res: Response) -> (StatusCode, HeaderMap, String) {
        let (parts, body) = res.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        (parts.status, parts.headers, String::from_utf8(bytes.to_vec()).unwrap())
    }

    async fn get(callers: &FakeCallers, store: &FakeStore, headers: HeaderMap) -> (StatusCode, HeaderMap, String) {
        text(finish(get_switch_with(callers, store, &headers).await)).await
    }

    async fn put(callers: &FakeCallers, store: &FakeStore, headers: HeaderMap, body: &str) -> (StatusCode, String) {
        let res = finish(put_switch_with(callers, store, &headers, Body::from(body.to_string())).await);
        let (status, _, body) = text(res).await;
        (status, body)
    }

    #[tokio::test]
    async fn reads_the_switch_with_a_read_token_and_answers_privately() {
        let callers = FakeCallers::token_ok(TokenScope::Read);
        let (status, headers, body) = get(&callers, &store(Some(false)), key(false)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, r#"{"enabled":false}"#);
        assert_eq!(headers[header::CACHE_CONTROL], "private, no-store");
        assert_eq!(callers.token_calls.lock().unwrap()[0].1, TokenCapability::Read);
        let (_, _, body) = get(&FakeCallers::token_ok(TokenScope::Full), &store(Some(true)), key(false)).await;
        assert_eq!(body, r#"{"enabled":true}"#);
    }

    #[tokio::test]
    async fn writes_the_switch_and_answers_what_was_stored() {
        let callers = FakeCallers::token_ok(TokenScope::Write);
        let s = store(Some(false));
        let (status, body) = put(&callers, &s, key(true), r#"{"enabled":true}"#).await;
        assert_eq!((status, body.as_str()), (StatusCode::OK, r#"{"enabled":true}"#));
        assert_eq!(s.written.lock().unwrap().as_slice(), &[("u1".to_string(), true)]);
        assert_eq!(callers.token_calls.lock().unwrap()[0].1, TokenCapability::Write);
    }

    #[tokio::test]
    async fn refuses_a_body_that_is_not_a_switch() {
        let callers = FakeCallers::token_ok(TokenScope::Write);
        let s = store(Some(false));
        for body in [r#"{}"#, r#"{"enabled":"yes"}"#, r#"{"enabled":1}"#, r#"{"enabled":null}"#] {
            let (status, text) = put(&callers, &s, key(true), body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(text, r#"{"message":"enabled must be true or false."}"#);
        }
        assert_eq!(put(&callers, &s, key(true), "[true]").await.0, StatusCode::BAD_REQUEST);
        assert_eq!(put(&callers, &s, key(false), r#"{"enabled":true}"#).await.0, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        let long = format!(r#"{{"enabled":true,"x":"{}"}}"#, "x".repeat(2000));
        assert_eq!(put(&callers, &s, key(true), &long).await.0, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(s.written.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_read_or_upload_token_may_not_write() {
        for scope in [TokenScope::Read, TokenScope::Upload] {
            let callers = FakeCallers::default();
            *callers.token.lock().unwrap() =
                Some(TokenAuth::Scope { scope, required: TokenCapability::Write, user: user("u1", false, None) });
            let s = store(Some(false));
            let (status, _) = put(&callers, &s, key(true), r#"{"enabled":true}"#).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{scope:?}");
            assert!(s.written.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn refuses_without_credentials_a_bad_token_or_a_gone_account() {
        let (status, _, _) = get(&FakeCallers::default(), &store(Some(true)), HeaderMap::new()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let callers = FakeCallers::default();
        *callers.token.lock().unwrap() = Some(TokenAuth::Unknown);
        *callers.session.lock().unwrap() = Some(session_of("someone", None));
        assert_eq!(get(&callers, &store(Some(true)), key(false)).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(*callers.session_calls.lock().unwrap(), 0);
        let banned = FakeCallers::default();
        *banned.token.lock().unwrap() = Some(TokenAuth::Banned { user: user("u1", true, None) });
        assert_eq!(get(&banned, &store(Some(true)), key(false)).await.0, StatusCode::UNAUTHORIZED);
        let (status, _, body) = get(&FakeCallers::token_ok(TokenScope::Full), &store(None), key(false)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body, r#"{"message":"Token not specified or not found."}"#);
    }

    #[tokio::test]
    async fn a_session_reads_and_writes_its_own_switch() {
        let callers = FakeCallers::default();
        *callers.session.lock().unwrap() = Some(session_of("u2", None));
        let s = store(Some(false));
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let (status, _) = put(&callers, &s, headers, r#"{"enabled":true}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(s.written.lock().unwrap()[0].0, "u2");
    }
}
