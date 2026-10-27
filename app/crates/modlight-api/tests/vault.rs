//! The file vault's calls against a stand-in server on 127.0.0.1 (the server's
//! `docs/ENCRYPTION.md` section 6): the status, unlocking and its refusals, locking, the
//! grant on content requests only, and `423 Locked` as its own error, never as a token
//! the server refused.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use modlight_api::{
    Api, ApiError, Credential, FilesLocked, MotionPreview, UnlockFailure, UnlockGrant, VaultState,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const TOKEN: &str = "token-of-alex-0123456789";
const GRANT: &str =
    "6f1c2a9e-0000-4000-8000-000000000001.c2VjcmV0LW9mLWFsZXgtMDEyMzQ1Njc4OWFiY2RlZg";
const PASSWORD: &str = "correct horse battery staple";

/// One request as the stand-in saw it.
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

type Reply = (u16, Vec<(&'static str, String)>, Vec<u8>);

/// A stand-in server: `answer` decides each reply from the request; every request is
/// kept in the returned log.
async fn stand_in(
    answer: impl Fn(&Seen) -> Reply + Send + Sync + 'static,
) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let answer = Arc::new(answer);
    let kept = log.clone();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let answer = answer.clone();
            let kept = kept.clone();
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
                let method = first.next().unwrap_or("").to_string();
                let path = first.next().unwrap_or("").to_string();
                let headers: HashMap<String, String> = lines
                    .filter_map(|line| line.split_once(':'))
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
                let body = String::from_utf8_lossy(&data[head_end..]).to_string();
                let seen = Seen {
                    method,
                    path,
                    headers,
                    body,
                };
                let (status, extra, payload) = answer(&seen);
                kept.lock().unwrap().push(seen);
                let mut reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                    payload.len()
                );
                for (name, value) in extra {
                    reply.push_str(&format!("{name}: {value}\r\n"));
                }
                reply.push_str("\r\n");
                let _ = stream.write_all(reply.as_bytes()).await;
                let _ = stream.write_all(&payload).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (origin, log)
}

fn json(status: u16, body: &str) -> Reply {
    (
        status,
        vec![("Content-Type", "application/json".into())],
        body.as_bytes().to_vec(),
    )
}

/// What the server's gate answers a bearer client asking for a locked file.
fn locked_reply() -> Reply {
    (
        423,
        vec![
            ("Content-Type", "application/json; charset=utf-8".into()),
            ("X-Modlight-Locked", "1".into()),
        ],
        br#"{"error":"files_locked","message":"This account's files are encrypted.","owner":"alex"}"#.to_vec(),
    )
}

