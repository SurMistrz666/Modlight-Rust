//! Per-account file encryption, the "file vault" (not upstream; the design,
//! the threat model and every deviation are in docs/ENCRYPTION.md).
//!
//! | Module | What |
//! |---|---|
//! | [`cipher`] | the stored form (chunked XChaCha20-Poly1305) and `uploads.encryption` |
//! | [`keys`] | account keys, Argon2id, sealed file keys, grants |
//! | [`store`] | the `file_vaults`, `vault_grants` and `vault_cleanup` rows |
//! | [`access`] | what a request may read: grants from the cookie or the header |
//! | [`view`] | the storage view that decrypts exactly what a request may read |
//! | [`write`] | encrypting new objects with the account's public key |
//! | [`gate`] | the locked answers (unlock form, 401, 423) |
//! | [`unlock`] | password checks, rate limits, grants for browsers and apps |
//! | [`pass`] | the background pass that encrypts or decrypts existing files |
//! | [`status`] | what the Settings card and the library banner show |

pub mod access;
pub mod cipher;
pub mod gate;
pub mod keys;
pub mod pass;
pub mod status;
pub mod store;
pub mod unlock;
pub mod view;
pub mod write;

/// Cookie carrying a browser's grant for one account: `ml_vault_<userId>`.
pub const COOKIE_PREFIX: &str = "ml_vault_";
/// Header carrying an API client's grant.
pub const UNLOCK_HEADER: &str = "x-modlight-unlock";
/// Set on every locked answer, whatever its status.
pub const LOCKED_HEADER: &str = "x-modlight-locked";
/// "Remember on this browser for 30 days".
pub const REMEMBER_TTL_SECONDS: i64 = 30 * 24 * 60 * 60;
/// A grant without "remember": a session cookie, and this long on the server.
pub const SESSION_TTL_SECONDS: i64 = 24 * 60 * 60;
/// File passwords are typed by link visitors: long enough to resist guessing
/// of a stolen database (with Argon2id), short enough to type.
pub const MIN_PASSWORD_CHARS: usize = 8;
/// Bounds the work a single request can ask Argon2 to hash.
pub const MAX_PASSWORD_BYTES: usize = 1024;

/// `file_vaults.state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultState {
    /// Switched on; the pass is encrypting the existing files.
    Encrypting,
    /// Every file is encrypted.
    On,
    /// Switched off; the pass is decrypting the files back.
    Decrypting,
}

impl VaultState {
    pub fn as_str(self) -> &'static str {
        match self {
            VaultState::Encrypting => "encrypting",
            VaultState::On => "on",
            VaultState::Decrypting => "decrypting",
        }
    }

    pub fn parse(s: &str) -> Option<VaultState> {
        match s {
            "encrypting" => Some(VaultState::Encrypting),
            "on" => Some(VaultState::On),
            "decrypting" => Some(VaultState::Decrypting),
            _ => None,
        }
    }

    /// Links ask for the password and new uploads are encrypted.
    pub fn is_gated(self) -> bool {
        matches!(self, VaultState::Encrypting | VaultState::On)
    }
}

/// The cookie of `user_id`'s grant.
pub fn cookie_name(user_id: &str) -> String {
    format!("{COOKIE_PREFIX}{user_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_round_trip() {
        for s in [VaultState::Encrypting, VaultState::On, VaultState::Decrypting] {
            assert_eq!(VaultState::parse(s.as_str()), Some(s));
        }
        assert_eq!(VaultState::parse("off"), None);
        assert!(VaultState::On.is_gated() && VaultState::Encrypting.is_gated());
        assert!(!VaultState::Decrypting.is_gated());
        assert_eq!(cookie_name("abc"), "ml_vault_abc");
    }
}
