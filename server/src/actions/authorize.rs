//! Port of `app/actions/authorize.ts`: issuing an API token to a desktop
//! client that asked for one over the loopback flow (see
//! [`crate::authorize::loopback`] for the whole handshake).
//!
//! This is a POST (a Server Action always is) and that is not an
//! implementation detail. If approving were a GET, then merely OPENING a link
//! would mint a credential, and the link would be one an attacker writes: the
//! whole flow rests on a human looking at the prompt and choosing.
//!
//! Refusals travel as data rather than as thrown errors: Next masks Server
//! Action error messages in production, so anything the user has to read
//! would reach them as "an error occurred".

use serde_json::{Map, Value, json};

use crate::api_token::{TokenScope, create_api_token};
use crate::authorize::loopback::{LoopbackRequest, parse_authorize_request_with_challenge, token_name_for};
use crate::authorize::pickup::{Outcome, PickupStore, now_ms};
use crate::password::check_rate_limit;
use crate::rpc::{ActionCtx, ActionResult, RpcRegistry};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("authorize", "approveDesktopClient", approve_desktop_client);
    // Not upstream: the refusal reaches the app through the pickup too.
    reg.register("authorize", "denyDesktopClient", deny_desktop_client);
}

/// Declining is not minting, but it still writes to the pickup map.
pub const MAX_DENIALS: u32 = 30;

/// Minting credentials is rare; a flood of it is somebody else's script.
pub const MAX_APPROVALS: u32 = 10;
pub const APPROVAL_WINDOW_MS: i64 = 15 * 60 * 1000;

/// What the action needs besides the session and the request (stubbed in tests).
pub trait AuthorizeDeps {
    fn create_token(
        &self,
        user_id: &str,
        name: &str,
        scope: TokenScope,
    ) -> impl Future<Output = anyhow::Result<String>> + Send;
    fn audit(&self, user_id: &str, action: &str, detail: Value) -> impl Future<Output = ()> + Send;
    /// Where outcomes wait for the app's pickup (the process-wide map in production).
    fn pickups(&self) -> &PickupStore {
        crate::authorize::pickup::store()
    }
}

struct LiveDeps<'a>(&'a ActionCtx);

impl AuthorizeDeps for LiveDeps<'_> {
    async fn create_token(&self, user_id: &str, name: &str, scope: TokenScope) -> anyhow::Result<String> {
        create_api_token(&self.0.state.db, user_id, name, scope).await
    }
    async fn audit(&self, user_id: &str, action: &str, detail: Value) {
        crate::audit::record_audit(&self.0.state.db, user_id, action, detail, &self.0.client_ip()).await;
    }
}

fn refusal(reason: &str) -> Value {
    json!({ "ok": false, "reason": reason })
}

/// `input?.port`, `input?.state`, `input?.name`: only an object has
/// properties worth reading; anything else reads as `undefined`.
fn input_field<'a>(input: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    match input {
        Some(v @ Value::Object(map)) if !crate::rpc::tagged::is_undefined(v) => map.get(key),
        _ => None,
    }
}

/// The part of `approveDesktopClient` after the session lookup: validate the
/// raw values again, throttle, mint, log. `Ok(true)` in the second slot means
/// a token was issued (the action then revalidates).
pub async fn approve_with<D: AuthorizeDeps>(
    deps: &D,
    user_id: &str,
    client_ip: &str,
    input: Option<&Value>,
) -> (Value, bool) {
    // The arguments are the RAW query values the prompt was rendered from, and
    // they are re-validated here from scratch. A Server Action is a public
    // endpoint that anything can call with anything: the page having checked
    // them earlier says nothing about what arrives now, and `callback` is
    // built from the port this function parsed, never from a URL the caller
    // supplied.
    let Some(request) = parse_input(input) else {
        return (refusal("invalid-request"), false);
    };

    // Keyed by caller AND account: one compromised session cannot be milked
    // for tokens in a loop, and one noisy IP cannot lock out everybody behind it.
    let key = format!("{client_ip}|authorize:{user_id}");
    if !check_rate_limit(&key, MAX_APPROVALS, APPROVAL_WINDOW_MS).allowed {
        return (refusal("rate-limited"), false);
    }

    match issue(deps, user_id, &request).await {
        Ok(token) => {
            // Not upstream: the app collects it from the server too, in case
            // the browser never delivers the loopback POST (SKELETON.md 9).
            // Only for a state THIS account's page render registered (review
            // 2026-10-24, P1): somebody who merely saw the URL gets his own
            // token back on his own page, for his own loopback, and the app
            // that wrote the link never sees it. `pickup: false` tells the page
            // to fall back to the loopback POST and the manual key.
            let mut pickup = false;
            if let Some(challenge) = &request.challenge {
                pickup = deps.pickups().complete(
                    &request.state,
                    challenge,
                    user_id,
                    Outcome::Token(token.clone()),
                    now_ms(),
                );
                if !pickup {
                    tracing::warn!("[authorize] pickup not stored: the state was not registered by this account");
                }
            }
            let mut out = json!({ "ok": true, "token": token, "callback": request.callback, "state": request.state });
            if request.challenge.is_some() {
                out["pickup"] = Value::Bool(pickup);
            }
            (out, true)
        }
        Err(err) => {
            tracing::error!("[authorize] could not issue a desktop token: {err:#}");
            (refusal("error"), false)
        }
    }
}

