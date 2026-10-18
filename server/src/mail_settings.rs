//! Port of `lib/mail-settings.ts`: where the mail configuration comes from,
//! and what it currently is.
//!
//! The one non-obvious rule here is BLOCK PRECEDENCE: either the whole SMTP
//! configuration comes from the panel, or the whole of it comes from the
//! environment. Never a field from each. Merged per field, an administrator
//! who changes only the host in the panel would inherit SMTP_USER and
//! SMTP_PASSWORD from .env, and the application would then send those
//! credentials to a server the administrator just named.
//!
//! The panel wins over the environment because a "Save" button that changes
//! nothing is a lie, and because the rest of the app already works this way
//! (`isRegistrationEnabled`: row first, env as the seed).
//!
//! A per-process SNAPSHOT (30 s) answers [`verification_required_sync`]
//! without awaiting: Better Auth reads `requireEmailVerification` on every
//! sign-in through a getter, which is what lets an administrator turn address
//! confirmation on from the panel without a restart.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::config::{Config, js_number, js_trim};
use crate::secret_box::{SecretPurpose, encrypt_secret, try_decrypt_secret};
use crate::settings::{get_settings, is_on, keys, set_setting};
use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SmtpSecurity {
    None,
    Starttls,
    Tls,
}

impl SmtpSecurity {
    pub fn as_str(self) -> &'static str {
        match self {
            SmtpSecurity::None => "none",
            SmtpSecurity::Starttls => "starttls",
            SmtpSecurity::Tls => "tls",
        }
    }

    /// `toSecurity(value)`: anything unknown is `starttls`.
    pub fn parse_or_starttls(value: Option<&str>) -> SmtpSecurity {
        match value {
            Some("tls") => SmtpSecurity::Tls,
            Some("none") => SmtpSecurity::None,
            _ => SmtpSecurity::Starttls,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MailSource {
    Panel,
    Env,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MailPasswordState {
    Unset,
    Set,
    Undecryptable,
}

/// The `settings` rows this module reads.
pub const SMTP_SETTING_KEYS: &[&str] = &[
    keys::SMTP_HOST,
    keys::SMTP_PORT,
    keys::SMTP_SECURITY,
    keys::SMTP_USER,
    keys::SMTP_PASSWORD_ENC,
    keys::MAIL_FROM,
    keys::MAIL_FROM_NAME,
    keys::EMAIL_VERIFICATION_REQUIRED,
];

/// `EnvMail`: the SMTP_* environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvMail {
    pub host: String,
    pub port: i64,
    pub user: String,
    pub password: String,
    pub secure: bool,
    /// Explicit mode from SMTP_SECURITY; `None` = derive it from `secure`.
    pub security: Option<SmtpSecurity>,
    pub from: String,
    pub from_name: String,
}

/// `ResolvedMail`: the effective configuration. The password never leaves
/// the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMail {
    pub source: MailSource,
    pub host: String,
    pub port: i64,
    pub security: SmtpSecurity,
    pub user: String,
    pub password: String,
    pub from: String,
    pub from_name: String,
    /// Enough configuration to attempt a send.
    pub ready: bool,
    pub password_state: MailPasswordState,
    /// Must a new account confirm its address before it can sign in?
    pub verification_required: bool,
}

/// `MailStatus`: what a browser is allowed to learn. Never a credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailStatus {
    pub source: MailSource,
    pub host: String,
    pub port: i64,
    pub security: SmtpSecurity,
    pub user: String,
    pub from: String,
    pub from_name: String,
    pub ready: bool,
    pub password_state: MailPasswordState,
    pub verification_required: bool,
    /// True when SMTP_* is present in the environment (for the panel's badge).
    pub env_available: bool,
}

