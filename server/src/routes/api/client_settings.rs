//! Port of `app/api/client-settings/route.ts`: the desktop client's
//! configuration, stored against the account.
//!
//! GET hands back what is stored (or an empty default), PUT replaces it
//! whole. The server never looks inside: what a hotkey or a capture preset
//! means is the client's business. See `client_settings.rs` for the two things
//! the server DOES decide - the size cap, and what happens when two machines
//! save at once.
//!
//! Authenticates with an API TOKEN first and a session second, in that order.
//! The token is what a desktop app has; the session is a convenience so the
//! same URL can be opened in a logged-in browser while debugging. Either way
//! the answer is scoped to one account.
//!
//! The settings document travels as JavaScript sees it: the request through
//! `JSON.parse`/`JSON.stringify` ([`crate::js_json`]), the stored row read as
//! Postgres's jsonb text and printed the way node-postgres' `JSON.parse`
//! followed by the route's `JSON.stringify` printed it (integer-like keys
//! first, JavaScript number formatting), so either server answers with the
//! same bytes.

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::StreamExt;
use serde_json::json;
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::postgres::{PgArgumentBuffer, PgTypeInfo};
use sqlx::{PgPool, Postgres};

use crate::api_caller::{CallerDeps, LiveCallerDeps, resolve_api_caller_with};
use crate::api_json::{PRIVATE_NO_STORE, json_private};
use crate::api_token::TokenCapability;
use crate::auth_helpers::Sessions;
use crate::client_settings::{
    CLIENT_SETTINGS_BODY_MAX_BYTES, CLIENT_SETTINGS_MAX_BYTES, SettingsBody, body_too_large_message, normalize_client,
    parse_client_settings_body, unknown_client_message,
};
use crate::db::models::JsDate;
use crate::error::{AppError, AppResult};
use crate::js_json::{self, JsValue};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/client-settings", get(get_settings).put(put_settings))
}

/// The columns every response is built from: `version`, `settings` (as
/// Postgres prints the jsonb) and `updated_at`.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct StoredRow {
    pub version: i32,
    pub settings: String,
    pub updated_at: JsDate,
}

/// The canonical text as a `jsonb` parameter, the way node-postgres sends
/// drizzle's `JSON.stringify(settings)`: Postgres parses it when the
/// statement is bound, so a document it cannot hold (a lone surrogate) fails
/// even where the WHERE would have matched nothing, exactly as upstream.
struct JsonbText<'a>(&'a str);

impl sqlx::Type<Postgres> for JsonbText<'_> {
    fn type_info() -> PgTypeInfo {
        <sqlx::types::Json<serde_json::Value> as sqlx::Type<Postgres>>::type_info()
    }
}

impl sqlx::Encode<'_, Postgres> for JsonbText<'_> {
    fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
        // jsonb binary format: a version byte, then the JSON text.
        buf.push(1);
        buf.extend_from_slice(self.0.as_bytes());
        Ok(IsNull::No)
    }
}

const RETURNED: &str = "\"version\", \"settings\"::text AS \"settings\", \"updated_at\"";

/// The statements the route issues (stubbed in tests, as upstream's are
/// drizzle builders over `@/lib/db`).
pub trait SettingsStore {
    /// `SELECT ... WHERE owner_id = $1 AND client = $2 LIMIT 1`
    fn current(&self, owner_id: &str, client: &str) -> impl Future<Output = anyhow::Result<Option<StoredRow>>> + Send;
    /// No expected version: `INSERT ... ON CONFLICT (owner_id, client) DO
    /// UPDATE SET settings, version = client_settings.version + 1, updated_at`.
    fn replace(
        &self,
        owner_id: &str,
        client: &str,
        settings: &str,
        now: JsDate,
    ) -> impl Future<Output = anyhow::Result<Option<StoredRow>>> + Send;
    /// Expected version 0: `INSERT ... ON CONFLICT DO NOTHING`.
    fn insert_first(
        &self,
        owner_id: &str,
        client: &str,
        settings: &str,
        now: JsDate,
    ) -> impl Future<Output = anyhow::Result<Option<StoredRow>>> + Send;
    /// Expected version N: `UPDATE ... SET version = N + 1 WHERE owner_id AND
    /// client AND version = N`.
    fn compare_and_set(
        &self,
        owner_id: &str,
        client: &str,
        settings: &str,
        expected: i32,
        now: JsDate,
    ) -> impl Future<Output = anyhow::Result<Option<StoredRow>>> + Send;
}

