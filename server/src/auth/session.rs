//! Reading a session exactly like Better Auth 1.6.13's `getSession` endpoint
//! (`api/routes/session.mjs`) with the options lib/auth.ts uses: no cookie
//! cache, no secondary storage, `expiresIn` 7 days, `updateAge` 1 day, and the
//! `nextCookies()` plugin last in the list.
//!
//! The steps, in upstream order:
//!
//! 1. The signed `session_token` cookie is verified with AUTH_SECRET. Missing,
//!    malformed or wrongly signed: no session, no cookie changes.
//! 2. A `session_data` cookie (cookie cache) is still parsed when present even
//!    though the cache is off: an undecodable one is an error, a wrongly signed
//!    one expires that cookie and answers "no session".
//! 3. The session row is looked up by token with its user. Missing, or
//!    `expires_at` in the past: the session cookies are expired, an expired row
//!    is deleted, no session.
//! 4. A signed `dont_remember` cookie disables the refresh.
//! 5. Refresh when `expires_at - expiresIn + updateAge <= now`: the row gets
//!    `expires_at = now + 7d`, `updated_at = now`, and the cookie is re-issued
//!    with `Max-Age` = the remaining seconds.
//!
//! WHERE a refreshed cookie actually reaches the browser depends on the Next
//! context upstream ran in, which is what [`RefreshPolicy`] encodes:
//! - route handlers and server actions: the row is refreshed AND the cookie is
//!   re-issued (`nextCookies()` copies it into the response);
//! - a page render: the row is refreshed, but the cookie write fails silently
//!   (`cookies().set()` throws inside a Server Component);
//! - a client-side navigation (RSC request): the refresh is skipped entirely
//!   (`setShouldSkipSessionRefresh`);
//! - the raw upload paths of server.ts: refreshed, no cookie (no request scope).
//!
//! Banned accounts are NOT refused here, exactly as upstream: the admin
//! plugin deletes an account's sessions when it bans it and refuses to create
//! new ones ([`check_ban_before_session_create`]). Callers that must refuse a
//! banned user with a live session (lib/api-caller.ts `resolveApiOwner`) check
//! `user.banned` themselves.

use anyhow::Context;
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;

use super::cookies::{
    AuthCookies, SESSION_EXPIRES_IN, SESSION_UPDATE_AGE, SignedCookie, better_auth_base64_decode,
    hmac_base64url_verify, signed_value, verify_signed_cookie,
};
use crate::db::models::{Columns, JsDate, Session, User};
use crate::http::cookies::{parse_cookies_better_auth, parse_cookies_better_call};
use crate::http::set_cookie::SetCookie;

/// How a session read may write (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshPolicy {
    /// Client-side navigation (`RSC: 1`): never refresh.
    Skip,
    /// Page render or raw upload path: refresh the row, never set cookies.
    DbOnly,
    /// Route handler or server action: refresh and re-issue the cookie.
    DbAndCookie,
}

impl RefreshPolicy {
    fn refreshes(self) -> bool {
        self != RefreshPolicy::Skip
    }
    fn sets_cookies(self) -> bool {
        self == RefreshPolicy::DbAndCookie
    }
}

/// A live session and its account: the full rows.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthSession {
    pub session: Session,
    pub user: User,
}

impl AuthSession {
    /// `session.user.role === "admin"`
    pub fn is_admin(&self) -> bool {
        self.user.role.as_deref() == Some("admin")
    }

    /// What `auth.api.getSession()` returned upstream, field for field and in
    /// Better Auth's order (core fields, plugin fields, additionalFields, id).
    pub fn output(&self) -> SessionOutput {
        SessionOutput { session: SessionOut::from(&self.session), user: UserOut::from(&self.user) }
    }
}

/// `{ session, user }` as Better Auth serializes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionOutput {
    pub session: SessionOut,
    pub user: UserOut,
}

/// `parseSessionOutput(session)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOut {
    pub expires_at: JsDate,
    pub token: String,
    pub created_at: JsDate,
    pub updated_at: JsDate,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub user_id: String,
    pub impersonated_by: Option<String>,
    pub id: String,
}

