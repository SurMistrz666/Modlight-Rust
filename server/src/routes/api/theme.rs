//! Port of `app/api/theme/route.ts`: the account's skin, readable and
//! writable by the desktop client.
//!
//! The owner asked for the sync to run BOTH ways (docs/decyzje-modlight-app.md
//! item 20): changing the theme in the app changes it on the website, and
//! changing it on the website is what the app shows next time it asks.
//!
//! That works because `user.theme` is a single column that already drives
//! everything: the panel paints from it, and a shared link is painted in its
//! OWNER's skin (lib/media/serve.ts). There is no second copy to keep in step:
//! the `ml_skin` cookie is a first-paint hint, not a store, and the root
//! layout prefers this column over it precisely so a write that arrives
//! without a browser still shows up on the next page load.
//!
//! Authenticates with an API TOKEN first and a session second, in that order,
//! through `resolve_api_caller`. A token that is present and wrong is a
//! refusal, never a fall-through to whatever cookie the same request happened
//! to carry, and a token scoped away from writing can read this and not set it.

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::StreamExt;
use serde::{Serialize, Serializer};
use serde_json::json;
use sqlx::PgPool;

use crate::api_caller::{CallerDeps, LiveCallerDeps, UNAUTHORIZED_MESSAGE, resolve_api_caller_with};
use crate::api_json::{PRIVATE_NO_STORE, json_private};
use crate::api_token::TokenCapability;
use crate::auth_helpers::Sessions;
use crate::db::models::JsDate;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::theme_api::{DEFAULT_THEME, THEME_BODY_MAX_BYTES, ThemeBody, ThemeChoice, parse_theme_body, theme_catalog};
use crate::themes::{THEME_ALIASES, is_theme_id, resolve_theme};

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/theme", get(get_theme).put(put_theme))
}

/// The two statements the route issues (stubbed in tests, as the upstream
/// test mocks `@/lib/db`). `None` means no row matched; the inner `Option`
/// is the nullable column.
pub trait ThemeStore {
    /// `SELECT theme FROM "user" WHERE id = $1 LIMIT 1`
    fn read_theme(&self, user_id: &str) -> impl Future<Output = anyhow::Result<Option<Option<String>>>> + Send;
    /// `UPDATE "user" SET theme = $1, updated_at = <now> WHERE id = $2 RETURNING theme`
    /// (`updated_at` is the column's drizzle `$onUpdate`, which every update writes).
    fn write_theme(
        &self,
        user_id: &str,
        theme: &str,
    ) -> impl Future<Output = anyhow::Result<Option<Option<String>>>> + Send;
}

/// The real statements.
pub struct PgThemeStore<'a>(pub &'a PgPool);

impl ThemeStore for PgThemeStore<'_> {
    async fn read_theme(&self, user_id: &str) -> anyhow::Result<Option<Option<String>>> {
        Ok(sqlx::query_scalar::<_, Option<String>>("SELECT \"theme\" FROM \"user\" WHERE \"user\".\"id\" = $1 LIMIT 1")
            .bind(user_id)
            .fetch_optional(self.0)
            .await?)
    }

    async fn write_theme(&self, user_id: &str, theme: &str) -> anyhow::Result<Option<Option<String>>> {
        Ok(sqlx::query_scalar::<_, Option<String>>(
            "UPDATE \"user\" SET \"theme\" = $1, \"updated_at\" = $2 WHERE \"user\".\"id\" = $3 RETURNING \"theme\"",
        )
        .bind(theme)
        .bind(JsDate::now())
        .bind(user_id)
        .fetch_optional(self.0)
        .await?)
    }
}

/// Same wording as /api/me, /api/folders and /api/shortlinks, so one client
/// branch covers all.
fn unauthorized() -> Response {
    json_private(StatusCode::UNAUTHORIZED, &json!({ "message": UNAUTHORIZED_MESSAGE }))
}

/// What `resolveTheme(row.theme)` puts in the answer.
///
/// Upstream resolves a retired id with `v in THEME_ALIASES`, and `in` also
/// sees what every object inherits. A column holding `constructor` (Better
/// Auth's `update-user` takes any string for `theme`) resolves to `Object`,
/// a function, which `JSON.stringify` drops, so the answer carries no `theme`
/// key at all; `__proto__` resolves to `Object.prototype`, which prints as
/// `{}`. Reproduced so a client reads the same document from either server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireTheme {
    Id(&'static str),
    /// `Object.prototype` (`__proto__`).
    EmptyObject,
    /// A function inherited from `Object.prototype`.
    Omitted,
}