/// `toPort(value, fallback)`: an integer in 1..65535, else the fallback.
fn to_port(value: Option<&str>, fallback: i64) -> i64 {
    // `Number(undefined)` is NaN.
    let n = value.map(js_number).unwrap_or(f64::NAN);
    if n.is_finite() && n > 0.0 && n < 65536.0 { n.trunc() as i64 } else { fallback }
}

/// `resolveMailSettings(rows, env, decrypt)`: pure, every input a parameter.
pub fn resolve_mail_settings(
    rows: &HashMap<String, String>,
    env: &EnvMail,
    app_name: &str,
    decrypt: &dyn Fn(&str) -> Option<String>,
) -> ResolvedMail {
    let row = |k: &str| rows.get(k).map(String::as_str);
    let verification_required = is_on(row(keys::EMAIL_VERIFICATION_REQUIRED));
    let panel_host = js_trim(row(keys::SMTP_HOST).unwrap_or("")).to_string();

    if !panel_host.is_empty() {
        let boxed = row(keys::SMTP_PASSWORD_ENC).unwrap_or("");
        let decrypted = if boxed.is_empty() { Some(String::new()) } else { decrypt(boxed) };
        let password_state = if boxed.is_empty() {
            MailPasswordState::Unset
        } else if decrypted.is_none() {
            MailPasswordState::Undecryptable
        } else {
            MailPasswordState::Set
        };
        let user = js_trim(row(keys::SMTP_USER).unwrap_or("")).to_string();
        // A password that cannot be read is NOT the same as no password.
        let usable = password_state != MailPasswordState::Undecryptable
            && (user.is_empty() || password_state == MailPasswordState::Set);
        let from = js_trim(row(keys::MAIL_FROM).unwrap_or(""));
        let from_name = js_trim(row(keys::MAIL_FROM_NAME).unwrap_or(""));
        return ResolvedMail {
            source: MailSource::Panel,
            host: panel_host,
            port: to_port(row(keys::SMTP_PORT), 587),
            security: SmtpSecurity::parse_or_starttls(row(keys::SMTP_SECURITY)),
            user,
            password: decrypted.unwrap_or_default(),
            from: if from.is_empty() { env.from.clone() } else { from.to_string() },
            from_name: if from_name.is_empty() { app_name.to_string() } else { from_name.to_string() },
            ready: usable,
            password_state,
            verification_required: verification_required && usable,
        };
    }

    if !js_trim(&env.host).is_empty() {
        return ResolvedMail {
            source: MailSource::Env,
            host: js_trim(&env.host).to_string(),
            port: env.port,
            // SMTP_SECURITY, when given, says exactly which of the three modes
            // to use; unset keeps the historical mapping (SMTP_SECURE=true is
            // implicit TLS, false a plain connect with opportunistic upgrade).
            security: env.security.unwrap_or(if env.secure { SmtpSecurity::Tls } else { SmtpSecurity::None }),
            user: env.user.clone(),
            password: env.password.clone(),
            from: env.from.clone(),
            from_name: if env.from_name.is_empty() { app_name.to_string() } else { env.from_name.clone() },
            ready: true,
            password_state: if env.password.is_empty() { MailPasswordState::Unset } else { MailPasswordState::Set },
            verification_required,
        };
    }

    ResolvedMail {
        source: MailSource::None,
        host: String::new(),
        port: 587,
        security: SmtpSecurity::Starttls,
        user: String::new(),
        password: String::new(),
        from: env.from.clone(),
        from_name: app_name.to_string(),
        ready: false,
        password_state: MailPasswordState::Unset,
        // Requiring a confirmation nobody can be sent locks every new account out.
        verification_required: false,
    }
}

/// `envMail()` from the configuration.
pub fn env_mail(config: &Config) -> EnvMail {
    let raw = js_trim(&config.mail.security).to_ascii_lowercase();
    EnvMail {
        host: config.mail.host.clone(),
        port: config.mail.port,
        user: config.mail.user.clone(),
        password: config.mail.password.clone(),
        secure: config.mail.secure,
        security: match raw.as_str() {
            "tls" => Some(SmtpSecurity::Tls),
            "starttls" => Some(SmtpSecurity::Starttls),
            "none" => Some(SmtpSecurity::None),
            _ => None,
        },
        from: config.mail.from.clone(),
        from_name: config.mail.from_name.clone(),
    }
}

