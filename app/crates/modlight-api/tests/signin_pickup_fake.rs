//! The app's real sign-in code against a small stand-in server that speaks the pickup
//! contract of `server/src/routes/api/authorize_pickup.rs` (202 pending, 200 once,
//! 404 for unknown or mismatched) and `/api/me`. Hermetic: runs everywhere, unlike
//! `signin_pickup.rs`, which needs a real server.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use modlight_api::Api;
use modlight_api::signin::{self, SignInFailure};
use sha2::Digest;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

const TOKEN: &str = "fake-server-token-0123456789";

#[derive(Default)]
struct Fake {
    state: Option<String>,
    challenge: Option<String>,
    /// `Some(token)` or `Some("")` for a refusal, once decided.
    outcome: Option<String>,
    polls: u32,
    bad_verifier_polls: u32,
}

async fn serve(listener: TcpListener, fake: Arc<Mutex<Fake>>) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let fake = fake.clone();
        tokio::spawn(async move {
            let mut data = Vec::new();
            let mut buf = [0u8; 4096];
            let head_end = loop {
                let n = stream.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    return;
                }
                data.extend_from_slice(&buf[..n]);
                if let Some(p) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    break p;
                }
            };
            let head = String::from_utf8_lossy(&data[..head_end]).to_string();
            let length: usize = head
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse().ok())?
                })
                .unwrap_or(0);
            while data.len() < head_end + 4 + length {
                let n = stream.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
            }
            let body = &data[head_end + 4..];
            let line = head.lines().next().unwrap_or_default().to_string();
            let api_key = head.lines().find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case("x-api-key")
                    .then(|| v.trim().to_string())
            });

            let (status, json) = if line.starts_with("POST /api/authorize/pickup ") {
                assert!(api_key.is_none(), "the pickup must not carry x-api-key");
                let req: serde_json::Value = serde_json::from_slice(body).unwrap();
                let mut f = fake.lock().unwrap();
                f.polls += 1;
                let verifier = req["verifier"].as_str().unwrap_or_default();
                let presented = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode(sha2::Sha256::digest(verifier.as_bytes()));
                let matches = f.state.as_deref() == req["state"].as_str()
                    && f.challenge.as_deref() == Some(presented.as_str());
                if !matches {
                    f.bad_verifier_polls += 1;
                    (404, r#"{"error":"not_found"}"#.to_string())
                } else {
                    match f.outcome.take() {
                        None => (202, r#"{"status":"pending"}"#.to_string()),
                        Some(t) if t.is_empty() => {
                            f.state = None;
                            (200, r#"{"error":"access_denied"}"#.to_string())
                        }
                        Some(t) => {
                            f.state = None;
                            (200, format!(r#"{{"token":"{t}"}}"#))
                        }
                    }
                }
            } else if line.starts_with("GET /api/me ") {
                if api_key.as_deref() == Some(TOKEN) {
                    (
                        200,
                        r#"{"tag":"e2eTag","email":"e2e@example.com"}"#.to_string(),
                    )
                } else {
                    (401, r#"{"message":"Unauthorized"}"#.to_string())
                }
            } else {
                (404, "{}".to_string())
            };
            let reason = match status {
                200 => "OK",
                202 => "Accepted",
                401 => "Unauthorized",
                _ => "Not Found",
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
                json.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}

async fn start() -> (String, Arc<Mutex<Fake>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let fake = Arc::new(Mutex::new(Fake::default()));
    tokio::spawn(serve(listener, fake.clone()));
    (origin, fake)
}

/// The browser: reads the link, "approves" after `delay`, never posts to the loopback.
fn browser(fake: Arc<Mutex<Fake>>, outcome: &'static str, delay: Duration) -> impl FnOnce(&str) {
    move |url: &str| {
        let parsed = url::Url::parse(url).unwrap();
        let q: std::collections::HashMap<String, String> =
            parsed.query_pairs().into_owned().collect();
        assert_eq!(q["challenge"].len(), 43);
        {
            let mut f = fake.lock().unwrap();
            f.state = Some(q["state"].clone());
            f.challenge = Some(q["challenge"].clone());
        }
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            fake.lock().unwrap().outcome = Some(outcome.to_string());
        });
    }
}

#[tokio::test]
async fn the_pickup_signs_in_without_any_loopback_delivery() {
    let (origin, fake) = start().await;
    let api = Api::new("0.0.0-test").unwrap();
    let cancel = CancellationToken::new();
    let started = Instant::now();
    let signed = signin::sign_in_with_browser(
        &api,
        &origin,
        "Modlight test",
        browser(fake.clone(), TOKEN, Duration::from_millis(2500)),
        &cancel,
    )
    .await
    .expect("signed in through the pickup");
    assert_eq!(signed.token, TOKEN);
    assert_eq!(signed.account.tag.as_deref(), Some("e2eTag"));
    // Approved at 2.5 s, polled every 2 s: collected on the poll at about 4 s.
    let took = started.elapsed();
    assert!(took < Duration::from_secs(7), "{took:?}");
    let f = fake.lock().unwrap();
    assert!(f.polls >= 2, "pending polls came first");
    assert_eq!(f.bad_verifier_polls, 0);
}

#[tokio::test]
async fn a_refusal_through_the_pickup_ends_the_sign_in() {
    let (origin, fake) = start().await;
    let api = Api::new("0.0.0-test").unwrap();
    let cancel = CancellationToken::new();
    let failure = signin::sign_in_with_browser(
        &api,
        &origin,
        "Modlight test",
        browser(fake, "", Duration::from_millis(100)),
        &cancel,
    )
    .await
    .unwrap_err();
    assert_eq!(failure, SignInFailure::Denied(None));
}

#[tokio::test]
async fn cancelling_stops_the_polling() {
    let (origin, fake) = start().await;
    let api = Api::new("0.0.0-test").unwrap();
    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(4500)).await;
        stopper.cancel();
    });
    let failure = signin::sign_in_with_browser(
        &api,
        &origin,
        "Modlight test",
        browser(fake.clone(), TOKEN, Duration::from_secs(3600)),
        &cancel,
    )
    .await
    .unwrap_err();
    assert_eq!(failure, SignInFailure::Canceled);
    let polls = fake.lock().unwrap().polls;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(fake.lock().unwrap().polls, polls, "no polls after cancel");
}

#[tokio::test]
async fn a_server_without_the_pickup_leaves_the_loopback_working() {
    // Every pickup answers 404 (an older server): the loopback still delivers.
    let (origin, _fake) = start().await;
    let api = Api::new("0.0.0-test").unwrap();
    let cancel = CancellationToken::new();
    let origin_for_page = origin.clone();
    let signed = signin::sign_in_with_browser(
        &api,
        &origin,
        "Modlight test",
        move |url: &str| {
            let parsed = url::Url::parse(url).unwrap();
            let q: std::collections::HashMap<String, String> =
                parsed.query_pairs().into_owned().collect();
            let port: u16 = q["port"].parse().unwrap();
            let body = format!(r#"{{"state":"{}","token":"{TOKEN}"}}"#, q["state"]);
            std::thread::spawn(move || {
                use std::io::{Read, Write};
                std::thread::sleep(Duration::from_millis(2500));
                let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
                write!(
                    s,
                    "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: {origin_for_page}\r\nContent-Type: text/plain;charset=UTF-8\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                let mut r = String::new();
                let _ = s.read_to_string(&mut r);
            });
        },
        &cancel,
    )
    .await
    .expect("signed in through the loopback");
    assert_eq!(signed.token, TOKEN);
}