impl From<&Session> for SessionOut {
    fn from(s: &Session) -> Self {
        SessionOut {
            expires_at: s.expires_at,
            token: s.token.clone(),
            created_at: s.created_at,
            updated_at: s.updated_at,
            ip_address: s.ip_address.clone(),
            user_agent: s.user_agent.clone(),
            user_id: s.user_id.clone(),
            impersonated_by: s.impersonated_by.clone(),
            id: s.id.clone(),
        }
    }
}

/// `parseUserOutput(user)`: only the fields Better Auth knows about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserOut {
    pub name: String,
    pub email: String,
    pub email_verified: bool,
    pub image: Option<String>,
    pub created_at: JsDate,
    pub updated_at: JsDate,
    pub role: Option<String>,
    pub banned: Option<bool>,
    pub ban_reason: Option<String>,
    pub ban_expires: Option<JsDate>,
    pub two_factor_enabled: Option<bool>,
    pub user_code: Option<String>,
    pub used_quota: Option<i64>,
    pub max_quota: Option<i64>,
    pub hide_uploads: Option<bool>,
    pub copy_raw: Option<bool>,
    pub theme: Option<String>,
    pub locale: Option<String>,
    pub id: String,
}

impl From<&User> for UserOut {
    fn from(u: &User) -> Self {
        UserOut {
            name: u.name.clone(),
            email: u.email.clone(),
            email_verified: u.email_verified,
            image: u.image.clone(),
            created_at: u.created_at,
            updated_at: u.updated_at,
            role: u.role.clone(),
            banned: u.banned,
            ban_reason: u.ban_reason.clone(),
            ban_expires: u.ban_expires,
            two_factor_enabled: u.two_factor_enabled,
            user_code: u.user_code.clone(),
            used_quota: u.used_quota,
            max_quota: u.max_quota,
            hide_uploads: u.hide_uploads,
            copy_raw: u.copy_raw,
            theme: u.theme.clone(),
            locale: u.locale.clone(),
            id: u.id.clone(),
        }
    }
}

/// The answer of one session read.
#[derive(Debug, Clone, Default)]
pub struct SessionLookup {
    pub session: Option<AuthSession>,
    /// Cookies Better Auth set while answering (refresh or expiry). Only ever
    /// non-empty under [`RefreshPolicy::DbAndCookie`].
    pub cookies: Vec<SetCookie>,
}

/// `FAILED_TO_GET_SESSION`: what `getSession` throws when it cannot answer.
#[derive(Debug, thiserror::Error)]
#[error("Failed to get session")]
pub struct FailedToGetSession(#[source] pub anyhow::Error);

/// `findSession(token)`: the session row and its user, or `None`.
pub async fn find_session(db: &PgPool, token: &str) -> anyhow::Result<Option<AuthSession>> {
    let sql = format!("SELECT {} FROM {} WHERE \"token\" = $1 LIMIT 1", Session::COLUMNS, Session::TABLE);
    let Some(session) = sqlx::query_as::<_, Session>(&sql).bind(token).fetch_optional(db).await? else {
        return Ok(None);
    };
    let Some(user) = find_user(db, &session.user_id).await? else {
        return Ok(None);
    };
    Ok(Some(AuthSession { session, user }))
}

/// The full `user` row by id.
pub async fn find_user(db: &PgPool, id: &str) -> anyhow::Result<Option<User>> {
    let sql = format!("SELECT {} FROM {} WHERE \"id\" = $1 LIMIT 1", User::COLUMNS, User::TABLE);
    Ok(sqlx::query_as::<_, User>(&sql).bind(id).fetch_optional(db).await?)
}

/// `deleteSession(token)`.
pub async fn delete_session(db: &PgPool, token: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM \"session\" WHERE \"token\" = $1").bind(token).execute(db).await?;
    Ok(())
}

