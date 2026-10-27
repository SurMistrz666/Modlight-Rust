//! Everything that takes the file password: switching encryption on, opening
//! the vault (a grant for a browser or an app), changing the password,
//! switching encryption off. Shared by the RPC module `encryption` and the
//! `/api/encryption` routes.

use uuid::Uuid;
use zeroize::Zeroizing;

use super::access::encode_grant;
use super::keys::{
    AccountSecret, KdfParams, KeyError, box_for_job, derive_password_key, new_grant_secret, new_salt, public_key_mac,
    server_key, unwrap_with_password_key, wrap_for_grant, wrap_with_password_key,
};
use super::store::{self, GrantKind, NewGrant, NewVault};
use super::{
    MAX_PASSWORD_BYTES, MIN_PASSWORD_CHARS, REMEMBER_TTL_SECONDS, SESSION_TTL_SECONDS, VaultState, cookie_name,
};
use crate::db::models::FileVault;
use crate::db::types::JsDate;
use crate::http::set_cookie::{SameSite, SetCookie};
use crate::password::{check_rate_limit, reset_rate_limit};
use crate::state::AppState;

const WINDOW_MS: i64 = 15 * 60 * 1000;
/// Attempts per address and account.
pub const PER_ADDRESS: u32 = 10;
/// Visitor attempts per account, whatever the address.
pub const PER_ACCOUNT_VISITORS: u32 = 50;
/// The owner's own attempts (panel or app): a bucket strangers cannot fill.
pub const PER_OWNER: u32 = 10;

/// Who is typing the password: decides the rate-limit buckets and the kind
/// of grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asker {
    /// The account holder in the panel.
    Owner,
    /// Somebody who opened a link.
    Visitor,
    /// A bearer client of the account.
    App,
}

