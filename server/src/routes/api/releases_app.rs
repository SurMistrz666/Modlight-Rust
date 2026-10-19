//! Port of `app/api/releases/app/route.ts`: the update manifest for Modlight
//! App, and the endpoint that publishes it.
//!
//! GET is deliberately PUBLIC and unauthenticated: every installed copy of the
//! app polls it on startup, and an updater that needed a credential to discover
//! a new version would stop working the moment a token was rotated, which is
//! the one time you most want people to be on a current build.
//!
//! POST is for an administrator, by session (the panel) or by API token (a
//! release script). It only ever names an upload that already exists.
//!
//! # Security fix 4 (PORT.md 12)
//!
//! Upstream read the POST body with an unchecked `req.json()`: any content
//! type, any size, any JSON value. Here it goes through
//! [`read_json_object`] with [`JSON_BODY_MAX`] (16 KiB), the rules of every
//! other write endpoint, and only after the caller has been authenticated, as
//! upstream already ordered it. What changes for a caller that has passed the
//! admin check:
//!
//! | Body | Reference | Rust |
//! |---|---|---|
//! | valid JSON object, `Content-Type` not `application/json` (e.g. `text/plain`) | processed | 415 `Content-Type must be application/json.` |
//! | not JSON | 400 `Expected a JSON body.` | 400 `Body must be a JSON object.` |
//! | JSON `null` | 500 (TypeError reading `body.version`) | 400 `Body must be a JSON object.` |
//! | JSON array, string, number | 400 `version and code are required.` | 400 `Body must be a JSON object.` |
//! | more than 16 KiB (declared or read) | buffered whole, processed | 413 `Body must be at most 16384 bytes.` |
//!
//! Every refusal keeps this route's own `json()` shape: `{"message"}` with
//! `Cache-Control: no-store`. Everything else is 1:1, including the cache
//! headers, the 401/403 bodies and the `release.published` audit row.

use std::net::SocketAddr;

use axum::Router;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::get;
use serde::Serialize;
use serde_json::{Value, json};

use crate::api_json::{JSON_BODY_MAX, json_response, read_json_object};
use crate::api_token::{TokenAuth, TokenCapability, authenticate_api_token, bearer_from_request, token_refusal};
use crate::auth::session::AuthSession;
use crate::auth_helpers::{RequestSession, Sessions};
use crate::error::AppResult;
use crate::releases::{APP_NAME, PublishInput, PublishResult, ReleaseNotes, current_release, publish_release};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/releases/app", get(get_manifest).post(post_release))
}

/// The route's `json(body, status, cache)` helper: `application/json`, and a
/// `Cache-Control` on every answer.
fn json_answer<T: Serialize + ?Sized>(body: &T, status: StatusCode, cache: &str) -> Response {
    json_response(status, body, Some(cache))
}

async fn get_manifest(State(state): State<AppState>) -> AppResult<Response> {
    let Some(manifest) = current_release(&state.db, &state.config).await? else {
        // 404 rather than an empty 200: an updater checking for "is there a build"
        // should not have to inspect a body to find out.
        return Ok(json_answer(
            &json!({ "name": APP_NAME, "message": "No release published." }),
            StatusCode::NOT_FOUND,
            "no-store",
        ));
    };
    // Five minutes of shared caching. The manifest is public and identical for
    // everybody, and a release does not appear so urgently that a stale answer
    // for one poll interval costs anything.
    Ok(json_answer(&manifest, StatusCode::OK, "public, max-age=300"))
}

/// An administrator, by id and address (for the audit row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminCaller {
    pub id: String,
    pub email: String,
}

const NOT_ADMIN: &str = "Administrator credentials required.";

fn not_admin() -> (StatusCode, Value) {
    (StatusCode::UNAUTHORIZED, json!({ "message": NOT_ADMIN }))
}

/// What the admin check needs from the outside world (stubbed in tests).
pub trait AdminCallerDeps {
    fn authenticate_admin(&self, token: &str) -> impl Future<Output = anyhow::Result<TokenAuth>> + Send;
    fn session(&self) -> impl Future<Output = anyhow::Result<Option<AuthSession>>> + Send;
}

struct LiveDeps<'a> {
    state: &'a AppState,
    session: &'a RequestSession,
}

impl AdminCallerDeps for LiveDeps<'_> {
    async fn authenticate_admin(&self, token: &str) -> anyhow::Result<TokenAuth> {
        authenticate_api_token(&self.state.db, Some(token), TokenCapability::Admin).await
    }
    async fn session(&self) -> anyhow::Result<Option<AuthSession>> {
        Ok(self.session.get().await?)
    }
}

