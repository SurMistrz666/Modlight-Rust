//! Port of `lib/registration.ts`: who is allowed to create an account, and on
//! what terms.
//!
//! The single most important thing in this file is the SEMANTICS:
//! `register_enabled` means "a stranger may sign themselves up at /register".
//! It is NOT a global "no accounts may be created" switch: an administrator
//! creating an account from the panel is a different act, through an endpoint
//! with its own authorisation check. That is why the gate is attached to the
//! `/sign-up/email` path and not to the user-create database hook.
//!
//! The bootstrap case is the second exception: on an empty install there is
//! no administrator to invite anybody, so the first account is always allowed
//! and becomes the admin.

use serde::Serialize;
use sqlx::PgPool;

use crate::password::{check_rate_limit, reset_rate_limit};
use crate::secret_box::{SecretPurpose, digests_equal, encrypt_secret, secret_digest, try_decrypt_secret};
use crate::settings::{get_setting, is_registration_enabled, keys, set_setting};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SignUpRefusal {
    Closed,
    KeyRequired,
    KeyInvalid,
    RateLimited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignUpDecision {
    Allow,
    Refuse(SignUpRefusal),
}

/// Everything [`decide_sign_up`] looks at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignUpFacts {
    /// No account exists yet: this sign-up creates the administrator.
    pub is_first_account: bool,
    /// `settings.register_enabled`.
    pub registration_enabled: bool,
    /// Digest of the configured registration key, or `None` when no key is set.
    pub expected_key_digest: Option<String>,
    /// A key IS configured but cannot be read back. Carried separately because
    /// "no key" and "unreadable key" both arrive as a missing digest, and
    /// collapsing them is a fail-open.
    pub key_unreadable: bool,
    /// Digest of what the visitor typed, or `None` when they sent no key.
    pub supplied_key_digest: Option<String>,
    /// Whether the caller has already burnt through its attempt budget.
    pub rate_limited: bool,
}

/// Decide a self-service sign-up. Order matters and is deliberate:
///
///  1. **Bootstrap wins over everything.**
///  2. **Rate limit before any secret is looked at.**
///  3. **Closed before key.** A key does NOT open a closed registration.
pub fn decide_sign_up(facts: &SignUpFacts) -> SignUpDecision {
    if facts.is_first_account {
        return SignUpDecision::Allow;
    }
    if facts.rate_limited {
        return SignUpDecision::Refuse(SignUpRefusal::RateLimited);
    }
    if !facts.registration_enabled {
        return SignUpDecision::Refuse(SignUpRefusal::Closed);
    }
    // A key that exists but cannot be read closes the door rather than opening it.
    if facts.key_unreadable {
        return SignUpDecision::Refuse(SignUpRefusal::KeyInvalid);
    }
    let Some(expected) = &facts.expected_key_digest else {
        return SignUpDecision::Allow;
    };
    let Some(supplied) = &facts.supplied_key_digest else {
        return SignUpDecision::Refuse(SignUpRefusal::KeyRequired);
    };
    if !digests_equal(supplied, expected) {
        return SignUpDecision::Refuse(SignUpRefusal::KeyInvalid);
    }
    SignUpDecision::Allow
}

// ── The registration key ────────────────────────────────────────────────────

pub const REGISTRATION_KEY_MAX: usize = 200;

/// `setRegistrationKey(key)`: stored ENCRYPTED rather than hashed, because this
/// key exists to be handed out and has to be readable by an administrator.
pub async fn set_registration_key(db: &PgPool, auth_secret: &str, key: Option<&str>) -> anyhow::Result<()> {
    let trimmed = crate::config::js_trim(key.unwrap_or(""));
    let value = if trimmed.is_empty() {
        None
    } else {
        Some(encrypt_secret(auth_secret, trimmed, SecretPurpose::RegistrationKey)?)
    };
    set_setting(db, keys::REGISTRATION_KEY_ENC, value.as_deref()).await?;
    Ok(())
}