/// `updateSession(token, { expiresAt, updatedAt })`, returning the new row.
pub async fn update_session_expiry(
    db: &PgPool,
    token: &str,
    expires_at: JsDate,
    updated_at: JsDate,
) -> anyhow::Result<Option<Session>> {
    let sql = format!(
        "UPDATE \"session\" SET \"expires_at\" = $1, \"updated_at\" = $2 WHERE \"token\" = $3 RETURNING {}",
        Session::COLUMNS
    );
    Ok(sqlx::query_as::<_, Session>(&sql).bind(expires_at).bind(updated_at).bind(token).fetch_optional(db).await?)
}

/// `deleteSessionCookie(ctx)`: expire the token, the cache cookie (and any
/// chunk of it the request carried) and `dont_remember`.
pub fn delete_session_cookies(cookies: &AuthCookies, headers: &HeaderMap) -> Vec<SetCookie> {
    let mut out = vec![cookies.session_token.expire(), cookies.session_data.expire()];
    // `createSessionStore(...).clean()`: every request cookie whose name starts
    // with the session_data name.
    for name in parse_cookies_better_auth(headers).keys() {
        if name.starts_with(&cookies.session_data.name) {
            let mut c = cookies.session_data.expire();
            c.name = name.clone();
            out.push(c);
        }
    }
    out.push(cookies.dont_remember.expire());
    out
}

/// The `Set-Cookie` that (re)issues a session token cookie.
pub fn session_token_cookie(cookies: &AuthCookies, token: &str, max_age: Option<i64>) -> SetCookie {
    let mut c = cookies.session_token.set(signed_value(token, &cookies.secret));
    c.max_age = max_age;
    c
}

enum SessionDataCheck {
    Fine,
    WrongSignature,
}

/// `getChunkedCookie(ctx, session_data)`.
fn chunked_session_data(cookies: &AuthCookies, headers: &HeaderMap) -> Option<String> {
    let name = &cookies.session_data.name;
    if let Some(v) = parse_cookies_better_call(headers).get(name).filter(|v| !v.is_empty()) {
        return Some(v.clone());
    }
    let mut chunks: Vec<(i64, String)> = Vec::new();
    for (cookie_name, value) in parse_cookies_better_auth(headers) {
        if let Some(suffix) = cookie_name.strip_prefix(&format!("{name}.")) {
            let index_str = suffix.rsplit('.').next().unwrap_or("0");
            if let Some(index) = parse_int_prefix(if index_str.is_empty() { "0" } else { index_str }) {
                chunks.push((index, value));
            }
        }
    }
    if chunks.is_empty() {
        return None;
    }
    chunks.sort_by_key(|(i, _)| *i);
    Some(chunks.into_iter().map(|(_, v)| v).collect())
}

/// `parseInt(s, 10)` (leading digits, optional sign), `None` for NaN.
fn parse_int_prefix(s: &str) -> Option<i64> {
    let t = crate::config::js_trim(s);
    let (neg, digits) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let end = digits.find(|c: char| !c.is_ascii_digit()).unwrap_or(digits.len());
    if end == 0 {
        return None;
    }
    let n: i64 = digits[..end].parse().ok()?;
    Some(if neg { -n } else { n })
}

/// Step 2 of the module docs.
fn check_session_data(cookies: &AuthCookies, headers: &HeaderMap) -> anyhow::Result<SessionDataCheck> {
    let Some(data) = chunked_session_data(cookies, headers) else {
        return Ok(SessionDataCheck::Fine);
    };
    let bytes = better_auth_base64_decode(&data).context("invalid base64 in session_data")?;
    let text = String::from_utf8_lossy(&bytes);
    let Ok(parsed) = serde_json::from_str::<Value>(&text) else {
        return Ok(SessionDataCheck::Fine);
    };
    if !js_truthy(&parsed) {
        return Ok(SessionDataCheck::Fine);
    }
    let Some(signature) = parsed.get("signature").and_then(Value::as_str) else {
        anyhow::bail!("session_data carries no signature");
    };
    // JSON.stringify({ ...parsed.session, expiresAt: parsed.expiresAt })
    let mut signed = serde_json::Map::new();
    if let Some(Value::Object(session)) = parsed.get("session") {
        for (k, v) in session {
            signed.insert(k.clone(), v.clone());
        }
    }
    if let Some(expires) = parsed.get("expiresAt") {
        signed.insert("expiresAt".to_string(), expires.clone());
    }
    let data = js_json_stringify(&Value::Object(signed));
    match hmac_base64url_verify(&cookies.secret, &data, signature) {
        None => anyhow::bail!("invalid base64 in session_data signature"),
        Some(true) => Ok(SessionDataCheck::Fine),
        Some(false) => Ok(SessionDataCheck::WrongSignature),
    }
}

