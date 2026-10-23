//! Not upstream: `POST /api/authorize/pickup`, where the desktop app collects
//! what the person decided on `/authorize` when the browser never delivered
//! the loopback POST (see [`crate::authorize::pickup`] for the whole design).
//!
//! Body `{"state": "...", "verifier": "..."}`, JSON, read by
//! [`read_json_object`]. No session and no API key: the verifier is the
//! credential, and it only ever existed inside the app that wrote the link.
//!
//! * `200 {"token": "..."}` or `200 {"error": "access_denied"}`: once, then the
//!   entry is gone;
//! * `202 {"status": "pending"}`: the page was opened, nothing decided yet;
//! * `404 {"error": "not_found"}`: unknown state, expired, or wrong verifier,
//!   one answer for all three so the route is no oracle;
//! * `400 {"error": "invalid_request"}`, plus the body reader's own 400 / 413 /
//!   415 refusals: the app stops polling on these;
//! * `429 {"error": "rate_limited"}` with `Retry-After`: the app waits.
//!
//! Rate-limited by client IP like the other unauthenticated auth routes. An
//! app polls every 2 seconds, so the budget is generous enough for several
//! sign-ins behind one address. Neither the token nor the verifier is ever logged.

use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use axum::routing::post;
use serde_json::{Value, json};

use crate::api_json::{json_response, read_json_object};
use crate::authorize::loopback::parse_state;
use crate::authorize::pickup::{Outcome, Pickup, PickupStore, is_plausible_verifier, now_ms, store};
use crate::state::AppState;

/// The body is two short strings.
pub const BODY_MAX: usize = 1024;

/// Polls per client address per window: 2 s polling is 30 a minute, so this
/// leaves room for four sign-ins at once behind one NAT.
pub const POLLS_PER_WINDOW: u32 = 120;
pub const POLL_WINDOW_MS: i64 = 60 * 1000;

const NO_STORE: &str = "no-store";

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/authorize/pickup", post(pickup))
}

async fn pickup(State(state): State<AppState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    let ip = crate::password::client_ip(&parts.headers, peer, state.config.trust_proxy);
    handle(store(), &ip, &parts.headers, body, now_ms()).await
}

fn answer(status: StatusCode, body: &Value) -> Response {
    json_response(status, body, Some(NO_STORE))
}

