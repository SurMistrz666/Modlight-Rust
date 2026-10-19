//! Port of `lib/invites.ts`: invitations for accounts an administrator creates.
//!
//! Why a token of our own rather than reusing the password-reset link: the
//! reset mail says "somebody asked to reset your password", and a person who
//! has never had an account on this site has no password to reset. The wording
//! of a transactional email is not decoration, it is the only context the
//! recipient gets, and one that describes the wrong event trains people to
//! click links they do not understand.
//!
//! Stored in Better Auth's `verification` table, which was empty and is
//! exactly the right shape (identifier, value, expiry). The identifier carries
//! the token HASH so the lookup is the indexed one, and the value carries the
//! user id. Storing the raw token would mean a database read is enough to take
//! over every pending account.

use std::collections::HashSet;

use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::db::types::JsDate;

const PREFIX: &str = "invite:";
const TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Passwords set through an invitation: Better Auth's own minimum.
pub const INVITE_PASSWORD_MIN: usize = 8;

fn digest(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Mint an invitation for `user_id`, replacing any earlier one.
///
/// Replacing rather than adding is what makes "resend the invitation" safe:
/// two live tokens for one account would mean revoking an invitation is
/// impossible without knowing how many were issued.
pub async fn create_invite(db: &PgPool, user_id: &str) -> anyhow::Result<String> {
    revoke_invites(db, user_id).await?;
    let token = crate::ids::secure_token(32);
    let now = JsDate::now();
    let expires = JsDate::from_millis(now.millis() + TTL_MS).unwrap_or(now);
    sqlx::query(
        r#"INSERT INTO "verification" ("id", "identifier", "value", "expires_at", "created_at", "updated_at")
           VALUES ($1, $2, $3, $4, $5, $5)"#,
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(format!("{PREFIX}{}", digest(&token)))
    .bind(user_id)
    .bind(expires)
    .bind(now)
    .execute(db)
    .await?;
    Ok(token)
}

pub async fn revoke_invites(db: &PgPool, user_id: &str) -> sqlx::Result<()> {
    sqlx::query(r#"DELETE FROM "verification" WHERE "identifier" LIKE 'invite:%' AND "value" = $1"#)
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Is there a live invitation for this account? Drives the "Invited" badge.
/// (The FIRST matching row decides, as upstream's `LIMIT 1` does.)
pub async fn has_pending_invite(db: &PgPool, user_id: &str) -> sqlx::Result<bool> {
    let row: Option<(JsDate,)> = sqlx::query_as(
        r#"SELECT "expires_at" FROM "verification" WHERE "identifier" LIKE 'invite:%' AND "value" = $1 LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    Ok(row.is_some_and(|(expires,)| expires.millis() > JsDate::now().millis()))
}

/// Every account with a live invitation, in one query (the user table is drawn
/// as a list; one round trip per row for a badge would be silly).
pub async fn pending_invite_user_ids(db: &PgPool) -> sqlx::Result<HashSet<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        r#"SELECT "value" FROM "verification" WHERE "identifier" LIKE 'invite:%' AND "expires_at" > $1"#,
    )
    .bind(JsDate::now())
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteSubject {
    pub user_id: String,
    pub email: String,
    pub name: String,
}

/// Resolve a token without consuming it, so the page can be rendered first.
/// An expired invitation, or one whose account is gone, is deleted.
pub async fn read_invite(db: &PgPool, token: &str) -> sqlx::Result<Option<InviteSubject>> {
    if token.is_empty() {
        return Ok(None);
    }
    let row: Option<(String, String, JsDate)> =
        sqlx::query_as(r#"SELECT "id", "value", "expires_at" FROM "verification" WHERE "identifier" = $1 LIMIT 1"#)
            .bind(format!("{PREFIX}{}", digest(token)))
            .fetch_optional(db)
            .await?;
    let Some((id, user_id, expires_at)) = row else {
        return Ok(None);
    };
    if expires_at.millis() <= JsDate::now().millis() {
        sqlx::query(r#"DELETE FROM "verification" WHERE "id" = $1"#).bind(&id).execute(db).await?;
        return Ok(None);
    }
    let account: Option<(String, String, String)> =
        sqlx::query_as(r#"SELECT "id", "email", "name" FROM "user" WHERE "id" = $1 LIMIT 1"#)
            .bind(&user_id)
            .fetch_optional(db)
            .await?;
    let Some((user_id, email, name)) = account else {
        // The account was deleted between the invitation and the click.
        sqlx::query(r#"DELETE FROM "verification" WHERE "id" = $1"#).bind(&id).execute(db).await?;
        return Ok(None);
    };
    Ok(Some(InviteSubject { user_id, email, name }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptInviteResult {
    Ok { user_id: String, email: String },
    Invalid,
    WeakPassword,
}

/// Set the password the invitation was for, and mark the address confirmed.
///
/// Confirming the address here is not a shortcut: the invitation was delivered
/// to that mailbox and the token proves the recipient read it, which is a
/// stronger demonstration than the confirmation mail would have been.
pub async fn accept_invite(db: &PgPool, token: &str, password: &str) -> anyhow::Result<AcceptInviteResult> {
    // `password.length` counts UTF-16 code units.
    if password.is_empty() || password.encode_utf16().count() < INVITE_PASSWORD_MIN {
        return Ok(AcceptInviteResult::WeakPassword);
    }
    let Some(subject) = read_invite(db, token).await? else {
        return Ok(AcceptInviteResult::Invalid);
    };
    crate::credentials::write_credential_password(db, &subject.user_id, password).await?;
    sqlx::query(r#"UPDATE "user" SET "email_verified" = true, "updated_at" = $2 WHERE "id" = $1"#)
        .bind(&subject.user_id)
        .bind(JsDate::now())
        .execute(db)
        .await?;
    revoke_invites(db, &subject.user_id).await?;
    Ok(AcceptInviteResult::Ok { user_id: subject.user_id, email: subject.email })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_carry_the_token_hash() {
        // sha256("abc")
        assert_eq!(digest("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
