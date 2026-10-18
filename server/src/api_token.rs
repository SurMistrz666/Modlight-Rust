//! Port of `lib/api-token.ts`: per-user API tokens for ShareX, the desktop
//! app and the JSON API (stored as SHA-256 hex in `api_tokens.token_hash`).
//!
//! The database work is behind [`TokenDb`] so the logic can be tested with a
//! stub, the way the upstream unit tests mock `@/lib/db`; [`PgTokenDb`] is the
//! real one.

use axum::http::{HeaderMap, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

pub use crate::api_scopes::{
    LEGACY_TOKEN_SCOPE, TOKEN_SCOPES, TokenCapability, TokenScope, capabilities_of, is_narrowing, is_token_scope,
    narrower_scopes, parse_token_scope, scope_for, scope_grants,
};
use crate::db::models::{ApiToken, Columns, JsDate, User};

pub type UserRow = User;

/// `createHash("sha256").update(token).digest("hex")`
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Pull an API token out of request headers.
///
/// `x-api-key` wins, but only when it carries something: a header that is
/// present and EMPTY must fall through to `Authorization`, not swallow it.
pub fn bearer_from_request(headers: &HeaderMap) -> Option<String> {
    let header = |name: &str| {
        headers
            .get(name)
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .map(|v| crate::config::js_trim(&v).to_string())
    };
    if let Some(x_api) = header("x-api-key").filter(|v| !v.is_empty()) {
        return Some(x_api);
    }
    let auth = header("authorization").filter(|v| !v.is_empty())?;
    if let Some(prefix) = auth.get(..7)
        && prefix.eq_ignore_ascii_case("bearer ")
    {
        let token = crate::config::js_trim(&auth[7..]);
        return if token.is_empty() { None } else { Some(token.to_string()) };
    }
    None
}

/// The outcome of presenting a token at a surface that needs `required`.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenAuth {
    Ok {
        user: UserRow,
        scope: TokenScope,
    },
    /// No such token, or revoked, or its account is gone (401).
    Unknown,
    /// The account is switched off (401).
    Banned {
        user: UserRow,
    },
    /// A stored scope this build does not recognise (403, fail closed).
    InvalidScope,
    /// A real token that may not do this (403, names the scope needed).
    Scope {
        scope: TokenScope,
        required: TokenCapability,
        user: UserRow,
    },
}

/// The statements `authenticate_api_token` and `narrow_api_token_scope` issue.
pub trait TokenDb {
    /// `UPDATE api_tokens SET last_used_at = now() WHERE token_hash = $1 AND
    /// revoked_at IS NULL RETURNING user_id, scope`
    fn stamp_and_resolve(
        &self,
        token_hash: &str,
    ) -> impl Future<Output = anyhow::Result<Option<(String, String)>>> + Send;
    /// `SELECT * FROM "user" WHERE id = $1 LIMIT 1`
    fn load_user(&self, id: &str) -> impl Future<Output = anyhow::Result<Option<UserRow>>> + Send;
    /// `SELECT scope, revoked_at FROM api_tokens WHERE id = $1 AND user_id = $2 LIMIT 1`
    fn token_scope(
        &self,
        id: &str,
        user_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<(String, Option<JsDate>)>>> + Send;
    /// `UPDATE api_tokens SET scope = next WHERE id AND user_id AND scope = current
    /// AND revoked_at IS NULL RETURNING scope`
    fn set_scope_if(
        &self,
        id: &str,
        user_id: &str,
        current: &str,
        next: TokenScope,
    ) -> impl Future<Output = anyhow::Result<Option<String>>> + Send;
}

/// The real statements.
#[derive(Clone)]
pub struct PgTokenDb<'a>(pub &'a PgPool);

