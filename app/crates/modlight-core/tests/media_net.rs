//! Components, the yt-dlp update and the app's own update check against a stub HTTP
//! server on 127.0.0.1 (random port). Nothing here touches the internet: the stub's
//! origin is let through by `net::testing`, which exists only in debug builds.

#![cfg(debug_assertions)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use modlight_core::components::download::{self, DownloadOutcome};
use modlight_core::components::install::{self, InstallOutcome};
use modlight_core::components::updates::{self, UpdateOutcome};
use modlight_core::components::{self, ComponentSource, net};
use modlight_core::update;
use sha2::Digest;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

/// What the stub answers on one path.
#[derive(Clone)]
enum Answer {
    /// 200 with Content-Length; honours a Range with 206.
    Bytes(Vec<u8>),
    /// 200 with no Content-Length: the body ends when the connection closes.
    NoLength(Vec<u8>),
    /// Claims `claim` bytes and sends these, then closes.
    Short {
        bytes: Vec<u8>,
        claim: usize,
    },
    /// Sends these bytes and then nothing at all, holding the connection open.
    Stall(Vec<u8>),
    Status(u16),
}

/// The requests seen: path and Range header.
type Hits = Arc<Mutex<Vec<(String, Option<String>)>>>;

#[derive(Clone)]
struct Stub {
    origin: String,
    routes: Arc<Mutex<HashMap<String, Answer>>>,
    hits: Hits,
}

impl Stub {
    async fn start() -> Stub {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        net::testing::allow_origin(&origin);
        let stub = Stub {
            origin,
            routes: Arc::new(Mutex::new(HashMap::new())),
            hits: Arc::new(Mutex::new(Vec::new())),
        };
        let served = stub.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let served = served.clone();
                tokio::spawn(async move { served.answer(socket).await });
            }
        });
        stub
    }

    fn set(&self, path: &str, answer: Answer) -> String {
        self.routes.lock().unwrap().insert(path.to_string(), answer);
        format!("{}{path}", self.origin)
    }

    fn ranges_asked(&self) -> Vec<String> {
        self.hits
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(_, r)| r.clone())
            .collect()
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
        let path = text
            .lines()
            .next()
            .and_then(|l| l.split(' ').nth(1))
            .unwrap_or("/")
            .to_string();
        let range = text
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("range:"))
            .map(|l| l[6..].trim().to_string());
        self.hits
            .lock()
            .unwrap()
            .push((path.clone(), range.clone()));
        let answer = self.routes.lock().unwrap().get(&path).cloned();
        let write = |head: String, body: &[u8]| {
            let mut out = head.into_bytes();
            out.extend_from_slice(body);
            out
        };
        let bytes = match answer {
            None => write(
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                b"",
            ),
            Some(Answer::Status(code)) => write(
                format!("HTTP/1.1 {code} Nope\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                b"",
            ),
            Some(Answer::Bytes(body)) => {
                let start = range
                    .as_deref()
                    .and_then(|r| r.strip_prefix("bytes="))
                    .and_then(|r| r.trim_end_matches('-').parse::<usize>().ok());
                match start {
                    Some(start) if start >= body.len() => write(
                        "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                        b"",
                    ),
                    Some(start) => write(
                        format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{}/{}\r\nConnection: close\r\n\r\n",
                            body.len() - start,
                            body.len() - 1,
                            body.len()
                        ),
                        &body[start..],
                    ),
                    None => write(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        ),
                        &body,
                    ),
                }
            }
            Some(Answer::NoLength(body)) => {
                write("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".into(), &body)
            }
            Some(Answer::Short { bytes, claim }) => write(
                format!("HTTP/1.1 200 OK\r\nContent-Length: {claim}\r\nConnection: close\r\n\r\n"),
                &bytes,
            ),
            Some(Answer::Stall(bytes)) => {
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len() + 1000
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&bytes).await;
                tokio::time::sleep(Duration::from_secs(30)).await;
                return;
            }
        };
        let _ = socket.write_all(&bytes).await;
        let _ = socket.shutdown().await;
    }
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

