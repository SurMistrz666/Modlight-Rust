//! Port of `app/api/me/route.ts`: who this token belongs to, and how much
//! room is left.
//!
//! Exists for one concrete failure: the downloader companion queues a
//! playlist, spends forty minutes fetching six gigabytes, and only then
//! discovers the account had four left. `POST /upload` answers that with 507,
//! correctly but far too late to be useful. A client that can ask FIRST can
//! tell the user before the download starts.
//!
//! It is also how a client verifies a config file works ("connected as
//! alicja") without uploading a probe file into somebody's library.
//!
//! Returns nothing that is not already the caller's own - the role included:
//! an account already knows whether it administers the server.
//!
//! SCOPE: `identify`, which every scope grants. This is the one surface a
//! token of any width may reach, because it is how a client checks that its
//! config works at all - refusing it to an upload-only token would break the
//! test-connection button on exactly the tokens we want people to choose. It
//! answers with the token's own scope for the same reason: a client that can
//! see "this credential may only upload" can say so, instead of letting the
//! user discover it as an unexplained 403 three screens later.

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::api_caller::{CallerDeps, LiveCallerDeps, UNAUTHORIZED_MESSAGE, resolve_api_caller_with};
use crate::api_json::{PRIVATE_NO_STORE, json_private};
use crate::api_token::{TokenCapability, capabilities_of};
use crate::auth_helpers::Sessions;
use crate::avatar::avatar_url;
use crate::config::Config;
use crate::error::{AppError, AppResult};
use crate::js_json::{self, JsObject, JsValue};
use crate::media::urls::user_base_url;
use crate::quota::is_unlimited;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/me", get(get_me))
}

/// The columns the answer is built from.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct MeRow {
    pub name: String,
    pub email: String,
    pub user_code: Option<String>,
    pub image: Option<String>,
    pub used_quota: Option<i64>,
    pub max_quota: Option<i64>,
    pub copy_raw: Option<bool>,
    pub hide_uploads: Option<bool>,
    pub default_folder_id: Option<Uuid>,
    pub preferred_domain: Option<String>,
}

/// The one statement the route issues (stubbed in tests).
pub trait MeStore {
    /// `SELECT name, email, user_code, ... FROM "user" WHERE id = $1 LIMIT 1`
    fn load(&self, user_id: &str) -> impl Future<Output = anyhow::Result<Option<MeRow>>> + Send;
}

/// The real statement.
pub struct PgMeStore<'a>(pub &'a PgPool);

impl MeStore for PgMeStore<'_> {
    async fn load(&self, user_id: &str) -> anyhow::Result<Option<MeRow>> {
        Ok(sqlx::query_as::<_, MeRow>(
            "SELECT \"name\", \"email\", \"user_code\", \"image\", \"used_quota\", \"max_quota\", \"copy_raw\", \
             \"hide_uploads\", \"default_folder_id\", \"preferred_domain\" FROM \"user\" WHERE \"user\".\"id\" = $1 LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(self.0)
        .await?)
    }
}