/// The real statements.
pub struct PgSettingsStore<'a>(pub &'a PgPool);

impl SettingsStore for PgSettingsStore<'_> {
    async fn current(&self, owner_id: &str, client: &str) -> anyhow::Result<Option<StoredRow>> {
        let sql = format!(
            "SELECT {RETURNED} FROM \"client_settings\" WHERE (\"client_settings\".\"owner_id\" = $1 AND \"client_settings\".\"client\" = $2) LIMIT 1"
        );
        Ok(sqlx::query_as::<_, StoredRow>(&sql).bind(owner_id).bind(client).fetch_optional(self.0).await?)
    }

    async fn replace(
        &self,
        owner_id: &str,
        client: &str,
        settings: &str,
        now: JsDate,
    ) -> anyhow::Result<Option<StoredRow>> {
        // In DO UPDATE, the qualified column is the row already there, so
        // this is "one more than whatever the loser of the race left", not
        // "one more than what I read a moment ago".
        let sql = format!(
            "INSERT INTO \"client_settings\" (\"owner_id\", \"client\", \"version\", \"settings\", \"updated_at\") VALUES ($1, $2, 1, $3, $4) \
             ON CONFLICT (\"owner_id\", \"client\") DO UPDATE SET \"settings\" = $3, \"version\" = \"client_settings\".\"version\" + 1, \"updated_at\" = $4 \
             RETURNING {RETURNED}"
        );
        Ok(sqlx::query_as::<_, StoredRow>(&sql)
            .bind(owner_id)
            .bind(client)
            .bind(JsonbText(settings))
            .bind(now)
            .fetch_optional(self.0)
            .await?)
    }

    async fn insert_first(
        &self,
        owner_id: &str,
        client: &str,
        settings: &str,
        now: JsDate,
    ) -> anyhow::Result<Option<StoredRow>> {
        let sql = format!(
            "INSERT INTO \"client_settings\" (\"owner_id\", \"client\", \"version\", \"settings\", \"updated_at\") VALUES ($1, $2, 1, $3, $4) \
             ON CONFLICT (\"owner_id\", \"client\") DO NOTHING RETURNING {RETURNED}"
        );
        Ok(sqlx::query_as::<_, StoredRow>(&sql)
            .bind(owner_id)
            .bind(client)
            .bind(JsonbText(settings))
            .bind(now)
            .fetch_optional(self.0)
            .await?)
    }

    async fn compare_and_set(
        &self,
        owner_id: &str,
        client: &str,
        settings: &str,
        expected: i32,
        now: JsDate,
    ) -> anyhow::Result<Option<StoredRow>> {
        let sql = format!(
            "UPDATE \"client_settings\" SET \"settings\" = $1, \"version\" = $2, \"updated_at\" = $3 \
             WHERE (\"client_settings\".\"owner_id\" = $4 AND \"client_settings\".\"client\" = $5 AND \"client_settings\".\"version\" = $6) \
             RETURNING {RETURNED}"
        );
        Ok(sqlx::query_as::<_, StoredRow>(&sql)
            .bind(JsonbText(settings))
            .bind(expected + 1)
            .bind(now)
            .bind(owner_id)
            .bind(client)
            .bind(expected)
            .fetch_optional(self.0)
            .await?)
    }
}

/// JSON text with a status, never held by a shared cache. The body is built
/// by hand because the settings document is printed by [`js_json`].
fn raw_json(status: StatusCode, body: String) -> Response {
    let mut res = (status, body).into_response();
    let headers = res.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(PRIVATE_NO_STORE));
    res
}

fn message(status: StatusCode, text: &str) -> Response {
    json_private(status, &json!({ "message": text }))
}