impl TokenDb for PgTokenDb<'_> {
    async fn stamp_and_resolve(&self, token_hash: &str) -> anyhow::Result<Option<(String, String)>> {
        Ok(sqlx::query_as::<_, (String, String)>(
            "UPDATE \"api_tokens\" SET \"last_used_at\" = $1 WHERE (\"api_tokens\".\"token_hash\" = $2 AND \"api_tokens\".\"revoked_at\" IS NULL) RETURNING \"user_id\", \"scope\"",
        )
        .bind(JsDate::now())
        .bind(token_hash)
        .fetch_optional(self.0)
        .await?)
    }

    async fn load_user(&self, id: &str) -> anyhow::Result<Option<UserRow>> {
        crate::auth::session::find_user(self.0, id).await
    }

    async fn token_scope(&self, id: &str, user_id: &str) -> anyhow::Result<Option<(String, Option<JsDate>)>> {
        Ok(sqlx::query_as::<_, (String, Option<JsDate>)>(
            "SELECT \"scope\", \"revoked_at\" FROM \"api_tokens\" WHERE (\"api_tokens\".\"id\" = $1::uuid AND \"api_tokens\".\"user_id\" = $2) LIMIT 1",
        )
        .bind(id)
        .bind(user_id)
        .fetch_optional(self.0)
        .await?)
    }

    async fn set_scope_if(
        &self,
        id: &str,
        user_id: &str,
        current: &str,
        next: TokenScope,
    ) -> anyhow::Result<Option<String>> {
        Ok(sqlx::query_scalar::<_, String>(
            "UPDATE \"api_tokens\" SET \"scope\" = $1 WHERE (\"api_tokens\".\"id\" = $2::uuid AND \"api_tokens\".\"user_id\" = $3 AND \"api_tokens\".\"scope\" = $4 AND \"api_tokens\".\"revoked_at\" IS NULL) RETURNING \"scope\"",
        )
        .bind(next.as_str())
        .bind(id)
        .bind(user_id)
        .bind(current)
        .fetch_optional(self.0)
        .await?)
    }
}

/// Verify a token, check it may do `required`, and return its owner, stamping
/// `last_used_at` in the same statement that resolves it (an earlier upstream
/// version built that UPDATE and never ran it). The stamp lands even when the
/// scope check then refuses. FAIL CLOSED on an unrecognised scope.
pub async fn authenticate_api_token_with<D: TokenDb>(
    db: &D,
    token: Option<&str>,
    required: TokenCapability,
) -> anyhow::Result<TokenAuth> {
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        return Ok(TokenAuth::Unknown);
    };
    let Some((user_id, stored_scope)) = db.stamp_and_resolve(&hash_token(token)).await? else {
        return Ok(TokenAuth::Unknown);
    };
    let Some(scope) = parse_token_scope(&stored_scope) else {
        return Ok(TokenAuth::InvalidScope);
    };
    let Some(owner) = db.load_user(&user_id).await? else {
        return Ok(TokenAuth::Unknown);
    };
    // Checked before the scope so a disabled account is told it is disabled.
    if owner.banned == Some(true) {
        return Ok(TokenAuth::Banned { user: owner });
    }
    if !scope_grants(scope, required) {
        return Ok(TokenAuth::Scope { scope, required, user: owner });
    }
    Ok(TokenAuth::Ok { user: owner, scope })
}

/// [`authenticate_api_token_with`] against the database.
pub async fn authenticate_api_token(
    db: &PgPool,
    token: Option<&str>,
    required: TokenCapability,
) -> anyhow::Result<TokenAuth> {
    authenticate_api_token_with(&PgTokenDb(db), token, required).await
}

/// The body of a scope refusal, identical on every surface.
pub fn scope_refusal_body(scope: TokenScope, required: TokenCapability) -> Value {
    let needed = scope_for(required);
    json!({
        "message": format!(
            "This API token is scoped to \"{scope}\" and this call needs \"{needed}\". Create a token with the \"{needed}\" scope (or wider) in Integrations."
        ),
        "reason": "insufficient_scope",
        "scope": scope.as_str(),
        "required_scope": needed.as_str(),
    })
}

/// The body of an unrecognised-scope refusal. Also a 403: the token is real.
pub fn invalid_scope_body() -> Value {
    json!({
        "message": "This API token carries a scope this server does not recognise and has been refused. Create a new token in Integrations.",
        "reason": "invalid_scope",
    })
}

/// Turn a refusal into the status and body it reaches the wire as.
/// `unknown_message` is the one line that differs per surface: `POST /upload`
/// answers "Token specified not found.", the JSON API "Token not specified or
/// not found.". Returns `None` for [`TokenAuth::Ok`].
pub fn token_refusal(auth: &TokenAuth, unknown_message: &str) -> Option<(StatusCode, Value)> {
    match auth {
        TokenAuth::Ok { .. } => None,
        TokenAuth::Scope { scope, required, .. } => {
            Some((StatusCode::FORBIDDEN, scope_refusal_body(*scope, *required)))
        }
        TokenAuth::InvalidScope => Some((StatusCode::FORBIDDEN, invalid_scope_body())),
        TokenAuth::Banned { .. } => Some((StatusCode::UNAUTHORIZED, json!({ "message": "Account disabled." }))),
        TokenAuth::Unknown => Some((StatusCode::UNAUTHORIZED, json!({ "message": unknown_message }))),
    }
}

