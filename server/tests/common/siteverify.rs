//! A stand-in for Cloudflare's siteverify endpoint on 127.0.0.1 (the
//! optional Turnstile check, src/turnstile.rs). Answers like Cloudflare does:
//!
//! - secret `bad-secret`: `{"success":false,"error-codes":["invalid-input-secret"]}`;
//! - secret `always-secret`: success for any token (Cloudflare's "always
//!   passes" test secret);
//! - any other secret: success when the token starts with `pass`, else
//!   `invalid-input-response`.
//!
//! Every request body is recorded, decoded, for the assertions.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub struct Stub {
    pub addr: SocketAddr,
    pub requests: Arc<Mutex<Vec<HashMap<String, String>>>>,
}

impl Stub {
    pub fn url(&self) -> String {
        format!("http://{}/turnstile/v0/siteverify", self.addr)
    }

    pub fn calls(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    pub fn last(&self) -> HashMap<String, String> {
        self.requests.lock().unwrap().last().cloned().expect("a siteverify call")
    }
}

fn answer(form: &HashMap<String, String>) -> &'static str {
    let secret = form.get("secret").map(String::as_str).unwrap_or("");
    let token = form.get("response").map(String::as_str).unwrap_or("");
    match secret {
        "bad-secret" => r#"{"success":false,"error-codes":["invalid-input-secret"]}"#,
        "always-secret" => r#"{"success":true,"error-codes":[],"hostname":"example.com"}"#,
        _ if token.starts_with("pass") => r#"{"success":true,"error-codes":[],"hostname":"127.0.0.1"}"#,
        _ => r#"{"success":false,"error-codes":["invalid-input-response"]}"#,
    }
}

/// Start the stub; it serves until the test's runtime ends.
pub async fn start() -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind the siteverify stub");
    let addr = listener.local_addr().unwrap();
    let requests: Arc<Mutex<Vec<HashMap<String, String>>>> = Arc::default();
    let seen = requests.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                // Headers, then Content-Length bytes of body.
                let (head_end, length) = loop {
                    let Ok(n) = socket.read(&mut chunk).await else { return };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..i]).to_lowercase();
                        let length = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (i + 4, length);
                    }
                };
                while buf.len() < head_end + length {
                    let Ok(n) = socket.read(&mut chunk).await else { return };
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let body = &buf[head_end..(head_end + length).min(buf.len())];
                let form: HashMap<String, String> = form_urlencoded::parse(body).into_owned().collect();
                let reply = answer(&form);
                seen.lock().unwrap().push(form);
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    Stub { addr, requests }
}

/// A listener that accepts and never answers (the timeout case).
pub async fn silent() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind the silent stub");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    addr
}

/// An address nothing listens on (the network-failure case).
pub async fn closed() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}