/// The wire shape, identical for GET, PUT and the 409 body, as the members
/// of an object (`message` goes first in the 409).
///
/// `limitBytes` is included so a client can refuse an over-sized save locally
/// instead of discovering the cap by being rejected, and so raising it later
/// does not need a client release.
fn shape_members(client: &str, row: Option<&StoredRow>) -> anyhow::Result<String> {
    // 0 is "nothing stored yet", which is what a client PUTs back as its
    // expected version on a fresh machine. A stored row starts at 1.
    let version = row.map_or(0, |r| r.version);
    let settings = match row {
        // `row.settings ?? {}`: a jsonb `null` reads back as null and is
        // answered as an empty object too.
        Some(r) => match js_json::parse(&r.settings).map_err(|e| anyhow::anyhow!("stored settings: {e}"))? {
            JsValue::Null => "{}".to_string(),
            value => js_json::stringify(&value),
        },
        None => "{}".to_string(),
    };
    let updated_at = match row {
        Some(r) => serde_json::to_string(&r.updated_at.to_iso_string())?,
        None => "null".to_string(),
    };
    Ok(format!(
        "\"client\":{},\"version\":{version},\"settings\":{settings},\"updatedAt\":{updated_at},\"limitBytes\":{CLIENT_SETTINGS_MAX_BYTES}",
        serde_json::to_string(client)?
    ))
}

fn shape(client: &str, row: Option<&StoredRow>) -> AppResult<Response> {
    Ok(raw_json(StatusCode::OK, format!("{{{}}}", shape_members(client, row)?)))
}

/// Somebody else wrote first.
///
/// The current state travels WITH the 409 so the client can merge and retry
/// from one round trip. A bare 409 would force a follow-up GET, and a client
/// that skipped it would be tempted to just PUT again without a version -
/// which is precisely the silent clobber the version exists to prevent.
async fn conflict<S: SettingsStore>(store: &S, owner_id: &str, client: &str) -> AppResult<Response> {
    let row = store.current(owner_id, client).await?;
    let body = format!(
        "{{\"message\":\"Settings were changed elsewhere. Merge and retry with the returned version.\",{}}}",
        shape_members(client, row.as_ref())?
    );
    Ok(raw_json(StatusCode::CONFLICT, body))
}

/// `new URL(req.url).searchParams.get("client")`: the first `client` pair of
/// the query, decoded as application/x-www-form-urlencoded.
fn client_param(uri: &Uri) -> Option<String> {
    let query = uri.query()?;
    form_urlencoded::parse(query.as_bytes()).find(|(k, _)| k == "client").map(|(_, v)| v.into_owned())
}

/// GET: reading the blob back onto another machine.
pub async fn get_settings_with<C: CallerDeps, S: SettingsStore>(
    callers: &C,
    store: &S,
    uri: &Uri,
    headers: &HeaderMap,
) -> AppResult<Response> {
    let (caller, _) =
        resolve_api_caller_with(callers, headers, TokenCapability::Read).await?.or_respond(Some(PRIVATE_NO_STORE))?;

    let Some(client) = normalize_client(client_param(uri).as_deref()) else {
        return Ok(message(StatusCode::BAD_REQUEST, &unknown_client_message()));
    };

    let row = store.current(&caller.id, client).await?;
    shape(client, row.as_ref())
}

