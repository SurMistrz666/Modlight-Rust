//! Port of `lib/api-caller.ts`: who is calling a token-or-session API route.
//!
//! The order and, more importantly, the EXCLUSIVITY are the whole point: if a
//! token was offered, the token decides; only a request that offered none
//! falls back to the session. A present-but-wrong token is a refusal, never a
//! fall-through to whatever cookie the same request carried.
//!
//! A SESSION is not scoped (a signed-in browser is the account itself), so it
//! satisfies every capability and reports `scope: None`.

use axum::http::{HeaderMap, StatusCode};
use serde_json::{Value, json};

use crate::api_json::json_response;
use crate::api_token::{
    TokenAuth, TokenCapability, TokenScope, UserRow, authenticate_api_token, bearer_from_request, token_refusal,
};
use crate::auth::session::{AuthSession, find_user};
use crate::auth_helpers::RequestSession;
use crate::error::AppError;
use crate::state::AppState;

/// The lightweight caller: id and role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiCaller {
    pub id: String,
    /// "admin" for an administrator; whatever Better Auth stored otherwise.
    pub role: Option<String>,
}

/// The result of authenticating a route call. A refusal carries the status
/// and the body, which every route still renders through its own `json()` so
/// per-route cache headers are untouched.
#[derive(Debug, Clone, PartialEq)]
pub enum ApiAuth<T> {
    Ok { caller: T, scope: Option<TokenScope> },
    Refused { status: StatusCode, body: Value },
}

impl<T> ApiAuth<T> {
    /// The caller, or the refusal rendered with `cache_control`
    /// (`api_json::json_response`), for `let (who, scope) = auth.or_respond(...)?`
    /// in a handler returning [`crate::error::AppResult`].
    pub fn or_respond(self, cache_control: Option<&str>) -> Result<(T, Option<TokenScope>), AppError> {
        match self {
            ApiAuth::Ok { caller, scope } => Ok((caller, scope)),
            ApiAuth::Refused { status, body } => {
                Err(AppError::Response(Box::new(json_response(status, &body, cache_control))))
            }
        }
    }
}

/// The 401 every token surface has always answered with. Unchanged wording.
pub const UNAUTHORIZED_MESSAGE: &str = "Token not specified or not found.";

fn unauthorized<T>() -> ApiAuth<T> {
    ApiAuth::Refused { status: StatusCode::UNAUTHORIZED, body: json!({ "message": UNAUTHORIZED_MESSAGE }) }
}

fn refusal_for<T>(auth: &TokenAuth) -> ApiAuth<T> {
    match token_refusal(auth, UNAUTHORIZED_MESSAGE) {
        Some((status, body)) => ApiAuth::Refused { status, body },
        None => unauthorized(),
    }
}

/// What the resolution needs from the outside world (stubbed in tests, as the
/// upstream test mocks `authenticateApiToken` and `getSession`).
pub trait CallerDeps {
    fn authenticate(
        &self,
        token: &str,
        required: TokenCapability,
    ) -> impl Future<Output = anyhow::Result<TokenAuth>> + Send;
    fn session(&self) -> impl Future<Output = anyhow::Result<Option<AuthSession>>> + Send;
    fn load_user(&self, id: &str) -> impl Future<Output = anyhow::Result<Option<UserRow>>> + Send;
}

/// The real dependencies: the database and the request's session.
pub struct LiveCallerDeps<'a> {
    pub state: &'a AppState,
    pub session: &'a RequestSession,
}

impl CallerDeps for LiveCallerDeps<'_> {
    async fn authenticate(&self, token: &str, required: TokenCapability) -> anyhow::Result<TokenAuth> {
        authenticate_api_token(&self.state.db, Some(token), required).await
    }
    async fn session(&self) -> anyhow::Result<Option<AuthSession>> {
        Ok(self.session.get().await?)
    }
    async fn load_user(&self, id: &str) -> anyhow::Result<Option<UserRow>> {
        find_user(&self.state.db, id).await
    }
}

pub async fn resolve_api_caller_with<D: CallerDeps>(
    deps: &D,
    headers: &HeaderMap,
    required: TokenCapability,
) -> anyhow::Result<ApiAuth<ApiCaller>> {
    if let Some(token) = bearer_from_request(headers) {
        let auth = deps.authenticate(&token, required).await?;
        return Ok(match auth {
            TokenAuth::Ok { user, scope } => {
                ApiAuth::Ok { caller: ApiCaller { id: user.id, role: user.role }, scope: Some(scope) }
            }
            other => refusal_for(&other),
        });
    }
    match deps.session().await? {
        None => Ok(unauthorized()),
        Some(s) => Ok(ApiAuth::Ok { caller: ApiCaller { id: s.user.id, role: s.user.role }, scope: None }),
    }
}