/// A server whose account `alex` has its files encrypted and opened by [`GRANT`] only.
async fn encrypted_server() -> (String, Arc<Mutex<Vec<Seen>>>) {
    stand_in(|seen| {
        let unlocked = seen.header("x-modlight-unlock") == Some(GRANT);
        if seen.header("x-api-key") != Some(TOKEN) {
            return json(401, r#"{"message":"Token not specified or not found."}"#);
        }
        let path = seen.path.split('?').next().unwrap_or("");
        match (seen.method.as_str(), path) {
            ("GET", "/api/encryption") => json(
                200,
                &format!(
                    r#"{{"state":"on","unlocked":{unlocked},"progress":{{"done":9,"total":9}}}}"#
                ),
            ),
            ("GET", "/api/uploads") => json(200, r#"{"page":1,"perPage":30,"total":0,"items":[]}"#),
            ("GET", "/api/me") => json(200, r#"{"tag":"alex"}"#),
            ("GET", _) if path.starts_with("/alex/") => {
                if !unlocked {
                    return locked_reply();
                }
                if path.ends_with("/missing.webp") {
                    return (404, vec![], Vec::new());
                }
                if seen.path.contains("check=1") {
                    return (200, vec![], Vec::new());
                }
                (200, vec![], b"PICTURE".to_vec())
            }
            _ => (404, vec![], Vec::new()),
        }
    })
    .await
}

fn api() -> Api {
    Api::new("0.7.7").unwrap()
}

#[tokio::test]
async fn the_status_says_whether_the_grant_opens_the_files() {
    let (origin, log) = encrypted_server().await;
    let api = api();
    let me = Credential::new(origin.as_str(), TOKEN);

    let locked = api.get_encryption(&me).await.unwrap();
    assert_eq!(locked.status.state, VaultState::On);
    assert!(!locked.status.unlocked);
    assert_eq!(locked.status.progress.total, 9);
    assert_eq!(locked.grant, None, "no grant was held, none was sent");

    let number = api.hold_grant(&me, &UnlockGrant::new(GRANT, None));
    let open = api.get_encryption(&me).await.unwrap();
    assert!(open.status.unlocked);
    assert_eq!(open.grant, Some(number));
    let seen = log.lock().unwrap().clone();
    assert_eq!(seen[0].header("x-modlight-unlock"), None);
    assert_eq!(seen[1].header("x-modlight-unlock"), Some(GRANT));
    assert_eq!(seen[1].header("x-api-key"), Some(TOKEN));

    // A refused token is still a refused token.
    let wrong = Credential::new(origin.as_str(), "token-of-nobody-000000");
    assert_eq!(
        api.get_encryption(&wrong).await.unwrap_err(),
        ApiError::Unauthorized
    );
}

/// A server that does not know file encryption (the TypeScript server, an older Rust
/// one) answers 404: nothing of it is ever locked.
#[tokio::test]
async fn a_server_without_encryption_reads_as_off() {
    let (origin, _) = stand_in(|_| (404, vec![], Vec::new())).await;
    let me = Credential::new(origin.as_str(), TOKEN);
    let answer = api().get_encryption(&me).await.unwrap();
    assert_eq!(answer.status.state, VaultState::Off);
    assert!(!answer.status.state.is_gated());
}

#[tokio::test]
async fn unlocking_trades_the_password_for_a_grant() {
    let (origin, log) = stand_in(|seen| {
        let body: serde_json::Value = serde_json::from_str(&seen.body).unwrap_or_default();
        if body["password"] == PASSWORD {
            json(
                200,
                &format!(r#"{{"grant":"{GRANT}","expiresAt":"2026-11-26T12:00:00.000Z"}}"#),
            )
        } else {
            json(401, r#"{"error":"wrong_password"}"#)
        }
    })
    .await;
    let api = api();
    let me = Credential::new(origin.as_str(), TOKEN);
    // Even with a grant held, asking for a new one sends none.
    api.hold_grant(&me, &UnlockGrant::new("old.grant", None));

    let grant = api
        .unlock_files(&me, PASSWORD, "Modlight on QA Mac")
        .await
        .unwrap();
    assert_eq!(grant.value, GRANT);
    assert_eq!(
        grant.expires_at.unwrap().to_rfc3339(),
        "2026-11-26T12:00:00+00:00"
    );
    let seen = log.lock().unwrap()[0].clone();
    assert_eq!(seen.method, "POST");
    assert_eq!(seen.path, "/api/encryption/unlock");
    assert_eq!(seen.header("x-api-key"), Some(TOKEN));
    assert_eq!(seen.header("x-modlight-unlock"), None);
    assert!(
        seen.header("content-type")
            .unwrap()
            .starts_with("application/json")
    );
    let body: serde_json::Value = serde_json::from_str(&seen.body).unwrap();
    assert_eq!(body["password"], PASSWORD);
    assert_eq!(body["label"], "Modlight on QA Mac");

    assert_eq!(
        api.unlock_files(&me, "not it", "Modlight").await,
        Err(UnlockFailure::WrongPassword)
    );
}

#[tokio::test]
async fn each_refusal_of_an_unlock_is_told_apart() {
    let (origin, _) = stand_in(|seen| match seen.body.as_str() {
        b if b.contains("limited-body") => (
            429,
            vec![("Retry-After", "7".into())],
            br#"{"error":"rate_limited","retryAfter":420}"#.to_vec(),
        ),
        b if b.contains("limited-header") => (429, vec![("Retry-After", "90".into())], Vec::new()),
        b if b.contains("limited-silent") => json(429, r#"{"error":"rate_limited"}"#),
        b if b.contains("not-encrypted") => json(409, r#"{"error":"not_encrypted"}"#),
        b if b.contains("token-gone") => {
            json(401, r#"{"message":"Token not specified or not found."}"#)
        }
        b if b.contains("narrow") => json(403, r#"{"message":"Forbidden"}"#),
        b if b.contains("broken") => json(500, r#"{"error":"internal"}"#),
        b if b.contains("no-grant") => json(200, r#"{"expiresAt":"2026-11-26T12:00:00Z"}"#),
        b if b.contains("odd-grant") => json(200, r#"{"grant":"two\nlines.x"}"#),
        _ => json(400, r#"{"error":"invalid_request"}"#),
    })
    .await;
    let api = api();
    let me = Credential::new(origin.as_str(), TOKEN);
    let unlock = |password: &'static str| {
        let api = api.clone();
        let me = me.clone();
        async move { api.unlock_files(&me, password, "").await.unwrap_err() }
    };
    assert_eq!(
        unlock("limited-body").await,
        UnlockFailure::RateLimited(Duration::from_secs(420)),
        "the body's figure first"
    );
    assert_eq!(
        unlock("limited-header").await,
        UnlockFailure::RateLimited(Duration::from_secs(90))
    );
    assert_eq!(
        unlock("limited-silent").await,
        UnlockFailure::RateLimited(Duration::from_secs(60))
    );
    assert_eq!(unlock("not-encrypted").await, UnlockFailure::NotEncrypted);
    assert_eq!(unlock("").await, UnlockFailure::Invalid);
    // A 401 that is not about the password is about the token, as everywhere else.
    assert_eq!(
        unlock("token-gone").await,
        UnlockFailure::Failed(ApiError::Unauthorized)
    );
    assert_eq!(
        unlock("narrow").await,
        UnlockFailure::Failed(ApiError::Unauthorized)
    );
    assert_eq!(
        unlock("broken").await,
        UnlockFailure::Failed(ApiError::Status(500))
    );
    assert!(matches!(
        unlock("no-grant").await,
        UnlockFailure::Failed(ApiError::Decode(_))
    ));
    assert!(matches!(
        unlock("odd-grant").await,
        UnlockFailure::Failed(ApiError::Decode(_))
    ));
}

#[tokio::test]
async fn locking_revokes_the_grant_it_names() {
    let (origin, log) = stand_in(|_| (204, vec![], Vec::new())).await;
    let me = Credential::new(origin.as_str(), TOKEN);
    api()
        .lock_files(&me, &UnlockGrant::new(GRANT, None))
        .await
        .unwrap();
    let seen = log.lock().unwrap()[0].clone();
    assert_eq!(
        (seen.method.as_str(), seen.path.as_str()),
        ("POST", "/api/encryption/lock")
    );
    assert_eq!(seen.header("x-modlight-unlock"), Some(GRANT));
}

/// Thumbnails, moving tiles, conversions and downloads of a locked account answer 423:
/// each says so as its own error, and each carries the grant once one is held.
#[tokio::test]
async fn content_requests_carry_the_grant_and_say_when_they_are_locked() {
    let (origin, log) = encrypted_server().await;
    let api = api();
    let me = Credential::new(origin.as_str(), TOKEN);
    // Links come back on the account's link host; the token and grant go to `me` only.
    let thumb = "https://files.example.com/alex/abc.webp?w=480";
    let motion = "https://files.example.com/alex/abc.webp?motion=1";
    let convert = "https://files.example.com/alex/abc/convert?to=jpg";
    let download = "https://files.example.com/alex/abc/download";
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("abc.png");
    std::fs::write(&path, b"what was there").unwrap();

    let locked = FilesLocked {
        owner: Some("alex".into()),
        grant: None,
    };
    assert_eq!(
        api.get_thumbnail(&me, thumb).await,
        Err(ApiError::Locked(locked.clone()))
    );
    assert_eq!(
        api.get_motion_thumbnail(&me, motion).await,
        Err(ApiError::Locked(locked.clone()))
    );
    let verdict = api.check_conversion(&me, convert).await;
    assert!(!verdict.ok);
    assert_eq!(verdict.locked, Some(locked.clone()));
    assert_eq!(
        api.download_to_file(&me, download, &path).await,
        Err(ApiError::Locked(locked.clone()))
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"what was there");
    assert_eq!(
        ApiError::Locked(locked).status(),
        Some(423),
        "not 401: the token is fine"
    );

    let number = api.hold_grant(&me, &UnlockGrant::new(GRANT, None));
    assert_eq!(
        api.get_thumbnail(&me, thumb).await.unwrap().as_deref(),
        Some(&b"PICTURE"[..])
    );
    assert_eq!(
        api.get_motion_thumbnail(&me, motion).await.unwrap(),
        MotionPreview::Ok(b"PICTURE".to_vec())
    );
    assert!(api.check_conversion(&me, convert).await.ok);
    api.download_to_file(&me, download, &path).await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"PICTURE");
    // A missing picture is still no picture, not a lock.
    assert_eq!(
        api.get_thumbnail(&me, "https://files.example.com/alex/missing.webp")
            .await,
        Ok(None)
    );

    // A grant the server stopped honouring: the refusal names it.
    api.hold_grant(&me, &UnlockGrant::new("6f1c2a9e.revoked", None));
    let refused = api.get_thumbnail(&me, thumb).await.unwrap_err();
    let ApiError::Locked(refused) = refused else {
        panic!("{refused:?}")
    };
    assert_eq!(refused.grant, Some(number + 1));

    // The grant goes with content requests of its own credential only.
    api.hold_grant(&me, &UnlockGrant::new(GRANT, None));
    api.get_uploads(&me, &Default::default()).await.unwrap();
    api.get_account(&me).await.unwrap();
    let seen = log.lock().unwrap().clone();
    for request in &seen {
        let content = request.path.starts_with("/alex/") || request.path == "/api/encryption";
        if !content {
            assert_eq!(
                request.header("x-modlight-unlock"),
                None,
                "{} carried the grant",
                request.path
            );
        }
        assert_eq!(request.header("x-api-key"), Some(TOKEN));
    }
}

/// A grant held for one account never travels with another account's requests, even to
/// the same server.
#[tokio::test]
async fn a_grant_stays_with_its_credential() {
    let (origin, log) = stand_in(|_| locked_reply()).await;
    let api = api();
    let alex = Credential::new(origin.as_str(), TOKEN);
    let other = Credential::new(origin.as_str(), "token-of-somebody-else-0");
    api.hold_grant(&alex, &UnlockGrant::new(GRANT, None));
    let refused = api
        .get_thumbnail(&other, "https://files.example.com/alex/abc.webp")
        .await
        .unwrap_err();
    assert_eq!(
        refused,
        ApiError::Locked(FilesLocked {
            owner: Some("alex".into()),
            grant: None,
        })
    );
    assert_eq!(log.lock().unwrap()[0].header("x-modlight-unlock"), None);
}

/// 401 on a content request keeps meaning "the token is not valid": a download says so
/// as before, and a thumbnail is just missing. Neither reads as locked files.
#[tokio::test]
async fn a_refused_token_is_not_locked_files() {
    let (origin, _) =
        stand_in(|_| json(401, r#"{"message":"Token not specified or not found."}"#)).await;
    let api = api();
    let me = Credential::new(origin.as_str(), TOKEN);
    assert_eq!(
        api.get_thumbnail(&me, "https://files.example.com/alex/abc.webp")
            .await,
        Ok(None)
    );
    let folder = tempfile::tempdir().unwrap();
    assert_eq!(
        api.download_to_file(
            &me,
            "https://files.example.com/alex/abc/download",
            &folder.path().join("abc")
        )
        .await,
        Err(ApiError::Unauthorized)
    );
}

/// The JSON calls and plain reads say "locked" too, should a server ever answer them so.
#[tokio::test]
async fn reads_and_json_calls_know_the_locked_answer() {
    let (origin, _) = stand_in(|_| locked_reply()).await;
    let api = api();
    let me = Credential::new(origin.as_str(), TOKEN);
    let error = api.get_folders(&me).await.unwrap_err();
    assert!(error.is_locked(), "{error:?}");
    let error = api
        .delete_uploads(&me, &["a".into()], "trash")
        .await
        .unwrap_err();
    assert!(error.is_locked(), "{error:?}");
}
