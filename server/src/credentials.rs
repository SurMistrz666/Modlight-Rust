//! Port of `lib/credentials.ts`: set the password on an account's credential
//! login, creating that login if it does not exist.
//!
//! The fallback is not theoretical: accounts imported from elsewhere without a
//! usable password hash (LDAP logins, or rows where the hash was missing) have
//! no `account` row at all, and Better Auth's own `setUserPassword` updates
//! zero rows for them and reports success. An admin would set a password, be
//! told it worked, and the user still could not sign in.
//!
//! Hashing goes through the same function `lib/auth.ts` installs as
//! `emailAndPassword.password.hash`, so what lands here is byte-for-byte what a
//! self-service password change would write.

use sqlx::PgPool;

use crate::db::types::JsDate;

pub async fn write_credential_password(db: &PgPool, user_id: &str, password: &str) -> anyhow::Result<()> {
    let hash = crate::auth::password::hash_password(password).await?;
    let now = JsDate::now();
    // drizzle's `$onUpdate` stamps `updated_at` on every UPDATE of `account`.
    let updated = sqlx::query(
        r#"UPDATE "account" SET "password" = $1, "updated_at" = $2 WHERE "user_id" = $3 AND "provider_id" = 'credential' RETURNING "id""#,
    )
    .bind(&hash)
    .bind(now)
    .bind(user_id)
    .fetch_all(db)
    .await?;
    if !updated.is_empty() {
        return Ok(());
    }
    sqlx::query(
        r#"INSERT INTO "account" ("id", "account_id", "provider_id", "user_id", "password", "created_at", "updated_at")
           VALUES ($1, $2, 'credential', $2, $3, $4, $4)"#,
    )
    .bind(uuid::Uuid::new_v4().to_string())
    // Better Auth keys credential accounts by user id.
    .bind(user_id)
    .bind(&hash)
    .bind(now)
    .execute(db)
    .await?;
    Ok(())
}