/// PUT: replacing it. The blob carries the client's hotkeys and capture
/// presets, so an upload-only credential has no business rewriting how a
/// machine behaves.
pub async fn put_settings_with<C: CallerDeps, S: SettingsStore>(
    callers: &C,
    store: &S,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
) -> AppResult<Response> {
    let (caller, _) =
        resolve_api_caller_with(callers, headers, TokenCapability::Write).await?.or_respond(Some(PRIVATE_NO_STORE))?;

    let Some(client) = normalize_client(client_param(uri).as_deref()) else {
        return Ok(message(StatusCode::BAD_REQUEST, &unknown_client_message()));
    };

    let Some(text) = read_capped_body(headers, body, CLIENT_SETTINGS_BODY_MAX_BYTES).await? else {
        return Ok(message(StatusCode::PAYLOAD_TOO_LARGE, &body_too_large_message()));
    };

    let parsed = parse_client_settings_body(&text).map_err(|e| AppError::Internal(e.into()))?;
    let accepted = match parsed {
        SettingsBody::Ok(accepted) => accepted,
        SettingsBody::Refused { status, message: text } => return Ok(message(status, &text)),
    };

    let now = JsDate::now();
    match accepted.expected_version {
        // No expected version: "take this, whatever was there". The blind
        // path, and the one a client that has never synced should use.
        None => {
            let row = store.replace(&caller.id, client, &accepted.canonical, now).await?;
            shape(client, row.as_ref())
        }
        // An expected version of 0 asserts "nothing is stored". Only an insert
        // that actually inserts satisfies it; a row appearing in between is a
        // conflict, not something to overwrite.
        Some(0) => match store.insert_first(&caller.id, client, &accepted.canonical, now).await? {
            Some(row) => shape(client, Some(&row)),
            None => conflict(store, &caller.id, client).await,
        },
        // The compare-and-set that makes concurrent machines safe. Ownership
        // lives in the same WHERE as the version rather than in a SELECT
        // first: one statement cannot be raced between its check and its
        // write, and a row belonging to somebody else is simply not matched.
        // The new version is `expected + 1` rather than an increment
        // expression because the WHERE already pinned the old value.
        Some(expected) => {
            let expected = i32::try_from(expected).map_err(anyhow::Error::from)?;
            match store.compare_and_set(&caller.id, client, &accepted.canonical, expected, now).await? {
                Some(row) => shape(client, Some(&row)),
                None => conflict(store, &caller.id, client).await,
            }
        }
    }
}

/// Read the body, refusing anything over `limit` (`None`) INSTEAD OF
/// buffering it first.
///
/// The declared length is checked first because refusing before reading costs
/// nothing, and the running total is checked because a chunked body declares
/// no length at all. Dropping the stream is what tells the sender to stop,
/// rather than politely draining a body already decided against. A body that
/// fails mid-read is an error (upstream's `reader.read()` throws).
async fn read_capped_body(headers: &HeaderMap, body: Body, limit: usize) -> anyhow::Result<Option<String>> {
    let declared = crate::config::js_number(
        &headers
            .get(header::CONTENT_LENGTH)
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .unwrap_or_default(),
    );
    if declared.is_finite() && declared > limit as f64 {
        return Ok(None);
    }

    let mut buf: Vec<u8> = Vec::new();
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow::anyhow!("reading the request body: {e}"))?;
        if buf.len() + chunk.len() > limit {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk);
    }

    // Decoded in one pass over the joined buffer, so a multi-byte character
    // split across two chunks is still one character.
    Ok(Some(js_json::utf8_decode(&buf)))
}

