//! The server-side calls upstream makes through `auth.api.*` (PORT.md 9):
//! the same database effects as the Better Auth endpoints, as plain Rust
//! functions for actions and pages. Session resolution (`getSession` with the
//! request headers) is the caller's [`crate::auth_helpers::RequestSession`];
//! each function takes the session it resolved.

use sqlx::PgPool;

use super::session::AuthSession;
use crate::db::models::{Columns, Session};
use crate::db::types::JsDate;

/// `internalAdapter.listSessions(userId)`: every session row of the user (no
/// ORDER BY upstream; callers sort).
pub async fn list_user_session_rows(db: &PgPool, user_id: &str) -> sqlx::Result<Vec<Session>> {
    let sql = format!("SELECT {} FROM {} WHERE \"user_id\" = $1", Session::COLUMNS, Session::TABLE);
    sqlx::query_as::<_, Session>(&sql).bind(user_id).fetch_all(db).await
}

/// `auth.api.listSessions({ headers })` (`/list-sessions`): the caller's
/// sessions with `expires_at > now`, then the admin plugin's after-hook drops
/// the ones that carry `impersonated_by`.
pub async fn list_sessions(db: &PgPool, current: &AuthSession) -> sqlx::Result<Vec<Session>> {
    let now = JsDate::now();
    let sql =
        format!("SELECT {} FROM {} WHERE \"user_id\" = $1 AND \"expires_at\" > $2", Session::COLUMNS, Session::TABLE);
    let rows = sqlx::query_as::<_, Session>(&sql).bind(&current.user.id).bind(now).fetch_all(db).await?;
    Ok(rows
        .into_iter()
        .filter(|s| s.expires_at.millis() > now.millis())
        .filter(|s| s.impersonated_by.as_deref().is_none_or(str::is_empty))
        .collect())
}

/// `auth.api.revokeSession({ body: { token }, headers })` (`/revoke-session`):
/// deletes the session with that token only when it belongs to the caller;
/// answers `{status: true}` either way.
pub async fn revoke_session(db: &PgPool, current: &AuthSession, token: &str) -> sqlx::Result<()> {
    let owner: Option<(String,)> = sqlx::query_as("SELECT \"user_id\" FROM \"session\" WHERE \"token\" = $1 LIMIT 1")
        .bind(token)
        .fetch_optional(db)
        .await?;
    if owner.is_some_and(|(user_id,)| user_id == current.user.id) {
        sqlx::query("DELETE FROM \"session\" WHERE \"token\" = $1").bind(token).execute(db).await?;
    }
    Ok(())
}

/// `auth.api.revokeOtherSessions({ headers })` (`/revoke-other-sessions`):
/// every unexpired session of the caller except the current one (impersonated
/// ones included: no admin filter on this path).
pub async fn revoke_other_sessions(db: &PgPool, current: &AuthSession) -> sqlx::Result<()> {
    let now = JsDate::now();
    for s in list_user_session_rows(db, &current.user.id).await? {
        if s.expires_at.millis() > now.millis() && s.token != current.session.token {
            sqlx::query("DELETE FROM \"session\" WHERE \"token\" = $1").bind(&s.token).execute(db).await?;
        }
    }
    Ok(())
}