/// The same decision resolved to the WHOLE user row (routes that build links
/// need the tag, preferred domain, `copy_raw`...). A banned account with a
/// live session is refused here too.
pub async fn resolve_api_owner_with<D: CallerDeps>(
    deps: &D,
    headers: &HeaderMap,
    required: TokenCapability,
) -> anyhow::Result<ApiAuth<UserRow>> {
    if let Some(token) = bearer_from_request(headers) {
        let auth = deps.authenticate(&token, required).await?;
        return Ok(match auth {
            TokenAuth::Ok { user, scope } => ApiAuth::Ok { caller: user, scope: Some(scope) },
            other => refusal_for(&other),
        });
    }
    let Some(session) = deps.session().await? else {
        return Ok(unauthorized());
    };
    match deps.load_user(&session.user.id).await? {
        Some(row) if row.banned != Some(true) => Ok(ApiAuth::Ok { caller: row, scope: None }),
        _ => Ok(unauthorized()),
    }
}

pub async fn resolve_api_caller(
    state: &AppState,
    session: &RequestSession,
    headers: &HeaderMap,
    required: TokenCapability,
) -> anyhow::Result<ApiAuth<ApiCaller>> {
    resolve_api_caller_with(&LiveCallerDeps { state, session }, headers, required).await
}

pub async fn resolve_api_owner(
    state: &AppState,
    session: &RequestSession,
    headers: &HeaderMap,
    required: TokenCapability,
) -> anyhow::Result<ApiAuth<UserRow>> {
    resolve_api_owner_with(&LiveCallerDeps { state, session }, headers, required).await
}

#[cfg(test)]
mod tests {
    //! tests/unit/api-caller.test.ts
    use std::sync::Mutex;

    use axum::http::HeaderValue;

    use super::*;
    use crate::api_token::test_support::user;
    use crate::db::models::{JsDate, Session};
    use TokenCapability as C;

    #[derive(Default)]
    struct Fake {
        token_answer: Mutex<Option<TokenAuth>>,
        session_answer: Mutex<Option<AuthSession>>,
        authenticate_calls: Mutex<Vec<(String, TokenCapability)>>,
        session_calls: Mutex<u32>,
    }

    impl CallerDeps for Fake {
        async fn authenticate(&self, token: &str, required: TokenCapability) -> anyhow::Result<TokenAuth> {
            self.authenticate_calls.lock().unwrap().push((token.to_string(), required));
            Ok(self.token_answer.lock().unwrap().clone().unwrap_or(TokenAuth::Unknown))
        }
        async fn session(&self) -> anyhow::Result<Option<AuthSession>> {
            *self.session_calls.lock().unwrap() += 1;
            Ok(self.session_answer.lock().unwrap().clone())
        }
        async fn load_user(&self, id: &str) -> anyhow::Result<Option<UserRow>> {
            Ok(Some(user(id, false, None)))
        }
    }