/// The route without axum's extractors: what the tests drive.
pub async fn handle(store: &PickupStore, ip: &str, headers: &HeaderMap, body: Body, now: i64) -> Response {
    let gate = crate::password::check_rate_limit(&format!("authorize-pickup:{ip}"), POLLS_PER_WINDOW, POLL_WINDOW_MS);
    if !gate.allowed {
        let mut response = answer(StatusCode::TOO_MANY_REQUESTS, &json!({ "error": "rate_limited" }));
        let seconds = (gate.retry_after_ms.max(0) + 999) / 1000;
        if let Ok(value) = HeaderValue::from_str(&seconds.max(1).to_string()) {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        return response;
    }

    let body = match read_json_object(headers, body, BODY_MAX).await {
        Ok(b) => b,
        Err(e) => return answer(e.status, &json!({ "error": e.message })),
    };
    let state = parse_state(body.get("state"));
    let verifier = body.get("verifier").and_then(Value::as_str).filter(|v| is_plausible_verifier(v));
    let (Some(state), Some(verifier)) = (state, verifier) else {
        return answer(StatusCode::BAD_REQUEST, &json!({ "error": "invalid_request" }));
    };

    match store.pick_up(&state, verifier, now) {
        Pickup::Ready(Outcome::Token(token)) => answer(StatusCode::OK, &json!({ "token": token })),
        Pickup::Ready(Outcome::Denied) => answer(StatusCode::OK, &json!({ "error": "access_denied" })),
        Pickup::Pending => answer(StatusCode::ACCEPTED, &json!({ "status": "pending" })),
        Pickup::NotFound => answer(StatusCode::NOT_FOUND, &json!({ "error": "not_found" })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorize::pickup::challenge_for;

    const STATE: &str = "Abcdefgh12345678Abcdefgh12345678";
    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

    fn json_headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        h
    }

    fn ip() -> String {
        format!("pickup-test-{}", rand::random::<u64>())
    }

    async fn call(store: &PickupStore, ip: &str, headers: &HeaderMap, body: &str) -> (StatusCode, Value, HeaderMap) {
        let response = handle(store, ip, headers, Body::from(body.to_string()), now_ms()).await;
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 16).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null), headers)
    }

    fn body(state: &str, verifier: &str) -> String {
        json!({ "state": state, "verifier": verifier }).to_string()
    }

    #[tokio::test]
    async fn pending_then_the_token_once_then_not_found() {
        let store = PickupStore::default();
        let ip = ip();
        let h = json_headers();
        let (status, out, headers) = call(&store, &ip, &h, &body(STATE, VERIFIER)).await;
        assert_eq!((status, out.clone()), (StatusCode::NOT_FOUND, json!({ "error": "not_found" })));
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");

        store.register(STATE, &challenge_for(VERIFIER), "u1", now_ms());
        let (status, out, _) = call(&store, &ip, &h, &body(STATE, VERIFIER)).await;
        assert_eq!((status, out), (StatusCode::ACCEPTED, json!({ "status": "pending" })));

        store.complete(STATE, &challenge_for(VERIFIER), "u1", Outcome::Token("the-token".into()), now_ms());
        let (status, out, _) = call(&store, &ip, &h, &body(STATE, VERIFIER)).await;
        assert_eq!((status, out), (StatusCode::OK, json!({ "token": "the-token" })));

        let (status, out, _) = call(&store, &ip, &h, &body(STATE, VERIFIER)).await;
        assert_eq!((status, out), (StatusCode::NOT_FOUND, json!({ "error": "not_found" })));
    }

    #[tokio::test]
    async fn a_denial_is_delivered() {
        let store = PickupStore::default();
        store.register(STATE, &challenge_for(VERIFIER), "u1", now_ms());
        store.complete(STATE, &challenge_for(VERIFIER), "u1", Outcome::Denied, now_ms());
        let (status, out, _) = call(&store, &ip(), &json_headers(), &body(STATE, VERIFIER)).await;
        assert_eq!((status, out), (StatusCode::OK, json!({ "error": "access_denied" })));
    }

    #[tokio::test]
    async fn unknown_expired_and_mismatched_look_the_same() {
        let store = PickupStore::default();
        let ip = ip();
        let h = json_headers();
        store.register(STATE, &challenge_for(VERIFIER), "u1", now_ms());
        store.complete(STATE, &challenge_for(VERIFIER), "u1", Outcome::Token("t".into()), now_ms());
        let other_state = "Zbcdefgh12345678Abcdefgh12345678";
        let long_ago = now_ms() - 10 * 60 * 1000;
        store.register(other_state, &challenge_for(VERIFIER), "u1", long_ago);
        store.complete(other_state, &challenge_for(VERIFIER), "u1", Outcome::Token("t".into()), long_ago);

        let unknown = call(&store, &ip, &h, &body("Qbcdefgh12345678Abcdefgh12345678", VERIFIER)).await;
        let expired = call(&store, &ip, &h, &body(other_state, VERIFIER)).await;
        let mismatch = call(&store, &ip, &h, &body(STATE, &"A".repeat(43))).await;
        for (status, out, _) in [&unknown, &expired, &mismatch] {
            assert_eq!(*status, StatusCode::NOT_FOUND);
            assert_eq!(*out, json!({ "error": "not_found" }));
        }
        // The mismatch took nothing.
        let (status, _, _) = call(&store, &ip, &h, &body(STATE, VERIFIER)).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn malformed_requests_are_refused() {
        let store = PickupStore::default();
        let ip = ip();
        let (status, _, _) = call(&store, &ip, &HeaderMap::new(), &body(STATE, VERIFIER)).await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        for b in [
            "[]".to_string(),
            "{}".to_string(),
            body("short", VERIFIER),
            body(STATE, "short"),
            json!({ "state": STATE, "verifier": 5 }).to_string(),
        ] {
            let (status, _, _) = call(&store, &ip, &json_headers(), &b).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{b}");
        }
        let (status, _, _) = call(&store, &ip, &json_headers(), &"x".repeat(BODY_MAX + 1)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn polling_is_rate_limited_per_address() {
        let store = PickupStore::default();
        let ip = ip();
        let h = json_headers();
        for _ in 0..POLLS_PER_WINDOW {
            assert_eq!(call(&store, &ip, &h, &body(STATE, VERIFIER)).await.0, StatusCode::NOT_FOUND);
        }
        let (status, out, headers) = call(&store, &ip, &h, &body(STATE, VERIFIER)).await;
        assert_eq!((status, out), (StatusCode::TOO_MANY_REQUESTS, json!({ "error": "rate_limited" })));
        assert!(headers.contains_key(header::RETRY_AFTER));
        // Another address is not affected.
        assert_eq!(call(&store, &self::ip(), &h, &body(STATE, VERIFIER)).await.0, StatusCode::NOT_FOUND);
    }
}