/// Every other own property name of `Object.prototype` in Node 24 (all
/// functions).
const INHERITED_FUNCTIONS: &[&str] = &[
    "constructor",
    "__defineGetter__",
    "__defineSetter__",
    "hasOwnProperty",
    "__lookupGetter__",
    "__lookupSetter__",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toString",
    "valueOf",
    "toLocaleString",
];

pub fn resolve_wire_theme(stored: Option<&str>) -> WireTheme {
    match stored {
        Some(v) if is_theme_id(v) || THEME_ALIASES.iter().any(|(retired, _)| *retired == v) => {
            WireTheme::Id(resolve_theme(Some(v)))
        }
        Some("__proto__") => WireTheme::EmptyObject,
        Some(v) if INHERITED_FUNCTIONS.contains(&v) => WireTheme::Omitted,
        _ => WireTheme::Id(DEFAULT_THEME),
    }
}

impl WireTheme {
    fn is_omitted(&self) -> bool {
        *self == WireTheme::Omitted
    }
}

impl Serialize for WireTheme {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            WireTheme::Id(id) => serializer.serialize_str(id),
            _ => serde_json::Map::new().serialize(serializer),
        }
    }
}

/// The wire shape, identical for GET and PUT so a client parses one thing.
///
/// `theme` is the RESOLVED id, never the raw column: a row still holding the
/// retired `modern` answers `default`, which is the same skin under the name
/// the catalogue uses. Without that, a client doing the obvious
/// read-then-write round trip would send back a value its own picker could
/// not match.
///
/// `defaultTheme` rather than `default` because a client generating a model
/// class from this JSON would land on a reserved word in most of the
/// languages a desktop app is written in.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Shape {
    #[serde(skip_serializing_if = "WireTheme::is_omitted")]
    theme: WireTheme,
    default_theme: &'static str,
    themes: Vec<ThemeChoice>,
}

fn shape(theme: WireTheme) -> Shape {
    Shape { theme, default_theme: DEFAULT_THEME, themes: theme_catalog() }
}

/// GET: reading which skin the account wears.
pub async fn get_theme_with<C: CallerDeps, S: ThemeStore>(
    callers: &C,
    store: &S,
    headers: &HeaderMap,
) -> AppResult<Response> {
    let (caller, _) =
        resolve_api_caller_with(callers, headers, TokenCapability::Read).await?.or_respond(Some(PRIVATE_NO_STORE))?;

    // No row means the account was deleted while the credential lived on.
    // Same answer as a bad credential: there is nobody to have preferences.
    let Some(theme) = store.read_theme(&caller.id).await? else {
        return Ok(unauthorized());
    };

    Ok(json_private(StatusCode::OK, &shape(resolve_wire_theme(theme.as_deref()))))
}

/// PUT: changing it. It repaints the account's PUBLIC pages too (a shared
/// link is painted in its owner's skin), so this is not a private preference.
pub async fn put_theme_with<C: CallerDeps, S: ThemeStore>(
    callers: &C,
    store: &S,
    headers: &HeaderMap,
    body: Body,
) -> AppResult<Response> {
    let (caller, _) =
        resolve_api_caller_with(callers, headers, TokenCapability::Write).await?.or_respond(Some(PRIVATE_NO_STORE))?;

    // No Content-Type check, unlike POST /api/shortlinks. There it is the CSRF
    // control, because a POST with a safelisted type is a "simple" request a
    // stranger's page can send at a logged-in visitor without a preflight.
    // PUT is never simple: the browser preflights it whatever the body says,
    // and nothing here answers a preflight. PUT /api/client-settings relies on
    // the same thing.
    let Some(text) = read_capped_body(headers, body).await else {
        return Ok(json_private(
            StatusCode::PAYLOAD_TOO_LARGE,
            &json!({ "message": format!("Body must be at most {THEME_BODY_MAX_BYTES} bytes.") }),
        ));
    };

    let theme = match parse_theme_body(&text) {
        ThemeBody::Ok { theme } => theme,
        ThemeBody::Refused { status, message } => return Ok(json_private(status, &json!({ "message": message }))),
    };

    // Ownership lives in the WHERE, not in a SELECT first: one statement
    // cannot be raced between its check and its write, and the row is read
    // back rather than assumed. An UPDATE that matched nothing is otherwise
    // indistinguishable from a successful save until the next page load puts
    // the old skin back, which is exactly how "the theme picker does nothing"
    // presented in the panel.
    let Some(row) = store.write_theme(&caller.id, theme).await? else {
        return Ok(unauthorized());
    };

    // Deliberately no cookie and no revalidation. The panel's Server Action
    // writes `ml_skin` because the browser making that request is the one
    // about to repaint; a desktop client has no cookie jar to write into and
    // no router cache to drop, and the pages that matter are rendered per
    // request. The database is what the next page load reads.
    Ok(json_private(StatusCode::OK, &shape(resolve_wire_theme(row.as_deref()))))
}