/// `readRegistrationKey()`: the plaintext key, or `None` when none is set or it
/// can no longer be decrypted.
pub async fn read_registration_key(db: &PgPool, auth_secret: &str) -> sqlx::Result<Option<String>> {
    let boxed = get_setting(db, keys::REGISTRATION_KEY_ENC).await?;
    Ok(try_decrypt_secret(auth_secret, boxed.as_deref(), SecretPurpose::RegistrationKey))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RegistrationKeyState {
    Unset,
    Set,
    Undecryptable,
}

/// `registrationKeyState()`: what the panel is allowed to know.
pub async fn registration_key_state(db: &PgPool, auth_secret: &str) -> sqlx::Result<RegistrationKeyState> {
    let boxed = get_setting(db, keys::REGISTRATION_KEY_ENC).await?;
    Ok(match boxed.filter(|b| !b.is_empty()) {
        None => RegistrationKeyState::Unset,
        Some(b) => {
            if try_decrypt_secret(auth_secret, Some(&b), SecretPurpose::RegistrationKey).is_some() {
                RegistrationKeyState::Set
            } else {
                RegistrationKeyState::Undecryptable
            }
        }
    })
}

/// `isRegistrationKeyRequired()`: an undecryptable key counts as required.
pub async fn is_registration_key_required(db: &PgPool, auth_secret: &str) -> sqlx::Result<bool> {
    Ok(registration_key_state(db, auth_secret).await? != RegistrationKeyState::Unset)
}

/// `isFirstAccount()`: is the users table empty?
pub async fn is_first_account(db: &PgPool) -> sqlx::Result<bool> {
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM \"user\"").fetch_one(db).await?;
    Ok(n == 0)
}

/// `registrationPageState()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrationPageState {
    pub open: bool,
    pub bootstrap: bool,
    pub key_required: bool,
}

/// May /register show a sign-up form at all? Same two-part answer as the gate,
/// so the page and the endpoint can never disagree.
pub async fn registration_page_state(
    db: &PgPool,
    auth_secret: &str,
    env_registration_enabled: bool,
) -> sqlx::Result<RegistrationPageState> {
    let bootstrap = is_first_account(db).await?;
    let enabled = is_registration_enabled(db, env_registration_enabled).await?;
    let key_required = is_registration_key_required(db, auth_secret).await?;
    Ok(RegistrationPageState {
        open: bootstrap || enabled,
        bootstrap,
        // The first account is the administrator setting the key up.
        key_required: key_required && !bootstrap,
    })
}

/// Key attempts allowed per caller, and the window they are counted over.
const KEY_ATTEMPTS: u32 = 5;
const KEY_WINDOW_MS: i64 = 15 * 60 * 1000;

