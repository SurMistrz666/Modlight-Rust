//! The optional Cloudflare Turnstile check (src/turnstile.rs, SKELETON.md 9
//! item 43) against a siteverify stub on 127.0.0.1: the verdicts, what is
//! sent to Cloudflare, and the answers of every protected entry point. No
//! database: the configuration comes from TURNSTILE_* (the panel half is in
//! tests/turnstile_db.rs), and every request that the check lets through
//! fails a step later on the unreachable database or on body validation,
//! which is how "let through" shows here.

mod common;

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use common::{Answer, call, siteverify, state_with};
use modlight_server::http::App;
use modlight_server::turnstile::{ResolvedTurnstile, TOKEN_HEADER, Turnstile, Verdict, verify_request};
use serde_json::{Value, json};

const HOST: &str = "127.0.0.1:8421";
const ORIGIN: &str = "http://127.0.0.1:8421";

fn on(secret: &str) -> ResolvedTurnstile {
    let config = modlight_server::config::TurnstileEnv { site_key: "site".into(), secret_key: secret.into() };
    modlight_server::turnstile::resolve_turnstile(&Default::default(), &config, &|_| None)
}

fn with_token(token: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(TOKEN_HEADER, token.parse().unwrap());
    h
}

// ── The verdicts ────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_token_cloudflare_accepts_passes_and_the_request_is_described_exactly() {
    let stub = siteverify::start().await;
    let t = Turnstile::with_siteverify_url(stub.url());
    let ip: IpAddr = "203.0.113.9".parse().unwrap();
    assert_eq!(verify_request(&t, &on("good-secret"), &with_token("pass-1"), Some(ip)).await, Verdict::Passed);
    let sent = stub.last();
    assert_eq!(sent.get("secret").map(String::as_str), Some("good-secret"));
    assert_eq!(sent.get("response").map(String::as_str), Some("pass-1"));
    assert_eq!(sent.get("remoteip").map(String::as_str), Some("203.0.113.9"));
    assert_eq!(sent.len(), 3, "secret, response, remoteip and nothing else: {sent:?}");
}

#[tokio::test]
async fn a_token_cloudflare_refuses_fails() {
    let stub = siteverify::start().await;
    let t = Turnstile::with_siteverify_url(stub.url());
    assert_eq!(verify_request(&t, &on("good-secret"), &with_token("nope"), None).await, Verdict::Failed);
    assert!(!stub.last().contains_key("remoteip"), "no address, no remoteip");
    // A wrong secret is a refusal too, never a pass.
    assert_eq!(verify_request(&t, &on("bad-secret"), &with_token("pass-1"), None).await, Verdict::Failed);
}

#[tokio::test]
async fn a_missing_or_oversized_token_never_reaches_cloudflare() {
    let stub = siteverify::start().await;
    let t = Turnstile::with_siteverify_url(stub.url());
    assert_eq!(verify_request(&t, &on("good-secret"), &HeaderMap::new(), None).await, Verdict::Missing);
    assert_eq!(verify_request(&t, &on("good-secret"), &with_token("   "), None).await, Verdict::Missing);
    let long = format!("pass{}", "x".repeat(2048));
    assert_eq!(verify_request(&t, &on("good-secret"), &with_token(&long), None).await, Verdict::Failed);
    assert_eq!(stub.calls(), 0);
}