/// Read the body, refusing anything oversized (`None`).
///
/// Declared length first, because refusing by reading a header is free where
/// refusing afterwards means having buffered the thing in order to reject it.
/// The text is measured again once in hand: a chunked body declares no length
/// at all. Counted in UTF-16 units rather than bytes, which errs the safe way.
///
/// Upstream buffers the whole body before measuring it; this stops reading
/// once the answer is certain (a UTF-16 unit takes at most three bytes, plus
/// the two byte-order marks `req.text()` drops), which answers the same
/// without holding an unbounded body. A body that fails to arrive reads as
/// empty, as upstream's `catch` around `req.text()` does.
async fn read_capped_body(headers: &HeaderMap, body: Body) -> Option<String> {
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .map(|v| crate::config::js_number(&String::from_utf8_lossy(v.as_bytes())))
        .unwrap_or(0.0);
    if declared.is_finite() && declared > THEME_BODY_MAX_BYTES as f64 {
        return None;
    }

    let certainly_too_large = THEME_BODY_MAX_BYTES * 3 + 6;
    let mut buf: Vec<u8> = Vec::new();
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return Some(String::new());
        };
        buf.extend_from_slice(&chunk);
        if buf.len() > certainly_too_large {
            return None;
        }
    }
    let text = crate::js_json::request_text(&buf);
    if text.encode_utf16().count() > THEME_BODY_MAX_BYTES {
        return None;
    }
    Some(text)
}