impl Asker {
    fn grant_kind(self) -> GrantKind {
        match self {
            Asker::Owner => GrantKind::Owner,
            Asker::Visitor => GrantKind::Visitor,
            Asker::App => GrantKind::App,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnlockError {
    #[error("wrong password")]
    Wrong,
    #[error("too many attempts")]
    RateLimited { retry_after_ms: i64 },
    #[error("this account's files are not encrypted")]
    NotEncrypted,
}

/// Check the buckets of one attempt (every attempt counts, refused ones too).
pub fn admit_attempt(user_id: &str, asker: Asker, ip: &str) -> Result<(), UnlockError> {
    let mut wait = 0i64;
    let mut check = |key: String, max: u32| {
        let r = check_rate_limit(&key, max, WINDOW_MS);
        if !r.allowed {
            wait = wait.max(r.retry_after_ms);
        }
    };
    match asker {
        Asker::Owner => check(format!("vault-owner|{user_id}"), PER_OWNER),
        Asker::App => check(format!("vault-app|{user_id}"), PER_OWNER),
        Asker::Visitor => {
            check(format!("vault-ip|{ip}|{user_id}"), PER_ADDRESS);
            check(format!("vault-visitors|{user_id}"), PER_ACCOUNT_VISITORS);
        }
    }
    if wait > 0 { Err(UnlockError::RateLimited { retry_after_ms: wait }) } else { Ok(()) }
}

fn forgive(user_id: &str, asker: Asker, ip: &str) {
    match asker {
        Asker::Owner => reset_rate_limit(&format!("vault-owner|{user_id}")),
        Asker::App => reset_rate_limit(&format!("vault-app|{user_id}")),
        Asker::Visitor => reset_rate_limit(&format!("vault-ip|{ip}|{user_id}")),
    }
}

fn params_of(vault: &FileVault) -> KdfParams {
    KdfParams {
        memory_kib: vault.kdf_memory_kib.max(0) as u32,
        iterations: vault.kdf_iterations.max(0) as u32,
        parallelism: vault.kdf_parallelism.max(0) as u32,
    }
}

/// Open the account's private key with the file password. A wrong password
/// and a damaged record are the same answer.
pub async fn open_with_password(
    vault: &FileVault,
    password: &str,
) -> anyhow::Result<Result<AccountSecret, UnlockError>> {
    if password.is_empty() || password.len() > MAX_PASSWORD_BYTES {
        return Ok(Err(UnlockError::Wrong));
    }
    let key = match derive_password_key(password, vault.kdf_salt.clone(), params_of(vault)).await? {
        Ok(k) => k,
        Err(_) => return Ok(Err(UnlockError::Wrong)),
    };
    Ok(unwrap_with_password_key(&vault.wrapped_key, &key, &vault.user_id, &vault.public_key)
        .map_err(|_| UnlockError::Wrong))
}

/// A new grant: what the browser (cookie) or the app (header) keeps.
pub struct Granted {
    pub id: Uuid,
    pub value: Zeroizing<String>,
    pub expires_at: JsDate,
    pub remember: bool,
}

/// Store a grant of `secret` for `vault`'s account.
pub async fn grant(
    state: &AppState,
    vault: &FileVault,
    secret: &AccountSecret,
    kind: GrantKind,
    remember: bool,
    label: Option<&str>,
    ip: Option<&str>,
) -> anyhow::Result<Granted> {
    let id = Uuid::new_v4();
    let grant_secret = new_grant_secret();
    let server = server_key(&state.config.auth.secret)?;
    let wrapped = wrap_for_grant(secret, &server, &grant_secret, &id.to_string(), &vault.user_id);
    let ttl = if remember { REMEMBER_TTL_SECONDS } else { SESSION_TTL_SECONDS };
    let expires_at = JsDate::from_millis(JsDate::now().millis() + ttl * 1000)
        .ok_or_else(|| anyhow::anyhow!("grant expiry out of range"))?;
    store::insert_grant(
        &state.db,
        &NewGrant {
            id,
            user_id: &vault.user_id,
            kind,
            generation: vault.generation,
            wrapped_key: &wrapped,
            label,
            ip,
            expires_at,
        },
    )
    .await?;
    Ok(Granted { id, value: encode_grant(id, &grant_secret), expires_at, remember })
}

/// The browser's cookie for a grant of `owner_id`: httpOnly, Lax, Secure in
/// production; 30 days when remembered, else a session cookie.
pub fn grant_cookie(state: &AppState, owner_id: &str, granted: &Granted) -> SetCookie {
    let cookie = SetCookie::new(cookie_name(owner_id), granted.value.as_str())
        .http_only(true)
        .path("/")
        .same_site(SameSite::Lax)
        .secure(state.config.is_prod);
    if granted.remember { cookie.max_age(REMEMBER_TTL_SECONDS) } else { cookie }
}

/// The deletion of that cookie.
pub fn clear_cookie(state: &AppState, owner_id: &str) -> SetCookie {
    SetCookie::new(cookie_name(owner_id), "")
        .http_only(true)
        .path("/")
        .same_site(SameSite::Lax)
        .secure(state.config.is_prod)
        .max_age(0)
}

/// Unlock `vault` for one browser or app.
pub async fn unlock(
    state: &AppState,
    vault: &FileVault,
    password: &str,
    asker: Asker,
    ip: &str,
    remember: bool,
    label: Option<&str>,
) -> anyhow::Result<Result<Granted, UnlockError>> {
    if !store::state_of(vault).is_some_and(VaultState::is_gated) {
        return Ok(Err(UnlockError::NotEncrypted));
    }
    if let Err(e) = admit_attempt(&vault.user_id, asker, ip) {
        return Ok(Err(e));
    }
    let secret = match open_with_password(vault, password).await? {
        Ok(s) => s,
        Err(e) => return Ok(Err(e)),
    };
    forgive(&vault.user_id, asker, ip);
    // A visitor's address is theirs: the owner's list names the browser only.
    let ip = if asker == Asker::Visitor { None } else { Some(ip) };
    let granted = grant(state, vault, &secret, asker.grant_kind(), remember, label, ip).await?;
    Ok(Ok(granted))
}

// ── Switching on, changing, switching off ───────────────────────────────────

/// Why a new file password is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordProblem {
    TooShort,
    TooLong,
    /// Equal to the account's sign-in password (link visitors type this one).
    SameAsSignIn,
}

impl PasswordProblem {
    pub fn as_str(self) -> &'static str {
        match self {
            PasswordProblem::TooShort => "too-short",
            PasswordProblem::TooLong => "too-long",
            PasswordProblem::SameAsSignIn => "same-as-sign-in",
        }
    }
}

/// The shape rules of a new file password, then "not the sign-in password".
pub async fn check_new_password(
    state: &AppState,
    user_id: &str,
    password: &str,
) -> anyhow::Result<Option<PasswordProblem>> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Ok(Some(PasswordProblem::TooShort));
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Ok(Some(PasswordProblem::TooLong));
    }
    let hash: Option<Option<String>> = sqlx::query_scalar(
        r#"select "password" from "account" where "user_id" = $1 and "provider_id" = 'credential' limit 1"#,
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?;
    if let Some(Some(hash)) = hash
        && crate::auth::password::verify_user_password(&hash, password).await.unwrap_or(false)
    {
        return Ok(Some(PasswordProblem::SameAsSignIn));
    }
    Ok(None)
}

