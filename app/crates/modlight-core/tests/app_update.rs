//! The app's own update against a stand-in for the server's Downloads on 127.0.0.1
//! (random port): the check, the download with the account's token and its verification.
//! Nothing here touches the internet.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};
use modlight_api::{Api, Credential};
use modlight_core::build_info::BuildIdentity;
use modlight_core::update::{self, DownloadResult, UpdateOutcome};
use sha2::Digest;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

const TOKEN: &str = "mlt_test_0123456789abcdef";

#[derive(Clone)]
enum Answer {
    Json(String),
    Bytes(Vec<u8>),
    /// Claims `claim` bytes, sends these, closes.
    Short(Vec<u8>, usize),
    Status(u16),
    /// A redirect to another address.
    Redirect(String),
}

/// One request as the stand-in saw it.
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    target: String,
    api_key: Option<String>,
    authorization: Option<String>,
}

#[derive(Clone)]
struct Server {
    origin: String,
    routes: Arc<Mutex<HashMap<String, Answer>>>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Server {
    async fn start() -> Server {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let server = Server {
            origin: format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port()),
            routes: Arc::default(),
            seen: Arc::default(),
        };
        let served = server.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let served = served.clone();
                tokio::spawn(async move { served.answer(socket).await });
            }
        });
        server
    }

    fn set(&self, path: &str, answer: Answer) {
        self.routes.lock().unwrap().insert(path.into(), answer);
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    async fn answer(&self, mut socket: tokio::net::TcpStream) {
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            match socket.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(n) => request.extend_from_slice(&buffer[..n]),
            }
        }
        let text = String::from_utf8_lossy(&request).to_string();
        let mut first = text.lines().next().unwrap_or_default().split(' ');
        let method = first.next().unwrap_or_default().to_string();
        let target = first.next().unwrap_or_default().to_string();
        let header = |name: &str| {
            text.lines().find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.trim()
                    .eq_ignore_ascii_case(name)
                    .then(|| value.trim().to_string())
            })
        };
        self.seen.lock().unwrap().push(Seen {
            method,
            target: target.clone(),
            api_key: header("x-api-key"),
            authorization: header("authorization"),
        });
        let answer = self.routes.lock().unwrap().get(&target).cloned();
        let head = |status: &str, length: usize, extra: &str| {
            format!(
                "HTTP/1.1 {status}\r\nContent-Length: {length}\r\n{extra}Connection: close\r\n\r\n"
            )
        };
        let bytes: Vec<u8> = match answer {
            None => head("404 Not Found", 0, "").into_bytes(),
            Some(Answer::Status(code)) => head(&format!("{code} Nope"), 0, "").into_bytes(),
            Some(Answer::Redirect(to)) => {
                head("302 Found", 0, &format!("Location: {to}\r\n")).into_bytes()
            }
            Some(Answer::Json(body)) => {
                let mut out =
                    head("200 OK", body.len(), "Content-Type: application/json\r\n").into_bytes();
                out.extend_from_slice(body.as_bytes());
                out
            }
            Some(Answer::Bytes(body)) => {
                let mut out = head("200 OK", body.len(), "").into_bytes();
                out.extend_from_slice(&body);
                out
            }
            Some(Answer::Short(body, claim)) => {
                let mut out = head("200 OK", claim, "").into_bytes();
                out.extend_from_slice(&body);
                out
            }
        };
        let _ = socket.write_all(&bytes).await;
        let _ = socket.shutdown().await;
    }
}

