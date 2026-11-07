//! The account's "Remove metadata from files on upload" switch against a stand-in server
//! on 127.0.0.1 (the server's `docs/CONTRACT.md` section 11): reading it, setting it, the token
//! only in `x-api-key`, and every refusal as its own error: 401 signs out, 403 is a token
//! too narrow, 404 a server without the switch.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use modlight_api::{Api, ApiError, Credential, STRIP_METADATA_PATH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const TOKEN: &str = "token-of-alex-0123456789";

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

type Reply = (u16, &'static str);

/// A stand-in that answers with `answer(request)` and keeps every request.
async fn stand_in(
    answer: impl Fn(&Seen) -> Reply + Send + Sync + 'static,
) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let (kept, answer) = (log.clone(), Arc::new(answer));
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let (kept, answer) = (kept.clone(), answer.clone());
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut buffer = [0u8; 4096];
                let head_end = loop {
                    if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        break at + 4;
                    }
                    match stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => data.extend_from_slice(&buffer[..n]),
                    }
                };
                let head = String::from_utf8_lossy(&data[..head_end]).to_string();
                let mut lines = head.split("\r\n");
                let mut first = lines.next().unwrap_or("").split(' ');
                let (method, path) = (
                    first.next().unwrap_or("").to_string(),
                    first.next().unwrap_or("").to_string(),
                );
                let headers: HashMap<String, String> = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
                    .collect();
                let length: usize = headers
                    .get("content-length")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                while data.len() < head_end + length {
                    match stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => data.extend_from_slice(&buffer[..n]),
                    }
                }
                let seen = Seen {
                    method,
                    path,
                    headers,
                    body: String::from_utf8_lossy(&data[head_end..]).to_string(),
                };
                let (status, body) = answer(&seen);
                kept.lock().unwrap().push(seen);
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    (origin, log)
}

/// A server that keeps the switch and checks the token.
fn keeper() -> impl Fn(&Seen) -> Reply + Send + Sync + 'static {
    let value = Arc::new(Mutex::new(false));
    move |seen: &Seen| {
        if seen.headers.get("x-api-key").map(String::as_str) != Some(TOKEN) {
            return (401, r#"{"message":"Token not specified or not found."}"#);
        }
        match (seen.method.as_str(), seen.path.as_str()) {
            ("GET", STRIP_METADATA_PATH) => {
                if *value.lock().unwrap() {
                    (200, r#"{"enabled":true}"#)
                } else {
                    (200, r#"{"enabled":false}"#)
                }
            }
            ("PUT", STRIP_METADATA_PATH) => {
                let on = seen.body.contains(r#""enabled":true"#);
                *value.lock().unwrap() = on;
                if on {
                    (200, r#"{"enabled":true}"#)
                } else {
                    (200, r#"{"enabled":false}"#)
                }
            }
            _ => (404, "{}"),
        }
    }
}

#[tokio::test]
async fn reads_and_sets_the_switch_with_the_token_in_its_header_only() {
    let (origin, log) = stand_in(keeper()).await;
    let api = Api::new("0.7.7").unwrap();
    let credential = Credential::new(origin.as_str(), TOKEN);
    assert_eq!(api.get_strip_metadata(&credential).await, Ok(false));
    assert_eq!(api.set_strip_metadata(&credential, true).await, Ok(true));
    assert_eq!(api.get_strip_metadata(&credential).await, Ok(true));
    assert_eq!(api.set_strip_metadata(&credential, false).await, Ok(false));
    let seen = log.lock().unwrap().clone();
    assert_eq!(
        seen.iter()
            .map(|s| format!("{} {}", s.method, s.path))
            .collect::<Vec<_>>(),
        vec![
            "GET /api/strip-metadata",
            "PUT /api/strip-metadata",
            "GET /api/strip-metadata",
            "PUT /api/strip-metadata"
        ]
    );
    let put = &seen[1];
    assert_eq!(put.body, r#"{"enabled":true}"#);
    assert!(put.headers["content-type"].starts_with("application/json"));
    for s in &seen {
        assert_eq!(s.headers.get("x-api-key").map(String::as_str), Some(TOKEN));
        assert!(!s.path.contains(TOKEN), "the token never travels in a URL");
    }
}

#[tokio::test]
async fn every_refusal_is_its_own_error() {
    let api = Api::new("0.7.7").unwrap();
    for (status, body, expected) in [
        (
            401,
            r#"{"message":"Token not specified or not found."}"#,
            ApiError::Unauthorized,
        ),
        (
            403,
            r#"{"message":"This token may not do that."}"#,
            ApiError::Refused {
                status: 403,
                detail: "This token may not do that.".into(),
                reason: None,
            },
        ),
        (404, "{}", ApiError::NotSupported),
        (
            400,
            r#"{"message":"enabled must be true or false."}"#,
            ApiError::Refused {
                status: 400,
                detail: "enabled must be true or false.".into(),
                reason: None,
            },
        ),
        (
            500,
            "",
            ApiError::Refused {
                status: 500,
                detail: String::new(),
                reason: None,
            },
        ),
        (
            302,
            "",
            ApiError::Refused {
                status: 302,
                detail: String::new(),
                reason: None,
            },
        ),
    ] {
        let (origin, _) = stand_in(move |_| (status, body)).await;
        let credential = Credential::new(origin.as_str(), TOKEN);
        assert_eq!(
            api.set_strip_metadata(&credential, true).await,
            Err(expected.clone()),
            "{status}"
        );
        assert_eq!(
            api.get_strip_metadata(&credential).await,
            Err(expected),
            "{status}"
        );
    }
    // An answer that is not the switch.
    let (origin, _) = stand_in(|_| (200, r#"{"enabled":"yes"}"#)).await;
    let credential = Credential::new(origin.as_str(), TOKEN);
    assert!(matches!(
        api.get_strip_metadata(&credential).await,
        Err(ApiError::Decode(_))
    ));
    // Nobody there.
    let credential = Credential::new("http://127.0.0.1:9", TOKEN);
    assert!(matches!(
        api.get_strip_metadata(&credential).await,
        Err(ApiError::Transport(_) | ApiError::Timeout)
    ));
}
