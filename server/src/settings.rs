//! Port of `lib/settings.ts`: global runtime settings (admin-configurable),
//! stored in the `settings` table.
//!
//! Deliberately tiny: a key earns its place here only once something in the
//! app actually reads it. Rows left behind by older versions are ignored.

use std::collections::HashMap;

use sqlx::PgPool;

/// Every key the application reads or writes (`SettingKey`).
pub mod keys {
    /// "on"/"off": may STRANGERS sign themselves up at /register? Nothing else.
    pub const REGISTER_ENABLED: &str = "register_enabled";
    /// Optional shared key a self-service sign-up must also present (encrypted).
    pub const REGISTRATION_KEY_ENC: &str = "registration_key_enc";
    pub const SMTP_HOST: &str = "smtp_host";
    pub const SMTP_PORT: &str = "smtp_port";
    /// "none" | "starttls" | "tls"
    pub const SMTP_SECURITY: &str = "smtp_security";
    pub const SMTP_USER: &str = "smtp_user";
    pub const SMTP_PASSWORD_ENC: &str = "smtp_password_enc";
    pub const MAIL_FROM: &str = "mail_from";
    pub const MAIL_FROM_NAME: &str = "mail_from_name";
    /// "on"/"off": must a new account confirm its address before signing in?
    pub const EMAIL_VERIFICATION_REQUIRED: &str = "email_verification_required";
    /// The published build of Modlight App, as JSON (lib/releases-core.ts).
    pub const RELEASE_APP: &str = "release_app";
}

/// Keys that may be handed to a client component (an ALLOWLIST).
pub const PANEL_SETTINGS: &[&str] = &[
    keys::REGISTER_ENABLED,
    keys::SMTP_HOST,
    keys::SMTP_PORT,
    keys::SMTP_SECURITY,
    keys::SMTP_USER,
    keys::MAIL_FROM,
    keys::MAIL_FROM_NAME,
    keys::EMAIL_VERIFICATION_REQUIRED,
];

/// Keys whose VALUE must never leave the server.
pub const SECRET_SETTINGS: &[&str] = &[keys::REGISTRATION_KEY_ENC, keys::SMTP_PASSWORD_ENC];

/// `getSetting(key)`: the value, or `None` for a missing row or a NULL value.
pub async fn get_setting(db: &PgPool, key: &str) -> sqlx::Result<Option<String>> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT \"value\" FROM \"settings\" WHERE \"key\" = $1 LIMIT 1")
            .bind(key)
            .fetch_optional(db)
            .await?;
    Ok(row.and_then(|(v,)| v))
}

/// `setSetting(key, value)`: upsert; `None` keeps the row with a NULL value.
pub async fn set_setting(db: &PgPool, key: &str, value: Option<&str>) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO \"settings\" (\"key\", \"value\") VALUES ($1, $2) ON CONFLICT (\"key\") DO UPDATE SET \"value\" = $2",
    )
    .bind(key)
    .bind(value)
    .execute(db)
    .await?;
    Ok(())
}

/// `getSettings(keys)`: several keys in one round trip; NULL values are left
/// out, exactly like rows that do not exist.
pub async fn get_settings(db: &PgPool, keys: &[&str]) -> sqlx::Result<HashMap<String, String>> {
    if keys.is_empty() {
        return Ok(HashMap::new());
    }
    let list: Vec<String> = keys.iter().map(|k| (*k).to_string()).collect();
    let rows: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT \"key\", \"value\" FROM \"settings\" WHERE \"key\" = ANY($1)")
            .bind(&list)
            .fetch_all(db)
            .await?;
    Ok(rows.into_iter().filter_map(|(k, v)| v.map(|v| (k, v))).collect())
}

/// `getPanelSettings()`: restricted to [`PANEL_SETTINGS`].
pub async fn get_panel_settings(db: &PgPool) -> sqlx::Result<HashMap<String, String>> {
    get_settings(db, PANEL_SETTINGS).await
}

/// `seedSetting(key, value)`: only if the key has never been written.
pub async fn seed_setting(db: &PgPool, key: &str, value: &str) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO \"settings\" (\"key\", \"value\") VALUES ($1, $2) ON CONFLICT (\"key\") DO NOTHING")
        .bind(key)
        .bind(value)
        .execute(db)
        .await?;
    Ok(())
}

/// `isOn(value)`: "on", "true" or "1".
pub fn is_on(value: Option<&str>) -> bool {
    matches!(value, Some("on" | "true" | "1"))
}

/// `isRegistrationEnabled()`: the database row is the answer;
/// REGISTRATION_ENABLED survives only as a fallback for installs that
/// predate the seeding step.
pub async fn is_registration_enabled(db: &PgPool, env_fallback: bool) -> sqlx::Result<bool> {
    // A row whose value is NULL reads as missing (`v == null`).
    Ok(match get_setting(db, keys::REGISTER_ENABLED).await? {
        None => env_fallback,
        Some(v) => is_on(Some(&v)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_on_accepts_the_three_spellings() {
        assert!(is_on(Some("on")));
        assert!(is_on(Some("true")));
        assert!(is_on(Some("1")));
        assert!(!is_on(Some("off")));
        assert!(!is_on(Some("ON")));
        assert!(!is_on(None));
    }

    #[test]
    fn secrets_never_appear_in_the_panel_allowlist() {
        for secret in SECRET_SETTINGS {
            assert!(!PANEL_SETTINGS.contains(secret));
        }
    }
}
