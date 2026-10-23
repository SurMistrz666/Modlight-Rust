//! Not upstream: the server-side pickup that sits next to the loopback handoff
//! (SKELETON.md 9, "Desktop sign-in pickup").
//!
//! The loopback POST of [`super::loopback`] is made by the PAGE, from an https
//! origin to `http://127.0.0.1:<port>/`. Browsers increasingly refuse that:
//! Safari treats it as mixed content / a loopback request it will not make,
//! and Chrome's Local Network Access asks for a permission a person may have
//! dismissed once and never sees again. The app's listener is fine; the bytes
//! never leave the browser. So the app also asks the SERVER, which the browser
//! has no say in, and whichever answer arrives first wins.
//!
//! It is PKCE (RFC 7636) cut down to what this needs:
//!
//!   1. the app makes a random `verifier` (32 bytes, base64url) and sends
//!      `challenge = base64url(SHA-256(verifier))` in the authorize link;
//!   2. the page carries `challenge` to `approveDesktopClient` (or to
//!      `denyDesktopClient`), which store the outcome here under `state`;
//!   3. the app polls `POST /api/authorize/pickup` with `{state, verifier}`;
//!      the verifier never appeared in any URL or page, so only the process
//!      that generated the link can collect what the link produced.
//!
//! The entry is handed out ONCE and then deleted, lives five minutes, and the
//! map is bounded and swept. Everything is in memory: a restart loses pending
//! pickups, and the loopback POST and the manual key still work then.
//! Unknown, expired and mismatched requests all get the same 404, so the
//! route says nothing about which states exist.

use std::sync::{LazyLock, Mutex};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use indexmap::IndexMap;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::loopback::CHALLENGE_LEN;

/// How long an approved (or pending) entry waits for the app.
pub const PICKUP_TTL_MS: i64 = 5 * 60 * 1000;

/// Entries kept at once. Each needs a signed-in session to create, so this is
/// a ceiling against a runaway script, not a working limit.
pub const MAX_ENTRIES: usize = 4096;

/// A full sweep at most this often (and whenever the map is full).
const SWEEP_EVERY_MS: i64 = 30 * 1000;

/// What the person decided in the browser.
#[derive(Clone, PartialEq, Eq)]
pub enum Outcome {
    Token(String),
    Denied,
}

impl std::fmt::Debug for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::Token(_) => f.write_str("Token(<redacted>)"),
            Outcome::Denied => f.write_str("Denied"),
        }
    }
}

/// What a pickup request gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pickup {
    /// The verifier matched and the outcome is ready: handed out, entry gone.
    Ready(Outcome),
    /// The verifier matched; the person has not decided yet.
    Pending,
    /// Unknown state, expired entry or wrong verifier: deliberately one answer.
    NotFound,
}

struct Entry {
    challenge: String,
    outcome: Option<Outcome>,
    expires_at: i64,
}

#[derive(Default)]
struct Inner {
    map: IndexMap<String, Entry>,
    last_sweep: i64,
}

/// The pickup map. One per process ([`store`]); tests make their own.
#[derive(Default)]
pub struct PickupStore {
    inner: Mutex<Inner>,
}