fn js_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// `JSON.stringify` key order: array-index-like keys first, ascending.
fn js_json_stringify(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut index_keys: Vec<(u32, &String)> = Vec::new();
            let mut other: Vec<&String> = Vec::new();
            for k in map.keys() {
                match k.parse::<u32>() {
                    Ok(n) if n.to_string() == *k && n != u32::MAX => index_keys.push((n, k)),
                    _ => other.push(k),
                }
            }
            index_keys.sort();
            let parts: Vec<String> = index_keys
                .into_iter()
                .map(|(_, k)| k)
                .chain(other)
                .map(|k| format!("{}:{}", serde_json::to_string(k).unwrap_or_default(), js_json_stringify(&map[k])))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(items) => format!("[{}]", items.iter().map(js_json_stringify).collect::<Vec<_>>().join(",")),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// Better Auth `getSession` (see the module docs for the steps and policies).
pub async fn get_session(
    db: &PgPool,
    cookies: &AuthCookies,
    headers: &HeaderMap,
    policy: RefreshPolicy,
) -> Result<SessionLookup, FailedToGetSession> {
    get_session_at(db, cookies, headers, policy, JsDate::now()).await.map_err(FailedToGetSession)
}

async fn get_session_at(
    db: &PgPool,
    cookies: &AuthCookies,
    headers: &HeaderMap,
    policy: RefreshPolicy,
    now: JsDate,
) -> anyhow::Result<SessionLookup> {
    let parsed = parse_cookies_better_call(headers);
    let token = match verify_signed_cookie(parsed.get(&cookies.session_token.name).map(String::as_str), &cookies.secret)
    {
        SignedCookie::Valid(token) => token,
        SignedCookie::Absent | SignedCookie::Invalid => return Ok(SessionLookup::default()),
    };
    // Cookies are collected as Better Auth sets them; whether they reach the
    // response is the policy's call.
    let finish = |session: Option<AuthSession>, set: Vec<SetCookie>| SessionLookup {
        session,
        cookies: if policy.sets_cookies() { set } else { Vec::new() },
    };

    if let SessionDataCheck::WrongSignature = check_session_data(cookies, headers)? {
        return Ok(finish(None, vec![cookies.session_data.expire()]));
    }

    let dont_remember = matches!(
        verify_signed_cookie(parsed.get(&cookies.dont_remember.name).map(String::as_str), &cookies.secret),
        SignedCookie::Valid(ref v) if !v.is_empty()
    );

    let Some(found) = find_session(db, &token).await? else {
        return Ok(finish(None, delete_session_cookies(cookies, headers)));
    };
    if found.session.expires_at < now {
        let expired = delete_session_cookies(cookies, headers);
        delete_session(db, &found.session.token).await?;
        return Ok(finish(None, expired));
    }
    if dont_remember {
        return Ok(finish(Some(found), Vec::new()));
    }

    let should_be_updated =
        found.session.expires_at.millis() - SESSION_EXPIRES_IN * 1000 + SESSION_UPDATE_AGE * 1000 <= now.millis();
    if !(should_be_updated && policy.refreshes()) {
        return Ok(finish(Some(found), Vec::new()));
    }

    let new_expiry = JsDate::from_millis(now.millis() + SESSION_EXPIRES_IN * 1000).unwrap_or(now);
    let Some(updated) = update_session_expiry(db, &found.session.token, new_expiry, now).await? else {
        // Deleted concurrently: Better Auth expires the cookies and throws.
        anyhow::bail!("session disappeared while it was being refreshed");
    };
    // `maxAge: (expiresAt - Date.now()) / 1000`, floored when serialized.
    let remaining_ms = updated.expires_at.millis() - JsDate::now().millis();
    let cookie = session_token_cookie(cookies, &updated.token, Some(remaining_ms.div_euclid(1000)));
    Ok(finish(Some(AuthSession { session: updated, user: found.user }), vec![cookie]))
}

/// The admin plugin's `databaseHooks.session.create.before`, for the sign-in
/// paths: a banned account may not get a session unless its ban has expired,
/// in which case the ban is lifted first. `Err` carries the BANNED_USER message.
pub async fn check_ban_before_session_create(db: &PgPool, user_id: &str) -> anyhow::Result<Result<(), &'static str>> {
    let Some(user) = find_user(db, user_id).await? else {
        return Ok(Ok(()));
    };
    if user.banned != Some(true) {
        return Ok(Ok(()));
    }
    if let Some(expires) = user.ban_expires
        && expires.millis() < JsDate::now().millis()
    {
        sqlx::query(
            "UPDATE \"user\" SET \"banned\" = false, \"ban_reason\" = NULL, \"ban_expires\" = NULL, \"updated_at\" = $2 WHERE \"id\" = $1",
        )
        .bind(user_id)
        .bind(JsDate::now())
        .execute(db)
        .await?;
        return Ok(Ok(()));
    }
    Ok(Err(BANNED_USER_MESSAGE))
}