/// `{ port, state, name, challenge? }` re-validated from scratch.
fn parse_input(input: Option<&Value>) -> Option<LoopbackRequest> {
    parse_authorize_request_with_challenge(
        input_field(input, "port"),
        input_field(input, "state"),
        input_field(input, "name"),
        input_field(input, "challenge"),
    )
    .request()
    .cloned()
}

/// Not upstream: `denyDesktopClient` after the session lookup. A request
/// without a challenge has nothing to record (the page's loopback POST is the
/// only way to tell such an app); one with a challenge leaves the refusal for
/// the app's pickup.
pub fn deny_with<D: AuthorizeDeps>(deps: &D, user_id: &str, client_ip: &str, input: Option<&Value>) -> Value {
    let Some(request) = parse_input(input) else {
        return refusal("invalid-request");
    };
    let key = format!("{client_ip}|authorize-deny:{user_id}");
    if !check_rate_limit(&key, MAX_DENIALS, APPROVAL_WINDOW_MS).allowed {
        return refusal("rate-limited");
    }
    // Only the account whose page render registered the state may cancel it
    // (P1); anyone else's "no" stays on his own page and loopback.
    if let Some(challenge) = &request.challenge {
        deps.pickups().complete(&request.state, challenge, user_id, Outcome::Denied, now_ms());
    }
    json!({ "ok": true })
}

async fn issue<D: AuthorizeDeps>(deps: &D, user_id: &str, request: &LoopbackRequest) -> anyhow::Result<String> {
    // SCOPE: `write`. The desktop client uploads, draws the library, moves
    // files between folders and empties the trash - that is `write`, and
    // nothing here needs `full`: publishing a release is a deliberate,
    // separate act by an administrator, not something a sign-in should hand
    // out. A client that later needs more says so by asking for another token.
    let token = deps.create_token(user_id, &token_name_for(request.client_name_for_token()), TokenScope::Write).await?;

    // A credential was issued, so it gets a line in the log, with the name the
    // client claimed and the port it asked for, because "which of these did I
    // approve" is the question this entry has to answer later. The last six
    // characters are the same ones `api_tokens.preview` already stores and the
    // Integrations page already shows, so the log points at a specific row
    // without holding anything that could be used as the token.
    if request.cut_surrogate {
        // Upstream's detail then carries a lone surrogate, which
        // JSON.stringify writes as a `\udXXX` escape that PostgreSQL refuses
        // for jsonb: the insert fails inside recordAudit, which only logs it.
        tracing::error!("[audit] could not record api_token.authorized: client name ends in a lone surrogate");
    } else {
        let mut detail = Map::new();
        detail.insert(
            "client".into(),
            if request.client_name.is_empty() { Value::Null } else { Value::String(request.client_name.clone()) },
        );
        detail.insert("port".into(), Value::from(request.port));
        detail.insert("preview".into(), Value::String(last_chars(&token, 6)));
        deps.audit(user_id, "api_token.authorized", Value::Object(detail)).await;
    }
    Ok(token)
}

/// `token.slice(-6)` (the token is base64url, so characters are code units).
fn last_chars(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    chars[chars.len().saturating_sub(n)..].iter().collect()
}

/// `approveDesktopClient(input)`: mint a named API token for a desktop client
/// and hand back where to deliver it.
async fn approve_desktop_client(ctx: ActionCtx) -> ActionResult {
    // Deliberately not `requireUser()`: that redirects to /login, and a
    // redirect out of an action the user just clicked loses the request they
    // were approving. The page tells them to sign in again instead.
    let Some(session) = ctx.get_session().await? else {
        return Ok(refusal("signed-out"));
    };
    let (out, issued) = approve_with(&LiveDeps(&ctx), &session.user.id, &ctx.client_ip(), ctx.args.raw(0)).await;
    if issued {
        ctx.revalidate_path("/dashboard/tokens");
    }
    Ok(out)
}