/// Parameters for new vaults and new passwords.
pub fn new_vault_params() -> KdfParams {
    KdfParams::DEFAULT
}

/// Create the vault of `user_id` (state `encrypting`) and a grant for the
/// browser that did it. None when the account already has one.
pub async fn create_vault(
    state: &AppState,
    user_id: &str,
    password: &str,
    params: KdfParams,
    label: Option<&str>,
    ip: &str,
) -> anyhow::Result<Option<(FileVault, Granted)>> {
    let secret = AccountSecret::generate();
    let public_key = secret.public_key();
    let salt = new_salt();
    let key = derive_password_key(password, salt.clone(), params)
        .await?
        .map_err(|e: KeyError| anyhow::anyhow!("key derivation failed: {e}"))?;
    let wrapped = wrap_with_password_key(&secret, &key, user_id, &public_key);
    let server = server_key(&state.config.auth.secret)?;
    let mac = public_key_mac(&server, user_id, &public_key);
    let made = store::insert(
        &state.db,
        &NewVault {
            user_id,
            public_key: &public_key,
            public_key_mac: &mac,
            wrapped_key: &wrapped,
            kdf_salt: &salt,
            params,
        },
    )
    .await?;
    if !made {
        return Ok(None);
    }
    let vault = store::load(&state.db, user_id).await?.ok_or_else(|| anyhow::anyhow!("vault vanished"))?;
    let granted = grant(state, &vault, &secret, GrantKind::Owner, true, label, Some(ip)).await?;
    super::pass::kick();
    Ok(Some((vault, granted)))
}

/// Re-wrap the private key under a new password; every grant is revoked and
/// the generation goes up. Returns the vault as it is now.
pub async fn change_password(
    state: &AppState,
    vault: &FileVault,
    secret: &AccountSecret,
    new_password: &str,
) -> anyhow::Result<FileVault> {
    let params = new_vault_params().max_of(params_of(vault));
    let salt = new_salt();
    let key = derive_password_key(new_password, salt.clone(), params)
        .await?
        .map_err(|e: KeyError| anyhow::anyhow!("key derivation failed: {e}"))?;
    let wrapped = wrap_with_password_key(secret, &key, &vault.user_id, &vault.public_key);
    let mut tx = state.db.begin().await?;
    sqlx::query(
        r#"update "file_vaults" set "wrapped_key" = $1, "kdf_salt" = $2, "kdf_memory_kib" = $3, "kdf_iterations" = $4,
             "kdf_parallelism" = $5, "generation" = "generation" + 1, "updated_at" = now()
           where "user_id" = $6 and "generation" = $7"#,
    )
    .bind(&wrapped)
    .bind(&salt)
    .bind(params.memory_kib as i32)
    .bind(params.iterations as i32)
    .bind(params.parallelism as i32)
    .bind(&vault.user_id)
    .bind(vault.generation)
    .execute(&mut *tx)
    .await?;
    store::delete_all_grants(&mut *tx, &vault.user_id).await?;
    tx.commit().await?;
    store::load(&state.db, &vault.user_id).await?.ok_or_else(|| anyhow::anyhow!("vault vanished"))
}