/// Create a new API token. Returns the PLAINTEXT token (shown once). `scope`
/// has no default on purpose.
pub async fn create_api_token(db: &PgPool, user_id: &str, name: &str, scope: TokenScope) -> anyhow::Result<String> {
    let token = crate::ids::secure_token(24); // ~32 url-safe chars
    let preview: String = {
        let chars: Vec<char> = token.chars().collect();
        chars[chars.len().saturating_sub(6)..].iter().collect()
    };
    sqlx::query(
        "INSERT INTO \"api_tokens\" (\"user_id\", \"name\", \"token_hash\", \"preview\", \"scope\") VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(user_id)
    .bind(name)
    .bind(hash_token(&token))
    .bind(preview)
    .bind(scope.as_str())
    .execute(db)
    .await?;
    Ok(token)
}

/// List a user's tokens (hashes included, as upstream: callers decide what to show).
pub async fn list_api_tokens(db: &PgPool, user_id: &str) -> anyhow::Result<Vec<ApiToken>> {
    let sql = format!("SELECT {} FROM {} WHERE \"user_id\" = $1", ApiToken::COLUMNS, ApiToken::TABLE);
    Ok(sqlx::query_as::<_, ApiToken>(&sql).bind(user_id).fetch_all(db).await?)
}

pub async fn revoke_api_token(db: &PgPool, id: &str, user_id: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE \"api_tokens\" SET \"revoked_at\" = $1 WHERE (\"api_tokens\".\"id\" = $2::uuid AND \"api_tokens\".\"user_id\" = $3)")
        .bind(JsDate::now())
        .bind(id)
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(())
}

/// `NarrowResult`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NarrowResult {
    Ok(TokenScope),
    NotFound,
    Revoked,
    NotNarrower,
}

impl NarrowResult {
    /// The `reason` string of a refusal (`"not-found" | "revoked" | "not-narrower"`).
    pub fn reason(&self) -> Option<&'static str> {
        match self {
            NarrowResult::Ok(_) => None,
            NarrowResult::NotFound => Some("not-found"),
            NarrowResult::Revoked => Some("revoked"),
            NarrowResult::NotNarrower => Some("not-narrower"),
        }
    }
}

/// Narrow a live token's scope in place; widening is refused here, not only
/// in the UI. The current scope is pinned in the UPDATE's WHERE so two
/// concurrent narrowings cannot interleave into a widening.
pub async fn narrow_api_token_scope_with<D: TokenDb>(
    db: &D,
    id: &str,
    user_id: &str,
    next: TokenScope,
) -> anyhow::Result<NarrowResult> {
    let Some((stored, revoked_at)) = db.token_scope(id, user_id).await? else {
        return Ok(NarrowResult::NotFound);
    };
    if revoked_at.is_some() {
        return Ok(NarrowResult::Revoked);
    }
    let Some(current) = parse_token_scope(&stored) else {
        return Ok(NarrowResult::NotNarrower);
    };
    if !is_narrowing(current, next) {
        return Ok(NarrowResult::NotNarrower);
    }
    match db.set_scope_if(id, user_id, &stored, next).await? {
        Some(_) => Ok(NarrowResult::Ok(next)),
        None => Ok(NarrowResult::NotNarrower),
    }
}