/// GET /api/me.
///
/// The body is printed by [`js_json`] because its numbers are JavaScript
/// numbers: the bigint quota columns arrive as `Number(...)` upstream, so a
/// value past 2^53 is rounded the way a client of the reference sees it.
pub async fn get_me_with<C: CallerDeps, S: MeStore>(
    callers: &C,
    store: &S,
    config: &Config,
    headers: &HeaderMap,
) -> AppResult<Response> {
    let (caller, scope) = resolve_api_caller_with(callers, headers, TokenCapability::Identify)
        .await?
        .or_respond(Some(PRIVATE_NO_STORE))?;

    let Some(row) = store.load(&caller.id).await? else {
        return Ok(json_private(StatusCode::UNAUTHORIZED, &json!({ "message": UNAUTHORIZED_MESSAGE })));
    };

    let max_quota = row.max_quota.unwrap_or(-1);
    let used = row.used_quota.unwrap_or(0) as f64;
    let max = max_quota as f64;
    let unlimited = is_unlimited(Some(max_quota));
    let host = user_base_url(config, row.preferred_domain.as_deref());
    // Absolute, unlike everywhere in the panel: a desktop client has no
    // origin to resolve a root-relative path against. Public and
    // unauthenticated to fetch, so the client can draw it without attaching a
    // token to an <img>.
    let avatar = avatar_url(row.user_code.as_deref(), row.image.as_deref())
        .filter(|path| !path.is_empty())
        .map(|path| format!("{host}{path}"));

    let token = match scope {
        // What the credential that asked may do. `null` for a browser
        // session, which is the account itself and has no scope to narrow.
        Some(scope) => JsValue::Object(JsObject::new(vec![
            ("scope", scope.as_str().into()),
            (
                "capabilities",
                JsValue::Array(capabilities_of(scope).iter().map(|c| capability_name(*c).into()).collect()),
            ),
        ])),
        None => JsValue::Null,
    };

    let body = JsObject::new(vec![
        ("tag", row.user_code.into()),
        ("token", token),
        ("name", row.name.into()),
        ("email", row.email.into()),
        ("avatar", avatar.into()),
        // The hostname THIS account's links are built on, so a client can
        // show the user which deployment their config points at.
        ("host", host.into()),
        (
            "quota",
            JsValue::Object(JsObject::new(vec![
                ("used", used.into()),
                // null rather than -1: a client doing arithmetic on the
                // sentinel would compute a negative allowance and refuse
                // every upload.
                ("max", if unlimited { JsValue::Null } else { max.into() }),
                ("remaining", if unlimited { JsValue::Null } else { 0f64.max(max - used).into() }),
                ("unlimited", unlimited.into()),
            ])),
        ),
        // What this account is on the server. The desktop client reads it to
        // decide whether to OFFER an administration screen at all; every
        // admin surface checks the role again on its own. Comes from the
        // caller resolution rather than the row above, so a token and a
        // session answer from the same place.
        ("role", caller.role.into()),
        // Mirrors of panel preferences a client may want to reflect. The
        // clipboard choice is still answered per upload by `copy_url`: this
        // is for showing the setting, never for deciding it.
        ("copyRaw", row.copy_raw.unwrap_or(false).into()),
        ("hideUploads", row.hide_uploads.unwrap_or(false).into()),
        ("defaultFolderId", row.default_folder_id.map(|id| id.to_string()).into()),
    ]);

    let mut res = (StatusCode::OK, js_json::stringify(&JsValue::Object(body))).into_response();
    let headers = res.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(PRIVATE_NO_STORE));
    Ok(res)
}

/// The capability as `capabilitiesOf(scope)` spells it.
fn capability_name(capability: TokenCapability) -> &'static str {
    match capability {
        TokenCapability::Identify => "identify",
        TokenCapability::Upload => "upload",
        TokenCapability::Read => "read",
        TokenCapability::Write => "write",
        TokenCapability::Admin => "admin",
    }
}