/// `base64url(SHA-256(verifier))` without padding: what the app put in the link.
pub fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// A verifier as RFC 7636 spells one: 43 to 128 unreserved characters.
pub fn is_plausible_verifier(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

impl PickupStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn sweep(inner: &mut Inner, now: i64) {
        inner.map.retain(|_, e| e.expires_at > now);
        inner.last_sweep = now;
    }

    /// Room for one more entry: sweep, and if that is not enough, drop the oldest.
    fn make_room(inner: &mut Inner, now: i64) {
        if now - inner.last_sweep >= SWEEP_EVERY_MS || inner.map.len() >= MAX_ENTRIES {
            Self::sweep(inner, now);
        }
        while inner.map.len() >= MAX_ENTRIES {
            inner.map.shift_remove_index(0);
        }
    }

    /// The page was shown for a request with a challenge: the app's polls now
    /// read "pending" instead of "not found". The first challenge seen for a
    /// state is the one it is bound to; a second, different one never replaces it.
    pub fn register(&self, state: &str, challenge: &str, now: i64) {
        let mut inner = self.lock();
        Self::make_room(&mut inner, now);
        match inner.map.get_mut(state) {
            Some(entry) if entry.expires_at > now => {
                if entry.challenge == challenge && entry.outcome.is_none() {
                    entry.expires_at = now + PICKUP_TTL_MS;
                }
            }
            _ => {
                inner.map.shift_remove(state);
                inner.map.insert(
                    state.to_string(),
                    Entry { challenge: challenge.to_string(), outcome: None, expires_at: now + PICKUP_TTL_MS },
                );
            }
        }
    }

    /// The person decided. Stored for `state` unless a live entry for it is
    /// bound to a DIFFERENT challenge (then nothing is stored and `false` comes
    /// back: the page still has its loopback POST and the manual key).
    pub fn complete(&self, state: &str, challenge: &str, outcome: Outcome, now: i64) -> bool {
        let mut inner = self.lock();
        Self::make_room(&mut inner, now);
        if let Some(entry) = inner.map.get_mut(state)
            && entry.expires_at > now
        {
            if entry.challenge != challenge {
                return false;
            }
            entry.outcome = Some(outcome);
            entry.expires_at = now + PICKUP_TTL_MS;
            return true;
        }
        inner.map.shift_remove(state);
        inner.map.insert(
            state.to_string(),
            Entry { challenge: challenge.to_string(), outcome: Some(outcome), expires_at: now + PICKUP_TTL_MS },
        );
        true
    }

    /// The app asks. The verifier is hashed and compared in constant time with
    /// the stored challenge; a ready outcome is handed out once.
    pub fn pick_up(&self, state: &str, verifier: &str, now: i64) -> Pickup {
        let presented = challenge_for(verifier);
        let mut inner = self.lock();
        if now - inner.last_sweep >= SWEEP_EVERY_MS {
            Self::sweep(&mut inner, now);
        }
        let Some(entry) = inner.map.get(state) else {
            return Pickup::NotFound;
        };
        if entry.expires_at <= now {
            inner.map.shift_remove(state);
            return Pickup::NotFound;
        }
        let matches = entry.challenge.len() == CHALLENGE_LEN
            && bool::from(presented.as_bytes().ct_eq(entry.challenge.as_bytes()));
        if !matches {
            return Pickup::NotFound;
        }
        if entry.outcome.is_none() {
            return Pickup::Pending;
        }
        match inner.map.shift_remove(state).and_then(|e| e.outcome) {
            Some(outcome) => Pickup::Ready(outcome),
            None => Pickup::NotFound,
        }
    }

    /// Entries held right now (expired ones included until the next sweep).
    pub fn len(&self) -> usize {
        self.lock().map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drop every expired entry now.
    pub fn sweep_now(&self, now: i64) {
        Self::sweep(&mut self.lock(), now);
    }
}

/// The process's pickup map.
pub fn store() -> &'static PickupStore {
    static STORE: LazyLock<PickupStore> = LazyLock::new(PickupStore::default);
    &STORE
}