/// `toMailStatus(resolved)`.
pub fn to_mail_status(resolved: &ResolvedMail, config: &Config) -> MailStatus {
    MailStatus {
        source: resolved.source,
        host: resolved.host.clone(),
        port: resolved.port,
        security: resolved.security,
        user: resolved.user.clone(),
        from: resolved.from.clone(),
        from_name: resolved.from_name.clone(),
        ready: resolved.ready,
        password_state: resolved.password_state,
        verification_required: resolved.verification_required,
        env_available: !js_trim(&config.mail.host).is_empty(),
    }
}

/// `mailFingerprint(resolved)`: identity of a configuration for transport
/// cache invalidation. The password takes part as a hash, never as itself.
pub fn mail_fingerprint(resolved: &ResolvedMail) -> String {
    let password_mark = if resolved.password.is_empty() {
        "-".to_string()
    } else {
        hex::encode(Sha256::digest(resolved.password.as_bytes()))[..16].to_string()
    };
    [
        match resolved.source {
            MailSource::Panel => "panel",
            MailSource::Env => "env",
            MailSource::None => "none",
        }
        .to_string(),
        resolved.host.clone(),
        resolved.port.to_string(),
        resolved.security.as_str().to_string(),
        resolved.user.clone(),
        resolved.from.clone(),
        password_mark,
    ]
    .join("|")
}

// ── Snapshot ────────────────────────────────────────────────────────────────

/// How long a snapshot is trusted before the table is consulted again.
const TTL_MS: i64 = 30_000;

struct Snapshot {
    value: ResolvedMail,
    read_at: i64,
}

static SNAPSHOT: Mutex<Option<Snapshot>> = Mutex::new(None);

fn now_ms() -> i64 {
    crate::db::types::JsDate::now().millis()
}

fn decrypt_smtp_password(secret: &str) -> impl Fn(&str) -> Option<String> + '_ {
    move |boxed: &str| try_decrypt_secret(secret, Some(boxed), SecretPurpose::Smtp)
}

/// `loadMailSettings(force)`: the effective configuration, refreshing the
/// snapshot when it is stale (or always, when `force`).
pub async fn load_mail_settings_with(db: &PgPool, config: &Config, force: bool) -> sqlx::Result<ResolvedMail> {
    if !force {
        let guard = SNAPSHOT.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = guard.as_ref()
            && now_ms() - s.read_at < TTL_MS
        {
            return Ok(s.value.clone());
        }
    }
    let rows = get_settings(db, SMTP_SETTING_KEYS).await?;
    let value =
        resolve_mail_settings(&rows, &env_mail(config), &config.app_name, &decrypt_smtp_password(&config.auth.secret));
    *SNAPSHOT.lock().unwrap_or_else(|e| e.into_inner()) = Some(Snapshot { value: value.clone(), read_at: now_ms() });
    Ok(value)
}

pub async fn load_mail_settings(state: &AppState, force: bool) -> sqlx::Result<ResolvedMail> {
    load_mail_settings_with(&state.db, &state.config, force).await
}

/// `mailSnapshot()`: the last known configuration WITHOUT awaiting. Before the
/// first load it falls back to the environment.
pub fn mail_snapshot(config: &Config) -> ResolvedMail {
    let guard = SNAPSHOT.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        Some(s) => s.value.clone(),
        None => resolve_mail_settings(
            &HashMap::new(),
            &env_mail(config),
            &config.app_name,
            &decrypt_smtp_password(&config.auth.secret),
        ),
    }
}