#[tokio::test]
async fn a_network_failure_or_a_timeout_fails_closed() {
    let closed = siteverify::closed().await;
    let t = Turnstile::with_siteverify_url(format!("http://{closed}/siteverify"));
    assert_eq!(verify_request(&t, &on("good-secret"), &with_token("pass-1"), None).await, Verdict::Unavailable);

    let silent = siteverify::silent().await;
    let t =
        Turnstile::with_siteverify_url(format!("http://{silent}/siteverify")).with_timeout(Duration::from_millis(300));
    let started = std::time::Instant::now();
    assert_eq!(verify_request(&t, &on("good-secret"), &with_token("pass-1"), None).await, Verdict::Unavailable);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn disabled_means_no_check_at_all() {
    let stub = siteverify::start().await;
    let t = Turnstile::with_siteverify_url(stub.url());
    let off = ResolvedTurnstile::off();
    assert_eq!(verify_request(&t, &off, &HeaderMap::new(), None).await, Verdict::Off);
    assert_eq!(verify_request(&t, &off, &with_token("nope"), None).await, Verdict::Off);
    assert_eq!(stub.calls(), 0);
}

#[tokio::test]
async fn the_secret_probe_tells_a_wrong_secret_from_a_test_secret() {
    use modlight_server::turnstile::Probe;
    let stub = siteverify::start().await;
    let t = Turnstile::with_siteverify_url(stub.url());
    assert_eq!(t.probe_secret("good-secret").await, Probe::Accepted { always_passes: false });
    assert_eq!(t.probe_secret("always-secret").await, Probe::Accepted { always_passes: true });
    assert_eq!(t.probe_secret("bad-secret").await, Probe::Rejected);
    assert_eq!(stub.last().get("response").map(String::as_str), Some("XXXX.DUMMY.TOKEN.XXXX"));
    let closed = siteverify::closed().await;
    assert_eq!(Turnstile::with_siteverify_url(format!("http://{closed}/")).probe_secret("x").await, Probe::Unreachable);
}

// ── The entry points ────────────────────────────────────────────────────────

/// The app with TURNSTILE_* set (`secret` = None: not configured), its
/// siteverify calls going to `url`. The database is unreachable.
fn app(secret: Option<&str>, url: &str) -> App {
    let mut vars = vec![("APP_URL", ORIGIN), ("NODE_ENV", "production"), ("SKIP_ENV_VALIDATION", "1")];
    if let Some(secret) = secret {
        vars.push(("TURNSTILE_SITE_KEY", "site-key-1"));
        vars.push(("TURNSTILE_SECRET_KEY", secret));
    }
    let mut state = state_with(&vars);
    // Nothing listens there: fail at once instead of after the pool's 10 s.
    state.db = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(250))
        .connect_lazy("postgresql://nobody@127.0.0.1:1/none")
        .unwrap();
    state.turnstile = Arc::new(Turnstile::with_siteverify_url(url));
    App::new(state)
}

/// A POST from its own address (each test its own rate-limit buckets).
async fn post(app: &App, uri: &str, ip: &str, token: Option<&str>, body: Value) -> Answer {
    let mut req = Request::post(uri)
        .header("host", HOST)
        .header("origin", ORIGIN)
        .header("content-type", "application/json")
        .header("x-real-ip", ip);
    if uri.starts_with("/_rpc/") {
        req = req.header("x-modlight-rpc", "1");
    }
    if let Some(token) = token {
        req = req.header(TOKEN_HEADER, token);
    }
    call(app, req.body(Body::from(body.to_string())).unwrap()).await
}

const GUARDED: [&str; 3] = ["/api/auth/sign-in/email", "/api/auth/sign-up/email", "/api/auth/request-password-reset"];

