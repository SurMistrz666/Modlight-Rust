//! Port of `app/api/admin/disk/route.ts`: the server disk report, for an
//! administrator's desktop client. The same two read models the admin page
//! uses (`server_storage_usage`, `disk_insights`), flattened field for field
//! into the shape the client's `ModlightDiskReport` deserialises. Read-only:
//! GET is the only verb.
//!
//! Two gates, because they answer two questions: the ROLE (only an
//! administrator may read what every account stores) and the SCOPE (an
//! administrator's everyday `write` token lives in a config file on a laptop,
//! so only `full` opens this). A scope refusal is spoken out loud only to an
//! administrator; to anybody else every refusal past the credential is the
//! 404 of a path with no route.

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::get;
use serde_json::json;

use crate::api_caller::UNAUTHORIZED_MESSAGE;
use crate::api_json::json_private;
use crate::api_token::{TokenAuth, TokenCapability, authenticate_api_token, bearer_from_request, scope_refusal_body};
use crate::auth::session::AuthSession;
use crate::auth_helpers::{RequestSession, Sessions};
use crate::data::admin_disk::{TRASH_DUE_SOON_DAYS, disk_insights, server_storage_usage};
use crate::db::types::JsDate;
use crate::error::{AppError, AppResult};
use crate::maintenance::trash::TRASH_RETENTION_DAYS;
use crate::shortlinks::text_response;
use crate::state::AppState;

/// Calls per account per minute: the report is the most expensive read in
/// the app, and a client with a (buggy) refresh timer does not pace itself.
/// Keyed on the account: the credential is what is being paced.
const CALLS_PER_MINUTE: u32 = 30;
const RATE_WINDOW_MS: i64 = 60_000;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/admin/disk", get(report))
}

/// What a non-administrator is told: 404, and the SAME response a path with
/// no route gets (`text/plain`, `Not found`, no cache-control). A 403 would
/// answer "is this account an administrator", and a JSON body would leak by
/// its shape what the status is careful not to say.
fn not_found() -> Response {
    text_response(StatusCode::NOT_FOUND, "Not found")
}

/// What the gate needs from the outside world (stubbed in the unit tests, as
/// upstream mocks `authenticateApiToken` and `getSession`).
pub trait DiskGateDeps {
    /// `authenticateApiToken(token, "admin")`
    fn authenticate_admin(&self, token: &str) -> impl Future<Output = anyhow::Result<TokenAuth>> + Send;
    /// `getSession()`
    fn session(&self) -> impl Future<Output = anyhow::Result<Option<AuthSession>>> + Send;
}

struct LiveGate<'a> {
    state: &'a AppState,
    session: &'a RequestSession,
}

impl DiskGateDeps for LiveGate<'_> {
    async fn authenticate_admin(&self, token: &str) -> anyhow::Result<TokenAuth> {
        authenticate_api_token(&self.state.db, Some(token), TokenCapability::Admin).await
    }
    async fn session(&self) -> anyhow::Result<Option<AuthSession>> {
        Ok(self.session.get().await?)
    }
}

/// The gate's verdict: the administrator's id, or the refusal to send.
#[derive(Debug)]
pub enum Gate {
    Ok(String),
    Refused(Response),
}

fn is_admin(role: Option<&str>) -> bool {
    role == Some("admin")
}

/// Both gates, in upstream's order: the credential first (there is no role
/// to check until it is resolved), then the role, then the scope.
pub async fn gate<D: DiskGateDeps>(deps: &D, headers: &HeaderMap) -> anyhow::Result<Gate> {
    let unauthorized = || json_private(StatusCode::UNAUTHORIZED, &json!({ "message": UNAUTHORIZED_MESSAGE }));
    if let Some(token) = bearer_from_request(headers) {
        return Ok(match deps.authenticate_admin(&token).await? {
            TokenAuth::Ok { user, .. } if is_admin(user.role.as_deref()) => Gate::Ok(user.id),
            TokenAuth::Ok { .. } => Gate::Refused(not_found()),
            // A disabled account is told it is disabled, on every surface:
            // a 404 would send an administrator looking for a server upgrade.
            TokenAuth::Banned { .. } => {
                Gate::Refused(json_private(StatusCode::UNAUTHORIZED, &json!({ "message": "Account disabled." })))
            }
            TokenAuth::Scope { scope, required, user } if is_admin(user.role.as_deref()) => {
                Gate::Refused(json_private(StatusCode::FORBIDDEN, &scope_refusal_body(scope, required)))
            }
            TokenAuth::Scope { .. } => Gate::Refused(not_found()),
            // No user on this branch, so administrator-ness cannot be
            // established: fail closed.
            TokenAuth::InvalidScope => Gate::Refused(not_found()),
            TokenAuth::Unknown => Gate::Refused(unauthorized()),
        });
    }
    // No token: the cookie may speak. A session is the account itself and
    // carries no scope, so the role is the only gate left.
    Ok(match deps.session().await? {
        None => Gate::Refused(unauthorized()),
        Some(s) if !is_admin(s.user.role.as_deref()) => Gate::Refused(not_found()),
        Some(s) => Gate::Ok(s.user.id),
    })
}