fn payload(length: usize) -> Vec<u8> {
    (0..length).map(|i| (i * 7 + 3) as u8).collect()
}

async fn fetch(url: &str, path: &Path, size: i64) -> download::DownloadResult {
    let client = net::file_client().unwrap();
    download::to_file(
        &client,
        url,
        path,
        size,
        Duration::from_millis(600),
        &|_| {},
        &CancellationToken::new(),
    )
    .await
}

#[tokio::test]
async fn a_download_resumes_from_its_part_file() {
    let stub = Stub::start().await;
    let body = payload(300_000);
    let url = stub.set("/resume.bin", Answer::Bytes(body.clone()));
    let dir = tempfile::tempdir().unwrap();
    let part = dir.path().join("x.part");
    std::fs::write(&part, &body[..120_000]).unwrap();
    let result = fetch(&url, &part, body.len() as i64).await;
    assert!(result.completed(), "{result:?}");
    assert_eq!(std::fs::read(&part).unwrap(), body);
    assert_eq!(stub.ranges_asked(), ["bytes=120000-"]);
}

#[tokio::test]
async fn a_short_or_long_body_is_never_complete() {
    let stub = Stub::start().await;
    let body = payload(50_000);
    let dir = tempfile::tempdir().unwrap();

    // Fewer bytes than promised: kept for a resume, never complete.
    let short = stub.set(
        "/short.bin",
        Answer::Short {
            bytes: body[..20_000].to_vec(),
            claim: 50_000,
        },
    );
    let part = dir.path().join("short.part");
    let result = fetch(&short, &part, 50_000).await;
    assert_ne!(result.outcome, DownloadOutcome::Completed);

    // No Content-Length and the connection closes early: not complete either.
    let cut = stub.set("/cut.bin", Answer::NoLength(body[..10_000].to_vec()));
    let part = dir.path().join("cut.part");
    let result = fetch(&cut, &part, 50_000).await;
    assert_eq!(result.outcome, DownloadOutcome::HttpFailed);

    // No Content-Length but every promised byte: the size we knew decides.
    let whole = stub.set("/whole.bin", Answer::NoLength(body.clone()));
    let part = dir.path().join("whole.part");
    assert!(fetch(&whole, &part, 50_000).await.completed());

    // More than promised: refused and thrown away.
    let long = stub.set("/long.bin", Answer::Bytes(payload(60_000)));
    let part = dir.path().join("long.part");
    let result = fetch(&long, &part, 50_000).await;
    assert_eq!(result.outcome, DownloadOutcome::HttpFailed);
    assert!(!part.exists());

    // A size nobody promised: nothing is even requested.
    let hits = stub.hits.lock().unwrap().len();
    let part = dir.path().join("unknown.part");
    let result = fetch(&whole, &part, 0).await;
    assert_ne!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(stub.hits.lock().unwrap().len(), hits);
}