fn payload(length: usize) -> Vec<u8> {
    (0..length).map(|i| (i * 7 + 3) as u8).collect()
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

/// This "app": 0.7.8 from commit 1a2b3c4d, built on 20 October.
fn mine() -> BuildIdentity {
    BuildIdentity {
        version: "0.7.8".into(),
        build: "1a2b3c4d".into(),
        built: Utc.with_ymd_and_hms(2026, 10, 20, 9, 41, 0).single(),
    }
}

fn build_json(
    id: &str,
    arch: &str,
    version: &str,
    commit: &str,
    built: &str,
    zip: &[u8],
) -> String {
    let filename = format!("Modlight-{version}-{commit}-macos-{arch}.zip");
    format!(
        r#"{{"id":"{id}","platform":"macos","arch":"{arch}","version":"{version}","build":"{commit}","built":"{built}","published":"{built}","size":{},"sha256":"{}","filename":"{filename}","uploader":"Alex","note":"Faster uploads","url":"/downloads/{id}/{filename}"}}"#,
        zip.len(),
        sha256(zip)
    )
}

fn listing(mac: &[String]) -> String {
    format!(
        r#"{{"platforms":[{{"platform":"macos","builds":[{}]}},{{"platform":"windows","builds":[]}}],"keep":5,"admin":false,"publishToken":null}}"#,
        mac.join(",")
    )
}

#[tokio::test]
async fn the_update_is_checked_and_fetched_with_the_accounts_token_and_verified() {
    let server = Server::start().await;
    let api = Api::new("0.7.8").unwrap();
    let credential = Credential::new(server.origin.as_str(), TOKEN);
    let me = mine();
    let check = || update::check_as(&api, &credential, &me, "macos", "arm64");

    // A server from before Downloads: up to date, not a failure.
    assert_eq!(check().await, UpdateOutcome::UpToDate);
    // A token the server no longer takes is a failed check.
    server.set("/api/downloads", Answer::Status(401));
    assert!(matches!(check().await, UpdateOutcome::CheckFailed(_)));

    let zip = payload(70_000);
    let newer = build_json(
        "m2",
        "arm64",
        "0.7.9",
        "9f8e7d6c",
        "2026-10-27T14:03:05Z",
        &zip,
    );
    let intel = build_json(
        "m3",
        "x64",
        "0.8.0",
        "0a0a0a0a",
        "2026-10-28T09:00:00Z",
        &zip,
    );
    let this = build_json(
        "m1",
        "arm64",
        "0.7.8",
        "1a2b3c4d",
        "2026-10-20T09:41:00Z",
        &zip,
    );
    // Only this build and another processor's: nothing for this app.
    server.set(
        "/api/downloads",
        Answer::Json(listing(&[intel.clone(), this.clone()])),
    );
    assert_eq!(check().await, UpdateOutcome::UpToDate);

    server.set(
        "/api/downloads",
        Answer::Json(listing(&[intel, newer, this])),
    );
    let UpdateOutcome::UpdateAvailable(release) = check().await else {
        panic!("no update offered");
    };
    assert_eq!(release.label(), "0.7.9 (9f8e7d6c)");
    assert_eq!(release.server, server.origin);
    let path = "/downloads/m2/Modlight-0.7.9-9f8e7d6c-macos-arm64.zip";
    assert_eq!(release.url, format!("{}{path}", server.origin));
    // The listing was asked with the token in its header, and nowhere else.
    let asked = server.seen();
    assert!(
        asked
            .iter()
            .all(|s| s.target == "/api/downloads" && s.api_key.as_deref() == Some(TOKEN))
    );
    assert!(asked.iter().all(|s| !s.target.contains(TOKEN)));

    // The download: the same token, verified, kept under the build's own name.
    server.set(path, Answer::Bytes(zip.clone()));
    let folder = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    let result = update::download(&release, &credential, folder.path(), &|_| {}, &cancel).await;
    let DownloadResult::Verified(file) = result else {
        panic!("{result:?}");
    };
    assert_eq!(file, folder.path().join(&release.file_name));
    assert_eq!(std::fs::read(&file).unwrap(), zip);
    let fetched = server
        .seen()
        .into_iter()
        .find(|s| s.target == path)
        .unwrap();
    assert_eq!(fetched.method, "GET");
    assert_eq!(fetched.api_key.as_deref(), Some(TOKEN));
    assert_eq!(fetched.authorization, None);

    // A verified copy is reused without asking again.
    server.set(path, Answer::Status(500));
    assert_eq!(
        update::download(&release, &credential, folder.path(), &|_| {}, &cancel).await,
        DownloadResult::Verified(file.clone())
    );

    // Bytes that are not the listing's are deleted.
    std::fs::remove_file(&file).unwrap();
    server.set(
        path,
        Answer::Bytes(zip.iter().map(|b| b ^ 1).collect::<Vec<u8>>()),
    );
    assert_eq!(
        update::download(&release, &credential, folder.path(), &|_| {}, &cancel).await,
        DownloadResult::VerificationFailed
    );
    assert!(std::fs::read_dir(folder.path()).unwrap().next().is_none());

    // A body shorter than the listing's size is not a download.
    server.set(path, Answer::Short(zip[..1000].to_vec(), zip.len()));
    assert!(matches!(
        update::download(&release, &credential, folder.path(), &|_| {}, &cancel).await,
        DownloadResult::Incomplete(_)
    ));
    assert!(std::fs::read_dir(folder.path()).unwrap().next().is_none());

    // A redirect is never followed: the token would go with it.
    let elsewhere = Server::start().await;
    server.set(
        path,
        Answer::Redirect(format!("{}/x.zip", elsewhere.origin)),
    );
    assert!(matches!(
        update::download(&release, &credential, folder.path(), &|_| {}, &cancel).await,
        DownloadResult::Incomplete(_)
    ));
    assert!(elsewhere.seen().is_empty());

    // Signed in elsewhere meanwhile: nothing is asked of anybody.
    let before = server.seen().len();
    let other = Credential::new(elsewhere.origin.as_str(), TOKEN);
    assert_eq!(
        update::download(&release, &other, folder.path(), &|_| {}, &cancel).await,
        DownloadResult::OtherServer
    );
    assert_eq!(server.seen().len(), before);
    assert!(elsewhere.seen().is_empty());
}

#[tokio::test]
async fn a_build_whose_address_leaves_the_server_is_not_offered() {
    let server = Server::start().await;
    let api = Api::new("0.7.8").unwrap();
    let credential = Credential::new(server.origin.as_str(), TOKEN);
    let zip = payload(1000);
    let sneaky = build_json(
        "m2",
        "arm64",
        "0.7.9",
        "9f8e7d6c",
        "2026-10-27T14:03:05Z",
        &zip,
    )
    .replace(
        "\"url\":\"/downloads/",
        "\"url\":\"//evil.example/downloads/",
    );
    server.set("/api/downloads", Answer::Json(listing(&[sneaky])));
    assert_eq!(
        update::check_as(&api, &credential, &mine(), "macos", "arm64").await,
        UpdateOutcome::UpToDate
    );
    // Plain http to anything but this computer is never asked at all.
    let plain = Credential::new("http://files.example.com", TOKEN);
    assert!(matches!(
        update::check_as(&api, &plain, &mine(), "macos", "arm64").await,
        UpdateOutcome::CheckFailed(_)
    ));
}