/// How Next answers a route handler that threw (a database error, a session
/// lookup that failed, the `RangeError` of an over-deep document): a bare
/// 500, no body and no content type.
fn finish(result: AppResult<Response>) -> Response {
    match result {
        Ok(response) => response,
        Err(AppError::Internal(err)) => {
            tracing::error!(error = ?err, "GET/PUT /api/client-settings failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        Err(other) => other.into_response(),
    }
}

async fn get_settings(
    State(state): State<AppState>,
    Sessions(session): Sessions,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let callers = LiveCallerDeps { state: &state, session: &session };
    finish(get_settings_with(&callers, &PgSettingsStore(&state.db), &uri, &headers).await)
}

async fn put_settings(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let callers = LiveCallerDeps { state: &state, session: &session };
    finish(put_settings_with(&callers, &PgSettingsStore(&state.db), &parts.uri, &parts.headers, body).await)
}

#[cfg(test)]
mod tests {
    //! The /api/client-settings rows of tests/unit/api-route-auth.test.ts,
    //! plus the route's own decisions (which statement each expected version
    //! runs, the 409 body, the shape of a stored row).
    use std::sync::Mutex;

    use axum::http::HeaderValue;
    use http_body_util::BodyExt;

    use super::*;
    use crate::api_token::test_support::user;
    use crate::api_token::{TokenAuth, TokenScope};
    use crate::routes::api::theme::test_support::{FakeCallers, session_of};

    fn row(version: i32, settings: &str) -> StoredRow {
        StoredRow {
            version,
            settings: settings.to_string(),
            updated_at: JsDate::parse_iso("2026-10-19T12:00:00.123Z").unwrap(),
        }
    }

    /// Answers per statement, and every statement issued.
    #[derive(Default)]
    struct FakeStore {
        current: Mutex<Option<StoredRow>>,
        write: Mutex<Option<StoredRow>>,
        calls: Mutex<Vec<String>>,
    }

    impl SettingsStore for FakeStore {
        async fn current(&self, owner: &str, client: &str) -> anyhow::Result<Option<StoredRow>> {
            self.calls.lock().unwrap().push(format!("current {owner} {client}"));
            Ok(self.current.lock().unwrap().clone())
        }
        async fn replace(&self, owner: &str, client: &str, s: &str, _: JsDate) -> anyhow::Result<Option<StoredRow>> {
            self.calls.lock().unwrap().push(format!("replace {owner} {client} {s}"));
            Ok(self.write.lock().unwrap().clone())
        }
        async fn insert_first(
            &self,
            owner: &str,
            client: &str,
            s: &str,
            _: JsDate,
        ) -> anyhow::Result<Option<StoredRow>> {
            self.calls.lock().unwrap().push(format!("insert_first {owner} {client} {s}"));
            Ok(self.write.lock().unwrap().clone())
        }
        async fn compare_and_set(
            &self,
            owner: &str,
            client: &str,
            s: &str,
            expected: i32,
            _: JsDate,
        ) -> anyhow::Result<Option<StoredRow>> {
            self.calls.lock().unwrap().push(format!("compare_and_set {owner} {client} {s} {expected}"));
            Ok(self.write.lock().unwrap().clone())
        }
    }

    fn key() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-api-key", HeaderValue::from_static("good"));
        h
    }

    async fn answer(res: AppResult<Response>) -> (StatusCode, HeaderMap, String) {
        let (parts, body) = finish(res).into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        (parts.status, parts.headers, String::from_utf8(bytes.to_vec()).unwrap())
    }

    async fn get(
        callers: &FakeCallers,
        store: &FakeStore,
        uri: &str,
        headers: HeaderMap,
    ) -> (StatusCode, HeaderMap, String) {
        answer(get_settings_with(callers, store, &uri.parse().unwrap(), &headers).await).await
    }

    async fn put(callers: &FakeCallers, store: &FakeStore, uri: &str, body: &str) -> (StatusCode, HeaderMap, String) {
        answer(put_settings_with(callers, store, &uri.parse().unwrap(), &key(), Body::from(body.to_string())).await)
            .await
    }

    const URL: &str = "/api/client-settings";

    #[tokio::test]
    async fn get_answers_an_empty_default_before_anything_is_stored() {
        let (status, headers, body) =
            get(&FakeCallers::token_ok(TokenScope::Read), &FakeStore::default(), URL, key()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, r#"{"client":"desktop","version":0,"settings":{},"updatedAt":null,"limitBytes":262144}"#);
        assert_eq!(headers[header::CACHE_CONTROL], "private, no-store");
        assert_eq!(headers[header::CONTENT_TYPE], "application/json");
    }

    #[tokio::test]
    async fn get_prints_the_stored_row_the_way_javascript_reads_it() {
        // Postgres's jsonb text: keys by length then bytes, numbers as numeric.
        let store = FakeStore {
            current: Mutex::new(Some(row(3, r#"{"b": 1, "10": 1000000000000000000000, "2": [1.50, "é"]}"#))),
            ..Default::default()
        };
        let (_, _, body) = get(&FakeCallers::token_ok(TokenScope::Full), &store, URL, key()).await;
        assert_eq!(
            body,
            r#"{"client":"desktop","version":3,"settings":{"2":[1.5,"é"],"10":1e+21,"b":1},"updatedAt":"2026-10-19T12:00:00.123Z","limitBytes":262144}"#
        );
        // A jsonb null in the column reads back as `{}` (`row.settings ?? {}`).
        *store.current.lock().unwrap() = Some(row(4, "null"));
        let (_, _, body) = get(&FakeCallers::token_ok(TokenScope::Full), &store, URL, key()).await;
        assert!(body.contains(r#""version":4,"settings":{},"#), "{body}");
    }

    #[tokio::test]
    async fn refuses_a_client_kind_that_is_not_on_the_allowlist() {
        let store = FakeStore::default();
        for uri in ["/api/client-settings?client=mobile", "/api/client-settings?client=%20%20"] {
            let (status, _, body) = get(&FakeCallers::token_ok(TokenScope::Full), &store, uri, key()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(body, r#"{"message":"Unknown client. Known clients: desktop."}"#);
        }
        for uri in [
            "/api/client-settings?client=+DESKTOP+",
            "/api/client-settings?client=desktop&client=mobile",
            "/api/client-settings?client=",
            "/api/client-settings?client",
            "/api/client-settings?other=1",
        ] {
            assert_eq!(
                get(&FakeCallers::token_ok(TokenScope::Full), &store, uri, key()).await.0,
                StatusCode::OK,
                "{uri}"
            );
        }
        assert!(store.calls.lock().unwrap().iter().all(|c| c == "current u1 desktop"));
    }

    #[tokio::test]
    async fn put_without_a_version_replaces_blindly() {
        let store = FakeStore { write: Mutex::new(Some(row(2, r#"{"a": 1}"#))), ..Default::default() };
        let (status, _, body) =
            put(&FakeCallers::token_ok(TokenScope::Write), &store, URL, r#"{"settings":{"a":1.0}}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(*store.calls.lock().unwrap(), vec![r#"replace u1 desktop {"a":1}"#.to_string()]);
        assert!(body.starts_with(r#"{"client":"desktop","version":2,"settings":{"a":1},"#));
    }

    #[tokio::test]
    async fn put_with_version_0_only_inserts_and_answers_409_with_the_current_row() {
        let store = FakeStore { current: Mutex::new(Some(row(5, "{}"))), ..Default::default() };
        let (status, headers, body) =
            put(&FakeCallers::token_ok(TokenScope::Write), &store, URL, r#"{"settings":{},"version":0}"#).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            body,
            r#"{"message":"Settings were changed elsewhere. Merge and retry with the returned version.","client":"desktop","version":5,"settings":{},"updatedAt":"2026-10-19T12:00:00.123Z","limitBytes":262144}"#
        );
        assert_eq!(headers[header::CACHE_CONTROL], "private, no-store");
        assert_eq!(*store.calls.lock().unwrap(), vec!["insert_first u1 desktop {}", "current u1 desktop"]);
    }

    #[tokio::test]
    async fn put_with_a_version_compares_and_sets() {
        let store = FakeStore { write: Mutex::new(Some(row(8, "{}"))), ..Default::default() };
        let (status, _, _) =
            put(&FakeCallers::token_ok(TokenScope::Full), &store, URL, r#"{"settings":{},"version":7}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(*store.calls.lock().unwrap(), vec!["compare_and_set u1 desktop {} 7"]);
        // Nothing matched (stale version, or nothing stored): 409 with the
        // current state, version 0 when there is none.
        let store = FakeStore::default();
        let (status, _, body) =
            put(&FakeCallers::token_ok(TokenScope::Full), &store, URL, r#"{"settings":{},"version":7}"#).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            body.ends_with(r#""client":"desktop","version":0,"settings":{},"updatedAt":null,"limitBytes":262144}"#)
        );
    }

    #[tokio::test]
    async fn put_refusals_come_before_any_statement() {
        let callers = FakeCallers::token_ok(TokenScope::Full);
        let store = FakeStore::default();
        for (uri, body, status, message) in [
            (
                "/api/client-settings?client=mobile",
                "{}",
                StatusCode::BAD_REQUEST,
                "Unknown client. Known clients: desktop.",
            ),
            (URL, "", StatusCode::BAD_REQUEST, r#"Body must be JSON: {"settings": { ... }}."#),
            (URL, "{", StatusCode::BAD_REQUEST, "Body is not valid JSON."),
            (URL, r#"{"settings":[]}"#, StatusCode::BAD_REQUEST, "`settings` must be a JSON object."),
            (
                URL,
                r#"{"settings":{"k":"\u0000"}}"#,
                StatusCode::BAD_REQUEST,
                "Settings must not contain NUL characters.",
            ),
        ] {
            let (got, _, text) = put(&callers, &store, uri, body).await;
            assert_eq!(got, status, "{body}");
            assert_eq!(serde_json::from_str::<serde_json::Value>(&text).unwrap()["message"], message);
        }
        assert!(store.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn put_refuses_a_body_over_the_transport_cap_by_header_or_by_count() {
        let callers = FakeCallers::token_ok(TokenScope::Full);
        let store = FakeStore::default();
        let mut h = key();
        h.insert(header::CONTENT_LENGTH, HeaderValue::from_static("524289"));
        let res = put_settings_with(&callers, &store, &URL.parse().unwrap(), &h, Body::from("{}")).await;
        let (status, _, body) = answer(res).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            body,
            r#"{"message":"Request body too large. Settings are limited to 262144 bytes (256 KiB) of JSON."}"#
        );
        // Counted in bytes, not characters: 524289 bytes of body.
        let big = format!(r#"{{"settings":{{"a":"{}"}}}}"#, "x".repeat(524289 - 21));
        assert_eq!(big.len(), 524289);
        assert_eq!(put(&callers, &store, URL, &big).await.0, StatusCode::PAYLOAD_TOO_LARGE);
        let fits = format!(r#"{{"settings":{{"a":"{}"}}}}"#, "x".repeat(524288 - 21));
        assert_eq!(
            put(&callers, &store, URL, &fits).await.0,
            StatusCode::PAYLOAD_TOO_LARGE,
            "fits the wire, not the column"
        );
        assert!(store.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_over_deep_document_is_the_bare_500_of_a_throwing_handler() {
        let store = FakeStore::default();
        let deep = format!(r#"{{"settings":{{"a":{}{}}}}}"#, "[".repeat(5000), "]".repeat(5000));
        let (status, headers, body) = put(&FakeCallers::token_ok(TokenScope::Full), &store, URL, &deep).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(headers.get(header::CONTENT_TYPE).is_none());
        assert!(body.is_empty());
        assert!(store.calls.lock().unwrap().is_empty());
    }

    // ── tests/unit/api-route-auth.test.ts (GET and PUT /api/client-settings) ──

    async fn both(callers: &FakeCallers, headers: HeaderMap) -> Vec<(StatusCode, HeaderMap)> {
        let store = FakeStore::default();
        let uri: Uri = URL.parse().unwrap();
        let g = answer(get_settings_with(callers, &store, &uri, &headers).await).await;
        let body = Body::from(r#"{"target":"https://example.com","settings":{}}"#);
        let p = answer(put_settings_with(callers, &store, &uri, &headers, body).await).await;
        assert!(store.calls.lock().unwrap().is_empty());
        vec![(g.0, g.1), (p.0, p.1)]
    }

    #[tokio::test]
    async fn refuses_a_request_carrying_neither_a_token_nor_a_session() {
        for (status, headers) in both(&FakeCallers::default(), HeaderMap::new()).await {
            assert_eq!(status, StatusCode::UNAUTHORIZED);
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
    async fn names_read_for_get_and_write_for_put() {
        let callers = FakeCallers::token_ok(TokenScope::Full);
        let store = FakeStore::default();
        let uri: Uri = URL.parse().unwrap();
        get_settings_with(&callers, &store, &uri, &key()).await.unwrap();
        put_settings_with(&callers, &store, &uri, &key(), Body::from(r#"{"settings":{}}"#)).await.unwrap();
        let caps: Vec<TokenCapability> = callers.token_calls.lock().unwrap().iter().map(|(_, c)| *c).collect();
        assert_eq!(caps, vec![TokenCapability::Read, TokenCapability::Write]);
    }

    #[tokio::test]
    async fn a_session_reaches_both_methods() {
        let callers = FakeCallers::default();
        *callers.session.lock().unwrap() = Some(session_of("u9", None));
        let store = FakeStore { write: Mutex::new(Some(row(1, "{}"))), ..Default::default() };
        let uri: Uri = URL.parse().unwrap();
        assert_eq!(answer(get_settings_with(&callers, &store, &uri, &HeaderMap::new()).await).await.0, StatusCode::OK);
        let res = put_settings_with(&callers, &store, &uri, &HeaderMap::new(), Body::from(r#"{"settings":{}}"#)).await;
        assert_eq!(answer(res).await.0, StatusCode::OK);
        assert_eq!(*store.calls.lock().unwrap(), vec!["current u9 desktop", "replace u9 desktop {}"]);
    }
}