/// Admin, by API token first and session second (same order as /api/folders).
///
/// TWO gates, and they are not the same gate. `admin` is the token's SCOPE: a
/// release script needs a credential explicitly issued to publish, so an
/// administrator's ordinary `write` token (the one in their own desktop app)
/// cannot push a build to every installation just because of who owns it. The
/// role check that follows is the ACCOUNT's: a `full` token belonging to a
/// regular user grants everything that user has, and publishing releases is not
/// among it.
pub async fn require_admin_caller<D: AdminCallerDeps>(
    deps: &D,
    headers: &HeaderMap,
) -> anyhow::Result<Result<AdminCaller, (StatusCode, Value)>> {
    if let Some(token) = bearer_from_request(headers) {
        return Ok(match deps.authenticate_admin(&token).await? {
            TokenAuth::Ok { user, .. } if user.role.as_deref() == Some("admin") => {
                Ok(AdminCaller { id: user.id, email: user.email })
            }
            TokenAuth::Ok { .. } => Err(not_admin()),
            refused => Err(token_refusal(&refused, NOT_ADMIN).unwrap_or_else(not_admin)),
        });
    }
    Ok(match deps.session().await? {
        Some(session) if session.is_admin() => Ok(AdminCaller { id: session.user.id, email: session.user.email }),
        _ => Err(not_admin()),
    })
}

/// The refusal message for a publish that did not happen.
fn refusal_message(result: &PublishResult) -> String {
    match result {
        PublishResult::NotFound => "No such upload.".to_string(),
        PublishResult::NotPublic => {
            "That upload is not publicly downloadable (unpublished, password-protected or expiring).".to_string()
        }
        PublishResult::BadVersion => {
            "version must be Major.Minor[.Build[.Revision]] - that is all an installed client can parse.".to_string()
        }
        PublishResult::VersionMismatch { file_version, filename } => format!(
            "The file is named {filename}, which says version {file_version}. Fix the version, or repeat the request with allowVersionMismatch: true."
        ),
        PublishResult::NoVersion | PublishResult::Ok(_) => "version and code are required.".to_string(),
    }
}