#[tokio::test]
async fn a_silent_connection_is_called_dead() {
    let stub = Stub::start().await;
    let url = stub.set("/stall.bin", Answer::Stall(payload(1000)));
    let dir = tempfile::tempdir().unwrap();
    let started = std::time::Instant::now();
    let result = fetch(&url, &dir.path().join("s.part"), 2000).await;
    assert_eq!(result.outcome, DownloadOutcome::Offline);
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn plain_http_elsewhere_is_refused_before_asking() {
    let dir = tempfile::tempdir().unwrap();
    let result = fetch("http://example.invalid/x", &dir.path().join("x.part"), 10).await;
    assert_eq!(result.outcome, DownloadOutcome::Refused);
}

#[tokio::test]
async fn a_wrong_hash_installs_nothing_and_leaves_nothing() {
    let stub = Stub::start().await;
    let body = payload(4096);
    let url = stub.set("/yt-dlp_macos", Answer::Bytes(body.clone()));
    let profile = tempfile::tempdir().unwrap();
    let yt = components::find(components::YT_DLP).unwrap();
    let source = ComponentSource {
        url,
        sha256: "0".repeat(64),
        size: body.len() as i64,
        version: "2026.09.01".into(),
    };
    let result =
        install::replace(profile.path(), yt, &source, None, &CancellationToken::new()).await;
    assert_eq!(result.outcome, InstallOutcome::ChecksumFailed);
    assert_eq!(result.message, sha256(&body));
    assert!(!components::is_installed(profile.path(), yt));
    let staging = profile.path().join("Tools").join(install::STAGING_FOLDER);
    let leftovers: Vec<_> = std::fs::read_dir(staging).unwrap().flatten().collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[tokio::test]
async fn every_packaging_installs_and_removes() {
    let stub = Stub::start().await;
    let profile = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();

    // A plain file, made executable.
    let tool = b"#!/bin/sh\necho 2026.08.19\n".to_vec();
    let yt = components::find(components::YT_DLP).unwrap();
    let source = ComponentSource {
        url: stub.set("/file", Answer::Bytes(tool.clone())),
        sha256: sha256(&tool),
        size: tool.len() as i64,
        version: "2026.08.19".into(),
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let result = install::replace(
        profile.path(),
        yt,
        &source,
        Some(Arc::new(move |p| sink.lock().unwrap().push(p.stage))),
        &cancel,
    )
    .await;
    assert_eq!(result.outcome, InstallOutcome::Installed, "{result:?}");
    let path = components::installed_path(profile.path(), "yt-dlp").unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let stages = seen.lock().unwrap().clone();
    assert_eq!(stages.first(), Some(&install::Stage::Queued));
    assert_eq!(stages.last(), Some(&install::Stage::Done));
    assert_eq!(
        updates::ask_version(&path, &["--version".to_string()], Duration::from_secs(10)).await,
        "2026.08.19"
    );

    // A zip with one binary.
    let mut zip_bytes = std::io::Cursor::new(Vec::new());
    {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(&mut zip_bytes);
        writer
            .start_file("ffmpeg", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"fake ffmpeg").unwrap();
        writer.finish().unwrap();
    }
    let zip_bytes = zip_bytes.into_inner();
    let ffmpeg = components::find(components::FFMPEG).unwrap();
    let source = ComponentSource {
        url: stub.set("/ffmpeg.zip", Answer::Bytes(zip_bytes.clone())),
        sha256: sha256(&zip_bytes),
        size: zip_bytes.len() as i64,
        version: "8.1".into(),
    };
    let result = install::replace(profile.path(), ffmpeg, &source, None, &cancel).await;
    assert_eq!(result.outcome, InstallOutcome::Installed, "{result:?}");
    assert_eq!(
        std::fs::read(components::installed_path(profile.path(), "ffmpeg").unwrap()).unwrap(),
        b"fake ffmpeg"
    );

    // A tarball with a folder of its own.
    let mut tar_gz = Vec::new();
    {
        let encoder = flate2::write::GzEncoder::new(&mut tar_gz, flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        for (name, body) in [
            (
                "Image-ExifTool-13.59/exiftool",
                &b"#!/usr/bin/env perl\n"[..],
            ),
            ("Image-ExifTool-13.59/lib/Image/ExifTool.pm", &b"1;\n"[..]),
            ("Image-ExifTool-13.59/t/images/big.jpg", &b"jpeg"[..]),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, body).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
    }
    let exiftool = components::find(components::EXIFTOOL).unwrap();
    let source = ComponentSource {
        url: stub.set("/exiftool.tar.gz", Answer::Bytes(tar_gz.clone())),
        sha256: sha256(&tar_gz),
        size: tar_gz.len() as i64,
        version: "13.59".into(),
    };
    let result = install::replace(profile.path(), exiftool, &source, None, &cancel).await;
    assert_eq!(result.outcome, InstallOutcome::Installed, "{result:?}");
    let script = components::installed_path(profile.path(), "exiftool").unwrap();
    assert_eq!(script, profile.path().join("Tools/ExifTool/exiftool"));
    assert!(
        profile
            .path()
            .join("Tools/ExifTool/lib/Image/ExifTool.pm")
            .is_file()
    );
    assert!(!profile.path().join("Tools/ExifTool/t").exists());

    for component in [yt, ffmpeg, exiftool] {
        assert!(install::remove(profile.path(), component));
        assert!(!components::is_installed(profile.path(), component));
    }
}

/// A GitHub release document whose assets live on the stub.
fn release(origin: &str, tag: &str, artefact: &[u8], api_digest: Option<&str>) -> String {
    let digest = api_digest
        .map(|d| format!("\"sha256:{d}\""))
        .unwrap_or_else(|| "null".into());
    format!(
        r#"{{"tag_name":"{tag}","draft":false,"prerelease":false,"assets":[
            {{"name":"SHA2-256SUMS","size":100,"digest":null,"browser_download_url":"{origin}/dl/SHA2-256SUMS"}},
            {{"name":"yt-dlp_macos","size":{},"digest":{digest},"browser_download_url":"{origin}/dl/yt-dlp_macos"}}]}}"#,
        artefact.len()
    )
}

/// Every step of the yt-dlp update, in one test because the GitHub API origin is a
/// process-wide switch.
#[tokio::test]
async fn yt_dlp_follows_its_publisher_only_on_two_agreeing_digests() {
    let stub = Stub::start().await;
    net::testing::set_github_api(Some(&stub.origin));
    let profile = tempfile::tempdir().unwrap();
    let yt = components::find(components::YT_DLP).unwrap();
    let tools = profile.path().join("Tools");
    std::fs::create_dir_all(&tools).unwrap();
    let installed = tools.join("yt-dlp");
    std::fs::write(&installed, "#!/bin/sh\necho 2026.08.19\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o755)).unwrap();
    let cancel = CancellationToken::new();

    let newer = b"#!/bin/sh\necho 2026.10.02\n".to_vec();
    let digest = sha256(&newer);
    stub.set("/dl/yt-dlp_macos", Answer::Bytes(newer.clone()));
    let latest = "/repos/yt-dlp/yt-dlp/releases/latest";

    // The API's digest alone: refused, nothing replaced.
    stub.set(
        latest,
        Answer::Bytes(release(&stub.origin, "2026.10.02", &newer, Some(&digest)).into_bytes()),
    );
    stub.set("/dl/SHA2-256SUMS", Answer::Status(404));
    let result = updates::update(profile.path(), yt, None, true, &cancel).await;
    assert_eq!(result.outcome, UpdateOutcome::CheckFailed, "{result:?}");
    assert!(result.detail.contains("MissingDigest"));

    // The checksum file alone: refused.
    stub.set(
        latest,
        Answer::Bytes(release(&stub.origin, "2026.10.02", &newer, None).into_bytes()),
    );
    stub.set(
        "/dl/SHA2-256SUMS",
        Answer::Bytes(format!("{digest}  yt-dlp_macos\n").into_bytes()),
    );
    let result = updates::update(profile.path(), yt, None, true, &cancel).await;
    assert_eq!(result.outcome, UpdateOutcome::CheckFailed);

    // Two publications that disagree: refused.
    stub.set(
        latest,
        Answer::Bytes(
            release(&stub.origin, "2026.10.02", &newer, Some(&"1".repeat(64))).into_bytes(),
        ),
    );
    let result = updates::update(profile.path(), yt, None, true, &cancel).await;
    assert_eq!(result.outcome, UpdateOutcome::CheckFailed);
    assert!(result.detail.contains("DigestConflict"));
    assert_eq!(
        std::fs::read_to_string(&installed).unwrap(),
        "#!/bin/sh\necho 2026.08.19\n"
    );

    // An artefact that is not on github.com (here: not on the allowed stub) is refused
    // however the digests agree.
    stub.set(
        latest,
        Answer::Bytes(
            release(&stub.origin, "2026.10.02", &newer, Some(&digest))
                .replace(
                    &format!("{}/dl/yt-dlp_macos", stub.origin),
                    "https://evil.invalid/yt-dlp_macos",
                )
                .into_bytes(),
        ),
    );
    let result = updates::update(profile.path(), yt, None, true, &cancel).await;
    assert_eq!(result.outcome, UpdateOutcome::CheckFailed);
    assert!(result.detail.contains("BadUrl"));

    // Both, agreeing, and newer: installed.
    stub.set(
        latest,
        Answer::Bytes(release(&stub.origin, "2026.10.02", &newer, Some(&digest)).into_bytes()),
    );
    let result = updates::update(profile.path(), yt, None, true, &cancel).await;
    assert_eq!(result.outcome, UpdateOutcome::Updated, "{result:?}");
    assert_eq!(result.version, "2026.10.02");
    assert_eq!(std::fs::read(&installed).unwrap(), newer);
    assert_eq!(updates::latest_known(yt), "2026.10.02");
    assert_eq!(
        updates::installed_version(profile.path(), yt).await,
        "2026.10.02"
    );

    // And now it is current.
    let result = updates::update(profile.path(), yt, None, true, &cancel).await;
    assert_eq!(result.outcome, UpdateOutcome::AlreadyCurrent);

    // A copy that does not say its version is never replaced automatically.
    std::fs::write(&installed, "#!/bin/sh\necho hello\n").unwrap();
    updates::forget_versions();
    let result = updates::update(profile.path(), yt, None, true, &cancel).await;
    assert_eq!(result.outcome, UpdateOutcome::VersionUnknown);
    net::testing::set_github_api(None);
}

#[tokio::test]
async fn the_app_update_is_offered_downloaded_and_verified() {
    let stub = Stub::start().await;
    let artefact = payload(70_000);
    let digest = sha256(&artefact);
    let manifest = |size: usize, digest: &str, file: &str| {
        format!(
            r#"{{"name":"Modlight App","version":"99.0.0","url":"{}/r/{file}","filename":"{file}","size":{size},"sha256":"{digest}","notes":"Notes"}}"#,
            stub.origin
        )
    };
    stub.set("/r/Modlight-99.dmg", Answer::Bytes(artefact.clone()));

    // Nothing published: up to date, not a failure.
    stub.set("/api/releases/app", Answer::Status(404));
    assert_eq!(
        update::check_host(&stub.origin).await,
        update::UpdateOutcome::UpToDate
    );

    // A Windows installer is not a release for this Mac.
    stub.set(
        "/api/releases/app",
        Answer::Bytes(manifest(artefact.len(), &digest, "Modlight-99-setup.exe").into_bytes()),
    );
    assert_eq!(
        update::check_host(&stub.origin).await,
        update::UpdateOutcome::UpToDate
    );

    stub.set(
        "/api/releases/app",
        Answer::Bytes(manifest(artefact.len(), &digest, "Modlight-99.dmg").into_bytes()),
    );
    let update::UpdateOutcome::UpdateAvailable(release) = update::check_host(&stub.origin).await
    else {
        panic!("no update offered");
    };
    assert_eq!(release.version_text, "99.0.0");
    let folder = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    let result = update::download(&release, folder.path(), &|_| {}, &cancel).await;
    let update::DownloadResult::Verified(path) = result else {
        panic!("{result:?}");
    };
    assert_eq!(std::fs::read(&path).unwrap(), artefact);
    // A second time reuses the verified file.
    stub.set("/r/Modlight-99.dmg", Answer::Status(500));
    assert_eq!(
        update::download(&release, folder.path(), &|_| {}, &cancel).await,
        update::DownloadResult::Verified(path.clone())
    );

    // Bytes that do not match the manifest are deleted.
    std::fs::remove_file(&path).unwrap();
    stub.set(
        "/r/Modlight-99.dmg",
        Answer::Bytes(payload(70_000).iter().map(|b| b ^ 1).collect()),
    );
    assert_eq!(
        update::download(&release, folder.path(), &|_| {}, &cancel).await,
        update::DownloadResult::VerificationFailed
    );
    assert!(std::fs::read_dir(folder.path()).unwrap().next().is_none());

    // A body shorter than the manifest's size is not a download.
    stub.set(
        "/r/Modlight-99.dmg",
        Answer::Short {
            bytes: artefact[..1000].to_vec(),
            claim: artefact.len(),
        },
    );
    assert!(matches!(
        update::download(&release, folder.path(), &|_| {}, &cancel).await,
        update::DownloadResult::Incomplete(_)
    ));
    assert!(std::fs::read_dir(folder.path()).unwrap().next().is_none());
}