/// How Next answers a route handler that threw: a bare 500, no body and no
/// content type.
fn finish(result: AppResult<Response>) -> Response {
    match result {
        Ok(response) => response,
        Err(AppError::Internal(err)) => {
            tracing::error!(error = ?err, "GET /api/me failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        Err(other) => other.into_response(),
    }
}

async fn get_me(State(state): State<AppState>, Sessions(session): Sessions, headers: HeaderMap) -> Response {
    let callers = LiveCallerDeps { state: &state, session: &session };
    finish(get_me_with(&callers, &PgMeStore(&state.db), &state.config, &headers).await)
}

#[cfg(test)]
mod tests {
    //! GET /api/me through the handler: the scope a token reports, the quota
    //! arithmetic, the absolute avatar, and the refusals every token surface
    //! shares (tests/unit/api-route-auth.test.ts lists /api/me only in the
    //! scope inventory; these pin the same doors for it).
    use std::collections::HashMap;
    use std::sync::Mutex;

    use axum::http::HeaderValue;
    use http_body_util::BodyExt;
    use serde_json::Value;

    use super::*;
    use crate::api_token::test_support::user;
    use crate::api_token::{TokenAuth, TokenScope};
    use crate::routes::api::theme::test_support::{FakeCallers, session_of};

    fn config() -> Config {
        let mut vars = HashMap::new();
        vars.insert("APP_URL".to_string(), "https://host.example.com".to_string());
        vars.insert("APP_DOMAINS".to_string(), "https://s.example.com".to_string());
        Config::from_env_map(&vars).unwrap()
    }

    fn me_row() -> MeRow {
        MeRow {
            name: "Alicja".into(),
            email: "alicja@example.com".into(),
            user_code: Some("alicja".into()),
            image: Some("avatars/u1-tok123.webp".into()),
            used_quota: Some(1_500),
            max_quota: Some(10_000),
            copy_raw: Some(true),
            hide_uploads: None,
            default_folder_id: Some(Uuid::parse_str("8f1b3a52-1c3e-4b8a-9a55-0d3c2a7b6e11").unwrap()),
            preferred_domain: Some("https://S.example.com/".into()),
        }
    }

    struct FakeStore(Mutex<Option<MeRow>>);

    impl MeStore for FakeStore {
        async fn load(&self, _user_id: &str) -> anyhow::Result<Option<MeRow>> {
            Ok(self.0.lock().unwrap().clone())
        }
    }

    fn key() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-api-key", HeaderValue::from_static("good"));
        h
    }

    async fn call(callers: &FakeCallers, row: Option<MeRow>, headers: HeaderMap) -> (StatusCode, HeaderMap, String) {
        let res = finish(get_me_with(callers, &FakeStore(Mutex::new(row)), &config(), &headers).await);
        let (parts, body) = res.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        (parts.status, parts.headers, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn answers_the_caller_its_own_account_in_upstream_order() {
        let (status, headers, body) = call(&FakeCallers::token_ok(TokenScope::Upload), Some(me_row()), key()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CACHE_CONTROL], "private, no-store");
        assert_eq!(headers[header::CONTENT_TYPE], "application/json");
        assert_eq!(
            body,
            concat!(
                r#"{"tag":"alicja","token":{"scope":"upload","capabilities":["identify","upload"]},"#,
                r#""name":"Alicja","email":"alicja@example.com","#,
                r#""avatar":"https://s.example.com/avatar/alicja?v=tok123","host":"https://s.example.com","#,
                r#""quota":{"used":1500,"max":10000,"remaining":8500,"unlimited":false},"role":"user","#,
                r#""copyRaw":true,"hideUploads":false,"defaultFolderId":"8f1b3a52-1c3e-4b8a-9a55-0d3c2a7b6e11"}"#
            )
        );
    }

    #[tokio::test]
    async fn every_scope_may_identify_itself_and_reports_its_capabilities() {
        for (scope, caps) in [
            (TokenScope::Upload, vec!["identify", "upload"]),
            (TokenScope::Read, vec!["identify", "read"]),
            (TokenScope::Write, vec!["identify", "upload", "read", "write"]),
            (TokenScope::Full, vec!["identify", "upload", "read", "write", "admin"]),
        ] {
            let callers = FakeCallers::token_ok(scope);
            let (status, _, body) = call(&callers, Some(me_row()), key()).await;
            assert_eq!(status, StatusCode::OK);
            let v: Value = serde_json::from_str(&body).unwrap();
            assert_eq!(v["token"]["scope"], scope.as_str());
            assert_eq!(v["token"]["capabilities"], json!(caps));
            assert_eq!(callers.token_calls.lock().unwrap()[0].1, TokenCapability::Identify);
        }
    }

    #[tokio::test]
    async fn a_session_has_no_token_and_answers_with_its_role() {
        let callers = FakeCallers::default();
        *callers.session.lock().unwrap() = Some(session_of("u2", Some("admin")));
        let (_, _, body) = call(&callers, Some(me_row()), HeaderMap::new()).await;
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["token"], Value::Null);
        assert_eq!(v["role"], "admin");
    }

    #[tokio::test]
    async fn an_unlimited_account_has_no_max_and_no_remaining() {
        let row = MeRow { max_quota: Some(-1), used_quota: None, image: None, preferred_domain: None, ..me_row() };
        let (_, _, body) = call(&FakeCallers::token_ok(TokenScope::Full), Some(row), key()).await;
        assert!(body.contains(r#""avatar":null,"host":"https://host.example.com","#), "{body}");
        assert!(body.contains(r#""quota":{"used":0,"max":null,"remaining":null,"unlimited":true}"#), "{body}");
        let row = MeRow { max_quota: None, ..me_row() };
        assert!(
            call(&FakeCallers::token_ok(TokenScope::Full), Some(row), key()).await.2.contains(r#""unlimited":true"#)
        );
    }

    #[tokio::test]
    async fn remaining_never_goes_below_zero_and_numbers_are_javascript_numbers() {
        let row = MeRow { used_quota: Some(12_000), max_quota: Some(10_000), ..me_row() };
        let (_, _, body) = call(&FakeCallers::token_ok(TokenScope::Full), Some(row), key()).await;
        assert!(body.contains(r#""quota":{"used":12000,"max":10000,"remaining":0,"unlimited":false}"#), "{body}");
        // bigint -> Number(): past 2^53 the value is rounded like JavaScript's.
        let row = MeRow { used_quota: Some(9_007_199_254_740_993), max_quota: Some(i64::MAX), ..me_row() };
        let (_, _, body) = call(&FakeCallers::token_ok(TokenScope::Full), Some(row), key()).await;
        assert!(
            body.contains(r#""used":9007199254740992,"max":9223372036854776000,"remaining":9214364837600035000"#),
            "{body}"
        );
    }

    #[tokio::test]
    async fn an_absolute_image_is_appended_to_the_host_as_upstream_does() {
        let row = MeRow { image: Some("https://cdn.example.com/a.png".into()), preferred_domain: None, ..me_row() };
        let (_, _, body) = call(&FakeCallers::token_ok(TokenScope::Full), Some(row), key()).await;
        assert!(body.contains(r#""avatar":"https://host.example.comhttps://cdn.example.com/a.png""#), "{body}");
    }

    #[tokio::test]
    async fn refuses_when_the_account_is_gone() {
        let (status, _, body) = call(&FakeCallers::token_ok(TokenScope::Full), None, key()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body, r#"{"message":"Token not specified or not found."}"#);
    }

    #[tokio::test]
    async fn refuses_without_credentials_or_with_a_bad_or_banned_token() {
        let (status, headers, _) = call(&FakeCallers::default(), Some(me_row()), HeaderMap::new()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(headers[header::CACHE_CONTROL], "private, no-store");

        let callers = FakeCallers::default();
        *callers.token.lock().unwrap() = Some(TokenAuth::Unknown);
        *callers.session.lock().unwrap() = Some(session_of("session-user", Some("admin")));
        for (name, value) in [("x-api-key", "revoked"), ("authorization", "Bearer revoked")] {
            let mut h = HeaderMap::new();
            h.insert(name, HeaderValue::from_static(value));
            assert_eq!(call(&callers, Some(me_row()), h).await.0, StatusCode::UNAUTHORIZED);
        }
        assert_eq!(*callers.session_calls.lock().unwrap(), 0);

        let banned = FakeCallers::default();
        *banned.token.lock().unwrap() = Some(TokenAuth::Banned { user: user("u1", true, Some("user")) });
        let (status, _, body) = call(&banned, Some(me_row()), key()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body, r#"{"message":"Account disabled."}"#);

        let invalid = FakeCallers::default();
        *invalid.token.lock().unwrap() = Some(TokenAuth::InvalidScope);
        assert_eq!(call(&invalid, Some(me_row()), key()).await.0, StatusCode::FORBIDDEN);
    }
}