/// Milliseconds since the epoch, the clock the store runs on.
pub fn now_ms() -> i64 {
    crate::db::types::JsDate::now().millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATE: &str = "s7Kd93jf0aQ2xxxxxxxxxxxx";
    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

    fn challenge() -> String {
        challenge_for(VERIFIER)
    }

    #[test]
    fn the_challenge_is_rfc_7636_s256() {
        // RFC 7636 appendix B.
        assert_eq!(challenge(), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(challenge().len(), CHALLENGE_LEN);
        assert!(is_plausible_verifier(VERIFIER));
        assert!(!is_plausible_verifier("short"));
        assert!(!is_plausible_verifier(&format!("{VERIFIER}!")));
    }

    #[test]
    fn a_token_is_handed_out_once() {
        let s = PickupStore::default();
        assert!(s.complete(STATE, &challenge(), Outcome::Token("tok".into()), 0));
        assert_eq!(s.pick_up(STATE, VERIFIER, 1), Pickup::Ready(Outcome::Token("tok".into())));
        assert_eq!(s.pick_up(STATE, VERIFIER, 2), Pickup::NotFound);
        assert!(s.is_empty());
    }

    #[test]
    fn a_denial_is_handed_out_too() {
        let s = PickupStore::default();
        s.complete(STATE, &challenge(), Outcome::Denied, 0);
        assert_eq!(s.pick_up(STATE, VERIFIER, 1), Pickup::Ready(Outcome::Denied));
    }

    #[test]
    fn pending_until_decided() {
        let s = PickupStore::default();
        assert_eq!(s.pick_up(STATE, VERIFIER, 0), Pickup::NotFound);
        s.register(STATE, &challenge(), 0);
        assert_eq!(s.pick_up(STATE, VERIFIER, 1), Pickup::Pending);
        assert_eq!(s.pick_up(STATE, VERIFIER, 2), Pickup::Pending);
        assert!(s.complete(STATE, &challenge(), Outcome::Token("tok".into()), 3));
        assert_eq!(s.pick_up(STATE, VERIFIER, 4), Pickup::Ready(Outcome::Token("tok".into())));
    }

    #[test]
    fn a_wrong_verifier_learns_nothing_and_takes_nothing() {
        let s = PickupStore::default();
        s.complete(STATE, &challenge(), Outcome::Token("tok".into()), 0);
        let wrong = "x".repeat(43);
        assert_eq!(s.pick_up(STATE, &wrong, 1), Pickup::NotFound);
        assert_eq!(s.pick_up("someone-elses-state", VERIFIER, 1), Pickup::NotFound);
        // Still there for the right verifier.
        assert_eq!(s.pick_up(STATE, VERIFIER, 2), Pickup::Ready(Outcome::Token("tok".into())));
    }

    #[test]
    fn entries_expire() {
        let s = PickupStore::default();
        s.complete(STATE, &challenge(), Outcome::Token("tok".into()), 0);
        assert_eq!(s.pick_up(STATE, VERIFIER, PICKUP_TTL_MS), Pickup::NotFound);
        assert!(s.is_empty());

        s.register(STATE, &challenge(), 0);
        assert_eq!(s.pick_up(STATE, VERIFIER, PICKUP_TTL_MS + 1), Pickup::NotFound);
    }

    #[test]
    fn the_first_challenge_binds_the_state() {
        let s = PickupStore::default();
        let other = challenge_for(&"y".repeat(43));
        s.register(STATE, &challenge(), 0);
        s.register(STATE, &other, 1);
        assert!(!s.complete(STATE, &other, Outcome::Token("stolen".into()), 2));
        assert_eq!(s.pick_up(STATE, &"y".repeat(43), 3), Pickup::NotFound);
        assert_eq!(s.pick_up(STATE, VERIFIER, 3), Pickup::Pending);
        // Once the old binding expired, a new request may use the state again.
        assert!(s.complete(STATE, &other, Outcome::Denied, PICKUP_TTL_MS + 10));
    }

    #[test]
    fn a_second_approval_replaces_the_first_token() {
        let s = PickupStore::default();
        s.complete(STATE, &challenge(), Outcome::Token("one".into()), 0);
        s.complete(STATE, &challenge(), Outcome::Token("two".into()), 1);
        assert_eq!(s.pick_up(STATE, VERIFIER, 2), Pickup::Ready(Outcome::Token("two".into())));
    }

    #[test]
    fn the_map_is_swept_and_bounded() {
        let s = PickupStore::default();
        for i in 0..10 {
            s.register(&format!("state-{i:08}"), &challenge(), 0);
        }
        assert_eq!(s.len(), 10);
        s.sweep_now(PICKUP_TTL_MS + 1);
        assert!(s.is_empty());

        // A periodic sweep also runs on the next write.
        let s = PickupStore::default();
        for i in 0..10 {
            s.register(&format!("state-{i:08}"), &challenge(), 0);
        }
        s.register("late-state-1", &challenge(), PICKUP_TTL_MS + SWEEP_EVERY_MS);
        assert_eq!(s.len(), 1);

        // Full: the oldest goes first.
        let s = PickupStore::default();
        for i in 0..MAX_ENTRIES + 5 {
            s.register(&format!("state-{i:08}"), &challenge(), 1);
        }
        assert_eq!(s.len(), MAX_ENTRIES);
        assert_eq!(s.pick_up("state-00000000", VERIFIER, 2), Pickup::NotFound);
        assert_eq!(s.pick_up(&format!("state-{:08}", MAX_ENTRIES + 4), VERIFIER, 2), Pickup::Pending);
    }

    #[test]
    fn a_token_never_shows_in_debug_output() {
        assert_eq!(format!("{:?}", Pickup::Ready(Outcome::Token("secret".into()))), "Ready(Token(<redacted>))");
    }
}