async fn post_release(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, body) = req.into_parts();
    let admin = match require_admin_caller(&LiveDeps { state: &state, session: &session }, &parts.headers).await? {
        Ok(admin) => admin,
        Err((status, body)) => return Ok(json_answer(&body, status, "no-store")),
    };

    // Security fix 4: the shared body rules instead of `req.json()` (module docs).
    let body = match read_json_object(&parts.headers, body, JSON_BODY_MAX).await {
        Ok(body) => body,
        Err(e) => return Ok(json_answer(&json!({ "message": e.message }), e.status, "no-store")),
    };

    let (Some(version), Some(code)) =
        (body.get("version").and_then(Value::as_str), body.get("code").and_then(Value::as_str))
    else {
        return Ok(json_answer(
            &json!({ "message": "version and code are required." }),
            StatusCode::BAD_REQUEST,
            "no-store",
        ));
    };

    let notes = match body.get("notes") {
        Some(Value::String(notes)) => ReleaseNotes::Text(notes),
        _ => ReleaseNotes::Absent,
    };
    let result = publish_release(
        &state.db,
        &state.config,
        PublishInput {
            version,
            code,
            notes,
            // Strictly opt-in, and only the literal true. A build script that means
            // to override should have to say so; anything falsy keeps the guard.
            allow_version_mismatch: body.get("allowVersionMismatch") == Some(&Value::Bool(true)),
        },
    )
    .await?;

    let manifest = match result {
        PublishResult::Ok(manifest) => manifest,
        refused => {
            let status = if matches!(refused, PublishResult::NotFound) {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            return Ok(json_answer(
                &json!({ "message": refusal_message(&refused), "reason": refused.reason() }),
                status,
                "no-store",
            ));
        }
    };

    let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    let ip = crate::password::client_ip(&parts.headers, peer, state.config.trust_proxy);
    crate::audit::record_audit(
        &state.db,
        &admin.id,
        "release.published",
        json!({ "by": admin.id, "byEmail": admin.email, "version": manifest.version, "code": code }),
        &ip,
    )
    .await;

    Ok(json_answer(&manifest, StatusCode::OK, "no-store"))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use axum::http::HeaderValue;

    use super::*;
    use crate::api_token::TokenScope;
    use crate::api_token::test_support::user;
    use crate::db::models::{JsDate, Session};

    #[derive(Default)]
    struct Fake {
        token_answer: Mutex<Option<TokenAuth>>,
        session_answer: Mutex<Option<AuthSession>>,
        tokens: Mutex<Vec<String>>,
        session_calls: Mutex<u32>,
    }

    impl AdminCallerDeps for Fake {
        async fn authenticate_admin(&self, token: &str) -> anyhow::Result<TokenAuth> {
            self.tokens.lock().unwrap().push(token.to_string());
            Ok(self.token_answer.lock().unwrap().clone().unwrap_or(TokenAuth::Unknown))
        }
        async fn session(&self) -> anyhow::Result<Option<AuthSession>> {
            *self.session_calls.lock().unwrap() += 1;
            Ok(self.session_answer.lock().unwrap().clone())
        }
    }

    fn session_of(role: Option<&str>) -> AuthSession {
        AuthSession {
            session: Session {
                id: "s".into(),
                expires_at: JsDate::now(),
                token: "t".into(),
                created_at: JsDate::now(),
                updated_at: JsDate::now(),
                ip_address: None,
                user_agent: None,
                user_id: "u1".into(),
                impersonated_by: None,
            },
            user: user("u1", false, role),
        }
    }

    fn bearer(token: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("authorization", HeaderValue::from_str(&format!("Bearer {token}")).unwrap());
        h
    }

    #[tokio::test]
    async fn a_token_decides_and_the_session_is_never_consulted() {
        let f = Fake::default();
        *f.token_answer.lock().unwrap() =
            Some(TokenAuth::Ok { user: user("a1", false, Some("admin")), scope: TokenScope::Full });
        *f.session_answer.lock().unwrap() = Some(session_of(Some("admin")));
        let got = require_admin_caller(&f, &bearer("tok")).await.unwrap();
        assert_eq!(got, Ok(AdminCaller { id: "a1".into(), email: "a1@example.test".into() }));
        assert_eq!(*f.tokens.lock().unwrap(), vec!["tok".to_string()]);
        assert_eq!(*f.session_calls.lock().unwrap(), 0);

        // A refused token is a refusal, never a fall-through to the cookie.
        *f.token_answer.lock().unwrap() = Some(TokenAuth::Unknown);
        let got = require_admin_caller(&f, &bearer("revoked")).await.unwrap();
        assert_eq!(got, Err((StatusCode::UNAUTHORIZED, json!({ "message": NOT_ADMIN }))));
        assert_eq!(*f.session_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn the_scope_and_the_role_are_two_gates() {
        let f = Fake::default();
        // An administrator's `write` token: refused by scope (403, names `full`).
        *f.token_answer.lock().unwrap() = Some(TokenAuth::Scope {
            scope: TokenScope::Write,
            required: TokenCapability::Admin,
            user: user("a1", false, Some("admin")),
        });
        let Err((status, body)) = require_admin_caller(&f, &bearer("tok")).await.unwrap() else {
            panic!("expected a refusal")
        };
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["reason"], "insufficient_scope");
        assert_eq!(body["required_scope"], "full");
        // A regular user's `full` token: refused by role (401).
        *f.token_answer.lock().unwrap() =
            Some(TokenAuth::Ok { user: user("u2", false, Some("user")), scope: TokenScope::Full });
        assert_eq!(require_admin_caller(&f, &bearer("tok")).await.unwrap(), Err(not_admin()));
        // A banned owner and an unknown stored scope keep their own refusals.
        *f.token_answer.lock().unwrap() = Some(TokenAuth::Banned { user: user("a1", true, Some("admin")) });
        assert_eq!(
            require_admin_caller(&f, &bearer("tok")).await.unwrap(),
            Err((StatusCode::UNAUTHORIZED, json!({ "message": "Account disabled." })))
        );
        *f.token_answer.lock().unwrap() = Some(TokenAuth::InvalidScope);
        let Err((status, body)) = require_admin_caller(&f, &bearer("tok")).await.unwrap() else {
            panic!("expected a refusal")
        };
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["reason"], "invalid_scope");
    }

    #[tokio::test]
    async fn without_a_token_only_an_admin_session_passes() {
        let f = Fake::default();
        assert_eq!(require_admin_caller(&f, &HeaderMap::new()).await.unwrap(), Err(not_admin()));
        *f.session_answer.lock().unwrap() = Some(session_of(Some("user")));
        assert_eq!(require_admin_caller(&f, &HeaderMap::new()).await.unwrap(), Err(not_admin()));
        *f.session_answer.lock().unwrap() = Some(session_of(Some("admin")));
        assert_eq!(
            require_admin_caller(&f, &HeaderMap::new()).await.unwrap(),
            Ok(AdminCaller { id: "u1".into(), email: "u1@example.test".into() })
        );
        assert!(f.tokens.lock().unwrap().is_empty());
    }

    #[test]
    fn refusal_messages_match_upstream() {
        assert_eq!(refusal_message(&PublishResult::NotFound), "No such upload.");
        assert_eq!(refusal_message(&PublishResult::NoVersion), "version and code are required.");
        assert_eq!(
            refusal_message(&PublishResult::VersionMismatch {
                file_version: "0.7.0".into(),
                filename: "Modlight-0.7.0-setup-x64.exe".into()
            }),
            "The file is named Modlight-0.7.0-setup-x64.exe, which says version 0.7.0. Fix the version, or repeat the request with allowVersionMismatch: true."
        );
    }
}