#[tokio::test]
async fn better_auth_routes_answer_with_the_captcha_plugins_codes() {
    let stub = siteverify::start().await;
    let app = app(Some("good-secret"), &stub.url());
    for (i, uri) in GUARDED.iter().enumerate() {
        let ip = format!("198.51.100.{}", 10 + i);
        let a = post(&app, uri, &ip, None, json!({})).await;
        assert_eq!(a.status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(a.json(), json!({"message": "Missing CAPTCHA response", "code": "MISSING_RESPONSE"}), "{uri}");
        assert_eq!(a.header("content-type").as_deref(), Some("application/json"));

        let a = post(&app, uri, &ip, Some("nope"), json!({})).await;
        assert_eq!(a.status, StatusCode::FORBIDDEN, "{uri}");
        assert_eq!(a.json()["code"], "VERIFICATION_FAILED", "{uri}");
        assert_eq!(stub.last().get("remoteip"), Some(&ip), "the rate limiter's address goes to Cloudflare");

        // Let through: the endpoint itself answers (an empty body fails its
        // validation, or the database is not there; nothing captcha about it).
        let a = post(&app, uri, &ip, Some("pass-1"), json!({})).await;
        assert_let_through(&a, uri);
    }
}

fn assert_let_through(a: &Answer, what: &str) {
    let text = a.text();
    for code in ["MISSING_RESPONSE", "VERIFICATION_FAILED", "SERVICE_UNAVAILABLE"] {
        assert!(!text.contains(code), "{what}: {} {text}", a.status);
    }
    assert!(a.status != StatusCode::FORBIDDEN && a.status.as_u16() != 503, "{what}: {} {text}", a.status);
}

#[tokio::test]
async fn better_auth_routes_fail_closed_when_cloudflare_is_unreachable() {
    let closed = siteverify::closed().await;
    let app = app(Some("good-secret"), &format!("http://{closed}/siteverify"));
    let a = post(&app, "/api/auth/sign-in/email", "198.51.100.40", Some("pass-1"), json!({})).await;
    assert_eq!(a.status.as_u16(), 503);
    assert_eq!(a.json(), json!({"message": "CAPTCHA service unavailable", "code": "SERVICE_UNAVAILABLE"}));
}

#[tokio::test]
async fn other_auth_routes_and_methods_are_not_checked() {
    let stub = siteverify::start().await;
    let app = app(Some("good-secret"), &stub.url());
    let a = post(&app, "/api/auth/reset-password", "198.51.100.50", None, json!({})).await;
    assert_let_through(&a, "/reset-password");
    let a = call(
        &app,
        Request::get("/api/auth/sign-in/email")
            .header("host", HOST)
            .header("x-real-ip", "198.51.100.51")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(a.status, StatusCode::NOT_FOUND, "a GET finds no route, as before");
    assert_eq!(stub.calls(), 0);
}

#[tokio::test]
async fn without_configuration_nothing_is_checked_and_nothing_changes() {
    let stub = siteverify::start().await;
    let app = app(None, &stub.url());
    for (i, uri) in GUARDED.iter().enumerate() {
        let a = post(&app, uri, &format!("198.51.100.{}", 60 + i), None, json!({})).await;
        assert_let_through(&a, uri);
    }
    let a =
        post(&app, "/_rpc/login-identifier/signInWithIdentifier", "198.51.100.70", None, json!(["alice", "pw"])).await;
    assert_eq!(a.json(), json!({"ok": {"status": "error"}}), "the lookup ran (and met no database)");
    let a =
        post(&app, "/_rpc/invite/acceptInvitation", "198.51.100.71", None, json!(["t", "long enough password"])).await;
    assert_let_through(&a, "acceptInvitation");
    assert!(!a.text().contains("captcha"), "{}", a.text());
    assert_eq!(stub.calls(), 0);
    let page = call(&app, Request::get("/login").header("host", HOST).body(Body::empty()).unwrap()).await;
    let csp = page.header("content-security-policy").unwrap();
    assert!(!csp.contains("challenges.cloudflare.com"), "{csp}");
}

#[tokio::test]
async fn the_sign_in_action_is_refused_with_its_own_status() {
    let stub = siteverify::start().await;
    let app = app(Some("good-secret"), &stub.url());
    let path = "/_rpc/login-identifier/signInWithIdentifier";
    let a = post(&app, path, "198.51.100.80", None, json!(["alice", "pw"])).await;
    assert_eq!(a.json(), json!({"ok": {"status": "captcha"}}));
    let a = post(&app, path, "198.51.100.80", Some("nope"), json!(["alice", "pw"])).await;
    assert_eq!(a.json(), json!({"ok": {"status": "captcha"}}));
    // Let through: the lookup runs and meets the unreachable database.
    let a = post(&app, path, "198.51.100.80", Some("pass-1"), json!(["alice", "pw"])).await;
    assert_eq!(a.json(), json!({"ok": {"status": "error"}}));
    assert_eq!(stub.calls(), 2, "one siteverify per checked attempt, none for a missing token");

    let closed = siteverify::closed().await;
    let down = self::app(Some("good-secret"), &format!("http://{closed}/"));
    let a = post(&down, path, "198.51.100.81", Some("pass-1"), json!(["alice", "pw"])).await;
    assert_eq!(a.json(), json!({"ok": {"status": "captcha-unavailable"}}));
}

#[tokio::test]
async fn the_invitation_action_is_refused_with_its_own_reason() {
    let stub = siteverify::start().await;
    let app = app(Some("good-secret"), &stub.url());
    let path = "/_rpc/invite/acceptInvitation";
    let args = json!(["some-token", "long enough password"]);
    let a = post(&app, path, "198.51.100.90", None, args.clone()).await;
    assert_eq!(a.json(), json!({"ok": {"ok": false, "reason": "captcha"}}));
    let a = post(&app, path, "198.51.100.90", Some("nope"), args.clone()).await;
    assert_eq!(a.json(), json!({"ok": {"ok": false, "reason": "captcha"}}));
    let a = post(&app, path, "198.51.100.90", Some("pass-1"), args.clone()).await;
    assert!(a.json().get("ok").is_none_or(|ok| ok["reason"] != "captcha"), "{}", a.text());

    let closed = siteverify::closed().await;
    let down = self::app(Some("good-secret"), &format!("http://{closed}/"));
    let a = post(&down, path, "198.51.100.91", Some("pass-1"), args).await;
    assert_eq!(a.json(), json!({"ok": {"ok": false, "reason": "captcha-unavailable"}}));
}

#[tokio::test]
async fn the_csp_lets_the_widget_in_only_while_it_is_on() {
    let stub = siteverify::start().await;
    let app = app(Some("good-secret"), &stub.url());
    let page = call(&app, Request::get("/login").header("host", HOST).body(Body::empty()).unwrap()).await;
    let csp = page.header("content-security-policy").unwrap();
    assert!(csp.contains("script-src 'self' 'unsafe-inline' https://challenges.cloudflare.com;"), "{csp}");
    assert!(csp.contains("frame-src 'self' http://127.0.0.1:8421 https://challenges.cloudflare.com;"), "{csp}");
    assert!(csp.ends_with("connect-src 'self' http://127.0.0.1:8421"), "{csp}");
}