/// How Next answers a route handler that threw (a database error, a session
/// lookup that failed): a bare 500, no body and no content type.
fn finish(result: AppResult<Response>) -> Response {
    match result {
        Ok(response) => response,
        Err(AppError::Internal(err)) => {
            tracing::error!(error = ?err, "GET/PUT /api/theme failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        Err(other) => other.into_response(),
    }
}

async fn get_theme(State(state): State<AppState>, Sessions(session): Sessions, headers: HeaderMap) -> Response {
    let callers = LiveCallerDeps { state: &state, session: &session };
    finish(get_theme_with(&callers, &PgThemeStore(&state.db), &headers).await)
}

async fn put_theme(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let callers = LiveCallerDeps { state: &state, session: &session };
    finish(put_theme_with(&callers, &PgThemeStore(&state.db), &parts.headers, body).await)
}

#[cfg(test)]
mod tests {
    //! tests/unit/theme-route.test.ts, and the /api/theme rows of
    //! tests/unit/api-route-auth.test.ts.
    use std::sync::Mutex;

    use axum::http::HeaderValue;
    use http_body_util::BodyExt;

    use super::test_support::{FakeCallers, session_of};
    use super::*;
    use crate::api_token::test_support::user;
    use crate::api_token::{TokenAuth, TokenScope};
    use crate::themes::THEMES;

    /// Rows the next SELECT / UPDATE resolve to, and every write attempted.
    #[derive(Default)]
    struct FakeStore {
        select: Mutex<Option<Option<String>>>,
        update: Mutex<Option<Option<String>>>,
        written: Mutex<Vec<(String, String)>>,
    }

    impl ThemeStore for FakeStore {
        async fn read_theme(&self, _user_id: &str) -> anyhow::Result<Option<Option<String>>> {
            Ok(self.select.lock().unwrap().clone())
        }
        async fn write_theme(&self, user_id: &str, theme: &str) -> anyhow::Result<Option<Option<String>>> {
            self.written.lock().unwrap().push((user_id.to_string(), theme.to_string()));
            Ok(self.update.lock().unwrap().clone())
        }
    }

    fn key() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-api-key", HeaderValue::from_static("good"));
        h
    }

    async fn answer(res: AppResult<Response>) -> (StatusCode, HeaderMap, serde_json::Value) {
        let res = finish(res);
        let (parts, body) = res.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        let json = if bytes.is_empty() { serde_json::Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
        (parts.status, parts.headers, json)
    }

    async fn get(callers: &FakeCallers, store: &FakeStore) -> (StatusCode, HeaderMap, serde_json::Value) {
        answer(get_theme_with(callers, store, &key()).await).await
    }

    async fn put(callers: &FakeCallers, store: &FakeStore, body: &str) -> (StatusCode, HeaderMap, serde_json::Value) {
        let mut h = key();
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        answer(put_theme_with(callers, store, &h, Body::from(body.to_string())).await).await
    }

    // ── GET /api/theme ──

    #[tokio::test]
    async fn answers_with_the_skin_the_column_holds_plus_the_whole_catalogue() {
        let store = FakeStore { select: Mutex::new(Some(Some("terminal".into()))), ..Default::default() };
        let (status, _, body) = get(&FakeCallers::token_ok(TokenScope::Full), &store).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["theme"], "terminal");
        let ids: Vec<&str> = body["themes"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
        assert_eq!(ids, THEMES.iter().map(|t| t.id).collect::<Vec<_>>());
        assert_eq!(body["defaultTheme"], DEFAULT_THEME);
    }

    #[tokio::test]
    async fn resolves_a_retired_id_so_a_read_then_write_round_trip_cannot_400() {
        let store = FakeStore { select: Mutex::new(Some(Some("modern".into()))), ..Default::default() };
        assert_eq!(get(&FakeCallers::token_ok(TokenScope::Full), &store).await.2["theme"], "graphite");
    }

    #[tokio::test]
    async fn keeps_the_answer_out_of_shared_caches() {
        let store = FakeStore { select: Mutex::new(Some(Some("paper".into()))), ..Default::default() };
        let (_, headers, _) = get(&FakeCallers::token_ok(TokenScope::Full), &store).await;
        assert_eq!(headers[header::CACHE_CONTROL], "private, no-store");
        assert_eq!(headers[header::CONTENT_TYPE], "application/json");
    }

    #[tokio::test]
    async fn refuses_when_the_account_behind_a_live_token_has_been_deleted() {
        let (status, _, body) = get(&FakeCallers::token_ok(TokenScope::Full), &FakeStore::default()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["message"], "Token not specified or not found.");
    }

    #[tokio::test]
    async fn answers_what_resolve_theme_would_for_inherited_names() {
        for (stored, expect) in [
            (Some("constructor"), None),
            (Some("toString"), None),
            (Some("__proto__"), Some(json!({}))),
            (Some("nope"), Some(json!("graphite"))),
            (None, Some(json!("graphite"))),
        ] {
            let store = FakeStore { select: Mutex::new(Some(stored.map(str::to_string))), ..Default::default() };
            let (_, _, body) = get(&FakeCallers::token_ok(TokenScope::Read), &store).await;
            assert_eq!(body.get("theme").cloned(), expect, "{stored:?}");
            assert_eq!(body.as_object().unwrap().keys().next_back().unwrap(), "themes");
        }
    }

    // ── PUT /api/theme ──

    #[tokio::test]
    async fn writes_the_requested_skin_and_answers_with_what_the_row_holds() {
        let store = FakeStore { update: Mutex::new(Some(Some("synthwave".into()))), ..Default::default() };
        let (status, _, body) = put(&FakeCallers::token_ok(TokenScope::Full), &store, r#"{"theme":"synthwave"}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(*store.written.lock().unwrap(), vec![("u1".to_string(), "synthwave".to_string())]);
        assert_eq!(body["theme"], "synthwave");
    }

    #[tokio::test]
    async fn never_writes_an_unknown_skin() {
        let store = FakeStore::default();
        let (status, _, _) = put(&FakeCallers::token_ok(TokenScope::Full), &store, r#"{"theme":"hunter2"}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(store.written.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn stores_the_canonical_id_for_a_retired_one() {
        let store = FakeStore { update: Mutex::new(Some(Some("graphite".into()))), ..Default::default() };
        let (status, _, _) = put(&FakeCallers::token_ok(TokenScope::Full), &store, r#"{"theme":"modern"}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(*store.written.lock().unwrap(), vec![("u1".to_string(), "graphite".to_string())]);
    }

    #[tokio::test]
    async fn refuses_an_oversized_body_without_writing() {
        let store = FakeStore::default();
        let body = json!({ "theme": "x".repeat(8192) }).to_string();
        let (status, _, json) = put(&FakeCallers::token_ok(TokenScope::Full), &store, &body).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(json["message"], "Body must be at most 4096 bytes.");
        assert!(store.written.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn reads_the_body_the_way_request_text_does() {
        // undici drops up to two leading byte-order marks; a third is left in
        // the text, where JSON.parse refuses it.
        for (marks, status) in
            [(0, StatusCode::OK), (1, StatusCode::OK), (2, StatusCode::OK), (3, StatusCode::BAD_REQUEST)]
        {
            let store = FakeStore { update: Mutex::new(Some(Some("mono".into()))), ..Default::default() };
            let mut body = "\u{FEFF}".repeat(marks);
            body.push_str(r#"{"theme":"mono"}"#);
            assert_eq!(put(&FakeCallers::token_ok(TokenScope::Full), &store, &body).await.0, status, "{marks} marks");
        }
    }

    #[tokio::test]
    async fn does_not_report_success_when_the_update_matched_no_row() {
        let store = FakeStore::default();
        let (status, _, _) = put(&FakeCallers::token_ok(TokenScope::Full), &store, r#"{"theme":"paper"}"#).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn measures_the_body_in_utf16_units_and_by_its_declared_length() {
        // 4096 two-byte characters: 8 KiB of UTF-8 but 4096 units plus the
        // envelope, so over the limit only because of the envelope.
        let store = FakeStore::default();
        let body = format!("{{\"theme\":\"{}\"}}", "ą".repeat(4096 - 12));
        assert_eq!(body.encode_utf16().count(), 4096);
        let (status, _, _) = put(&FakeCallers::token_ok(TokenScope::Full), &store, &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "at the limit: parsed, then refused as an unknown theme");
        let mut h = key();
        h.insert(header::CONTENT_LENGTH, HeaderValue::from_static("4097"));
        let res = put_theme_with(&FakeCallers::token_ok(TokenScope::Full), &store, &h, Body::from("{}")).await;
        assert_eq!(answer(res).await.0, StatusCode::PAYLOAD_TOO_LARGE);
    }

    // ── tests/unit/api-route-auth.test.ts (GET and PUT /api/theme) ──

    async fn both(callers: &FakeCallers, headers: HeaderMap) -> Vec<(StatusCode, HeaderMap)> {
        let store = FakeStore { select: Mutex::new(Some(Some("paper".into()))), ..Default::default() };
        let g = answer(get_theme_with(callers, &store, &headers).await).await;
        let p = answer(
            put_theme_with(callers, &store, &headers, Body::from(r#"{"target":"https://example.com","settings":{}}"#))
                .await,
        )
        .await;
        assert!(store.written.lock().unwrap().is_empty());
        vec![(g.0, g.1), (p.0, p.1)]
    }

    #[tokio::test]
    async fn refuses_a_request_carrying_neither_a_token_nor_a_session() {
        for (status, headers) in both(&FakeCallers::default(), HeaderMap::new()).await {
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            // marks the refusal private, so no shared cache keeps it
            assert_eq!(headers[header::CACHE_CONTROL], "private, no-store");
        }
    }

    #[tokio::test]
    async fn refuses_an_invalid_token_without_falling_through_to_the_session() {
        let callers = FakeCallers::default();
        *callers.token.lock().unwrap() = Some(TokenAuth::Unknown);
        *callers.session.lock().unwrap() = Some(session_of("session-user", Some("admin")));
        for (name, value) in [("x-api-key", "revoked"), ("authorization", "Bearer revoked")] {
            let mut h = HeaderMap::new();
            h.insert(name, HeaderValue::from_static(value));
            for (status, _) in both(&callers, h).await {
                assert_eq!(status, StatusCode::UNAUTHORIZED);
            }
        }
        assert_eq!(*callers.session_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn refuses_a_banned_account_holding_a_working_token() {
        let callers = FakeCallers::default();
        *callers.token.lock().unwrap() = Some(TokenAuth::Banned { user: user("u1", true, Some("user")) });
        for (status, _) in both(&callers, key()).await {
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn refuses_an_empty_x_api_key_when_there_is_no_session_behind_it() {
        let callers = FakeCallers::default();
        let mut h = HeaderMap::new();
        h.insert("x-api-key", HeaderValue::from_static(""));
        for (status, _) in both(&callers, h).await {
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        assert!(callers.token_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_read_token_may_read_and_may_not_write() {
        let callers = FakeCallers::default();
        *callers.token.lock().unwrap() = Some(TokenAuth::Scope {
            scope: TokenScope::Read,
            required: TokenCapability::Write,
            user: user("u1", false, Some("user")),
        });
        let store = FakeStore::default();
        let res = put_theme_with(&callers, &store, &key(), Body::from(r#"{"theme":"paper"}"#)).await;
        let (status, _, body) = answer(res).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["reason"], "insufficient_scope");
        assert_eq!(body["required_scope"], "write");
        assert!(store.written.lock().unwrap().is_empty());
        assert_eq!(callers.token_calls.lock().unwrap()[0].1, TokenCapability::Write);
    }

    #[tokio::test]
    async fn a_session_reaches_both_methods() {
        let callers = FakeCallers::default();
        *callers.session.lock().unwrap() = Some(session_of("u9", None));
        let store = FakeStore {
            select: Mutex::new(Some(Some("glass".into()))),
            update: Mutex::new(Some(Some("win98".into()))),
            ..Default::default()
        };
        assert_eq!(answer(get_theme_with(&callers, &store, &HeaderMap::new()).await).await.2["theme"], "glass");
        let res = put_theme_with(&callers, &store, &HeaderMap::new(), Body::from(r#"{"theme":"win98"}"#)).await;
        assert_eq!(answer(res).await.2["theme"], "win98");
        assert_eq!(*store.written.lock().unwrap(), vec![("u9".to_string(), "win98".to_string())]);
    }

    #[tokio::test]
    async fn a_database_error_is_a_bare_500_like_a_throwing_route_handler() {
        let res = finish(Err(AppError::Internal(anyhow::anyhow!("connection refused"))));
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(res.headers().get(header::CONTENT_TYPE).is_none());
        assert!(res.into_body().collect().await.unwrap().to_bytes().is_empty());
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Stand-ins for `authenticateApiToken` and `getSession`, shared by the
    //! route tests of /api/theme, /api/client-settings and /api/me.
    use std::sync::Mutex;

    use crate::api_caller::CallerDeps;
    use crate::api_token::test_support::user;
    use crate::api_token::{TokenAuth, TokenCapability, TokenScope, UserRow};
    use crate::auth::session::AuthSession;
    use crate::db::models::{JsDate, Session};

    /// `authenticateApiToken` and `getSession`, recorded.
    #[derive(Default)]
    pub struct FakeCallers {
        pub token: Mutex<Option<TokenAuth>>,
        pub session: Mutex<Option<AuthSession>>,
        pub token_calls: Mutex<Vec<(String, TokenCapability)>>,
        pub session_calls: Mutex<u32>,
    }

    impl FakeCallers {
        pub fn token_ok(scope: TokenScope) -> FakeCallers {
            let f = FakeCallers::default();
            *f.token.lock().unwrap() = Some(TokenAuth::Ok { user: user("u1", false, Some("user")), scope });
            f
        }
    }

    impl CallerDeps for FakeCallers {
        async fn authenticate(&self, token: &str, required: TokenCapability) -> anyhow::Result<TokenAuth> {
            self.token_calls.lock().unwrap().push((token.to_string(), required));
            Ok(self.token.lock().unwrap().clone().unwrap_or(TokenAuth::Unknown))
        }
        async fn session(&self) -> anyhow::Result<Option<AuthSession>> {
            *self.session_calls.lock().unwrap() += 1;
            Ok(self.session.lock().unwrap().clone())
        }
        async fn load_user(&self, id: &str) -> anyhow::Result<Option<UserRow>> {
            Ok(Some(user(id, false, None)))
        }
    }

    pub fn session_of(id: &str, role: Option<&str>) -> AuthSession {
        AuthSession {
            session: Session {
                id: "s".into(),
                expires_at: JsDate::now(),
                token: "t".into(),
                created_at: JsDate::now(),
                updated_at: JsDate::now(),
                ip_address: None,
                user_agent: None,
                user_id: id.into(),
                impersonated_by: None,
            },
            user: user(id, false, role),
        }
    }
}