/// `mailReadySync()`.
pub fn mail_ready_sync(config: &Config) -> bool {
    mail_snapshot(config).ready
}

/// `verificationRequiredSync()`: what `requireEmailVerification` answers.
/// Belt and braces: `resolveMailSettings` already refuses to require what it
/// cannot send, but this is the value that decides whether people can sign in.
pub fn verification_required_sync(config: &Config) -> bool {
    let snap = mail_snapshot(config);
    snap.verification_required && snap.ready
}

/// `mailReady()`.
pub async fn mail_ready(state: &AppState) -> sqlx::Result<bool> {
    Ok(load_mail_settings(state, false).await?.ready)
}

/// `invalidateMailSettings()`: drop the snapshot so the next read hits the table.
pub fn invalidate_mail_settings() {
    *SNAPSHOT.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// `warmMailSettings()`: populate the snapshot at boot so the first request is
/// not answered from the environment. A database that is not up yet must not
/// stop the server from starting (warn and carry on).
pub async fn warm_mail_settings(state: &AppState) {
    if let Err(err) = load_mail_settings(state, true).await {
        tracing::warn!("[mail] could not preload settings: {err}");
    }
}

// ── Writing ─────────────────────────────────────────────────────────────────

/// `MailSettingsPatch`.
#[derive(Debug, Clone)]
pub struct MailSettingsPatch {
    pub host: String,
    pub port: f64,
    pub security: String,
    pub user: String,
    /// Empty/absent means "leave the stored password alone".
    pub password: Option<String>,
    pub from: String,
    pub from_name: String,
}

/// `saveMailSettings(patch)`. Deliberately does NOT touch
/// `email_verification_required` (the form sends back the EFFECTIVE value,
/// which is masked to false whenever the block cannot send).
pub async fn save_mail_settings(state: &AppState, patch: &MailSettingsPatch) -> anyhow::Result<(Vec<String>, bool)> {
    let before = get_settings(&state.db, SMTP_SETTING_KEYS).await?;
    let mut changed: Vec<String> = Vec::new();
    let port_text = crate::config::format_js_number(patch.port);
    let writes: Vec<(&str, String)> = vec![
        (keys::SMTP_HOST, js_trim(&patch.host).to_string()),
        (keys::SMTP_PORT, to_port(Some(&port_text), 587).to_string()),
        (keys::SMTP_SECURITY, SmtpSecurity::parse_or_starttls(Some(&patch.security)).as_str().to_string()),
        (keys::SMTP_USER, js_trim(&patch.user).to_string()),
        (keys::MAIL_FROM, js_trim(&patch.from).to_string()),
        (keys::MAIL_FROM_NAME, js_trim(&patch.from_name).to_string()),
    ];
    for (key, value) in &writes {
        if before.get(*key).map(String::as_str).unwrap_or("") != value {
            changed.push((*key).to_string());
        }
        set_setting(&state.db, key, Some(value)).await?;
    }

    let password_changed = patch.password.as_deref().is_some_and(|p| !p.is_empty());
    if password_changed {
        let boxed =
            encrypt_secret(&state.config.auth.secret, patch.password.as_deref().unwrap_or(""), SecretPurpose::Smtp)?;
        set_setting(&state.db, keys::SMTP_PASSWORD_ENC, Some(&boxed)).await?;
    } else if before.get(keys::SMTP_HOST).map(String::as_str).unwrap_or("") != js_trim(&patch.host)
        || before.get(keys::SMTP_USER).map(String::as_str).unwrap_or("") != js_trim(&patch.user)
    {
        // The stored password was issued BY a particular server TO a particular
        // account: pointing the block elsewhere forgets it.
        set_setting(&state.db, keys::SMTP_PASSWORD_ENC, None).await?;
        changed.push(keys::SMTP_PASSWORD_ENC.to_string());
    }

    invalidate_mail_settings();
    Ok((changed, password_changed))
}

/// `clearMailSettings()`: remove every panel row of the SMTP block.
pub async fn clear_mail_settings(state: &AppState) -> sqlx::Result<()> {
    for key in SMTP_SETTING_KEYS {
        if *key == keys::EMAIL_VERIFICATION_REQUIRED {
            continue; // not part of the SMTP block
        }
        set_setting(&state.db, key, None).await?;
    }
    invalidate_mail_settings();
    Ok(())
}

/// `clearMailPassword()`.
pub async fn clear_mail_password(state: &AppState) -> sqlx::Result<()> {
    set_setting(&state.db, keys::SMTP_PASSWORD_ENC, None).await?;
    invalidate_mail_settings();
    Ok(())
}

/// `setEmailVerificationRequired(required)`.
pub async fn set_email_verification_required(state: &AppState, required: bool) -> sqlx::Result<()> {
    set_setting(&state.db, keys::EMAIL_VERIFICATION_REQUIRED, Some(if required { "on" } else { "off" })).await?;
    invalidate_mail_settings();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> EnvMail {
        EnvMail {
            host: "smtp.env.invalid".into(),
            port: 2525,
            user: "env-user".into(),
            password: "env-password".into(),
            secure: true,
            security: None,
            from: "env@example.invalid".into(),
            from_name: "Env Sender".into(),
        }
    }

    /// Stand-in for the real AES-GCM box: the precedence rule does not need it.
    fn decrypt(boxed: &str) -> Option<String> {
        if boxed == "BROKEN" { None } else { Some(boxed.trim_start_matches("enc:").to_string()) }
    }

    fn rows(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect()
    }

    fn resolve(r: &[(&str, &str)], e: &EnvMail) -> ResolvedMail {
        resolve_mail_settings(&rows(r), e, "Modlight", &decrypt)
    }

    // tests/unit/mail-settings.test.ts: "configuration source"
    #[test]
    fn falls_back_to_the_environment_when_no_row_exists() {
        let r = resolve(&[], &env());
        assert_eq!(r.source, MailSource::Env);
        assert_eq!(r.host, "smtp.env.invalid");
        assert_eq!(r.user, "env-user");
        assert_eq!(r.password, "env-password");
        assert!(r.ready);
    }

    #[test]
    fn reports_none_when_neither_source_is_configured() {
        let r = resolve(&[], &EnvMail { host: String::new(), ..env() });
        assert_eq!(r.source, MailSource::None);
        assert!(!r.ready);
    }

    #[test]
    fn takes_the_whole_block_from_the_panel_once_a_host_is_stored() {
        let r = resolve(
            &[
                ("smtp_host", "smtp.panel.invalid"),
                ("smtp_port", "587"),
                ("smtp_security", "starttls"),
                ("smtp_user", "panel-user"),
                ("smtp_password_enc", "enc:panel-password"),
                ("mail_from", "panel@example.invalid"),
            ],
            &env(),
        );
        assert_eq!(r.source, MailSource::Panel);
        assert_eq!(r.host, "smtp.panel.invalid");
        assert_eq!(r.port, 587);
        assert_eq!(r.security, SmtpSecurity::Starttls);
        assert_eq!(r.user, "panel-user");
        assert_eq!(r.password, "panel-password");
        assert_eq!(r.from, "panel@example.invalid");
    }

    #[test]
    fn never_lets_an_environment_credential_leak_into_a_panel_block() {
        let r = resolve(&[("smtp_host", "smtp.panel.invalid")], &env());
        assert_eq!(r.source, MailSource::Panel);
        assert_eq!(r.user, "");
        assert_eq!(r.password, "");
    }

    #[test]
    fn treats_an_unreadable_password_as_unusable_not_as_no_password() {
        let r = resolve(
            &[("smtp_host", "smtp.panel.invalid"), ("smtp_user", "panel-user"), ("smtp_password_enc", "BROKEN")],
            &env(),
        );
        assert_eq!(r.password_state, MailPasswordState::Undecryptable);
        assert!(!r.ready);
    }

    #[test]
    fn allows_an_unauthenticated_relay() {
        let r = resolve(&[("smtp_host", "relay.internal.invalid")], &env());
        assert!(r.ready);
        assert_eq!(r.password_state, MailPasswordState::Unset);
    }

    #[test]
    fn maps_smtp_secure_to_implicit_tls_and_its_absence_to_opportunistic() {
        assert_eq!(resolve(&[], &env()).security, SmtpSecurity::Tls);
        assert_eq!(resolve(&[], &EnvMail { secure: false, ..env() }).security, SmtpSecurity::None);
        assert_eq!(
            resolve(&[], &EnvMail { secure: false, security: Some(SmtpSecurity::Starttls), ..env() }).security,
            SmtpSecurity::Starttls
        );
    }

    // "address confirmation"
    #[test]
    fn is_off_unless_switched_on() {
        assert!(!resolve(&[], &env()).verification_required);
    }

    #[test]
    fn follows_the_stored_switch_when_mail_works() {
        assert!(resolve(&[("email_verification_required", "on")], &env()).verification_required);
    }

    #[test]
    fn is_forced_off_when_mail_cannot_be_sent() {
        let r = resolve(&[("email_verification_required", "on")], &EnvMail { host: String::new(), ..env() });
        assert!(!r.verification_required);
    }

    // "transport fingerprint"
    fn base() -> ResolvedMail {
        resolve(
            &[
                ("smtp_host", "smtp.panel.invalid"),
                ("smtp_user", "panel-user"),
                ("smtp_password_enc", "enc:first-password"),
            ],
            &env(),
        )
    }

    #[test]
    fn fingerprint_changes_when_only_the_password_changes() {
        let next = resolve(
            &[
                ("smtp_host", "smtp.panel.invalid"),
                ("smtp_user", "panel-user"),
                ("smtp_password_enc", "enc:second-password"),
            ],
            &env(),
        );
        assert_ne!(mail_fingerprint(&next), mail_fingerprint(&base()));
    }

    #[test]
    fn fingerprint_is_stable_for_an_unchanged_configuration() {
        assert_eq!(mail_fingerprint(&base()), mail_fingerprint(&base()));
    }

    #[test]
    fn fingerprint_never_contains_the_password_itself() {
        assert!(!mail_fingerprint(&base()).contains("first-password"));
    }

    // "what may reach the browser"
    #[test]
    fn has_no_panel_key_whose_name_suggests_a_stored_secret() {
        for key in crate::settings::PANEL_SETTINGS {
            assert!(!key.ends_with("_enc"), "{key}");
            assert!(!key.contains("password"), "{key}");
        }
    }

    #[test]
    fn ports_and_security_parse_like_upstream() {
        assert_eq!(to_port(Some("465"), 587), 465);
        assert_eq!(to_port(Some("0"), 587), 587);
        assert_eq!(to_port(Some("70000"), 587), 587);
        assert_eq!(to_port(Some("25.9"), 587), 25);
        assert_eq!(to_port(None, 587), 587);
        assert_eq!(SmtpSecurity::parse_or_starttls(Some("bogus")), SmtpSecurity::Starttls);
        let status = serde_json::to_value(MailStatus {
            source: MailSource::None,
            host: String::new(),
            port: 587,
            security: SmtpSecurity::Starttls,
            user: String::new(),
            from: "a".into(),
            from_name: "b".into(),
            ready: false,
            password_state: MailPasswordState::Unset,
            verification_required: false,
            env_available: false,
        })
        .unwrap();
        let keys: Vec<&String> = status.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "source",
                "host",
                "port",
                "security",
                "user",
                "from",
                "fromName",
                "ready",
                "passwordState",
                "verificationRequired",
                "envAvailable"
            ]
        );
    }
}
