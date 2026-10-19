//! Port of `lib/auth-helpers.ts`: `getSession`, `requireUser`,
//! `requireAdmin`, `headersWithoutSession`, as plain functions on a
//! per-request [`RequestSession`] and as axum extractors for route handlers.
//!
//! Upstream re-ran Better Auth's `getSession` on every call within a request.
//! Here the first answer is memoised per request: the answer cannot differ
//! (the first call already performed any refresh), and the cookies it set are
//! the same ones a second call would set again.
//!
//! Redirects match upstream: `requireUser` sends a signed-out caller to
//! `/login` (no `next`), `requireAdmin` sends a non-admin to `/dashboard`.

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tokio::sync::OnceCell;

use crate::auth::session::{AuthSession, FailedToGetSession, RefreshPolicy, get_session};
use crate::http::set_cookie::ResponseCookies;
use crate::state::AppState;

/// Why `require_user` / `require_admin` did not return a session.
#[derive(Debug)]
pub enum AuthRefusal {
    /// `redirect(path)`: "/login" or "/dashboard".
    Redirect(&'static str),
    /// `getSession()` threw (database down, garbage cache cookie).
    Failed(FailedToGetSession),
}

impl IntoResponse for AuthRefusal {
    /// How a route handler answers it: Next turns `redirect()` in a route
    /// handler into a bodiless 307 with `Location`.
    fn into_response(self) -> Response {
        match self {
            AuthRefusal::Redirect(to) => redirect_307(to),
            AuthRefusal::Failed(err) => crate::error::AppError::Internal(err.into()).into_response(),
        }
    }
}

/// A bodiless 307 to `location`.
pub fn redirect_307(location: &str) -> Response {
    let mut res = StatusCode::TEMPORARY_REDIRECT.into_response();
    if let Ok(v) = HeaderValue::from_str(location) {
        res.headers_mut().insert(header::LOCATION, v);
    }
    res
}

/// The session of one request, read at most once.
#[derive(Clone)]
pub struct RequestSession {
    state: AppState,
    headers: Arc<HeaderMap>,
    policy: RefreshPolicy,
    cookies: ResponseCookies,
    cell: Arc<OnceCell<Option<AuthSession>>>,
}

impl RequestSession {
    pub fn new(state: AppState, headers: Arc<HeaderMap>, policy: RefreshPolicy, cookies: ResponseCookies) -> Self {
        RequestSession { state, headers, policy, cookies, cell: Arc::new(OnceCell::new()) }
    }

    pub fn policy(&self) -> RefreshPolicy {
        self.policy
    }

    /// `getSession()`: the session or `None`; cookies Better Auth set while
    /// answering go to the response (when the policy allows it).
    pub async fn get(&self) -> Result<Option<AuthSession>, FailedToGetSession> {
        let session = self
            .cell
            .get_or_try_init(|| async {
                let lookup = get_session(&self.state.db, &self.state.auth_cookies, &self.headers, self.policy).await?;
                for cookie in lookup.cookies {
                    self.cookies.push(cookie);
                }
                Ok::<_, FailedToGetSession>(lookup.session)
            })
            .await?;
        Ok(session.clone())
    }

    /// `getSession().catch(() => null)`, as the root layout reads it.
    pub async fn get_or_none(&self) -> Option<AuthSession> {
        match self.get().await {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!(error = ?err, "session lookup failed");
                None
            }
        }
    }

    /// `requireUser()`: a signed-in user, else `redirect("/login")`.
    pub async fn require_user(&self) -> Result<AuthSession, AuthRefusal> {
        match self.get().await {
            Ok(Some(session)) => Ok(session),
            Ok(None) => Err(AuthRefusal::Redirect("/login")),
            Err(err) => Err(AuthRefusal::Failed(err)),
        }
    }

    /// `requireAdmin()`: an administrator, else `redirect("/dashboard")`.
    pub async fn require_admin(&self) -> Result<AuthSession, AuthRefusal> {
        let session = self.require_user().await?;
        if !session.is_admin() {
            return Err(AuthRefusal::Redirect("/dashboard"));
        }
        Ok(session)
    }
}

/// `headersWithoutSession()`: the request headers minus anything that asserts
/// an identity (`cookie`, `authorization`), for calling Better Auth on
/// somebody else's behalf. Origin, host and user-agent are kept.
pub fn headers_without_session(headers: &HeaderMap) -> HeaderMap {
    let mut copy = headers.clone();
    copy.remove(header::COOKIE);
    copy.remove(header::AUTHORIZATION);
    copy
}

// ── Extractors for route handlers (refresh + cookie re-issue) ───────────────

/// The request's [`RequestSession`] with the route-handler policy
/// (`RefreshPolicy::DbAndCookie`). Shared by every extractor of one request.
pub struct Sessions(pub RequestSession);

impl FromRequestParts<AppState> for Sessions {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        if let Some(existing) = parts.extensions.get::<RequestSession>() {
            return Ok(Sessions(existing.clone()));
        }
        let cookies = parts.extensions.get::<ResponseCookies>().cloned().unwrap_or_default();
        let session =
            RequestSession::new(state.clone(), Arc::new(parts.headers.clone()), RefreshPolicy::DbAndCookie, cookies);
        parts.extensions.insert(session.clone());
        Ok(Sessions(session))
    }
}

/// `await getSession()` in a route handler. A lookup failure is a 500.
pub struct MaybeUser(pub Option<AuthSession>);

impl FromRequestParts<AppState> for MaybeUser {
    type Rejection = AuthRefusal;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let Sessions(s) = Sessions::from_request_parts(parts, state).await.unwrap_or_else(|e| match e {});
        s.get().await.map(MaybeUser).map_err(AuthRefusal::Failed)
    }
}

/// `await requireUser()` in a route handler.
pub struct RequireUser(pub AuthSession);

impl FromRequestParts<AppState> for RequireUser {
    type Rejection = AuthRefusal;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let Sessions(s) = Sessions::from_request_parts(parts, state).await.unwrap_or_else(|e| match e {});
        s.require_user().await.map(RequireUser)
    }
}

/// `await requireAdmin()` in a route handler.
pub struct RequireAdmin(pub AuthSession);

impl FromRequestParts<AppState> for RequireAdmin {
    type Rejection = AuthRefusal;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let Sessions(s) = Sessions::from_request_parts(parts, state).await.unwrap_or_else(|e| match e {});
        s.require_admin().await.map(RequireAdmin)
    }
}
