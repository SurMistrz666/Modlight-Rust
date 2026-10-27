//! `app/scripts/build-app.sh --publish <zip>` against a stand-in for the server's Downloads
//! on 127.0.0.1: the upload is started with the zip's description, sent in chunks of the
//! server's size and finished, with the publish token in the Authorization header only
//! and never in what the script prints. macOS only (the script is the Mac's; it reads and
//! writes JSON with `plutil`). Nothing is built and nothing leaves this computer.

#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use sha2::Digest;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const TOKEN: &str = "mlp_publish_0123456789abcdef";
const CHUNK: usize = 30_000;

/// One request the stand-in received.
#[derive(Clone, Debug)]
struct Received {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

#[derive(Clone, Default)]
struct Downloads {
    received: Arc<Mutex<Vec<Received>>>,
    /// A chunk the stand-in refuses (its number).
    refuse_chunk: Arc<Mutex<Option<usize>>>,
}

impl Downloads {
    async fn start(&self) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let served = self.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let served = served.clone();
                tokio::spawn(async move { served.answer(socket).await });
            }
        });
        origin
    }

    fn received(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }

    async fn answer(&self, mut socket: tokio::net::TcpStream) {
        let mut data = Vec::new();
        let mut buffer = [0u8; 65536];
        let head_end = loop {
            if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                break at + 4;
            }
            match socket.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(n) => data.extend_from_slice(&buffer[..n]),
            }
        };
        let head = String::from_utf8_lossy(&data[..head_end]).to_string();
        let header = |name: &str| {
            head.lines().find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.trim()
                    .eq_ignore_ascii_case(name)
                    .then(|| value.trim().to_string())
            })
        };
        let length: usize = header("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        while data.len() < head_end + length {
            match socket.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(n) => data.extend_from_slice(&buffer[..n]),
            }
        }
        let mut first = head.lines().next().unwrap_or_default().split(' ');
        let request = Received {
            method: first.next().unwrap_or_default().to_string(),
            path: first.next().unwrap_or_default().to_string(),
            authorization: header("authorization"),
            body: data[head_end..head_end + length].to_vec(),
        };
        self.received.lock().unwrap().push(request.clone());
        let (status, body) = self.route(&request);
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = socket.write_all(response.as_bytes()).await;
        let _ = socket.shutdown().await;
    }

    fn route(&self, request: &Received) -> (&'static str, String) {
        if request.authorization.as_deref() != Some(&format!("Bearer {TOKEN}")) {
            return ("401 Unauthorized", r#"{"error":"no"}"#.into());
        }
        let parts: Vec<&str> = request.path.trim_start_matches('/').split('/').collect();
        match (request.method.as_str(), parts.as_slice()) {
            ("POST", ["api", "downloads", "upload"]) => (
                "200 OK",
                format!(r#"{{"upload":"up-1","chunkSize":{CHUNK}}}"#),
            ),
            ("PUT", ["api", "downloads", "upload", "up-1", n]) => {
                if *self.refuse_chunk.lock().unwrap() == n.parse().ok() {
                    (
                        "500 Internal Server Error",
                        r#"{"error":"disk full"}"#.into(),
                    )
                } else {
                    ("200 OK", "{}".into())
                }
            }
            ("POST", ["api", "downloads", "upload", "up-1", "finish"]) => {
                // The chunks in order make the file the description promised.
                let received = self.received();
                let started = received
                    .iter()
                    .find(|r| r.method == "POST" && r.path == "/api/downloads/upload")
                    .unwrap();
                let description: serde_json::Value = serde_json::from_slice(&started.body).unwrap();
                let mut file = Vec::new();
                for chunk in received.iter().filter(|r| r.method == "PUT") {
                    file.extend_from_slice(&chunk.body);
                }
                let digest = hex::encode(sha2::Sha256::digest(&file));
                if description["size"].as_u64() != Some(file.len() as u64)
                    || description["sha256"].as_str() != Some(digest.as_str())
                {
                    return ("400 Bad Request", r#"{"error":"size or sha256"}"#.into());
                }
                (
                    "200 OK",
                    format!(
                        r#"{{"id":"b1","filename":"{}","url":"/downloads/b1/{}"}}"#,
                        description["filename"].as_str().unwrap(),
                        description["filename"].as_str().unwrap()
                    ),
                )
            }
            _ => ("404 Not Found", "{}".into()),
        }
    }
}

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/build-app.sh")
}

/// A zip (any bytes do) and the description `build-app.sh` writes beside it.
fn build_output(folder: &Path, length: usize) -> (PathBuf, Vec<u8>) {
    let name = "Modlight-0.7.9-9f8e7d6c-macos-arm64";
    let bytes: Vec<u8> = (0..length).map(|i| (i * 31 + 7) as u8).collect();
    let zip = folder.join(format!("{name}.zip"));
    std::fs::write(&zip, &bytes).unwrap();
    let description = serde_json::json!({
        "platform": "macos",
        "arch": "arm64",
        "version": "0.7.9",
        "build": "9f8e7d6c",
        "built": "2026-10-27T14:03:05Z",
        "size": bytes.len(),
        "sha256": hex::encode(sha2::Sha256::digest(&bytes)),
        "filename": format!("{name}.zip"),
        "note": "",
    });
    std::fs::write(folder.join(format!("{name}.json")), description.to_string()).unwrap();
    (zip, bytes)
}

/// Run the script's publish mode with a home of its own (no real configuration is read).
fn publish(zip: &Path, home: &Path, env: &[(&str, &str)]) -> (Output, String) {
    let mut command = Command::new("/bin/sh");
    command
        .arg(script())
        .arg("--publish")
        .arg(zip)
        .env("HOME", home)
        .env_remove("MODLIGHT_PUBLISH_SERVER")
        .env_remove("MODLIGHT_PUBLISH_TOKEN")
        .env_remove("MODLIGHT_PUBLISH")
        .env_remove("MODLIGHT_PUBLISH_NOTE");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output, text)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_build_is_published_in_chunks_with_the_token_in_its_header_only() {
    let downloads = Downloads::default();
    let server = downloads.start().await;
    let folder = tempfile::tempdir().unwrap();
    let (zip, bytes) = build_output(folder.path(), 70_000);
    let (output, text) = tokio::task::spawn_blocking({
        let zip = zip.clone();
        let home = folder.path().to_path_buf();
        let server = server.clone();
        move || {
            publish(
                &zip,
                &home,
                &[
                    ("MODLIGHT_PUBLISH_SERVER", &format!("{server}/")),
                    ("MODLIGHT_PUBLISH_TOKEN", TOKEN),
                    ("MODLIGHT_PUBLISH_NOTE", "Faster \"uploads\"\nand a fix"),
                ],
            )
        }
    })
    .await
    .unwrap();
    assert!(output.status.success(), "{text}");
    assert!(!text.contains(TOKEN), "the token was printed: {text}");
    assert!(text.contains("published as b1"), "{text}");

    let received = downloads.received();
    let methods: Vec<(&str, &str)> = received
        .iter()
        .map(|r| (r.method.as_str(), r.path.as_str()))
        .collect();
    assert_eq!(
        methods,
        [
            ("POST", "/api/downloads/upload"),
            ("PUT", "/api/downloads/upload/up-1/0"),
            ("PUT", "/api/downloads/upload/up-1/1"),
            ("PUT", "/api/downloads/upload/up-1/2"),
            ("POST", "/api/downloads/upload/up-1/finish"),
        ]
    );
    assert!(received.iter().all(|r| !r.path.contains(TOKEN)));
    let description: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(description["platform"], "macos");
    assert_eq!(description["arch"], "arm64");
    assert_eq!(description["version"], "0.7.9");
    assert_eq!(description["build"], "9f8e7d6c");
    assert_eq!(description["built"], "2026-10-27T14:03:05Z");
    assert_eq!(description["size"], 70_000);
    assert_eq!(
        description["filename"],
        "Modlight-0.7.9-9f8e7d6c-macos-arm64.zip"
    );
    assert_eq!(description["note"], "Faster \"uploads\"\nand a fix");
    assert_eq!(received[1].body.len(), CHUNK);
    assert_eq!(received[3].body.len(), 70_000 - 2 * CHUNK);
    let sent: Vec<u8> = received[1..4].iter().flat_map(|r| r.body.clone()).collect();
    assert_eq!(sent, bytes);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_publish_says_how_to_retry_and_never_shows_the_token() {
    let downloads = Downloads::default();
    *downloads.refuse_chunk.lock().unwrap() = Some(1);
    let server = downloads.start().await;
    let folder = tempfile::tempdir().unwrap();
    let (zip, _) = build_output(folder.path(), 70_000);
    let (output, text) = tokio::task::spawn_blocking({
        let zip = zip.clone();
        let home = folder.path().to_path_buf();
        move || {
            publish(
                &zip,
                &home,
                &[
                    ("MODLIGHT_PUBLISH_SERVER", server.as_str()),
                    ("MODLIGHT_PUBLISH_TOKEN", TOKEN),
                ],
            )
        }
    })
    .await
    .unwrap();
    assert!(!output.status.success());
    assert!(!text.contains(TOKEN), "the token was printed: {text}");
    assert!(
        text.contains("not published: chunk 2 of 3 answered 500"),
        "{text}"
    );
    assert!(text.contains("--publish"), "{text}");
    // Tried three times, never finished.
    let puts = downloads
        .received()
        .iter()
        .filter(|r| r.path.ends_with("/up-1/1"))
        .count();
    assert_eq!(puts, 3);
    assert!(
        !downloads
            .received()
            .iter()
            .any(|r| r.path.ends_with("/finish"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_configuration_file_must_be_private_and_nothing_is_sent_without_one() {
    use std::os::unix::fs::PermissionsExt;
    let downloads = Downloads::default();
    let server = downloads.start().await;
    let folder = tempfile::tempdir().unwrap();
    let (zip, _) = build_output(folder.path(), 1000);
    let home = folder.path().join("home");
    let run = |env: Vec<(&'static str, String)>| {
        let zip = zip.clone();
        let home = home.clone();
        tokio::task::spawn_blocking(move || {
            let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
            publish(&zip, &home, &env)
        })
    };

    // Nothing configured: not published, not an error, nothing asked.
    std::fs::create_dir_all(&home).unwrap();
    let (output, text) = run(Vec::new()).await.unwrap();
    assert!(output.status.success(), "{text}");
    assert!(text.contains("not published"), "{text}");
    // Switched off.
    let (output, text) = run(vec![
        ("MODLIGHT_PUBLISH", "0".into()),
        ("MODLIGHT_PUBLISH_SERVER", server.clone()),
        ("MODLIGHT_PUBLISH_TOKEN", TOKEN.into()),
    ])
    .await
    .unwrap();
    assert!(output.status.success(), "{text}");
    assert!(text.contains("MODLIGHT_PUBLISH=0"), "{text}");
    // Plain http to another computer is refused.
    let (output, text) = run(vec![
        ("MODLIGHT_PUBLISH_SERVER", "http://files.example.com".into()),
        ("MODLIGHT_PUBLISH_TOKEN", TOKEN.into()),
    ])
    .await
    .unwrap();
    assert!(!output.status.success(), "{text}");
    assert!(downloads.received().is_empty());

    // The file, readable by others: refused, with the fix named.
    let config = home.join(".config/modlight/publish.json");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(
        &config,
        serde_json::json!({ "server": server, "token": TOKEN }).to_string(),
    )
    .unwrap();
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o644)).unwrap();
    let (output, text) = run(Vec::new()).await.unwrap();
    assert!(!output.status.success(), "{text}");
    assert!(text.contains("chmod 600"), "{text}");
    assert!(downloads.received().is_empty());
    // Private: used.
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
    let (output, text) = run(Vec::new()).await.unwrap();
    assert!(output.status.success(), "{text}");
    assert!(!text.contains(TOKEN));
    assert_eq!(downloads.received().len(), 3, "start, one chunk, finish");

    // A zip that is not the one its description describes is never sent.
    std::fs::write(&zip, b"something else").unwrap();
    let before = downloads.received().len();
    let (output, text) = run(Vec::new()).await.unwrap();
    assert!(!output.status.success(), "{text}");
    assert_eq!(downloads.received().len(), before);
}
