//! End to end: the app's real browser sign-in against a real Rust server, with the
//! browser played by this test (the approval goes through the same RPC action the
//! /authorize page calls, with a real session).
//!
//! Runs only when `MODLIGHT_PICKUP_E2E` names an env file with
//!
//! ```text
//! HOST=http://127.0.0.1:8461     # a server with POST /api/authorize/pickup
//! EMAIL=...                      # an account on it (email + password sign-in)
//! PASSWORD=...
//! ```
//!
//! Without the variable the tests say so and pass. Values are never printed.
//!
//! * `pickup_signs_in_when_the_browser_never_delivers`: the loopback POST is NOT
//!   made (what Safari does with an https page's request to 127.0.0.1); the app
//!   must end up signed in through the server pickup within a few seconds.
//! * `loopback_still_signs_in_when_the_browser_delivers`: the old path.
//! * `a_refusal_arrives_through_the_pickup`: deny without any loopback POST.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use modlight_api::Api;
use modlight_api::signin::{self, SignInFailure};
use tokio_util::sync::CancellationToken;

struct Env {
    host: String,
    email: String,
    password: String,
}

fn env() -> Option<Env> {
    let Some(path) = std::env::var_os("MODLIGHT_PICKUP_E2E").map(PathBuf::from) else {
        eprintln!("sign-in pickup e2e skipped: MODLIGHT_PICKUP_E2E is not set");
        return None;
    };
    let text = std::fs::read_to_string(path).expect("MODLIGHT_PICKUP_E2E file");
    let map: HashMap<String, String> = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    Some(Env {
        host: map["HOST"].trim_end_matches('/').to_string(),
        email: map["EMAIL"].clone(),
        password: map["PASSWORD"].clone(),
    })
}

/// A browser session: email + password through Better Auth, cookies kept by hand.
async fn browser_session(env: &Env) -> String {
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = http
        .post(format!("{}/api/auth/sign-in/email", env.host))
        .header("origin", &env.host)
        .json(&serde_json::json!({ "email": env.email, "password": env.password }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "sign-in answered {}",
        response.status()
    );
    let cookies: Vec<String> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .map(str::to_string)
        .collect();
    assert!(!cookies.is_empty(), "no session cookie");
    cookies.join("; ")
}

/// What the /authorize page does: load the page (registers the pending pickup),
/// then call the action the buttons call.
async fn act_in_browser(host: &str, cookie: &str, url: &str, action: &str) -> serde_json::Value {
    let parsed = url::Url::parse(url).unwrap();
    assert!(url.starts_with(host));
    let query: HashMap<String, String> = parsed.query_pairs().into_owned().collect();
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let page = http
        .get(format!(
            "{host}/_page?path=%2Fauthorize&{}",
            parsed.query().unwrap()
        ))
        .header("cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), 200, "authorize page payload");
    let payload: serde_json::Value = page.json().await.unwrap();
    assert_eq!(
        payload["props"]["parsed"]["request"]["challenge"], query["challenge"],
        "the page carries the challenge"
    );

    let input = serde_json::json!([{
        "port": query["port"],
        "state": query["state"],
        "name": query["name"],
        "challenge": query["challenge"],
    }]);
    let answer: serde_json::Value = http
        .post(format!("{host}/_rpc/authorize/{action}"))
        .header("cookie", cookie)
        .header("origin", host)
        .header("x-modlight-rpc", "1")
        .header("content-type", "application/json")
        .body(input.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(answer["ok"]["ok"], true, "{action} refused: {answer}");
    answer["ok"].clone()
}

/// The page's loopback POST, as a browser that is allowed to make it would.
fn post_like_the_page(port: u16, origin: &str, body: &str) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "POST / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {origin}\r\nContent-Type: text/plain;charset=UTF-8\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut reply = String::new();
    let _ = stream.read_to_string(&mut reply);
}

async fn sign_in(
    env: &Env,
    cookie: String,
    action: &'static str,
    deliver_loopback: bool,
) -> (Result<signin::SignedIn, SignInFailure>, Duration) {
    let api = Api::new("0.0.0-e2e").unwrap();
    let cancel = CancellationToken::new();
    let host = env.host.clone();
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let mut tx = Some(tx);
    let open = move |url: &str| {
        let _ = tx.take().unwrap().send(url.to_string());
    };
    let browser = async move {
        let url = rx.await.unwrap();
        let out = act_in_browser(&host, &cookie, &url, action).await;
        let approved = Instant::now();
        if deliver_loopback {
            let parsed = url::Url::parse(&url).unwrap();
            let q: HashMap<String, String> = parsed.query_pairs().into_owned().collect();
            let port: u16 = q["port"].parse().unwrap();
            let body =
                serde_json::json!({ "state": q["state"], "token": out["token"] }).to_string();
            let origin = host.clone();
            tokio::task::spawn_blocking(move || post_like_the_page(port, &origin, &body))
                .await
                .unwrap();
        }
        approved
    };
    let flow = signin::sign_in_with_browser(&api, &env.host, "Modlight e2e", open, &cancel);
    let (result, approved) = tokio::join!(flow, browser);
    (result, approved.elapsed())
}

#[tokio::test]
async fn pickup_signs_in_when_the_browser_never_delivers() {
    let Some(env) = env() else { return };
    let cookie = browser_session(&env).await;
    let (result, after_approval) = sign_in(&env, cookie, "approveDesktopClient", false).await;
    let signed = result.expect("signed in through the pickup");
    assert_eq!(signed.origin, env.host);
    assert!(signed.account.tag.is_some() || signed.account.email.is_some());
    eprintln!("pickup: signed in {after_approval:?} after the approval");
    assert!(
        after_approval < Duration::from_secs(5),
        "{after_approval:?}"
    );
}

#[tokio::test]
async fn loopback_still_signs_in_when_the_browser_delivers() {
    let Some(env) = env() else { return };
    let cookie = browser_session(&env).await;
    let (result, after_approval) = sign_in(&env, cookie, "approveDesktopClient", true).await;
    result.expect("signed in through the loopback");
    eprintln!("loopback: signed in {after_approval:?} after the approval");
    // The loopback answers at once; the pickup's first poll is 2 s away.
    assert!(
        after_approval < Duration::from_secs(2),
        "{after_approval:?}"
    );
}

#[tokio::test]
async fn a_refusal_arrives_through_the_pickup() {
    let Some(env) = env() else { return };
    let cookie = browser_session(&env).await;
    let (result, after) = sign_in(&env, cookie, "denyDesktopClient", false).await;
    assert_eq!(result.unwrap_err(), SignInFailure::Denied(None));
    assert!(after < Duration::from_secs(5), "{after:?}");
}
