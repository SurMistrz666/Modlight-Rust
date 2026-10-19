//! Account passwords (Better Auth `emailAndPassword.password`, as configured
//! in lib/auth.ts).
//!
//! - **hash** (`@better-auth/utils` 0.4.1 `password.node.mjs`): `${salt}:${key}`
//!   where `salt` = 16 random bytes as lower-case hex, and the HEX STRING (not
//!   its bytes) is the scrypt salt; `key` = scrypt(password NFKC-normalised,
//!   salt, N=16384, r=16, p=1, dkLen=64) as hex.
//! - **verify**: split on `:`, recompute, compare; a hash without both parts
//!   is an error ("Invalid password hash"), which Better Auth lets escape as a
//!   500.
//! - **legacy** (`verifyUserPassword` in lib/auth.ts): hashes matching
//!   `^\$2[aby]\$` are PHP `password_hash()` bcrypt. Accounts imported from a
//!   PHP install keep signing in with the password they already have; bcrypt
//!   only ever considered the first 72 bytes of a password, which is baked into
//!   the hashes we inherited.
//!
//! The hashes live in `account.password` (provider_id `credential`). Per-file
//! and default upload passwords are a different scheme ([`crate::password`]).
//!
//! scrypt with these parameters costs ~16 MB and tens of milliseconds, so it
//! runs on the blocking pool, like Node's libuv thread pool.

use rand::RngCore;
use rand::rngs::OsRng;
use unicode_normalization::UnicodeNormalization;

/// Better Auth's default bounds (`minPasswordLength` / `maxPasswordLength`).
pub const MIN_PASSWORD_LENGTH: usize = 8;
pub const MAX_PASSWORD_LENGTH: usize = 128;

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("Invalid password hash")]
    InvalidHash,
    #[error("password hashing failed: {0}")]
    Internal(String),
}

fn scrypt_key(password: &str, salt: &str) -> Result<[u8; 64], PasswordError> {
    let params = scrypt::Params::new(14, 16, 1, 64).map_err(|e| PasswordError::Internal(e.to_string()))?;
    let normalized: String = password.nfkc().collect();
    let mut key = [0u8; 64];
    scrypt::scrypt(normalized.as_bytes(), salt.as_bytes(), &params, &mut key)
        .map_err(|e| PasswordError::Internal(e.to_string()))?;
    Ok(key)
}

/// `hashPassword(password)` (synchronous; see [`hash_password`]).
pub fn hash_password_blocking(password: &str) -> Result<String, PasswordError> {
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let salt = hex::encode(salt);
    let key = scrypt_key(password, &salt)?;
    Ok(format!("{salt}:{}", hex::encode(key)))
}

/// `verifyPassword(hash, password)` (synchronous; see [`verify_user_password`]).
pub fn verify_password_blocking(hash: &str, password: &str) -> Result<bool, PasswordError> {
    let mut parts = hash.split(':');
    let (Some(salt), Some(key)) = (parts.next(), parts.next()) else {
        return Err(PasswordError::InvalidHash);
    };
    if salt.is_empty() || key.is_empty() {
        return Err(PasswordError::InvalidHash);
    }
    let target = hex::encode(scrypt_key(password, salt)?);
    Ok(crate::auth::crypto::constant_time_eq(target.as_bytes(), key.as_bytes()))
}

/// PHP `password_hash()` output: `$2y$`, plus the `$2a$`/`$2b$` variants.
pub fn is_legacy_bcrypt(hash: &str) -> bool {
    let b = hash.as_bytes();
    b.len() >= 4 && b[0] == b'$' && b[1] == b'2' && matches!(b[2], b'a' | b'b' | b'y') && b[3] == b'$'
}

/// `verifyUserPassword({ hash, password })` (synchronous).
pub fn verify_user_password_blocking(hash: &str, password: &str) -> Result<bool, PasswordError> {
    if is_legacy_bcrypt(hash) {
        // bcryptjs.compare: a malformed hash is a mismatch, not an error.
        return Ok(bcrypt::verify(password, hash).unwrap_or(false));
    }
    verify_password_blocking(hash, password)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> anyhow::Result<T> {
    Ok(tokio::task::spawn_blocking(f).await?)
}

/// `ctx.context.password.hash` (Better Auth's `hashPassword`).
pub async fn hash_password(password: &str) -> anyhow::Result<String> {
    let password = password.to_string();
    Ok(blocking(move || hash_password_blocking(&password)).await??)
}

/// `ctx.context.password.verify` = lib/auth.ts `verifyUserPassword`.
pub async fn verify_user_password(hash: &str, password: &str) -> anyhow::Result<bool> {
    let (hash, password) = (hash.to_string(), password.to_string());
    Ok(blocking(move || verify_user_password_blocking(&hash, &password)).await??)
}

#[cfg(test)]
mod tests {
    use super::*;

    // tests/unit/auth-password.test.ts: the native path and the legacy path.
    #[test]
    fn scrypt_round_trip_in_better_auths_format() {
        let hash = hash_password_blocking("correct horse").unwrap();
        let (salt, key) = hash.split_once(':').unwrap();
        assert_eq!(salt.len(), 32);
        assert_eq!(key.len(), 128);
        assert!(verify_password_blocking(&hash, "correct horse").unwrap());
        assert!(!verify_password_blocking(&hash, "wrong horse").unwrap());
        assert!(verify_password_blocking("nocolon", "x").is_err());
    }

    #[test]
    fn nfkc_normalises_before_hashing() {
        // "ﬁ" (U+FB01) normalises to "fi" under NFKC.
        let hash = hash_password_blocking("\u{FB01}nal").unwrap();
        assert!(verify_password_blocking(&hash, "final").unwrap());
    }

    #[test]
    fn legacy_bcrypt_hashes_still_verify() {
        let hash = bcrypt::hash("hunter22", 4).unwrap();
        // The bcrypt crate writes $2b$; PHP writes $2y$ for the same algorithm.
        let php = hash.replacen("$2b$", "$2y$", 1);
        assert!(is_legacy_bcrypt(&php));
        assert!(verify_user_password_blocking(&php, "hunter22").unwrap());
        assert!(!verify_user_password_blocking(&php, "hunter23").unwrap());
        assert!(verify_user_password_blocking(&hash, "hunter22").unwrap());
        let native = hash_password_blocking("hunter22").unwrap();
        assert!(!is_legacy_bcrypt(&native));
        assert!(verify_user_password_blocking(&native, "hunter22").unwrap());
    }

    #[test]
    fn bcrypt_considers_only_72_bytes() {
        let long = "a".repeat(80);
        let hash = bcrypt::hash(&long, 4).unwrap().replacen("$2b$", "$2y$", 1);
        assert!(verify_user_password_blocking(&hash, &"a".repeat(72)).unwrap());
    }

    /// `$2a$` hashes from older PHP / bcryptjs are accepted too.
    #[test]
    fn two_a_prefix_is_accepted() {
        let hash = bcrypt::hash("pw-123456", 4).unwrap().replacen("$2b$", "$2a$", 1);
        assert!(verify_user_password_blocking(&hash, "pw-123456").unwrap());
    }
}