async fn report(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, _) = req.into_parts();
    let id = match gate(&LiveGate { state: &state, session: &session }, &parts.headers).await? {
        Gate::Ok(id) => id,
        Gate::Refused(res) => return Ok(res),
    };

    let limit = crate::password::check_rate_limit(&format!("admin-disk:{id}"), CALLS_PER_MINUTE, RATE_WINDOW_MS);
    if !limit.allowed {
        let mut res = json_private(
            StatusCode::TOO_MANY_REQUESTS,
            &json!({ "message": "Too many requests.", "reason": "rate_limited" }),
        );
        let secs = (limit.retry_after_ms as f64 / 1000.0).ceil() as i64;
        if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
            res.headers_mut().insert("retry-after", v);
        }
        return Ok(res);
    }

    // The same two calls the admin page runs, in parallel as there.
    let (usage, insights) = tokio::try_join!(
        server_storage_usage(&state.db, &state.config),
        disk_insights(&state.db, &state.config, JsDate::now()),
    )
    .map_err(AppError::Internal)?;

    // Nothing renamed, regrouped or recomputed: every value is `usage.x` or
    // `insights.x`. `disk` is the only MEASUREMENT (a statfs, null on an
    // object store); everything else is the database's count, a floor.
    Ok(json_private(
        StatusCode::OK,
        &json!({
            "driver": usage.driver,
            "local": insights.local,
            "liveFiles": usage.live_files,
            "liveBytes": usage.live_bytes,
            "trashedFiles": usage.trashed_files,
            "trashedBytes": usage.trashed_bytes,
            "disk": usage.disk,
            "minFreeBytes": usage.min_free_bytes,
            "kinds": insights.kinds,
            "kindBytes": insights.kind_bytes,
            "accounts": insights.accounts,
            "biggest": insights.biggest,
            "cold": insights.cold,
            "coldTotals": insights.cold_totals,
            "trash": insights.trash,
            // This deployment's policy, so a client never captions the trash
            // with a hard-coded 30.
            "trashRetentionDays": TRASH_RETENTION_DAYS,
            "trashDueSoonDays": TRASH_DUE_SOON_DAYS,
        }),
    ))
}

#[cfg(test)]
mod tests {
    //! tests/unit/admin-disk-api.test.ts (the gate; the report runs against a
    //! real database in tests/library_routes_db.rs).
    use std::sync::Mutex;

    use axum::http::HeaderValue;
    use http_body_util::BodyExt;

    use super::*;
    use crate::api_token::TokenScope;
    use crate::api_token::test_support::user;
    use crate::db::models::Session;

    #[derive(Default)]
    struct Fake {
        token: Mutex<Option<TokenAuth>>,
        session: Mutex<Option<AuthSession>>,
        tokens_seen: Mutex<Vec<String>>,
        session_calls: Mutex<u32>,
    }

    impl DiskGateDeps for Fake {
        async fn authenticate_admin(&self, token: &str) -> anyhow::Result<TokenAuth> {
            self.tokens_seen.lock().unwrap().push(token.to_string());
            Ok(self.token.lock().unwrap().clone().unwrap_or(TokenAuth::Unknown))
        }
        async fn session(&self) -> anyhow::Result<Option<AuthSession>> {
            *self.session_calls.lock().unwrap() += 1;
            Ok(self.session.lock().unwrap().clone())
        }
    }

    fn fake(token: Option<TokenAuth>, session: Option<AuthSession>) -> Fake {
        Fake { token: Mutex::new(token), session: Mutex::new(session), ..Default::default() }
    }

    fn admin() -> crate::api_token::UserRow {
        user("admin-1", false, Some("admin"))
    }

    fn plain() -> crate::api_token::UserRow {
        user("user-1", false, Some("user"))
    }