/// `denyDesktopClient(input)` (not upstream): record a refusal for the pickup.
async fn deny_desktop_client(ctx: ActionCtx) -> ActionResult {
    let Some(session) = ctx.get_session().await? else {
        return Ok(refusal("signed-out"));
    };
    Ok(deny_with(&LiveDeps(&ctx), &session.user.id, &ctx.client_ip(), ctx.args.raw(0)))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Fake {
        created: Mutex<Vec<(String, String, TokenScope)>>,
        audits: Mutex<Vec<(String, String, Value)>>,
        fail: bool,
        store: PickupStore,
    }

    impl AuthorizeDeps for Fake {
        async fn create_token(&self, user_id: &str, name: &str, scope: TokenScope) -> anyhow::Result<String> {
            if self.fail {
                anyhow::bail!("database down");
            }
            self.created.lock().unwrap().push((user_id.into(), name.into(), scope));
            Ok("AAAAAAAAAAAAAAAAAAAAAAAAAAtoken6".into())
        }
        async fn audit(&self, user_id: &str, action: &str, detail: Value) {
            self.audits.lock().unwrap().push((user_id.into(), action.into(), detail));
        }
        fn pickups(&self) -> &PickupStore {
            &self.store
        }
    }

    use crate::authorize::pickup::{Pickup, challenge_for};

    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

    #[tokio::test]
    async fn an_approval_with_a_challenge_waits_for_the_pickup() {
        let f = Fake::default();
        // The page render registered the state for this account.
        f.store.register("s7Kd93jf0aQ2", &challenge_for(VERIFIER), "u1", now_ms());
        let input = json!({ "port": "49732", "state": "s7Kd93jf0aQ2", "name": "Modlight", "challenge": challenge_for(VERIFIER) });
        let (out, issued) = approve_with(&f, "u1", &ip(), Some(&input)).await;
        assert!(issued);
        // The page still gets everything it got before, and learns the app collects it.
        assert_eq!(out["token"], "AAAAAAAAAAAAAAAAAAAAAAAAAAtoken6");
        assert_eq!(out["callback"], "http://127.0.0.1:49732/");
        assert_eq!(out["pickup"], true);
        assert_eq!(
            f.store.pick_up("s7Kd93jf0aQ2", VERIFIER, now_ms()),
            Pickup::Ready(Outcome::Token("AAAAAAAAAAAAAAAAAAAAAAAAAAtoken6".into()))
        );
    }

    #[tokio::test]
    async fn an_approval_without_a_challenge_stores_nothing() {
        let f = Fake::default();
        let input = json!({ "port": "49732", "state": "s7Kd93jf0aQ2" });
        assert!(approve_with(&f, "u1", &ip(), Some(&input)).await.1);
        assert!(f.store.is_empty());
    }

    #[tokio::test]
    async fn a_malformed_challenge_is_an_invalid_request() {
        let f = Fake::default();
        for challenge in [json!("short"), json!("!".repeat(43)), json!(["x"]), json!(43)] {
            let input = json!({ "port": "49732", "state": "s7Kd93jf0aQ2", "challenge": challenge });
            let (out, issued) = approve_with(&f, "u1", &ip(), Some(&input)).await;
            assert_eq!(out["reason"], "invalid-request", "{input}");
            assert!(!issued);
            assert_eq!(deny_with(&f, "u1", &ip(), Some(&input))["reason"], "invalid-request");
        }
        assert!(f.created.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_denial_with_a_challenge_reaches_the_pickup() {
        let f = Fake::default();
        f.store.register("s7Kd93jf0aQ2", &challenge_for(VERIFIER), "u1", now_ms());
        let input = json!({ "port": "49732", "state": "s7Kd93jf0aQ2", "challenge": challenge_for(VERIFIER) });
        assert_eq!(deny_with(&f, "u1", &ip(), Some(&input)), json!({ "ok": true }));
        assert_eq!(f.store.pick_up("s7Kd93jf0aQ2", VERIFIER, now_ms()), Pickup::Ready(Outcome::Denied));
        assert!(f.created.lock().unwrap().is_empty());
        // Without a challenge there is nothing to record.
        let plain = json!({ "port": "49732", "state": "s7Kd93jf0aQ2" });
        assert_eq!(deny_with(&f, "u1", &ip(), Some(&plain)), json!({ "ok": true }));
        assert!(f.store.is_empty());
    }

    #[tokio::test]
    async fn an_approval_for_a_state_this_account_never_rendered_stays_on_its_page() {
        let f = Fake::default();
        let input = json!({ "port": "49732", "state": "s7Kd93jf0aQ2", "challenge": challenge_for(VERIFIER) });
        let (out, issued) = approve_with(&f, "u1", &ip(), Some(&input)).await;
        // The token is the approver's own, for his own page and loopback only.
        assert!(issued);
        assert_eq!(out["pickup"], false);
        assert!(f.store.is_empty());
        assert_eq!(f.store.pick_up("s7Kd93jf0aQ2", VERIFIER, now_ms()), Pickup::NotFound);
    }

    fn ip() -> String {
        format!("test-{}", rand::random::<u64>())
    }

    #[tokio::test]
    async fn issues_a_write_token_and_says_where_it_goes() {
        let f = Fake::default();
        let input = json!({ "port": "49732", "state": "s7Kd93jf0aQ2", "name": "Modlight Desktop" });
        let (out, issued) = approve_with(&f, "u1", &ip(), Some(&input)).await;
        assert!(issued);
        assert_eq!(
            out,
            json!({
                "ok": true,
                "token": "AAAAAAAAAAAAAAAAAAAAAAAAAAtoken6",
                "callback": "http://127.0.0.1:49732/",
                "state": "s7Kd93jf0aQ2"
            })
        );
        assert_eq!(f.created.lock().unwrap()[0], ("u1".into(), "Desktop: Modlight Desktop".into(), TokenScope::Write));
        assert_eq!(
            f.audits.lock().unwrap()[0],
            (
                "u1".into(),
                "api_token.authorized".into(),
                json!({ "client": "Modlight Desktop", "port": 49732, "preview": "token6" })
            )
        );
    }

    #[tokio::test]
    async fn an_unnamed_client_is_logged_with_a_null_name() {
        let f = Fake::default();
        let input = json!({ "port": "49732", "state": "s7Kd93jf0aQ2" });
        approve_with(&f, "u1", &ip(), Some(&input)).await;
        assert_eq!(f.created.lock().unwrap()[0].1, "Desktop client");
        assert_eq!(f.audits.lock().unwrap()[0].2["client"], Value::Null);
    }

    #[tokio::test]
    async fn refuses_anything_that_is_not_a_string_request() {
        let f = Fake::default();
        for input in [
            Value::Null,
            json!("49732"),
            json!(["49732", "s7Kd93jf0aQ2"]),
            json!({ "port": 49732, "state": "s7Kd93jf0aQ2" }),
            json!({ "port": ["49732"], "state": "s7Kd93jf0aQ2" }),
            json!({ "port": "49732", "state": "short" }),
            json!({ "$undefined": 1 }),
        ] {
            let (out, issued) = approve_with(&f, "u1", &ip(), Some(&input)).await;
            assert_eq!(out, json!({ "ok": false, "reason": "invalid-request" }), "{input}");
            assert!(!issued);
        }
        let (out, _) = approve_with(&f, "u1", &ip(), None).await;
        assert_eq!(out["reason"], "invalid-request");
        assert!(f.created.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_eleventh_approval_in_a_window_is_refused() {
        let f = Fake::default();
        let caller = ip();
        let input = json!({ "port": "49732", "state": "s7Kd93jf0aQ2" });
        for _ in 0..MAX_APPROVALS {
            assert!(approve_with(&f, "u1", &caller, Some(&input)).await.1);
        }
        let (out, issued) = approve_with(&f, "u1", &caller, Some(&input)).await;
        assert_eq!(out, json!({ "ok": false, "reason": "rate-limited" }));
        assert!(!issued);
        // Another account behind the same address has its own budget.
        assert!(approve_with(&f, "u2", &caller, Some(&input)).await.1);
        // Invalid requests do not count.
        let other = ip();
        for _ in 0..20 {
            approve_with(&f, "u3", &other, Some(&json!({}))).await;
        }
        assert!(approve_with(&f, "u3", &other, Some(&input)).await.1);
    }

    #[tokio::test]
    async fn a_failed_insert_is_reported_as_data() {
        let f = Fake { fail: true, ..Default::default() };
        let input = json!({ "port": "49732", "state": "s7Kd93jf0aQ2" });
        let (out, issued) = approve_with(&f, "u1", &ip(), Some(&input)).await;
        assert_eq!(out, json!({ "ok": false, "reason": "error" }));
        assert!(!issued);
        assert!(f.audits.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_name_cut_inside_a_surrogate_pair_issues_the_token_but_logs_nothing() {
        let f = Fake::default();
        let input = json!({ "port": "49732", "state": "s7Kd93jf0aQ2", "name": format!("{}\u{1F600}", "a".repeat(63)) });
        let (out, issued) = approve_with(&f, "u1", &ip(), Some(&input)).await;
        assert!(issued, "{out}");
        assert_eq!(f.created.lock().unwrap()[0].1, format!("Desktop: {}", "a".repeat(63)));
        assert!(f.audits.lock().unwrap().is_empty());
    }
}