    fn session_of(id: &str, role: Option<&str>) -> AuthSession {
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

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[tokio::test]
    async fn resolves_an_account_from_a_valid_token_without_touching_the_session() {
        let f = Fake::default();
        *f.token_answer.lock().unwrap() =
            Some(TokenAuth::Ok { user: user("u1", false, Some("user")), scope: TokenScope::Full });
        let got = resolve_api_caller_with(&f, &headers(&[("x-api-key", "tok")]), C::Read).await.unwrap();
        assert_eq!(
            got,
            ApiAuth::Ok {
                caller: ApiCaller { id: "u1".into(), role: Some("user".into()) },
                scope: Some(TokenScope::Full)
            }
        );
        assert_eq!(*f.authenticate_calls.lock().unwrap(), vec![("tok".to_string(), C::Read)]);
        assert_eq!(*f.session_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn refuses_an_invalid_token_instead_of_falling_back_to_the_session() {
        let f = Fake::default();
        *f.token_answer.lock().unwrap() = Some(TokenAuth::Unknown);
        *f.session_answer.lock().unwrap() = Some(session_of("u2", Some("admin")));
        for h in [headers(&[("x-api-key", "revoked")]), headers(&[("authorization", "Bearer revoked")])] {
            let got = resolve_api_caller_with(&f, &h, C::Read).await.unwrap();
            assert!(matches!(got, ApiAuth::Refused { status: StatusCode::UNAUTHORIZED, .. }));
        }
        assert_eq!(*f.session_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn refuses_a_banned_account_holding_a_working_token() {
        let f = Fake::default();
        *f.token_answer.lock().unwrap() = Some(TokenAuth::Banned { user: user("u3", true, Some("user")) });
        let got = resolve_api_caller_with(&f, &headers(&[("x-api-key", "tok")]), C::Read).await.unwrap();
        assert!(matches!(got, ApiAuth::Refused { status: StatusCode::UNAUTHORIZED, .. }));
    }

    #[tokio::test]
    async fn answers_a_scope_refusal_with_403_and_the_scope_the_call_needed() {
        let f = Fake::default();
        *f.token_answer.lock().unwrap() = Some(TokenAuth::Scope {
            scope: TokenScope::Read,
            required: C::Write,
            user: user("u6", false, Some("user")),
        });
        let got = resolve_api_caller_with(&f, &headers(&[("x-api-key", "tok")]), C::Write).await.unwrap();
        let ApiAuth::Refused { status, body } = got else { panic!("expected a refusal") };
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["reason"], "insufficient_scope");
        assert_eq!(body["scope"], "read");
        assert_eq!(body["required_scope"], "write");
        assert!(body["message"].as_str().unwrap().contains("write"));
    }

    #[tokio::test]
    async fn refuses_a_token_whose_stored_scope_this_build_does_not_recognise() {
        let f = Fake::default();
        *f.token_answer.lock().unwrap() = Some(TokenAuth::InvalidScope);
        let got = resolve_api_caller_with(&f, &headers(&[("x-api-key", "tok")]), C::Read).await.unwrap();
        assert!(matches!(got, ApiAuth::Refused { status: StatusCode::FORBIDDEN, .. }));
    }

    #[tokio::test]
    async fn falls_back_to_the_session_only_when_no_token_was_offered() {
        let f = Fake::default();
        *f.session_answer.lock().unwrap() = Some(session_of("u4", Some("admin")));
        let got = resolve_api_caller_with(&f, &headers(&[]), C::Write).await.unwrap();
        assert_eq!(got, ApiAuth::Ok { caller: ApiCaller { id: "u4".into(), role: Some("admin".into()) }, scope: None });
        assert!(f.authenticate_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn treats_a_present_but_empty_x_api_key_as_no_token_offered() {
        let f = Fake::default();
        *f.session_answer.lock().unwrap() = Some(session_of("u5", None));
        let got = resolve_api_caller_with(&f, &headers(&[("x-api-key", "")]), C::Read).await.unwrap();
        assert_eq!(got, ApiAuth::Ok { caller: ApiCaller { id: "u5".into(), role: None }, scope: None });
        assert!(f.authenticate_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn refuses_when_there_is_neither_a_token_nor_a_session() {
        let f = Fake::default();
        let got = resolve_api_caller_with(&f, &headers(&[]), C::Read).await.unwrap();
        assert!(matches!(got, ApiAuth::Refused { status: StatusCode::UNAUTHORIZED, .. }));
    }

    #[tokio::test]
    async fn owner_resolution_refuses_a_banned_session() {
        struct Banned;
        impl CallerDeps for Banned {
            async fn authenticate(&self, _: &str, _: TokenCapability) -> anyhow::Result<TokenAuth> {
                unreachable!()
            }
            async fn session(&self) -> anyhow::Result<Option<AuthSession>> {
                Ok(Some(session_of("u7", None)))
            }
            async fn load_user(&self, id: &str) -> anyhow::Result<Option<UserRow>> {
                Ok(Some(user(id, true, None)))
            }
        }
        let got = resolve_api_owner_with(&Banned, &headers(&[]), C::Read).await.unwrap();
        assert!(matches!(got, ApiAuth::Refused { status: StatusCode::UNAUTHORIZED, .. }));
        let ok = resolve_api_owner_with(
            &Fake { session_answer: Mutex::new(Some(session_of("u8", None))), ..Default::default() },
            &headers(&[]),
            C::Read,
        )
        .await
        .unwrap();
        assert!(matches!(ok, ApiAuth::Ok { scope: None, .. }));
    }
}