    fn session_of(u: crate::api_token::UserRow) -> AuthSession {
        AuthSession {
            session: Session {
                id: "s".into(),
                expires_at: JsDate::now(),
                token: "t".into(),
                created_at: JsDate::now(),
                updated_at: JsDate::now(),
                ip_address: None,
                user_agent: None,
                user_id: u.id.clone(),
                impersonated_by: None,
            },
            user: u,
        }
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
        }
        h
    }

    struct Seen {
        status: StatusCode,
        content_type: Option<String>,
        cache_control: Option<String>,
        body: String,
    }

    async fn refused(g: Gate) -> Seen {
        let Gate::Refused(res) = g else { panic!("expected a refusal") };
        let (parts, body) = res.into_parts();
        let header = |n: &str| parts.headers.get(n).map(|v| v.to_str().unwrap().to_string());
        Seen {
            status: parts.status,
            content_type: header("content-type"),
            cache_control: header("cache-control"),
            body: String::from_utf8(body.collect().await.unwrap().to_bytes().to_vec()).unwrap(),
        }
    }

    const KEY: &[(&str, &str)] = &[("x-api-key", "good")];

    #[tokio::test]
    async fn refuses_a_non_administrator_holding_a_full_scope_token_as_a_missing_route() {
        let f = fake(Some(TokenAuth::Ok { user: plain(), scope: TokenScope::Full }), None);
        let seen = refused(gate(&f, &headers(KEY)).await.unwrap()).await;
        assert_eq!(seen.status, StatusCode::NOT_FOUND);
        assert_eq!(seen.content_type.as_deref(), Some("text/plain;charset=UTF-8"));
        assert_eq!(seen.body, "Not found");
        assert_eq!(seen.cache_control, None);
    }

    #[tokio::test]
    async fn refuses_a_signed_in_non_administrator() {
        let f = fake(None, Some(session_of(plain())));
        assert_eq!(refused(gate(&f, &headers(&[])).await.unwrap()).await.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn answers_an_administrator_with_an_admin_capable_token() {
        let f = fake(Some(TokenAuth::Ok { user: admin(), scope: TokenScope::Full }), None);
        assert!(matches!(gate(&f, &headers(KEY)).await.unwrap(), Gate::Ok(id) if id == "admin-1"));
        assert_eq!(*f.tokens_seen.lock().unwrap(), vec!["good".to_string()]);
        let s = fake(None, Some(session_of(admin())));
        assert!(matches!(gate(&s, &headers(&[])).await.unwrap(), Gate::Ok(_)));
    }

    #[tokio::test]
    async fn refuses_an_administrators_write_token_and_names_the_scope() {
        let f = fake(
            Some(TokenAuth::Scope { scope: TokenScope::Write, required: TokenCapability::Admin, user: admin() }),
            None,
        );
        let seen = refused(gate(&f, &headers(KEY)).await.unwrap()).await;
        assert_eq!(seen.status, StatusCode::FORBIDDEN);
        let body: serde_json::Value = serde_json::from_str(&seen.body).unwrap();
        assert_eq!(body["reason"], "insufficient_scope");
        assert_eq!(body["required_scope"], "full");
        let upload = fake(
            Some(TokenAuth::Scope { scope: TokenScope::Upload, required: TokenCapability::Admin, user: admin() }),
            None,
        );
        assert_eq!(refused(gate(&upload, &headers(KEY)).await.unwrap()).await.status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn gives_a_non_administrators_narrow_token_a_404() {
        let f = fake(
            Some(TokenAuth::Scope { scope: TokenScope::Write, required: TokenCapability::Admin, user: plain() }),
            None,
        );
        let seen = refused(gate(&f, &headers(KEY)).await.unwrap()).await;
        assert_eq!((seen.status, seen.body.as_str()), (StatusCode::NOT_FOUND, "Not found"));
        let invalid = fake(Some(TokenAuth::InvalidScope), None);
        assert_eq!(refused(gate(&invalid, &headers(KEY)).await.unwrap()).await.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn credential_refusals_stay_401_and_private() {
        let none = fake(None, None);
        let seen = refused(gate(&none, &headers(&[])).await.unwrap()).await;
        assert_eq!(seen.status, StatusCode::UNAUTHORIZED);
        assert_eq!(seen.cache_control.as_deref(), Some("private, no-store"));
        assert_eq!(seen.body, r#"{"message":"Token not specified or not found."}"#);

        // An unknown token never falls through to a valid session.
        let f = fake(Some(TokenAuth::Unknown), Some(session_of(admin())));
        for h in [headers(&[("x-api-key", "revoked")]), headers(&[("authorization", "Bearer revoked")])] {
            assert_eq!(refused(gate(&f, &h).await.unwrap()).await.status, StatusCode::UNAUTHORIZED);
        }
        assert_eq!(*f.session_calls.lock().unwrap(), 0);

        let banned = fake(Some(TokenAuth::Banned { user: user("admin-1", true, Some("admin")) }), None);
        let seen = refused(gate(&banned, &headers(KEY)).await.unwrap()).await;
        assert_eq!((seen.status, seen.body.as_str()), (StatusCode::UNAUTHORIZED, r#"{"message":"Account disabled."}"#));

        // An empty x-api-key is no token offered, and still shuts the door.
        let empty = fake(None, None);
        assert_eq!(
            refused(gate(&empty, &headers(&[("x-api-key", "")])).await.unwrap()).await.status,
            StatusCode::UNAUTHORIZED
        );
        assert!(empty.tokens_seen.lock().unwrap().is_empty());
    }
}
