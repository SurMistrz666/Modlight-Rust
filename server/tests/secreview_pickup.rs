//! Regression tests for the security review of 2026-10-24 (docs/audit/rust-delta-2026-10-24.md,
//! P1-P3). They started as the review's attack tests, each asserting that the attack WORKED
//! (`_scratch/sec-review-3/srv/server/tests/secreview_pickup.rs`); here the same scenarios run
//! against the fixed code and assert that the attack FAILS.
use modlight_server::actions::authorize::{AuthorizeDeps, approve_with, deny_with};
use modlight_server::api_token::TokenScope;
use modlight_server::authorize::pickup::{Outcome, Pickup, PickupStore, Registered, challenge_for, now_ms};
use serde_json::{Value, json};

#[derive(Default)]
struct Fake {
    store: PickupStore,
}

impl AuthorizeDeps for Fake {
    async fn create_token(&self, user_id: &str, _n: &str, _s: TokenScope) -> anyhow::Result<String> {
        Ok(format!("TOKEN-OF-{user_id}-AAAAAAAAAAAAAAAAAAAAAAAA"))
    }
    async fn audit(&self, _u: &str, _a: &str, _d: Value) {}
    fn pickups(&self) -> &PickupStore {
        &self.store
    }
}

/// Known only to the victim's app.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const ATTACKER_VERIFIER: &str = "AttackerVerifierAttackerVerifierAttackerVer";
const STATE: &str = "VictimState0123456789abcdefABCDE";

fn ip() -> String {
    format!("203.0.113.{}", rand::random::<u8>())
}

/// P1: the attacker replays the victim's URL values through `approveDesktopClient` from his own
/// session. Before the fix the victim's app collected `TOKEN-OF-attacker`.
#[tokio::test]
async fn attacker_who_saw_the_url_cannot_plant_his_token_in_the_victims_app() {
    let f = Fake::default();
    let challenge = challenge_for(VERIFIER); // public: it is in the URL
    // The victim's signed-in browser rendered /authorize.
    assert_eq!(f.store.register(STATE, &challenge, "victim", now_ms()), Registered::Bound);
    // The attacker's own page render of the same URL does not take the state over.
    assert_eq!(f.store.register(STATE, &challenge, "attacker", now_ms()), Registered::Taken);
    let input = json!({ "port": "49732", "state": STATE, "name": "Modlight", "challenge": challenge });
    let (out, issued) = approve_with(&f, "attacker", &ip(), Some(&input)).await;
    // He gets his own token on his own page (for his own loopback), and that is all.
    assert!(issued);
    assert_eq!(out["pickup"], false);
    // The victim's app, polling with its secret verifier, is still waiting.
    assert_eq!(f.store.pick_up(STATE, VERIFIER, now_ms()), Pickup::Pending);
    // ...and the victim's own approval goes through.
    let (out, _) = approve_with(&f, "victim", &ip(), Some(&input)).await;
    assert_eq!(out["pickup"], true);
    assert_eq!(
        f.store.pick_up(STATE, VERIFIER, now_ms()),
        Pickup::Ready(Outcome::Token("TOKEN-OF-victim-AAAAAAAAAAAAAAAAAAAAAAAA".into()))
    );
}

/// P1, a state nobody registered: approving it stores nothing for the pickup.
#[tokio::test]
async fn an_approval_for_a_state_never_registered_reaches_no_app() {
    let f = Fake::default();
    let input = json!({ "port": "49732", "state": STATE, "challenge": challenge_for(VERIFIER) });
    let (out, issued) = approve_with(&f, "attacker", &ip(), Some(&input)).await;
    assert!(issued);
    assert_eq!(out["pickup"], false);
    assert_eq!(f.store.pick_up(STATE, VERIFIER, now_ms()), Pickup::NotFound);
}

/// P1: before the fix the attacker's deny ended the victim's sign-in.
#[tokio::test]
async fn attacker_who_saw_the_url_cannot_cancel_the_sign_in() {
    let f = Fake::default();
    let challenge = challenge_for(VERIFIER);
    f.store.register(STATE, &challenge, "victim", now_ms());
    let input = json!({ "port": "49732", "state": STATE, "challenge": challenge });
    assert_eq!(deny_with(&f, "attacker", &ip(), Some(&input)), json!({ "ok": true }));
    assert_eq!(f.store.pick_up(STATE, VERIFIER, now_ms()), Pickup::Pending);
    // The victim's own refusal still arrives.
    deny_with(&f, "victim", &ip(), Some(&input));
    assert_eq!(f.store.pick_up(STATE, VERIFIER, now_ms()), Pickup::Ready(Outcome::Denied));
}

/// P1 residual (accepted in the review): an attacker who renders the URL first, with another
/// challenge, blocks the pickup (the victim keeps the loopback and the manual key), but nothing
/// of his can ever reach the victim's app.
#[test]
fn registering_first_with_another_challenge_gives_the_attacker_nothing() {
    let s = PickupStore::default();
    let attacker = challenge_for(ATTACKER_VERIFIER);
    assert_eq!(s.register(STATE, &attacker, "attacker", now_ms()), Registered::Bound);
    assert_eq!(s.register(STATE, &challenge_for(VERIFIER), "victim", now_ms()), Registered::Taken);
    assert!(!s.complete(STATE, &challenge_for(VERIFIER), "victim", Outcome::Token("victim".into()), now_ms()));
    assert!(!s.complete(STATE, &attacker, "victim", Outcome::Token("victim".into()), now_ms()));
    assert_eq!(s.pick_up(STATE, VERIFIER, now_ms()), Pickup::NotFound);
    assert_eq!(s.pick_up(STATE, ATTACKER_VERIFIER, now_ms()), Pickup::Pending);
}