pub async fn narrow_api_token_scope(
    db: &PgPool,
    id: &str,
    user_id: &str,
    next: TokenScope,
) -> anyhow::Result<NarrowResult> {
    narrow_api_token_scope_with(&PgTokenDb(db), id, user_id, next).await
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Mutex;

    use super::*;

    /// A user row for tests.
    pub fn user(id: &str, banned: bool, role: Option<&str>) -> UserRow {
        UserRow {
            id: id.into(),
            name: "Test".into(),
            email: format!("{id}@example.test"),
            email_verified: true,
            image: None,
            created_at: JsDate::from_millis(0).unwrap(),
            updated_at: JsDate::from_millis(0).unwrap(),
            role: role.map(str::to_string),
            banned: Some(banned),
            ban_reason: None,
            ban_expires: None,
            two_factor_enabled: Some(false),
            user_code: Some("tag".into()),
            naming_scheme: Some("random".into()),
            naming_pattern: None,
            public_profile: Some(false),
            used_quota: Some(0),
            max_quota: Some(-1),
            hide_uploads: Some(false),
            copy_raw: Some(false),
            theme: Some("discord".into()),
            locale: None,
            username: None,
            skip_tag_in_url: Some(false),
            allow_root_short_links: Some(false),
            default_folder_id: None,
            naming_word_count: Some(2),
            naming_random_length: Some(6),
            preferred_domain: None,
            default_password_hash: None,
            default_password_version: 0,
            background_key: None,
            background_dim: 70,
        }
    }

    /// The stub `@/lib/db` of tests/unit/api-token-*.test.ts, recording calls.
    #[derive(Default)]
    pub struct FakeDb {
        pub token: Mutex<Option<(String, String)>>,
        pub user: Mutex<Option<UserRow>>,
        pub row: Mutex<Option<(String, Option<JsDate>)>>,
        pub update_matched: Mutex<bool>,
        pub updates: Mutex<u32>,
        pub selects: Mutex<u32>,
        pub updated_to: Mutex<Option<TokenScope>>,
    }

    impl TokenDb for FakeDb {
        async fn stamp_and_resolve(&self, _hash: &str) -> anyhow::Result<Option<(String, String)>> {
            *self.updates.lock().unwrap() += 1;
            Ok(self.token.lock().unwrap().clone())
        }
        async fn load_user(&self, _id: &str) -> anyhow::Result<Option<UserRow>> {
            *self.selects.lock().unwrap() += 1;
            Ok(self.user.lock().unwrap().clone())
        }
        async fn token_scope(&self, _id: &str, _user_id: &str) -> anyhow::Result<Option<(String, Option<JsDate>)>> {
            Ok(self.row.lock().unwrap().clone())
        }
        async fn set_scope_if(
            &self,
            _id: &str,
            _uid: &str,
            _cur: &str,
            next: TokenScope,
        ) -> anyhow::Result<Option<String>> {
            *self.updated_to.lock().unwrap() = Some(next);
            Ok(if *self.update_matched.lock().unwrap() { Some(next.as_str().to_string()) } else { None })
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::test_support::{FakeDb, user};
    use super::*;
    use TokenCapability as C;

    fn have(db: &FakeDb, scope: &str, u: UserRow) {
        *db.token.lock().unwrap() = Some(("u1".into(), scope.into()));
        *db.user.lock().unwrap() = Some(u);
    }

    fn me() -> UserRow {
        user("u1", false, Some("user"))
    }

    // ── tests/unit/api-token-auth.test.ts ──

    #[tokio::test]
    async fn resolves_a_token_whose_scope_covers_the_call() {
        let db = FakeDb::default();
        have(&db, "write", me());
        assert_eq!(
            authenticate_api_token_with(&db, Some("tok"), C::Write).await.unwrap(),
            TokenAuth::Ok { user: me(), scope: TokenScope::Write }
        );
    }

    #[tokio::test]
    async fn refuses_a_token_whose_scope_does_not_and_says_which_would() {
        let db = FakeDb::default();
        have(&db, "read", me());
        let auth = authenticate_api_token_with(&db, Some("tok"), C::Write).await.unwrap();
        let TokenAuth::Scope { scope, required, .. } = auth else { panic!("expected a scope refusal") };
        assert_eq!((scope, required), (TokenScope::Read, C::Write));
        let body = scope_refusal_body(scope, required);
        assert_eq!(body["reason"], "insufficient_scope");
        assert_eq!(body["scope"], "read");
        assert_eq!(body["required_scope"], "write");
    }

    #[tokio::test]
    async fn still_stamps_last_used_at_when_it_then_refuses_on_scope() {
        let db = FakeDb::default();
        have(&db, "upload", me());
        authenticate_api_token_with(&db, Some("tok"), C::Read).await.unwrap();
        assert_eq!(*db.updates.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn fails_closed_on_a_scope_value_this_build_does_not_recognise() {
        for stored in ["superuser", "", "FULL", "7"] {
            let db = FakeDb::default();
            have(&db, stored, me());
            assert_eq!(
                authenticate_api_token_with(&db, Some("tok"), C::Identify).await.unwrap(),
                TokenAuth::InvalidScope
            );
        }
    }

    #[tokio::test]
    async fn refuses_a_banned_account_before_it_ever_looks_at_the_scope() {
        let db = FakeDb::default();
        have(&db, "upload", user("u1", true, Some("user")));
        assert!(matches!(
            authenticate_api_token_with(&db, Some("tok"), C::Write).await.unwrap(),
            TokenAuth::Banned { .. }
        ));
    }

    #[tokio::test]
    async fn refuses_an_unknown_or_revoked_token_without_reading_a_user_row() {
        let db = FakeDb::default();
        assert_eq!(authenticate_api_token_with(&db, Some("tok"), C::Identify).await.unwrap(), TokenAuth::Unknown);
        assert_eq!(*db.selects.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn refuses_a_token_whose_account_has_since_been_deleted() {
        let db = FakeDb::default();
        *db.token.lock().unwrap() = Some(("gone".into(), "full".into()));
        assert_eq!(authenticate_api_token_with(&db, Some("tok"), C::Read).await.unwrap(), TokenAuth::Unknown);
    }

    #[tokio::test]
    async fn asks_the_database_nothing_when_no_token_was_offered() {
        let db = FakeDb::default();
        assert_eq!(authenticate_api_token_with(&db, None, C::Read).await.unwrap(), TokenAuth::Unknown);
        assert_eq!(authenticate_api_token_with(&db, Some(""), C::Read).await.unwrap(), TokenAuth::Unknown);
        assert_eq!(*db.updates.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn answers_a_scope_refusal_with_403_not_a_bare_401() {
        let db = FakeDb::default();
        have(&db, "read", me());
        let auth = authenticate_api_token_with(&db, Some("tok"), C::Write).await.unwrap();
        let (status, body) = token_refusal(&auth, "Token specified not found.").unwrap();
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body["message"].as_str().unwrap().contains("\"write\""));
        assert_eq!(body["reason"], "insufficient_scope");
    }

    #[tokio::test]
    async fn answers_an_unrecognised_scope_with_403_too() {
        let db = FakeDb::default();
        have(&db, "superuser", me());
        let auth = authenticate_api_token_with(&db, Some("tok"), C::Read).await.unwrap();
        let (status, body) = token_refusal(&auth, "x").unwrap();
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["reason"], "invalid_scope");
    }

    #[tokio::test]
    async fn keeps_each_surfaces_own_wording_for_an_unknown_token() {
        let db = FakeDb::default();
        let auth = authenticate_api_token_with(&db, Some("tok"), C::Read).await.unwrap();
        assert_eq!(
            token_refusal(&auth, "Token specified not found.").unwrap(),
            (StatusCode::UNAUTHORIZED, json!({ "message": "Token specified not found." }))
        );
        assert_eq!(
            token_refusal(&auth, "Token not specified or not found.").unwrap().1,
            json!({ "message": "Token not specified or not found." })
        );
    }

    #[tokio::test]
    async fn answers_a_banned_account_with_401() {
        let db = FakeDb::default();
        have(&db, "full", user("u1", true, Some("user")));
        let auth = authenticate_api_token_with(&db, Some("tok"), C::Read).await.unwrap();
        assert_eq!(
            token_refusal(&auth, "x").unwrap(),
            (StatusCode::UNAUTHORIZED, json!({ "message": "Account disabled." }))
        );
    }

    // ── tests/unit/api-token-narrow.test.ts ──

    fn stored(db: &FakeDb, scope: &str, revoked: bool) {
        *db.row.lock().unwrap() = Some((scope.into(), if revoked { Some(JsDate::now()) } else { None }));
        *db.update_matched.lock().unwrap() = true;
    }

    #[tokio::test]
    async fn narrows_a_full_token_to_write() {
        let db = FakeDb::default();
        stored(&db, "full", false);
        assert_eq!(
            narrow_api_token_scope_with(&db, "t1", "u1", TokenScope::Write).await.unwrap(),
            NarrowResult::Ok(TokenScope::Write)
        );
        assert_eq!(*db.updated_to.lock().unwrap(), Some(TokenScope::Write));
    }

    #[tokio::test]
    async fn refuses_every_widening_and_writes_nothing() {
        use TokenScope::*;
        for (from, to) in [("read", Write), ("read", Full), ("upload", Write), ("upload", Full), ("write", Full)] {
            let db = FakeDb::default();
            stored(&db, from, false);
            assert_eq!(narrow_api_token_scope_with(&db, "t1", "u1", to).await.unwrap(), NarrowResult::NotNarrower);
            assert_eq!(*db.updated_to.lock().unwrap(), None);
        }
    }

    #[tokio::test]
    async fn refuses_a_sideways_move_between_incomparable_scopes() {
        let db = FakeDb::default();
        stored(&db, "upload", false);
        assert_ne!(narrow_api_token_scope_with(&db, "t1", "u1", TokenScope::Read).await.unwrap().reason(), None);
        stored(&db, "read", false);
        assert_ne!(narrow_api_token_scope_with(&db, "t1", "u1", TokenScope::Upload).await.unwrap().reason(), None);
        assert_eq!(*db.updated_to.lock().unwrap(), None);
    }

    #[tokio::test]
    async fn refuses_a_no_op() {
        let db = FakeDb::default();
        stored(&db, "write", false);
        assert_ne!(narrow_api_token_scope_with(&db, "t1", "u1", TokenScope::Write).await.unwrap().reason(), None);
    }

    #[tokio::test]
    async fn refuses_a_token_that_is_not_the_callers_or_does_not_exist() {
        let db = FakeDb::default();
        assert_eq!(
            narrow_api_token_scope_with(&db, "t1", "u1", TokenScope::Read).await.unwrap(),
            NarrowResult::NotFound
        );
    }

    #[tokio::test]
    async fn refuses_a_revoked_token() {
        let db = FakeDb::default();
        stored(&db, "full", true);
        assert_eq!(
            narrow_api_token_scope_with(&db, "t1", "u1", TokenScope::Read).await.unwrap(),
            NarrowResult::Revoked
        );
        assert_eq!(*db.updated_to.lock().unwrap(), None);
    }

    #[tokio::test]
    async fn refuses_a_stored_scope_this_build_cannot_compare() {
        let db = FakeDb::default();
        stored(&db, "superuser", false);
        assert_eq!(
            narrow_api_token_scope_with(&db, "t1", "u1", TokenScope::Read).await.unwrap(),
            NarrowResult::NotNarrower
        );
        assert_eq!(*db.updated_to.lock().unwrap(), None);
    }

    #[tokio::test]
    async fn reports_a_lost_race_rather_than_claiming_success() {
        let db = FakeDb::default();
        stored(&db, "full", false);
        *db.update_matched.lock().unwrap() = false;
        assert_eq!(
            narrow_api_token_scope_with(&db, "t1", "u1", TokenScope::Read).await.unwrap(),
            NarrowResult::NotNarrower
        );
    }

    // ── tests/unit/bearer-from-request.test.ts ──

    fn h(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut m = HeaderMap::new();
        for (k, v) in pairs {
            m.insert(axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
        }
        m
    }

    #[test]
    fn takes_x_api_key_when_it_carries_a_value() {
        assert_eq!(bearer_from_request(&h(&[("x-api-key", "tok_abc")])).as_deref(), Some("tok_abc"));
    }

    #[test]
    fn falls_through_to_authorization_when_x_api_key_is_present_but_empty() {
        assert_eq!(
            bearer_from_request(&h(&[("x-api-key", ""), ("authorization", "Bearer tok_xyz")])).as_deref(),
            Some("tok_xyz")
        );
    }

    #[test]
    fn falls_through_when_x_api_key_is_only_whitespace() {
        assert_eq!(
            bearer_from_request(&h(&[("x-api-key", "   "), ("authorization", "Bearer tok_xyz")])).as_deref(),
            Some("tok_xyz")
        );
    }

    #[test]
    fn accepts_any_capitalisation_of_the_bearer_scheme() {
        assert_eq!(bearer_from_request(&h(&[("authorization", "bearer tok_1")])).as_deref(), Some("tok_1"));
        assert_eq!(bearer_from_request(&h(&[("authorization", "BEARER tok_2")])).as_deref(), Some("tok_2"));
    }

    #[test]
    fn trims_surrounding_whitespace_off_the_token() {
        assert_eq!(bearer_from_request(&h(&[("x-api-key", "  tok_pad  ")])).as_deref(), Some("tok_pad"));
        assert_eq!(bearer_from_request(&h(&[("authorization", "Bearer   tok_pad  ")])).as_deref(), Some("tok_pad"));
    }

    #[test]
    fn returns_none_when_there_is_nothing_usable() {
        assert_eq!(bearer_from_request(&h(&[])), None);
        assert_eq!(bearer_from_request(&h(&[("x-api-key", "")])), None);
        assert_eq!(bearer_from_request(&h(&[("authorization", "Bearer")])), None);
        assert_eq!(bearer_from_request(&h(&[("authorization", "Bearer    ")])), None);
        assert_eq!(bearer_from_request(&h(&[("authorization", "Basic dXNlcjpwYXNz")])), None);
    }

    #[test]
    fn hashes_like_node() {
        // printf tok | shasum -a 256
        assert_eq!(hash_token("tok"), "1a7674eb4ee78df7e1ac439a93c3fa8e3c945784d4dec9fd8e3011738b2f1d62");
    }
}
