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
//!   2. the signed-in page render REGISTERS `state` with `challenge` and the
//!      account that sees it; only that account's `approveDesktopClient` (or
//!      `denyDesktopClient`) with the same challenge stores an outcome here
//!      (review 2026-10-24, P1: the challenge sits in the URL, so it alone
//!      proves nothing about who decides);
//!   3. the app polls `POST /api/authorize/pickup` with `{state, verifier}`;
//!      the verifier never appeared in any URL or page, so only the process
//!      that generated the link can collect what the link produced.
//!
//! The entry is handed out ONCE and then deleted, lives five minutes, and the
//! map is bounded and swept. The prompt and the
//! app both show [`confirmation_code`] of the challenge, so a person approving
//! somebody else's link sees that no app of theirs shows that code (P2).
//! Everything is in memory: a restart loses pending pickups, and the
//! loopback POST and the manual key still work then.
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

/// What [`PickupStore::register`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registered {
    /// The state is (now, or still) bound to this account and challenge.
    Bound,
    /// A live entry for the state belongs to another account or another
    /// challenge: nothing changed (the first signed-in browser keeps it).
    Taken,
}

struct Entry {
    challenge: String,
    /// The account whose signed-in browser rendered the prompt (review
    /// 2026-10-24, P1): only that account may decide for this state.
    user_id: String,
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

/// Domain separation for [`confirmation_code`]: the code is never a prefix of
/// anything else derived from the challenge.
const CONFIRM_CONTEXT: &[u8] = b"modlight-confirm:";

/// RFC 4648 base32 alphabet.
const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// The short code the app and the prompt both show (review 2026-10-24, P2):
/// the first 6 base32 characters of `SHA-256("modlight-confirm:" || challenge)`,
/// as `XXX-XXX`. A phished person has no app on screen showing it, so a
/// mismatch is visible before Approve. 30 bits: not a secret, a comparison.
/// The app computes the same (`modlight_api::signin::confirmation_code`).
pub fn confirmation_code(challenge: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(CONFIRM_CONTEXT);
    hasher.update(challenge.as_bytes());
    let digest = hasher.finalize();
    // 30 bits from the first four bytes, five at a time.
    let bits = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    let chars: String = (0..6).map(|i| BASE32[((bits >> (27 - 5 * i)) & 31) as usize] as char).collect();
    format!("{}-{}", &chars[..3], &chars[3..])
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

    /// The page was shown, to the signed-in `user_id`, for a request with a
    /// challenge: the app's polls now read "pending" instead of "not found",
    /// and only this account may decide for the state. A live entry is never
    /// taken over by another account or another challenge.
    pub fn register(&self, state: &str, challenge: &str, user_id: &str, now: i64) -> Registered {
        let mut inner = self.lock();
        if let Some(entry) = inner.map.get_mut(state)
            && entry.expires_at > now
        {
            if entry.user_id != user_id || entry.challenge != challenge {
                return Registered::Taken;
            }
            if entry.outcome.is_none() {
                entry.expires_at = now + PICKUP_TTL_MS;
            }
            return Registered::Bound;
        }
        inner.map.shift_remove(state);
        Self::make_room(&mut inner, now);
        inner.map.insert(
            state.to_string(),
            Entry {
                challenge: challenge.to_string(),
                user_id: user_id.to_string(),
                outcome: None,
                expires_at: now + PICKUP_TTL_MS,
            },
        );
        Registered::Bound
    }

    /// Whether `user_id` may decide for `state` with `challenge`: a live entry
    /// that this account's own page render registered.
    pub fn is_bound(&self, state: &str, challenge: &str, user_id: &str, now: i64) -> bool {
        let inner = self.lock();
        inner.map.get(state).is_some_and(|e| e.expires_at > now && e.user_id == user_id && e.challenge == challenge)
    }

    /// The person decided. Stored only on the live entry this account's page
    /// render registered for `state` with this challenge (P1); anything else
    /// (never registered, expired, another account, another challenge) stores
    /// nothing and gives `false`: the page then falls back to its loopback POST
    /// and the manual key, which reach only the deciding browser's own machine.
    pub fn complete(&self, state: &str, challenge: &str, user_id: &str, outcome: Outcome, now: i64) -> bool {
        let mut inner = self.lock();
        match inner.map.get_mut(state) {
            Some(entry) if entry.expires_at > now && entry.user_id == user_id && entry.challenge == challenge => {
                entry.outcome = Some(outcome);
                entry.expires_at = now + PICKUP_TTL_MS;
                true
            }
            _ => false,
        }
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
    const ME: &str = "user-me";

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
    fn the_confirmation_code_is_six_base32_characters_of_the_challenge() {
        // The same vector is pinned in the app (modlight-api signin tests).
        let code = confirmation_code(&challenge());
        assert_eq!(code, "554-LIO");
        assert_eq!(code.len(), 7);
        assert!(code.chars().enumerate().all(|(i, c)| if i == 3 { c == '-' } else { BASE32.contains(&(c as u8)) }));
        assert_ne!(confirmation_code(&challenge_for(&"y".repeat(43))), code);
    }

    #[test]
    fn a_token_is_handed_out_once() {
        let s = PickupStore::default();
        assert_eq!(s.register(STATE, &challenge(), ME, 0), Registered::Bound);
        assert!(s.complete(STATE, &challenge(), ME, Outcome::Token("tok".into()), 0));
        assert_eq!(s.pick_up(STATE, VERIFIER, 1), Pickup::Ready(Outcome::Token("tok".into())));
        assert_eq!(s.pick_up(STATE, VERIFIER, 2), Pickup::NotFound);
        assert!(s.is_empty());
    }

    #[test]
    fn a_denial_is_handed_out_too() {
        let s = PickupStore::default();
        s.register(STATE, &challenge(), ME, 0);
        assert!(s.complete(STATE, &challenge(), ME, Outcome::Denied, 0));
        assert_eq!(s.pick_up(STATE, VERIFIER, 1), Pickup::Ready(Outcome::Denied));
    }

    #[test]
    fn pending_until_decided() {
        let s = PickupStore::default();
        assert_eq!(s.pick_up(STATE, VERIFIER, 0), Pickup::NotFound);
        s.register(STATE, &challenge(), ME, 0);
        assert_eq!(s.pick_up(STATE, VERIFIER, 1), Pickup::Pending);
        assert_eq!(s.pick_up(STATE, VERIFIER, 2), Pickup::Pending);
        assert!(s.complete(STATE, &challenge(), ME, Outcome::Token("tok".into()), 3));
        assert_eq!(s.pick_up(STATE, VERIFIER, 4), Pickup::Ready(Outcome::Token("tok".into())));
    }

    #[test]
    fn a_state_nobody_registered_takes_no_decision() {
        let s = PickupStore::default();
        assert!(!s.complete(STATE, &challenge(), ME, Outcome::Token("tok".into()), 0));
        assert!(!s.complete(STATE, &challenge(), ME, Outcome::Denied, 0));
        assert!(s.is_empty());
        assert_eq!(s.pick_up(STATE, VERIFIER, 1), Pickup::NotFound);
    }

    #[test]
    fn only_the_account_that_rendered_the_page_decides() {
        let s = PickupStore::default();
        s.register(STATE, &challenge(), ME, 0);
        assert!(!s.is_bound(STATE, &challenge(), "someone-else", 1));
        assert!(!s.complete(STATE, &challenge(), "someone-else", Outcome::Token("theirs".into()), 1));
        assert!(!s.complete(STATE, &challenge(), "someone-else", Outcome::Denied, 1));
        // Another account rendering the same URL changes nothing either.
        assert_eq!(s.register(STATE, &challenge(), "someone-else", 2), Registered::Taken);
        assert_eq!(s.pick_up(STATE, VERIFIER, 3), Pickup::Pending);
        assert!(s.is_bound(STATE, &challenge(), ME, 3));
        assert!(s.complete(STATE, &challenge(), ME, Outcome::Token("mine".into()), 4));
        assert_eq!(s.pick_up(STATE, VERIFIER, 5), Pickup::Ready(Outcome::Token("mine".into())));
    }

    #[test]
    fn a_wrong_verifier_learns_nothing_and_takes_nothing() {
        let s = PickupStore::default();
        s.register(STATE, &challenge(), ME, 0);
        s.complete(STATE, &challenge(), ME, Outcome::Token("tok".into()), 0);
        let wrong = "x".repeat(43);
        assert_eq!(s.pick_up(STATE, &wrong, 1), Pickup::NotFound);
        assert_eq!(s.pick_up("someone-elses-state", VERIFIER, 1), Pickup::NotFound);
        // Still there for the right verifier.
        assert_eq!(s.pick_up(STATE, VERIFIER, 2), Pickup::Ready(Outcome::Token("tok".into())));
    }

    #[test]
    fn entries_expire() {
        let s = PickupStore::default();
        s.register(STATE, &challenge(), ME, 0);
        s.complete(STATE, &challenge(), ME, Outcome::Token("tok".into()), 0);
        assert_eq!(s.pick_up(STATE, VERIFIER, PICKUP_TTL_MS), Pickup::NotFound);
        assert!(s.is_empty());

        s.register(STATE, &challenge(), ME, 0);
        assert_eq!(s.pick_up(STATE, VERIFIER, PICKUP_TTL_MS + 1), Pickup::NotFound);
        // An expired binding decides nothing.
        assert!(!s.complete(STATE, &challenge(), ME, Outcome::Denied, PICKUP_TTL_MS + 1));
    }

    #[test]
    fn the_first_challenge_binds_the_state() {
        let s = PickupStore::default();
        let other = challenge_for(&"y".repeat(43));
        s.register(STATE, &challenge(), ME, 0);
        assert_eq!(s.register(STATE, &other, ME, 1), Registered::Taken);
        assert!(!s.complete(STATE, &other, ME, Outcome::Token("stolen".into()), 2));
        assert_eq!(s.pick_up(STATE, &"y".repeat(43), 3), Pickup::NotFound);
        assert_eq!(s.pick_up(STATE, VERIFIER, 3), Pickup::Pending);
        // Once the old binding expired, a new render may use the state again.
        assert_eq!(s.register(STATE, &other, ME, PICKUP_TTL_MS + 10), Registered::Bound);
        assert!(s.complete(STATE, &other, ME, Outcome::Denied, PICKUP_TTL_MS + 11));
    }

    #[test]
    fn a_second_approval_replaces_the_first_token() {
        let s = PickupStore::default();
        s.register(STATE, &challenge(), ME, 0);
        s.complete(STATE, &challenge(), ME, Outcome::Token("one".into()), 0);
        s.complete(STATE, &challenge(), ME, Outcome::Token("two".into()), 1);
        assert_eq!(s.pick_up(STATE, VERIFIER, 2), Pickup::Ready(Outcome::Token("two".into())));
    }

    fn state(i: usize) -> String {
        format!("state-{i:08}")
    }

    #[test]
    fn the_map_is_swept_and_bounded() {
        let s = PickupStore::default();
        for i in 0..10 {
            s.register(&state(i), &challenge(), &format!("u{i}"), 0);
        }
        assert_eq!(s.len(), 10);
        s.sweep_now(PICKUP_TTL_MS + 1);
        assert!(s.is_empty());

        // A periodic sweep also runs on the next write.
        let s = PickupStore::default();
        for i in 0..10 {
            s.register(&state(i), &challenge(), &format!("u{i}"), 0);
        }
        s.register("late-state-1", &challenge(), ME, PICKUP_TTL_MS + SWEEP_EVERY_MS);
        assert_eq!(s.len(), 1);

        // Full: the oldest goes first.
        let s = PickupStore::default();
        for i in 0..MAX_ENTRIES + 5 {
            s.register(&state(i), &challenge(), &format!("u{i}"), 1);
        }
        assert_eq!(s.len(), MAX_ENTRIES);
        assert_eq!(s.pick_up(&state(0), VERIFIER, 2), Pickup::NotFound);
        assert_eq!(s.pick_up(&state(MAX_ENTRIES + 4), VERIFIER, 2), Pickup::Pending);
    }

    #[test]
    fn a_token_never_shows_in_debug_output() {
        assert_eq!(format!("{:?}", Pickup::Ready(Outcome::Token("secret".into()))), "Ready(Token(<redacted>))");
    }
}