/// The admin plugin's default `bannedUserMessage`.
pub const BANNED_USER_MESSAGE: &str =
    "You have been banned from this application. Please contact support if you believe this is an error.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_stringify_orders_index_keys_first() {
        let v: Value = serde_json::from_str(r#"{"b":1,"10":2,"a":{"2":true,"1":null},"1":3}"#).unwrap();
        assert_eq!(js_json_stringify(&v), r#"{"1":3,"10":2,"b":1,"a":{"1":null,"2":true}}"#);
    }

    #[test]
    fn parse_int_like_javascript() {
        assert_eq!(parse_int_prefix("12"), Some(12));
        assert_eq!(parse_int_prefix("3x"), Some(3));
        assert_eq!(parse_int_prefix("x"), None);
    }

    #[test]
    fn output_field_order_matches_better_auth() {
        let user = User {
            id: "u1".into(),
            name: "N".into(),
            email: "e@x".into(),
            email_verified: true,
            image: None,
            created_at: JsDate::from_millis(0).unwrap(),
            updated_at: JsDate::from_millis(0).unwrap(),
            role: Some("admin".into()),
            banned: Some(false),
            ban_reason: None,
            ban_expires: None,
            two_factor_enabled: Some(false),
            user_code: Some("tag".into()),
            naming_scheme: None,
            naming_pattern: None,
            public_profile: None,
            used_quota: Some(5),
            max_quota: Some(-1),
            hide_uploads: Some(false),
            copy_raw: Some(false),
            theme: Some("discord".into()),
            locale: None,
            username: Some("n".into()),
            skip_tag_in_url: None,
            allow_root_short_links: None,
            default_folder_id: None,
            naming_word_count: None,
            naming_random_length: None,
            preferred_domain: None,
            default_password_hash: None,
            default_password_version: 0,
            background_key: None,
            background_dim: 70,
        };
        let json = serde_json::to_value(UserOut::from(&user)).unwrap();
        let keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec![
                "name",
                "email",
                "emailVerified",
                "image",
                "createdAt",
                "updatedAt",
                "role",
                "banned",
                "banReason",
                "banExpires",
                "twoFactorEnabled",
                "userCode",
                "usedQuota",
                "maxQuota",
                "hideUploads",
                "copyRaw",
                "theme",
                "locale",
                "id"
            ]
        );
    }
}