/// Switch encryption off: the pass decrypts every file back with the private
/// key kept in the secret box meanwhile, then deletes the vault.
pub async fn start_decrypting(state: &AppState, vault: &FileVault, secret: &AccountSecret) -> anyhow::Result<()> {
    let boxed = box_for_job(secret, &state.config.auth.secret)?;
    let mut tx = state.db.begin().await?;
    sqlx::query(
        r#"update "file_vaults" set "state" = 'decrypting', "job_key" = $1, "job_error" = null, "updated_at" = now()
           where "user_id" = $2"#,
    )
    .bind(&boxed)
    .bind(&vault.user_id)
    .execute(&mut *tx)
    .await?;
    store::delete_all_grants(&mut *tx, &vault.user_id).await?;
    tx.commit().await?;
    super::pass::kick();
    Ok(())
}

impl KdfParams {
    /// The stronger of two parameter sets, field by field (a password change
    /// never weakens an existing vault).
    pub fn max_of(self, other: KdfParams) -> KdfParams {
        if !other.sane() {
            return self;
        }
        KdfParams {
            memory_kib: self.memory_kib.max(other.memory_kib),
            iterations: self.iterations.max(other.iterations),
            parallelism: self.parallelism.max(other.parallelism),
        }
    }
}

/// The audit's view of an asker.
pub fn audit_detail(asker: Asker, grant: Option<Uuid>) -> serde_json::Value {
    serde_json::json!({ "by": match asker { Asker::Owner => "owner", Asker::Visitor => "visitor", Asker::App => "app" },
                        "grant": grant.map(|g| g.to_string()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visitor_buckets_do_not_touch_the_owners() {
        let user = format!("rl-{}", rand::random::<u64>());
        for _ in 0..PER_ADDRESS {
            assert!(admit_attempt(&user, Asker::Visitor, "203.0.113.1").is_ok());
        }
        assert!(matches!(admit_attempt(&user, Asker::Visitor, "203.0.113.1"), Err(UnlockError::RateLimited { .. })));
        // Another address still gets in, and so does the owner.
        assert!(admit_attempt(&user, Asker::Visitor, "203.0.113.2").is_ok());
        assert!(admit_attempt(&user, Asker::Owner, "203.0.113.1").is_ok());
        // Spread over many addresses, the account's visitor bucket runs out.
        let mut refused = false;
        for i in 0..(PER_ACCOUNT_VISITORS + 5) {
            if admit_attempt(&user, Asker::Visitor, &format!("198.51.100.{i}")).is_err() {
                refused = true;
            }
        }
        assert!(refused);
        assert!(admit_attempt(&user, Asker::Owner, "198.51.100.1").is_ok());
        // A success forgives the address's own bucket only.
        forgive(&user, Asker::Visitor, "203.0.113.1");
    }

    #[test]
    fn a_password_change_never_weakens_the_kdf() {
        let strong = KdfParams::DEFAULT;
        assert_eq!(KdfParams::TEST.max_of(strong), strong);
        assert_eq!(strong.max_of(KdfParams::TEST), strong);
        let insane = KdfParams { memory_kib: 0, iterations: 0, parallelism: 0 };
        assert_eq!(KdfParams::TEST.max_of(insane), KdfParams::TEST);
    }
}