/// `evaluateSignUp({ clientIp, suppliedKey })`: one sign-up attempt. The
/// throttle is scoped to installs that HAVE a key: where there is no secret
/// there is nothing to guess.
pub async fn evaluate_sign_up(
    db: &PgPool,
    auth_secret: &str,
    env_registration_enabled: bool,
    client_ip: &str,
    supplied_key: Option<&str>,
) -> anyhow::Result<SignUpDecision> {
    let bootstrap = is_first_account(db).await?;
    let enabled = is_registration_enabled(db, env_registration_enabled).await?;
    let key_state = registration_key_state(db, auth_secret).await?;
    let expected_key = read_registration_key(db, auth_secret).await?.filter(|k| !k.is_empty());

    let throttle_key = format!("{client_ip}|signup-key");
    let rate_limited =
        !bootstrap && expected_key.is_some() && !check_rate_limit(&throttle_key, KEY_ATTEMPTS, KEY_WINDOW_MS).allowed;

    let supplied = crate::config::js_trim(supplied_key.unwrap_or("")).to_string();
    // Digested unconditionally: a correct key and a wrong key of any length must cost the same.
    let supplied_digest = secret_digest(auth_secret, &supplied, SecretPurpose::RegistrationKey)?;

    let decision = decide_sign_up(&SignUpFacts {
        is_first_account: bootstrap,
        registration_enabled: enabled,
        expected_key_digest: match &expected_key {
            Some(k) => Some(secret_digest(auth_secret, k, SecretPurpose::RegistrationKey)?),
            None => None,
        },
        key_unreadable: key_state == RegistrationKeyState::Undecryptable,
        supplied_key_digest: if supplied.is_empty() { None } else { Some(supplied_digest) },
        rate_limited,
    });

    // A correct key clears the budget, so onboarding colleagues from one office
    // does not lock out the sixth.
    if decision == SignUpDecision::Allow && expected_key.is_some() {
        reset_rate_limit(&throttle_key);
    }
    Ok(decision)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-secret-test-secret-test-secret-0123";
    const KEY: &str = "shared-key-for-the-team";

    fn digest(v: &str) -> String {
        secret_digest(SECRET, v, SecretPurpose::RegistrationKey).unwrap()
    }

    fn facts() -> SignUpFacts {
        SignUpFacts { registration_enabled: true, ..Default::default() }
    }

    fn refuse(r: SignUpRefusal) -> SignUpDecision {
        SignUpDecision::Refuse(r)
    }

    // tests/unit/registration-gate.test.ts: "self-service sign-up decision"
    #[test]
    fn lets_a_stranger_in_when_registration_is_open_and_no_key_is_set() {
        assert_eq!(decide_sign_up(&facts()), SignUpDecision::Allow);
    }

    #[test]
    fn refuses_a_stranger_when_registration_is_closed() {
        assert_eq!(
            decide_sign_up(&SignUpFacts { registration_enabled: false, ..facts() }),
            refuse(SignUpRefusal::Closed)
        );
    }

    #[test]
    fn always_allows_the_very_first_account_whatever_the_flag_says() {
        let f = SignUpFacts { is_first_account: true, registration_enabled: false, ..facts() };
        assert_eq!(decide_sign_up(&f), SignUpDecision::Allow);
    }

    #[test]
    fn does_not_ask_the_first_account_for_a_key() {
        let f = SignUpFacts {
            is_first_account: true,
            registration_enabled: false,
            expected_key_digest: Some(digest(KEY)),
            ..facts()
        };
        assert_eq!(decide_sign_up(&f), SignUpDecision::Allow);
    }

    // "registration key"
    #[test]
    fn accepts_the_right_key() {
        let f =
            SignUpFacts { expected_key_digest: Some(digest(KEY)), supplied_key_digest: Some(digest(KEY)), ..facts() };
        assert_eq!(decide_sign_up(&f), SignUpDecision::Allow);
    }

    #[test]
    fn rejects_a_wrong_key() {
        let f = SignUpFacts {
            expected_key_digest: Some(digest(KEY)),
            supplied_key_digest: Some(digest("nope")),
            ..facts()
        };
        assert_eq!(decide_sign_up(&f), refuse(SignUpRefusal::KeyInvalid));
    }

    #[test]
    fn rejects_a_missing_key_when_one_is_configured() {
        let f = SignUpFacts { expected_key_digest: Some(digest(KEY)), ..facts() };
        assert_eq!(decide_sign_up(&f), refuse(SignUpRefusal::KeyRequired));
    }

    #[test]
    fn ignores_a_key_that_was_sent_when_none_is_configured() {
        let f = SignUpFacts { supplied_key_digest: Some(digest("anything")), ..facts() };
        assert_eq!(decide_sign_up(&f), SignUpDecision::Allow);
    }

    #[test]
    fn fails_closed_when_the_configured_key_can_no_longer_be_decrypted() {
        let f = SignUpFacts { key_unreadable: true, ..facts() };
        assert_eq!(decide_sign_up(&f), refuse(SignUpRefusal::KeyInvalid));
    }

    #[test]
    fn does_not_let_a_supplied_key_past_an_unreadable_one() {
        let f = SignUpFacts { key_unreadable: true, supplied_key_digest: Some(digest(KEY)), ..facts() };
        assert_eq!(decide_sign_up(&f), refuse(SignUpRefusal::KeyInvalid));
    }

    #[test]
    fn still_lets_the_first_account_through_an_unreadable_key() {
        let f = SignUpFacts { is_first_account: true, key_unreadable: true, ..facts() };
        assert_eq!(decide_sign_up(&f), SignUpDecision::Allow);
    }

    #[test]
    fn does_not_open_a_closed_registration() {
        let f = SignUpFacts {
            registration_enabled: false,
            expected_key_digest: Some(digest(KEY)),
            supplied_key_digest: Some(digest(KEY)),
            ..facts()
        };
        assert_eq!(decide_sign_up(&f), refuse(SignUpRefusal::Closed));
    }

    // "throttling"
    #[test]
    fn refuses_before_the_key_is_even_looked_at() {
        let f = SignUpFacts {
            rate_limited: true,
            expected_key_digest: Some(digest(KEY)),
            supplied_key_digest: Some(digest(KEY)),
            ..facts()
        };
        assert_eq!(decide_sign_up(&f), refuse(SignUpRefusal::RateLimited));
    }

    #[test]
    fn does_not_throttle_the_bootstrap_account() {
        let f = SignUpFacts { is_first_account: true, rate_limited: true, ..facts() };
        assert_eq!(decide_sign_up(&f), SignUpDecision::Allow);
    }
}
