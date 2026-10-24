//! The two ways a file reaches the server: the live `POST /upload` and the resumable tus
//! upload of the offline queue.
//!
//! Most tests run against the ORIGINAL Modlight server on this Mac
//! (`http://127.0.0.1:8420`, see `docs/PORT.md` section 4), with the credentials of
//! `_wzorzec/qa-accounts.env` (found by walking up from this crate, or named by
//! `MODLIGHT_QA_ENV`). Without that file, or with nothing answering on the port, those
//! tests say so and pass. Broken links are played by a small TCP forwarder in front of
//! the server that can cut a connection, or stop moving bytes, after a set number of
//! bytes.
//!
//! The rest run against scripted local servers that record every request and answer
//! from a list: they pin the bytes on the wire, and the answers the reference server
//! never gives (429, a `Location` on another host, a finish without links...).
//!
//! Everything uploaded is deleted for good at the end of each test. Tokens are never
//! printed: credentials are only ever shown through their redacting `Debug`.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use modlight_api::tus::{self, TUS_PATH};
use modlight_api::{
    Api, ApiError, Credential, LiveOutcome, LiveUpload, TransferOutcome, TransferReason,
    TransferResult, TusClient, TusFile, TusSession, TusTimings, UploadQuery, UploadRefusal,
    UploadSource, signin,
};
use rand::RngCore;
use sha2::Digest;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

const SERVER: &str = "http://127.0.0.1:8420";
const SERVER_PORT: u16 = 8420;
const VERSION: &str = "0.7.7";
const MIB: u64 = 1024 * 1024;

// ---------------------------------------------------------------------------------
// The reference server and its QA account
// ---------------------------------------------------------------------------------

fn qa_env() -> Option<HashMap<String, String>> {
    let path = std::env::var_os("MODLIGHT_QA_ENV")
        .map(PathBuf::from)
        .or_else(|| {
            let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            loop {
                let candidate = dir.join("_wzorzec").join("qa-accounts.env");
                if candidate.is_file() {
                    return Some(candidate);
                }
                if !dir.pop() {
                    return None;
                }
            }
        })?;
    let text = std::fs::read_to_string(path).ok()?;
    Some(
        text.lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .filter_map(|line| line.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect(),
    )
}

struct Qa {
    api: Api,
    /// The QA account on the reference server itself.
    me: Credential,
}

impl Qa {
    /// The same account reached through another origin (a forwarder, a closed port).
    fn via(&self, origin: &str) -> Credential {
        Credential::new(origin, self.me.token.clone())
    }
}

async fn qa() -> Option<Qa> {
    let Some(env) = qa_env() else {
        eprintln!("upload tests against the reference server skipped: no _wzorzec/qa-accounts.env");
        return None;
    };
    let api = Api::new(VERSION).unwrap();
    if !api.health(SERVER).await {
        eprintln!("upload tests against the reference server skipped: nothing answers on {SERVER}");
        return None;
    }
    let token = env
        .get("QA_APP1_TOKEN_WRITE")
        .cloned()
        .unwrap_or_else(|| panic!("QA_APP1_TOKEN_WRITE missing in qa-accounts.env"));
    Some(Qa {
        api,
        me: Credential::new(SERVER, token),
    })
}

fn unique(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        signin::new_state()[..10].to_ascii_lowercase()
    )
}

fn random_bytes(length: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; length];
    rand::rng().fill_bytes(&mut bytes);
    bytes
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

/// A file of random bytes nobody has uploaded before, and those bytes.
fn random_file(length: usize) -> (tempfile::NamedTempFile, Vec<u8>) {
    let bytes = random_bytes(length);
    let mut file = tempfile::NamedTempFile::new().unwrap();
    std::io::Write::write_all(&mut file, &bytes).unwrap();
    (file, bytes)
}

/// A file that claims `length` bytes and occupies none (APFS keeps it sparse).
fn sparse_file(length: u64) -> tempfile::NamedTempFile {
    let file = tempfile::NamedTempFile::new().unwrap();
    file.as_file().set_len(length).unwrap();
    file
}

fn tus_file(path: &Path, name: &str, length: u64) -> TusFile {
    TusFile::file(path, name, length)
}

/// The rows of every upload called `name`, in the library and in the trash.
async fn find_rows(
    api: &Api,
    me: &Credential,
    name: &str,
) -> Result<Vec<modlight_api::Upload>, ApiError> {
    let mut rows = Vec::new();
    for trashed in [false, true] {
        let page = api
            .get_uploads(
                me,
                &UploadQuery {
                    page: 1,
                    per_page: 120,
                    search: Some(name.to_string()),
                    trashed,
                    ..Default::default()
                },
            )
            .await?;
        rows.extend(
            page.items
                .unwrap_or_default()
                .into_iter()
                .filter(|row| row.filename.as_deref() == Some(name)),
        );
    }
    Ok(rows)
}

async fn rows_named(qa: &Qa, name: &str) -> Vec<modlight_api::Upload> {
    find_rows(&qa.api, &qa.me, name).await.unwrap()
}

/// Delete every upload called `name` for good; how many there were.
async fn delete_named(api: &Api, me: &Credential, name: &str) -> Result<usize, ApiError> {
    let ids: Vec<String> = find_rows(api, me, name)
        .await?
        .into_iter()
        .filter_map(|row| row.id)
        .collect();
    if ids.is_empty() {
        return Ok(0);
    }
    api.delete_uploads(me, &ids, "trash").await?;
    let gone = api.delete_uploads(me, &ids, "permanent").await?;
    Ok(gone.affected as usize)
}

/// Delete every upload called `name` for good, and say how many there were.
async fn purge(qa: &Qa, name: &str) -> usize {
    delete_named(&qa.api, &qa.me, name).await.unwrap()
}

/// What a test put on the server, deleted when the test ends however it ends: a test
/// that fails half way must not leave its files in the QA account.
struct Cleanup {
    me: Credential,
    names: Vec<String>,
    folders: Vec<String>,
}

impl Cleanup {
    fn new(qa: &Qa) -> Cleanup {
        Cleanup {
            me: qa.me.clone(),
            names: Vec::new(),
            folders: Vec::new(),
        }
    }

    /// A unique upload name, remembered for deletion.
    fn name(&mut self, prefix: &str, extension: &str) -> String {
        let name = format!("{}.{extension}", unique(prefix));
        self.names.push(name.clone());
        name
    }

    fn folder(&mut self, id: &str) {
        self.folders.push(id.to_string());
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let me = self.me.clone();
        let names = std::mem::take(&mut self.names);
        let folders = std::mem::take(&mut self.folders);
        // The test's own runtime may be unwinding; a thread with a runtime of its own
        // can still talk to the server. Nothing here may panic.
        let _ = std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async {
                let Ok(api) = Api::new(VERSION) else { return };
                for name in &names {
                    let _ = delete_named(&api, &me, name).await;
                }
                for folder in &folders {
                    let _ = api.delete_folder(&me, folder, false).await;
                }
            });
        })
        .join();
    }
}

/// Fetch what the server stored behind `link` and return its SHA-256.
async fn stored_sha256(qa: &Qa, link: &str) -> String {
    let target = tempfile::tempdir().unwrap();
    let path = target.path().join("stored");
    qa.api.download_to_file(&qa.me, link, &path).await.unwrap();
    sha256(&std::fs::read(&path).unwrap())
}

/// A port on 127.0.0.1 that nothing listens on: the discard port, as the reference
/// tests use it, unless something does listen there. Not a port just released by a
/// listener, which a test running alongside could be given next.
async fn closed_port() -> u16 {
    if TcpStream::connect(("127.0.0.1", 9)).await.is_err() {
        return 9;
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    listener.local_addr().unwrap().port()
}

/// `HEAD` a tus session the way the protocol allows anyone holding its URL to: no token.
async fn raw_head(url: &str) -> (u16, Option<u64>) {
    let response = reqwest::Client::new()
        .head(url)
        .header("Tus-Resumable", "1.0.0")
        .send()
        .await
        .unwrap();
    let offset = response
        .headers()
        .get("upload-offset")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());
    (response.status().as_u16(), offset)
}

fn on_server(url: &str) -> String {
    modlight_api::rewrite_to_host(url, SERVER).unwrap()
}

// ---------------------------------------------------------------------------------
// A forwarder in front of the reference server that can break the link
// ---------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Link {
    /// Every byte both ways.
    Pass,
    /// Close both sides once this many bytes have gone towards the server.
    CutAfter(u64),
    /// Stop taking the client's bytes once this many have gone towards the server, and
    /// keep the sockets open: the link a sleeping laptop leaves behind.
    StallAfter(u64),
}

/// A TCP forwarder on 127.0.0.1 to the reference server. Dropping it closes everything.
struct Proxy {
    origin: String,
    shutdown: CancellationToken,
    forwarded: Arc<AtomicU64>,
}

impl Proxy {
    async fn start(link: Link) -> Proxy {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let shutdown = CancellationToken::new();
        let forwarded = Arc::new(AtomicU64::new(0));
        let (stop, count) = (shutdown.clone(), Arc::clone(&forwarded));
        tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    _ = stop.cancelled() => return,
                    accepted = listener.accept() => accepted,
                };
                let Ok((client, _)) = accepted else { return };
                tokio::spawn(forward(
                    client,
                    link,
                    Arc::clone(&count),
                    stop.child_token(),
                ));
            }
        });
        Proxy {
            origin,
            shutdown,
            forwarded,
        }
    }

    /// Bytes that went towards the server so far, over every connection.
    fn forwarded(&self) -> u64 {
        self.forwarded.load(Ordering::SeqCst)
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

async fn forward(
    client: TcpStream,
    link: Link,
    forwarded: Arc<AtomicU64>,
    stop: CancellationToken,
) {
    let Ok(server) = TcpStream::connect(("127.0.0.1", SERVER_PORT)).await else {
        return;
    };
    let (mut client_read, mut client_write) = client.into_split();
    let (mut server_read, mut server_write) = server.into_split();

    let upstream = async {
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let read = match client_read.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(read) => read,
            };
            let (limit, stall) = match link {
                Link::Pass => (u64::MAX, false),
                Link::CutAfter(limit) => (limit, false),
                Link::StallAfter(limit) => (limit, true),
            };
            let before = forwarded.fetch_add(read as u64, Ordering::SeqCst);
            let allowed = limit.saturating_sub(before).min(read as u64) as usize;
            if allowed > 0 && server_write.write_all(&buffer[..allowed]).await.is_err() {
                return;
            }
            if allowed < read {
                if stall {
                    // Read nothing more, write nothing more, close nothing.
                    std::future::pending::<()>().await;
                }
                return;
            }
        }
    };
    let downstream = async {
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let read = match server_read.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(read) => read,
            };
            if client_write.write_all(&buffer[..read]).await.is_err() {
                return;
            }
        }
    };
    // Whichever side ends first ends both: the halves are dropped and the sockets close.
    tokio::select! {
        _ = stop.cancelled() => {}
        _ = upstream => {}
        _ = downstream => {}
    }
}

// ---------------------------------------------------------------------------------
// A scripted server: records each request, answers from a list
// ---------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    target: String,
    /// Names in lower case, in the order sent.
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// The tus metadata, decoded.
    fn metadata(&self) -> HashMap<String, String> {
        self.header("upload-metadata")
            .unwrap_or_default()
            .split(',')
            .filter_map(|pair| pair.split_once(' '))
            .map(|(key, value)| {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(value)
                    .unwrap();
                (key.to_string(), String::from_utf8(decoded).unwrap())
            })
            .collect()
    }
}

#[derive(Debug, Clone, Default)]
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
    /// Take the request and never answer.
    silent: bool,
    /// Take this many bytes of the body, then close the connection without answering:
    /// a link that dies mid-body.
    cut_after: Option<usize>,
}

impl Reply {
    fn new(status: u16) -> Reply {
        Reply {
            status,
            ..Reply::default()
        }
    }

    fn silent() -> Reply {
        Reply {
            silent: true,
            ..Reply::default()
        }
    }

    fn cut_after(bytes: usize) -> Reply {
        Reply {
            cut_after: Some(bytes),
            ..Reply::default()
        }
    }

    fn header(mut self, name: &str, value: impl Into<String>) -> Reply {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    fn body(mut self, body: impl Into<String>) -> Reply {
        self.body = body.into();
        self
    }
}

struct Scripted {
    origin: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    shutdown: CancellationToken,
}

impl Scripted {
    async fn start(replies: Vec<Reply>) -> Scripted {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let shutdown = CancellationToken::new();
        let (stop, log) = (shutdown.clone(), Arc::clone(&seen));
        tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    _ = stop.cancelled() => return,
                    accepted = listener.accept() => accepted,
                };
                let Ok((stream, _)) = accepted else { return };
                tokio::spawn(serve(
                    stream,
                    Arc::clone(&replies),
                    Arc::clone(&log),
                    stop.child_token(),
                ));
            }
        });
        Scripted {
            origin,
            seen,
            shutdown,
        }
    }

    fn credential(&self) -> Credential {
        Credential::new(&self.origin, "scripted-token")
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for Scripted {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

async fn serve(
    stream: TcpStream,
    replies: Arc<Mutex<VecDeque<Reply>>>,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: CancellationToken,
) {
    let mut stream = BufReader::new(stream);
    loop {
        let head = tokio::select! {
            _ = stop.cancelled() => return,
            head = read_head(&mut stream) => head,
        };
        let Some((mut request, length)) = head else {
            return;
        };
        let reply = replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Reply::new(500).body("no more replies"));
        let take = reply.cut_after.map_or(length, |cut| cut.min(length));
        request.body = vec![0; take];
        if stream.read_exact(&mut request.body).await.is_err() {
            return;
        }
        let head_only = request.method == "HEAD";
        seen.lock().unwrap().push(request);
        if reply.cut_after.is_some() {
            // Dropping the stream closes the connection under the rest of the body.
            return;
        }
        if reply.silent {
            stop.cancelled().await;
            return;
        }
        let mut out = format!("HTTP/1.1 {} Scripted\r\n", reply.status);
        for (name, value) in &reply.headers {
            out.push_str(&format!("{name}: {value}\r\n"));
        }
        out.push_str(&format!("Content-Length: {}\r\n\r\n", reply.body.len()));
        let mut bytes = out.into_bytes();
        if !head_only {
            bytes.extend_from_slice(reply.body.as_bytes());
        }
        if stream.get_mut().write_all(&bytes).await.is_err() {
            return;
        }
    }
}

/// A request's line and headers, and the length its body declares (read by the caller).
async fn read_head(stream: &mut BufReader<TcpStream>) -> Option<(Seen, usize)> {
    let mut line = String::new();
    if stream.read_line(&mut line).await.ok()? == 0 {
        return None;
    }
    let mut words = line.trim_end().splitn(3, ' ');
    let method = words.next()?.to_string();
    let target = words.next()?.to_string();
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if stream.read_line(&mut line).await.ok()? == 0 {
            return None;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    Some((
        Seen {
            method,
            target,
            headers,
            body: Vec::new(),
        },
        length,
    ))
}

fn summary(result: &TransferResult) -> (TransferOutcome, TransferReason, &str) {
    (result.outcome, result.reason, result.detail.as_str())
}

/// The tus client's own request decoration, present on every request it makes.
fn assert_decorated(seen: &[Seen]) {
    for request in seen {
        assert_eq!(
            request.header("tus-resumable"),
            Some("1.0.0"),
            "{request:?}"
        );
        assert_eq!(request.header("x-api-key"), Some("scripted-token"));
        assert_eq!(request.header("user-agent"), Some("Modlight/9.9.9"));
        assert_eq!(request.header("cache-control"), None);
        assert_eq!(request.header("transfer-encoding"), None);
    }
}

// ---------------------------------------------------------------------------------
// The live upload against the reference server
// ---------------------------------------------------------------------------------

#[tokio::test]
async fn live_uploads_land_in_the_folder_they_name() {
    let Some(qa) = qa().await else { return };
    let mut cleanup = Cleanup::new(&qa);
    let folder = qa
        .api
        .create_folder(&qa.me, &unique("ut-live"), None)
        .await
        .unwrap()
        .expect("folder");
    let folder_id = folder.id.clone().unwrap();
    cleanup.folder(&folder_id);

    // Bytes in memory.
    let memory_name = cleanup.name("ut-live-mem", "txt");
    let memory = random_bytes(10_000);
    let mut told = Vec::new();
    let outcome = qa
        .api
        .upload(
            &qa.me,
            LiveUpload::new(&memory_name, UploadSource::Bytes(memory.clone()))
                .with_folder(Some(&folder_id)),
            |sent, total| told.push((sent, total)),
            &CancellationToken::new(),
        )
        .await;
    let LiveOutcome::Answered(answer) = outcome else {
        panic!("no answer: {outcome:?}");
    };
    assert_eq!(answer.status, 201, "{}", answer.body);
    assert_eq!(answer.reason_phrase, "Created");
    assert_eq!(answer.url, format!("{SERVER}/upload"));
    assert!(
        answer
            .header("Content-Type")
            .is_some_and(|value| value.starts_with("application/json"))
    );
    assert_eq!(answer.refusal(), None);
    let stored = answer.uploaded().expect("a stored upload").clone();
    assert_eq!(stored.folder.as_deref(), Some(folder_id.as_str()));
    assert_eq!(stored.folder_warning, None);
    assert!(stored.link().is_some_and(|link| link.starts_with("http")));
    assert_eq!(told, vec![(10_000, 10_000)], "one chunk, told once");
    assert_eq!(
        stored_sha256(&qa, stored.raw_url.as_deref().unwrap()).await,
        sha256(&memory),
        "the multipart framing is not part of the file"
    );

    // A file, a kilobyte per write.
    let (file, bytes) = random_file(100 * 1024 + 7);
    let file_name = cleanup.name("ut-live-file", "bin");
    let mut told = Vec::new();
    let outcome = qa
        .api
        .upload(
            &qa.me,
            LiveUpload::new(&file_name, UploadSource::File(file.path().to_path_buf()))
                .with_folder(Some(&folder_id))
                .with_buffer_size(1024),
            |sent, total| told.push((sent, total)),
            &CancellationToken::new(),
        )
        .await;
    let LiveOutcome::Answered(answer) = outcome else {
        panic!("no answer: {outcome:?}");
    };
    let stored = answer.uploaded().expect("a stored upload").clone();
    assert_eq!(stored.folder.as_deref(), Some(folder_id.as_str()));
    assert_eq!(told.len(), 101, "100 full writes and one of 7 bytes");
    assert_eq!(told[0], (1024, bytes.len() as u64));
    assert_eq!(told.last(), Some(&(bytes.len() as u64, bytes.len() as u64)));
    assert_eq!(
        stored_sha256(&qa, stored.raw_url.as_deref().unwrap()).await,
        sha256(&bytes)
    );

    // Both are listed in the folder.
    let listed = qa
        .api
        .get_uploads(
            &qa.me,
            &UploadQuery {
                page: 1,
                per_page: 120,
                folder_id: Some(folder_id.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .items
        .unwrap_or_default();
    for name in [&memory_name, &file_name] {
        assert!(
            listed
                .iter()
                .any(|row| row.filename.as_deref() == Some(name))
        );
    }

    assert_eq!(purge(&qa, &memory_name).await, 1);
    assert_eq!(purge(&qa, &file_name).await, 1);
    qa.api
        .delete_folder(&qa.me, &folder_id, false)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_folder_that_is_not_ours_stores_the_file_with_a_warning() {
    let Some(qa) = qa().await else { return };
    let mut cleanup = Cleanup::new(&qa);
    let name = cleanup.name("ut-live-warn", "txt");
    let outcome = qa
        .api
        .upload(
            &qa.me,
            LiveUpload::new(&name, UploadSource::Bytes(random_bytes(512)))
                .with_folder(Some("00000000-0000-0000-0000-000000000000")),
            |_, _| {},
            &CancellationToken::new(),
        )
        .await;
    let LiveOutcome::Answered(answer) = outcome else {
        panic!("no answer: {outcome:?}");
    };
    assert_eq!(answer.status, 201, "{}", answer.body);
    let stored = answer.uploaded().expect("stored anyway");
    assert_eq!(stored.folder_warning.as_deref(), Some("unknown-folder"));
    assert_eq!(stored.folder, None, "the account's default folder is none");
    assert_eq!(purge(&qa, &name).await, 1);
}

#[tokio::test]
async fn live_progress_reaches_the_full_length() {
    let Some(qa) = qa().await else { return };
    let mut cleanup = Cleanup::new(&qa);
    let (file, bytes) = random_file(2 * MIB as usize);
    let name = cleanup.name("ut-live-progress", "bin");
    let length = bytes.len() as u64;
    let mut told = Vec::new();
    let outcome = qa
        .api
        .upload(
            &qa.me,
            LiveUpload::new(&name, UploadSource::File(file.path().to_path_buf())),
            |sent, total| told.push((sent, total)),
            &CancellationToken::new(),
        )
        .await;
    assert!(
        matches!(&outcome, LiveOutcome::Answered(answer) if answer.status == 201),
        "{outcome:?}"
    );
    assert_eq!(told.len(), 64, "2 MiB in 32 KiB writes");
    assert!(told.iter().all(|(_, total)| *total == length));
    assert!(told.windows(2).all(|pair| pair[0].0 < pair[1].0));
    assert_eq!(told.first(), Some(&(32 * 1024, length)));
    assert_eq!(told.last(), Some(&(length, length)));
    assert_eq!(purge(&qa, &name).await, 1);
}

#[tokio::test]
async fn cancelling_a_live_upload_stops_it_mid_body() {
    let Some(qa) = qa().await else { return };
    let mut cleanup = Cleanup::new(&qa);
    let file = sparse_file(64 * MIB);
    let name = cleanup.name("ut-live-cancel", "bin");
    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    let mut cancelled_at = None;
    let mut furthest = 0;
    let outcome = qa
        .api
        .upload(
            &qa.me,
            LiveUpload::new(&name, UploadSource::File(file.path().to_path_buf())),
            |sent, _| {
                furthest = sent;
                if sent >= 4 * MIB && cancelled_at.is_none() {
                    cancelled_at = Some(Instant::now());
                    stopper.cancel();
                }
            },
            &cancel,
        )
        .await;
    assert_eq!(outcome, LiveOutcome::Cancelled);
    let stopped_after = cancelled_at.expect("cancelled mid-body").elapsed();
    assert!(stopped_after < Duration::from_secs(2), "{stopped_after:?}");
    assert!(furthest < 64 * MIB, "the body was not finished: {furthest}");

    // Nothing was stored: the server drops a body that never ended.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(rows_named(&qa, &name).await.is_empty());
}

#[tokio::test]
async fn live_refusals_and_silences_are_told_apart() {
    let Some(qa) = qa().await else { return };

    // A token nobody issued: 404 with the exact sentence old ShareX configs expect.
    let stranger = Credential::new(SERVER, "not-a-real-token-xxxxxxxxxxxxxxxx");
    let outcome = qa
        .api
        .upload(
            &stranger,
            LiveUpload::new("x.txt", UploadSource::Bytes(b"x".to_vec())),
            |_, _| {},
            &CancellationToken::new(),
        )
        .await;
    let LiveOutcome::Answered(answer) = outcome else {
        panic!("no answer: {outcome:?}");
    };
    assert_eq!(answer.status, 404);
    assert_eq!(answer.refusal(), Some(UploadRefusal::TokenNotRecognized));
    assert_eq!(
        answer.server_detail().as_deref(),
        Some("Token specified not found.")
    );
    assert_eq!(answer.uploaded(), None);

    // More than any disk holds: refused from Content-Length alone, before the body is
    // read, and the answer is taken as soon as it arrives.
    let mut cleanup = Cleanup::new(&qa);
    let huge = sparse_file(1 << 40);
    let name = cleanup.name("ut-live-huge", "bin");
    let mut furthest = 0;
    let started = Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(60),
        qa.api.upload(
            &qa.me,
            LiveUpload::new(&name, UploadSource::File(huge.path().to_path_buf())),
            |sent, _| furthest = sent,
            &CancellationToken::new(),
        ),
    )
    .await
    .expect("the early answer is taken without sending a terabyte");
    let LiveOutcome::Answered(answer) = outcome else {
        panic!("no answer: {outcome:?}");
    };
    assert_eq!(answer.status, 507, "{}", answer.body);
    assert_eq!(answer.refusal(), Some(UploadRefusal::OutOfSpace));
    assert_eq!(
        answer.server_detail().as_deref(),
        Some("Server is low on disk space; upload rejected.")
    );
    eprintln!(
        "507 for a declared terabyte after {:?}, {} MiB of the body sent",
        started.elapsed(),
        furthest / MIB
    );
    assert!(rows_named(&qa, &name).await.is_empty());

    // Nothing listening: no answer, and the reason in words.
    let nowhere = qa.via(&format!("http://127.0.0.1:{}", closed_port().await));
    let outcome = qa
        .api
        .upload(
            &nowhere,
            LiveUpload::new("x.txt", UploadSource::Bytes(b"x".to_vec())),
            |_, _| {},
            &CancellationToken::new(),
        )
        .await;
    assert!(
        matches!(&outcome, LiveOutcome::NoAnswer(text) if !text.is_empty()),
        "{outcome:?}"
    );

    // A file that is not there is not an answer either.
    let outcome = qa
        .api
        .upload(
            &qa.me,
            LiveUpload::new("x.txt", UploadSource::File("/nonexistent/x.txt".into())),
            |_, _| {},
            &CancellationToken::new(),
        )
        .await;
    assert!(matches!(outcome, LiveOutcome::NoAnswer(_)), "{outcome:?}");

    // Already cancelled: nothing is sent.
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let outcome = qa
        .api
        .upload(
            &qa.me,
            LiveUpload::new("x.txt", UploadSource::Bytes(b"x".to_vec())),
            |_, _| {},
            &cancelled,
        )
        .await;
    assert_eq!(outcome, LiveOutcome::Cancelled);
}

// ---------------------------------------------------------------------------------
// The live upload on the wire
// ---------------------------------------------------------------------------------

#[tokio::test]
async fn the_live_upload_is_sharexs_multipart_byte_for_byte() {
    let answer = r#"{"message":"OK","url":"http://h/u/a","raw_url":"http://h/u/a.png","copy_url":"http://h/u/a.png","deletionUrl":"http://h/d","folder":"f-1"}"#;
    let server = Scripted::start(vec![
        Reply::new(201)
            .header("Content-Type", "application/json")
            .body(answer),
        Reply::new(201).body(answer),
        Reply::new(201).body(answer),
    ])
    .await;
    let api = Api::new("9.9.9").unwrap();
    let credential = server.credential();
    let picture = random_bytes(70_000);

    let outcome = api
        .upload(
            &credential,
            LiveUpload::new("Zrzut ekranu ż.png", UploadSource::Bytes(picture.clone()))
                .with_folder(Some("root")),
            |_, _| {},
            &CancellationToken::new(),
        )
        .await;
    let LiveOutcome::Answered(answer) = outcome else {
        panic!("no answer: {outcome:?}");
    };
    assert_eq!(
        answer.uploaded().and_then(|stored| stored.link()),
        Some("http://h/u/a.png")
    );

    let seen = server.seen();
    let request = &seen[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/upload");
    let content_type = request.header("content-type").unwrap();
    let boundary = content_type
        .strip_prefix("multipart/form-data; boundary=")
        .expect("multipart with a boundary");
    assert_eq!(&boundary[..20], "-".repeat(20));
    assert!(
        i64::from_str_radix(&boundary[20..], 16).is_ok(),
        "hex ticks"
    );
    assert_eq!(
        request.header("content-length"),
        Some(request.body.len().to_string().as_str())
    );
    assert_eq!(request.header("transfer-encoding"), None, "never chunked");
    assert_eq!(request.header("x-api-key"), Some("scripted-token"));
    assert_eq!(request.header("user-agent"), Some("Modlight/9.9.9"));
    assert_eq!(request.header("cache-control"), None);

    let mut expected = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"folderId\"\r\n\r\nroot\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"Zrzut ekranu ż.png\"\r\nContent-Type: image/png\r\n\r\n"
    )
    .into_bytes();
    expected.extend_from_slice(&picture);
    expected.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    assert!(
        request.body == expected,
        "the body is ShareX's, byte for byte"
    );

    // No folder: no field at all. A name Windows could not have: escaped, not raw.
    let (file, bytes) = random_file(5000);
    api.upload(
        &credential,
        LiveUpload::new(
            "a\"b\nc.unknownext",
            UploadSource::File(file.path().to_path_buf()),
        ),
        |_, _| {},
        &CancellationToken::new(),
    )
    .await;
    let seen = server.seen();
    let boundary = boundary_of(&seen[1]);
    let mut expected = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"a%22b%0Ac.unknownext\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    expected.extend_from_slice(&bytes);
    expected.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    assert!(seen[1].body == expected);

    // An empty file still has its part (the stand-in of phase 1 goes the same way).
    api.upload_bytes(&credential, "empty.txt", Vec::new(), None)
        .await
        .unwrap();
    let seen = server.seen();
    let boundary = boundary_of(&seen[2]);
    assert_eq!(
        String::from_utf8(seen[2].body.clone()).unwrap(),
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"empty.txt\"\r\nContent-Type: text/plain\r\n\r\n\r\n--{boundary}--\r\n"
        )
    );
    assert_eq!(
        seen[2].header("content-length"),
        Some(seen[2].body.len().to_string().as_str())
    );
}

fn boundary_of(request: &Seen) -> String {
    request
        .header("content-type")
        .and_then(|value| value.strip_prefix("multipart/form-data; boundary="))
        .expect("a multipart request")
        .to_string()
}

// ---------------------------------------------------------------------------------
// tus against the reference server
// ---------------------------------------------------------------------------------

#[tokio::test]
async fn tus_sends_small_and_empty_files() {
    let Some(qa) = qa().await else { return };
    let client = TusClient::new(VERSION).unwrap();
    let mut cleanup = Cleanup::new(&qa);
    let folder = qa
        .api
        .create_folder(&qa.me, &unique("ut-tus"), None)
        .await
        .unwrap()
        .expect("folder");
    let folder_id = folder.id.clone().unwrap();
    cleanup.folder(&folder_id);

    // A small file, into a folder.
    let (file, bytes) = random_file(300 * 1024);
    let name = cleanup.name("ut-tus-small", "bin");
    let length = bytes.len() as u64;
    let mut session = TusSession::new(Some(&folder_id));
    let mut told = Vec::new();
    let result = client
        .send(
            &qa.me,
            &tus_file(file.path(), &name, length),
            &mut session,
            |sent, total| told.push((sent, total)),
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(result.outcome, TransferOutcome::Success, "{result:?}");
    assert!(!result.copy_url.is_empty() && !result.view_url.is_empty());
    assert!(!result.raw_url.is_empty());
    assert_eq!(session.url, None, "a finished session is forgotten");
    assert_eq!(session.offset, length);
    assert_eq!(session.folder_id.as_deref(), Some(folder_id.as_str()));
    assert_eq!(told.first(), Some(&(0, length)));
    assert_eq!(told.last(), Some(&(length, length)));
    assert_eq!(stored_sha256(&qa, &result.raw_url).await, sha256(&bytes));
    let row = rows_named(&qa, &name).await.pop().expect("listed");
    assert_eq!(row.folder_id.as_deref(), Some(folder_id.as_str()));
    assert_eq!(row.size, length as i64);

    // An empty file is finished by its creation: no PATCH, links at once.
    let empty = tempfile::NamedTempFile::new().unwrap();
    let empty_name = cleanup.name("ut-tus-empty", "txt");
    let mut session = TusSession::default();
    let mut told = Vec::new();
    let result = client
        .send(
            &qa.me,
            &tus_file(empty.path(), &empty_name, 0),
            &mut session,
            |sent, total| told.push((sent, total)),
            &CancellationToken::new(),
        )
        .await;
    // Upstream expects 200 with the links here (the scripted test below plays that).
    // The reference server stores the file and then answers 404: its finish hook drops
    // the tus record, and @tus/server looks the record up again for Upload-Expires.
    // Upstream reads that 404 as a refusal, and so does this port.
    eprintln!("an empty file over tus: {result:?}");
    if result.outcome == TransferOutcome::Success {
        assert!(!result.copy_url.is_empty());
    } else {
        assert_eq!(
            summary(&result),
            (
                TransferOutcome::Blocked,
                TransferReason::Refused,
                "The file for this url was not found"
            )
        );
    }
    assert_eq!(session, TusSession::default());
    assert!(told.is_empty(), "nothing was sent, nothing to tell");
    assert_eq!(
        rows_named(&qa, &empty_name).await.len(),
        1,
        "stored either way"
    );

    assert_eq!(purge(&qa, &name).await, 1);
    assert_eq!(purge(&qa, &empty_name).await, 1);
    qa.api
        .delete_folder(&qa.me, &folder_id, false)
        .await
        .unwrap();
}

#[tokio::test]
async fn tus_resumes_after_a_cut_a_closed_port_and_a_restart() {
    let Some(qa) = qa().await else { return };
    let (file, bytes) = random_file(32 * MIB as usize);
    let length = bytes.len() as u64;
    let mut cleanup = Cleanup::new(&qa);
    let name = cleanup.name("ut-tus-resume", "bin");
    let upload = tus_file(file.path(), &name, length);
    let mut session = TusSession::default();
    let client = TusClient::new(VERSION).unwrap();

    // 1. The link dies 8 MiB in.
    let cut = Proxy::start(Link::CutAfter(8 * MIB)).await;
    let mut told = Vec::new();
    let result = client
        .send(
            &qa.via(&cut.origin),
            &upload,
            &mut session,
            |sent, total| told.push((sent, total)),
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(
        (result.outcome, result.reason),
        (TransferOutcome::Retry, TransferReason::Unreachable),
        "{result:?}"
    );
    assert!(!result.detail.is_empty(), "the reason in words");
    assert!(session.url.is_some(), "the session survives");
    assert_eq!(session.offset, 0, "nothing confirmed yet");
    assert_eq!(told.first(), Some(&(0, length)));
    let reached = told.last().unwrap().0;
    assert!(reached > 0 && reached < length, "cut mid-body at {reached}");
    assert!(cut.forwarded() >= 8 * MIB, "the cut point was reached");
    drop(cut);

    // 2. Pointed at a port nothing listens on: retry later, nothing forgotten.
    let before = session.clone();
    let nowhere = qa.via(&format!("http://127.0.0.1:{}", closed_port().await));
    let result = client
        .send(
            &nowhere,
            &upload,
            &mut session,
            |_, _| {},
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(
        (result.outcome, result.reason),
        (TransferOutcome::Retry, TransferReason::Unreachable),
        "{result:?}"
    );
    assert!(!result.detail.is_empty());
    assert_eq!(session, before, "url and offset kept");

    // 3. A restart: the session goes through JSON, the client is gone, a new one carries
    //    on, through a link that dies again 8 MiB later.
    let saved = serde_json::to_string(&session).unwrap();
    drop(client);
    let mut session: TusSession = serde_json::from_str(&saved).unwrap();
    let client = TusClient::new(VERSION).unwrap();
    let cut = Proxy::start(Link::CutAfter(8 * MIB)).await;
    let mut told = Vec::new();
    let result = client
        .send(
            &qa.via(&cut.origin),
            &upload,
            &mut session,
            |sent, total| told.push((sent, total)),
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(
        (result.outcome, result.reason),
        (TransferOutcome::Retry, TransferReason::Unreachable),
        "{result:?}"
    );
    let resumed_at = told.first().unwrap().0;
    assert!(
        resumed_at > 0,
        "resumed where the server got to, not from zero"
    );
    assert_eq!(
        session.offset, resumed_at,
        "the offset the server confirmed"
    );
    eprintln!(
        "after the first cut the server held {} MiB",
        resumed_at / MIB
    );
    drop(cut);

    // 4. A link that holds: the rest goes, and the file is whole.
    let pass = Proxy::start(Link::Pass).await;
    let mut told = Vec::new();
    let result = client
        .send(
            &qa.via(&pass.origin),
            &upload,
            &mut session,
            |sent, total| told.push((sent, total)),
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(result.outcome, TransferOutcome::Success, "{result:?}");
    let last_start = told.first().unwrap().0;
    assert!(
        last_start > resumed_at,
        "the second leg also left bytes behind"
    );
    eprintln!(
        "after the second cut the server held {} MiB",
        last_start / MIB
    );
    assert_eq!(told.last(), Some(&(length, length)));
    assert_eq!(session.url, None);
    assert_eq!(session.offset, length);
    assert!(
        pass.forwarded() < length - last_start + MIB,
        "only the remainder went over the last link"
    );
    drop(pass);

    assert_eq!(stored_sha256(&qa, &result.raw_url).await, sha256(&bytes));
    assert_eq!(purge(&qa, &name).await, 1, "stored once");
}

#[tokio::test]
async fn tus_drops_a_link_that_stops_moving_and_resumes_it() {
    let Some(qa) = qa().await else { return };
    let (file, bytes) = random_file(24 * MIB as usize);
    let length = bytes.len() as u64;
    let mut cleanup = Cleanup::new(&qa);
    let name = cleanup.name("ut-tus-stall", "bin");
    let upload = tus_file(file.path(), &name, length);
    let timings = TusTimings {
        stall_timeout: Duration::from_millis(1500),
        stall_poll: Duration::from_millis(250),
        ..TusTimings::default()
    };
    let client = TusClient::with_timings(VERSION, timings).unwrap();
    let mut session = TusSession::default();

    let stalled = Proxy::start(Link::StallAfter(2 * MIB)).await;
    let mut told = Vec::new();
    let started = Instant::now();
    let result = client
        .send(
            &qa.via(&stalled.origin),
            &upload,
            &mut session,
            |sent, total| told.push((sent, total)),
            &CancellationToken::new(),
        )
        .await;
    let took = started.elapsed();
    assert_eq!(
        summary(&result),
        (TransferOutcome::Retry, TransferReason::Unreachable, ""),
        "a stall is dropped quietly"
    );
    assert!(took >= Duration::from_millis(1500), "{took:?}");
    assert!(took < Duration::from_secs(30), "{took:?}");
    assert!(session.url.is_some());
    assert!(told.last().unwrap().0 < length);
    drop(stalled);

    // The server kept what arrived; the next attempt (with upstream's patience, so a
    // busy server finishing the file is not taken for a stall) carries on from there.
    let client = TusClient::new(VERSION).unwrap();
    let mut told = Vec::new();
    let result = client
        .send(
            &qa.me,
            &upload,
            &mut session,
            |sent, total| told.push((sent, total)),
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(result.outcome, TransferOutcome::Success, "{result:?}");
    assert!(told.first().unwrap().0 > 0, "resumed, not restarted");
    assert_eq!(stored_sha256(&qa, &result.raw_url).await, sha256(&bytes));
    assert_eq!(purge(&qa, &name).await, 1);
}

#[tokio::test]
async fn tus_abandons_a_half_finished_session() {
    let Some(qa) = qa().await else { return };
    let (file, bytes) = random_file(6 * MIB as usize);
    let length = bytes.len() as u64;
    let mut cleanup = Cleanup::new(&qa);
    let name = cleanup.name("ut-tus-abandon", "bin");
    let upload = tus_file(file.path(), &name, length);
    let client = TusClient::new(VERSION).unwrap();
    let mut session = TusSession::default();

    let cut = Proxy::start(Link::CutAfter(2 * MIB)).await;
    let result = client
        .send(
            &qa.via(&cut.origin),
            &upload,
            &mut session,
            |_, _| {},
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(result.outcome, TransferOutcome::Retry, "{result:?}");
    drop(cut);
    let url = session.url.clone().expect("a half-finished session");

    let (status, held) = raw_head(&on_server(&url)).await;
    assert_eq!(status, 200);
    assert!(
        held.is_some_and(|held| held > 0 && held < length),
        "{held:?}"
    );

    // The session URL names the forwarder's port; the DELETE goes to the account's host.
    assert_eq!(client.abandon(&qa.me, &url).await.unwrap(), Some(204));
    assert_eq!(raw_head(&on_server(&url)).await.0, 404, "forgotten");
    assert_eq!(client.abandon(&qa.me, "not a url").await.unwrap(), None);

    // A session the server forgot is not an error: the next attempt starts again.
    let mut told = Vec::new();
    let result = client
        .send(
            &qa.me,
            &upload,
            &mut session,
            |sent, total| told.push((sent, total)),
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(result.outcome, TransferOutcome::Success, "{result:?}");
    assert_eq!(told.first(), Some(&(0, length)), "from zero");
    assert_eq!(stored_sha256(&qa, &result.raw_url).await, sha256(&bytes));
    assert_eq!(purge(&qa, &name).await, 1);
}

#[tokio::test]
async fn tus_refusals_from_the_reference_server_are_classified() {
    let Some(qa) = qa().await else { return };
    let client = TusClient::new(VERSION).unwrap();
    let mut cleanup = Cleanup::new(&qa);
    let (file, bytes) = random_file(64 * 1024);
    let length = bytes.len() as u64;

    // A token nobody issued: blocked, and the token is the remedy.
    let stranger = Credential::new(SERVER, "not-a-real-token-xxxxxxxxxxxxxxxx");
    let mut session = TusSession::default();
    let result = client
        .send(
            &stranger,
            &tus_file(file.path(), &cleanup.name("ut-tus-stranger", "bin"), length),
            &mut session,
            |_, _| {},
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(
        summary(&result),
        (
            TransferOutcome::Blocked,
            TransferReason::TokenRejected,
            "Sign in or send a valid API token."
        )
    );
    assert_eq!(session, TusSession::default());

    // A folder that is not ours: 403 "Folder not found.", then once more without it.
    let name = cleanup.name("ut-tus-folder", "bin");
    let mut session = TusSession::new(Some("00000000-0000-0000-0000-000000000000"));
    let result = client
        .send(
            &qa.me,
            &tus_file(file.path(), &name, length),
            &mut session,
            |_, _| {},
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(result.outcome, TransferOutcome::Success, "{result:?}");
    assert_eq!(session.folder_id, None, "the refused folder is dropped");
    let row = rows_named(&qa, &name).await.pop().expect("stored");
    assert_eq!(row.folder_id, None, "in the account's default folder");
    assert_eq!(purge(&qa, &name).await, 1);

    // A declared length no disk holds: refused at creation, not worth repeating.
    let huge = sparse_file(1 << 40);
    let mut session = TusSession::default();
    let result = client
        .send(
            &qa.me,
            &tus_file(huge.path(), &cleanup.name("ut-tus-huge", "bin"), 1 << 40),
            &mut session,
            |_, _| {},
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(result.outcome, TransferOutcome::Blocked, "{result:?}");
    assert!(
        result.reason == TransferReason::OutOfSpace || result.reason == TransferReason::TooLarge,
        "{result:?}"
    );
    eprintln!("a declared terabyte: {result:?}");
    assert_eq!(session.url, None);

    // Requests this client never makes, classified the same way.
    let http = reqwest::Client::new();
    let response = http
        .post(format!("{SERVER}{TUS_PATH}"))
        .header("Upload-Length", "1")
        .header("Content-Length", "0")
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = response.text().await.unwrap();
    assert_eq!(status, 412, "no Tus-Resumable");
    assert_eq!(
        summary(&tus::classify(status, None, &body)),
        (
            TransferOutcome::Blocked,
            TransferReason::Refused,
            "Tus-Resumable Required"
        )
    );
    let response = http
        .post(format!("{SERVER}{TUS_PATH}"))
        .header("Tus-Resumable", "1.0.0")
        .header("Upload-Length", "abc")
        .header("Content-Length", "0")
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = response.text().await.unwrap();
    assert_eq!(status, 400, "a length that is not a number");
    assert_eq!(
        summary(&tus::classify(status, None, &body)),
        (
            TransferOutcome::Blocked,
            TransferReason::Refused,
            "Invalid upload-length"
        )
    );
}

// ---------------------------------------------------------------------------------
// tus on the wire, and the answers the reference server never gives
// ---------------------------------------------------------------------------------

#[tokio::test]
async fn the_tus_requests_are_upstreams_and_the_location_is_ours() {
    let server = Scripted::start(vec![
        Reply::new(201).header("Location", "http://evil.example:9999/api/files/abc?x=1"),
        Reply::new(200)
            .header("Upload-Offset", "204800")
            .header("X-Media-Url", "http://h/u/a")
            .header("X-Media-Raw-Url", "http://h/u/a.png")
            .header("X-Media-Copy-Url", "http://h/u/a.png"),
        Reply::new(204),
    ])
    .await;
    let client = TusClient::new("9.9.9").unwrap();
    let credential = server.credential();
    let (file, bytes) = random_file(204_800);
    let name = "Zrzut ekranu ż.png";
    let mut session = TusSession::new(Some("f-1"));
    let mut told = Vec::new();
    let result = client
        .send(
            &credential,
            &tus_file(file.path(), name, 204_800),
            &mut session,
            |sent, total| told.push((sent, total)),
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(result.outcome, TransferOutcome::Success, "{result:?}");
    assert_eq!(result.copy_url, "http://h/u/a.png");
    assert_eq!(result.view_url, "http://h/u/a");
    assert_eq!(result.raw_url, "http://h/u/a.png");
    assert_eq!(result.retry_after, None);
    assert_eq!(
        session,
        TusSession {
            url: None,
            offset: 204_800,
            folder_id: Some("f-1".into())
        }
    );
    let total = 204_800;
    assert_eq!(
        told,
        vec![
            (0, total),
            (81_920, total),
            (163_840, total),
            (204_800, total)
        ],
        "the resume point, then every 80 KiB read"
    );

    let seen = server.seen();
    assert_eq!(seen.len(), 2);
    let create = &seen[0];
    assert_eq!(
        (create.method.as_str(), create.target.as_str()),
        ("POST", "/api/files")
    );
    assert_eq!(create.header("upload-length"), Some("204800"));
    assert_eq!(create.header("content-length"), Some("0"));
    assert_eq!(create.header("content-type"), None);
    assert!(create.body.is_empty());
    assert_eq!(
        create.metadata(),
        HashMap::from([
            ("filename".to_string(), name.to_string()),
            ("filetype".to_string(), "image/png".to_string()),
            ("folderId".to_string(), "f-1".to_string()),
        ])
    );
    let patch = &seen[1];
    assert_eq!(patch.method, "PATCH");
    assert_eq!(
        patch.target, "/api/files/abc?x=1",
        "the path and query of Location, on our own origin"
    );
    assert_eq!(patch.header("upload-offset"), Some("0"));
    assert_eq!(
        patch.header("content-type"),
        Some("application/offset+octet-stream")
    );
    assert_eq!(patch.header("content-length"), Some("204800"));
    assert!(patch.body == bytes);
    assert_decorated(&seen);

    // Abandoning goes to our origin whatever host the URL names.
    assert_eq!(
        client
            .abandon(&credential, "http://evil.example/api/files/abc")
            .await
            .unwrap(),
        Some(204)
    );
    let seen = server.seen();
    assert_eq!(
        (seen[2].method.as_str(), seen[2].target.as_str()),
        ("DELETE", "/api/files/abc")
    );
    assert_decorated(&seen);
}

/// One attempt against a scripted server: what came back, what the session became and
/// what the server saw.
async fn scripted_attempt(
    replies: Vec<Reply>,
    mut session: TusSession,
    file: &tempfile::NamedTempFile,
    length: u64,
    timings: TusTimings,
) -> (TransferResult, TusSession, Vec<Seen>, Vec<(u64, u64)>) {
    let server = Scripted::start(replies).await;
    let client = TusClient::with_timings("9.9.9", timings).unwrap();
    if let Some(url) = session.url.as_mut() {
        // Stored sessions name the scripted server's origin, as they would have been made.
        *url = format!("{}{}", server.origin, url);
    }
    let mut told = Vec::new();
    let result = client
        .send(
            &server.credential(),
            &tus_file(file.path(), "shot.png", length),
            &mut session,
            |sent, total| told.push((sent, total)),
            &CancellationToken::new(),
        )
        .await;
    let seen = server.seen();
    assert_decorated(&seen);
    if let Some(url) = session.url.as_mut() {
        *url = url.replace(&server.origin, "");
    }
    (result, session, seen, told)
}

fn methods(seen: &[Seen]) -> Vec<&str> {
    seen.iter().map(|request| request.method.as_str()).collect()
}

#[tokio::test]
async fn every_answer_the_protocol_allows_is_read_like_upstream() {
    use TransferOutcome::{Blocked, Retry, Success};
    use TransferReason::{OutOfSpace, Refused, ServerError, TokenRejected, TooLarge, Unreachable};

    let (file, _) = random_file(1000);
    let timings = TusTimings::default();
    let fresh = TusSession::default;
    let stored = |offset| TusSession {
        url: Some("/api/files/s1".into()),
        offset,
        folder_id: None,
    };
    let links = |reply: Reply| {
        reply
            .header("X-Media-Url", "http://h/u/a")
            .header("X-Media-Copy-Url", "http://h/u/a.png")
    };

    // Too many requests: try again, when the server says.
    let (result, session, seen, _) = scripted_attempt(
        vec![Reply::new(429).header("Retry-After", "7").body("slow down")],
        fresh(),
        &file,
        1000,
        timings,
    )
    .await;
    assert_eq!(summary(&result), (Retry, ServerError, "429"));
    assert_eq!(result.retry_after, Some(Duration::from_secs(7)));
    assert_eq!(session, fresh());
    assert_eq!(methods(&seen), ["POST"]);

    // Unavailable until a date: the wait is counted from now.
    let later = (chrono::Utc::now() + chrono::Duration::seconds(120)).to_rfc2822();
    let later = later.replace("+0000", "GMT");
    let (result, ..) = scripted_attempt(
        vec![
            Reply::new(503)
                .header("Retry-After", later)
                .body("<html>down</html>"),
        ],
        fresh(),
        &file,
        1000,
        timings,
    )
    .await;
    assert_eq!(summary(&result), (Retry, ServerError, "503"));
    let wait = result.retry_after.unwrap();
    assert!(
        wait > Duration::from_secs(110) && wait <= Duration::from_secs(120),
        "{wait:?}"
    );

    // A 403 about the token: blocked at once, the folder is not to blame.
    let (result, session, seen, _) = scripted_attempt(
        vec![
            Reply::new(403)
                .body("This API token is scoped to \"read\" and this call needs \"upload\"."),
        ],
        TusSession::new(Some("f-1")),
        &file,
        1000,
        timings,
    )
    .await;
    assert_eq!(result.outcome, Blocked);
    assert_eq!(result.reason, TokenRejected);
    assert_eq!(session.folder_id.as_deref(), Some("f-1"));
    assert_eq!(methods(&seen), ["POST"]);

    // A 403 about the folder: once more without it, then the body; a 204 at the full
    // length without links is a stored file whose links never came.
    let (result, session, seen, told) = scripted_attempt(
        vec![
            Reply::new(403).body("Folder not found."),
            Reply::new(201).header("Location", "/api/files/d1"),
            Reply::new(204).header("Upload-Offset", "1000"),
        ],
        TusSession::new(Some("f-1")),
        &file,
        1000,
        timings,
    )
    .await;
    assert_eq!(
        summary(&result),
        (Blocked, ServerError, "no link in the response")
    );
    assert_eq!(
        session,
        TusSession {
            url: None,
            offset: 1000,
            folder_id: None
        }
    );
    assert_eq!(methods(&seen), ["POST", "POST", "PATCH"]);
    assert_eq!(
        seen[0].metadata().get("folderId").map(String::as_str),
        Some("f-1")
    );
    assert_eq!(seen[1].metadata().get("folderId"), None);
    assert_eq!(seen[2].target, "/api/files/d1");
    assert_eq!(told.last(), Some(&(1000, 1000)));

    // Any other refusal with a folder named also earns the one retry without it.
    let (result, _, seen, _) = scripted_attempt(
        vec![
            Reply::new(400).body("bad"),
            Reply::new(400).body("still bad"),
        ],
        TusSession::new(Some("f-1")),
        &file,
        1000,
        timings,
    )
    .await;
    assert_eq!(summary(&result), (Blocked, Refused, "still bad"));
    assert_eq!(methods(&seen), ["POST", "POST"]);

    // 201 without a Location: the server's fault, worth another go.
    let (result, session, ..) =
        scripted_attempt(vec![Reply::new(201)], fresh(), &file, 1000, timings).await;
    assert_eq!(
        summary(&result),
        (Retry, ServerError, "201 without Location")
    );
    assert_eq!(session.url, None);

    // A redirect is an answer, not an instruction.
    let (result, _, seen, _) = scripted_attempt(
        vec![Reply::new(302).header("Location", "http://elsewhere.example/api/files")],
        fresh(),
        &file,
        1000,
        timings,
    )
    .await;
    assert_eq!(summary(&result), (Retry, ServerError, "302"));
    assert_eq!(seen.len(), 1);

    // An empty file: created finished, links on the creation answer.
    let empty = tempfile::NamedTempFile::new().unwrap();
    let (result, session, seen, _) =
        scripted_attempt(vec![links(Reply::new(200))], fresh(), &empty, 0, timings).await;
    assert_eq!(result.outcome, Success);
    assert_eq!(result.copy_url, "http://h/u/a.png");
    assert_eq!(session, fresh());
    assert_eq!(methods(&seen), ["POST"]);
    assert_eq!(seen[0].header("upload-length"), Some("0"));

    // A session the server no longer has (410): a new one, from zero.
    let (result, session, seen, told) = scripted_attempt(
        vec![
            Reply::new(410),
            Reply::new(201).header("Location", "/api/files/g2"),
            links(Reply::new(200)),
        ],
        stored(400),
        &file,
        1000,
        timings,
    )
    .await;
    assert_eq!(result.outcome, Success);
    assert_eq!(methods(&seen), ["HEAD", "POST", "PATCH"]);
    assert_eq!(seen[0].target, "/api/files/s1");
    assert_eq!(seen[2].header("upload-offset"), Some("0"));
    assert_eq!(told.first(), Some(&(0, 1000)));
    assert_eq!(session.url, None);

    // The server holds more than the file has: the session is no use.
    let (result, _, seen, _) = scripted_attempt(
        vec![
            Reply::new(200).header("Upload-Offset", "5000"),
            Reply::new(201).header("Location", "/api/files/m2"),
            links(Reply::new(200)),
        ],
        stored(0),
        &file,
        1000,
        timings,
    )
    .await;
    assert_eq!(result.outcome, Success);
    assert_eq!(methods(&seen), ["HEAD", "POST", "PATCH"]);

    // Resuming at the server's offset; a 409 there means ask again next time.
    let (result, session, seen, told) = scripted_attempt(
        vec![
            Reply::new(200).header("Upload-Offset", "5"),
            Reply::new(409).body("Upload-Offset conflict\n"),
        ],
        stored(0),
        &file,
        1000,
        timings,
    )
    .await;
    assert_eq!(summary(&result), (Retry, ServerError, "409"));
    assert_eq!(session, stored(0), "offset reset, session kept");
    assert_eq!(seen[1].header("upload-offset"), Some("5"));
    assert_eq!(seen[1].header("content-length"), Some("995"));
    assert_eq!(seen[1].body.len(), 995);
    assert_eq!(told.first(), Some(&(5, 1000)));

    // A 204 short of the end: the link went, keep what the server confirmed.
    let (result, session, ..) = scripted_attempt(
        vec![
            Reply::new(200).header("Upload-Offset", "0"),
            Reply::new(204).header("Upload-Offset", "10"),
        ],
        stored(0),
        &file,
        1000,
        timings,
    )
    .await;
    assert_eq!(summary(&result), (Retry, Unreachable, ""));
    assert_eq!(session, stored(10));

    // Refusals of the body are read like refusals of the creation.
    for (reply, expected) in [
        (
            Reply::new(507).body("<html>full</html>"),
            (Blocked, OutOfSpace, ""),
        ),
        (
            Reply::new(413).body("Maximum size exceeded\n"),
            (Blocked, TooLarge, "Maximum size exceeded"),
        ),
        (Reply::new(500).body("oops"), (Retry, ServerError, "500")),
    ] {
        let (result, ..) = scripted_attempt(
            vec![Reply::new(200).header("Upload-Offset", "0"), reply],
            stored(0),
            &file,
            1000,
            timings,
        )
        .await;
        assert_eq!(summary(&result), expected);
    }

    // A long refusal is clipped to what a row holds.
    let (result, ..) = scripted_attempt(
        vec![Reply::new(400).body("x".repeat(300))],
        fresh(),
        &file,
        1000,
        timings,
    )
    .await;
    assert_eq!(result.detail, "x".repeat(200));

    // A handshake that never answers: given up on after the handshake deadline.
    let quick = TusTimings {
        handshake_timeout: Duration::from_millis(300),
        ..TusTimings::default()
    };
    let started = Instant::now();
    let (result, session, ..) =
        scripted_attempt(vec![Reply::silent()], stored(0), &file, 1000, quick).await;
    assert_eq!(
        (result.outcome, result.reason),
        (Retry, Unreachable),
        "{result:?}"
    );
    assert_eq!(result.detail, "The request timed out.");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(session, stored(0), "nothing learned, nothing forgotten");
}

/// Asking where a session stands and getting a gateway's or a busy server's answer says
/// nothing about the session: it is kept for the next attempt rather than replaced by a
/// new one that sends the whole file again. Only the server saying it does not have the
/// session (404, 410, any other refusal) starts over.
#[tokio::test]
async fn a_busy_server_does_not_cost_a_stored_session() {
    use TransferOutcome::{Retry, Success};
    use TransferReason::ServerError;

    let (file, _) = random_file(1000);
    let timings = TusTimings::default();
    let stored = |offset| TusSession {
        url: Some("/api/files/s1".into()),
        offset,
        folder_id: Some("f-1".into()),
    };
    for status in [408, 429, 502, 503, 504] {
        let (result, session, seen, told) = scripted_attempt(
            vec![
                Reply::new(status)
                    .header("Retry-After", "9")
                    .body("<html>busy</html>"),
            ],
            stored(400),
            &file,
            1000,
            timings,
        )
        .await;
        assert_eq!(
            summary(&result),
            (Retry, ServerError, status.to_string().as_str()),
            "{status}"
        );
        assert_eq!(result.status, Some(status));
        assert_eq!(result.retry_after, Some(Duration::from_secs(9)), "{status}");
        assert_eq!(session, stored(400), "{status}: the session is kept");
        assert_eq!(methods(&seen), ["HEAD"], "{status}: no new session");
        assert!(told.is_empty());
    }

    // A server error of its own about the session still starts over.
    let (result, session, seen, _) = scripted_attempt(
        vec![
            Reply::new(500),
            Reply::new(201).header("Location", "/api/files/s2"),
            Reply::new(200)
                .header("Upload-Offset", "1000")
                .header("X-Media-Url", "http://h/u/a"),
        ],
        stored(400),
        &file,
        1000,
        timings,
    )
    .await;
    assert_eq!(result.outcome, Success);
    assert_eq!(methods(&seen), ["HEAD", "POST", "PATCH"]);
    assert_eq!(session.url, None);
}

#[tokio::test]
async fn cancelling_a_tus_transfer_keeps_the_session() {
    // A server that takes the body and never answers: the attempt waits until cancelled.
    let server = Scripted::start(vec![
        Reply::new(201).header("Location", "/api/files/c1"),
        Reply::silent(),
    ])
    .await;
    let client = TusClient::new("9.9.9").unwrap();
    let (file, _) = random_file(100_000);
    let mut session = TusSession::default();
    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        stopper.cancel();
    });
    let result = client
        .send(
            &server.credential(),
            &tus_file(file.path(), "c.bin", 100_000),
            &mut session,
            |_, _| {},
            &cancel,
        )
        .await;
    assert_eq!(
        summary(&result),
        (TransferOutcome::Canceled, TransferReason::None, "")
    );
    assert_eq!(
        session.url.as_deref(),
        Some(format!("{}/api/files/c1", server.origin).as_str()),
        "the session to resume or abandon"
    );
}

// ---------------------------------------------------------------------------------
// Chunks: no PATCH carries more than one chunk (Cloudflare refuses bodies over 100 MB)
// ---------------------------------------------------------------------------------

/// What one chunked attempt left: the result, the session, what the server saw, every
/// progress report and every checkpoint (URL, offset) in order.
struct Chunked {
    result: TransferResult,
    session: TusSession,
    seen: Vec<Seen>,
    told: Vec<(u64, u64)>,
    saved: Vec<(Option<String>, u64)>,
}

async fn chunked_attempt(
    server: &Scripted,
    file: &TusFile,
    chunk: u64,
    session: TusSession,
) -> Chunked {
    let client = TusClient::new("9.9.9").unwrap().with_chunk_size(chunk);
    assert_eq!(client.chunk_size(), chunk);
    let mut session = session;
    let mut told = Vec::new();
    let mut saved = Vec::new();
    let before = server.seen().len();
    let result = client
        .send_with_checkpoints(
            &server.credential(),
            file,
            &mut session,
            |sent, total| told.push((sent, total)),
            |session| saved.push((session.url.clone(), session.offset)),
            &CancellationToken::new(),
        )
        .await;
    let seen = server.seen()[before..].to_vec();
    assert_decorated(&seen);
    Chunked {
        result,
        session,
        seen,
        told,
        saved,
    }
}

/// Progress never goes back, and every report is of the whole file.
fn assert_monotonic(told: &[(u64, u64)], total: u64) {
    assert!(!told.is_empty());
    for pair in told.windows(2) {
        assert!(pair[0].0 <= pair[1].0, "progress went back: {pair:?}");
    }
    assert!(
        told.iter()
            .all(|&(sent, all)| all == total && sent <= total)
    );
}

fn links(reply: Reply, page: &str) -> Reply {
    reply
        .header("X-Media-Url", page)
        .header("X-Media-Raw-Url", format!("{page}.bin"))
        .header("X-Media-Copy-Url", format!("{page}.bin"))
}

/// Each PATCH of `seen`: (offset header, content length, body length).
fn patches(seen: &[Seen]) -> Vec<(u64, u64, usize)> {
    seen.iter()
        .filter(|request| request.method == "PATCH")
        .map(|request| {
            (
                request.header("upload-offset").unwrap().parse().unwrap(),
                request.header("content-length").unwrap().parse().unwrap(),
                request.body.len(),
            )
        })
        .collect()
}

#[test]
fn the_chunk_is_fifty_mebibytes() {
    assert_eq!(
        tus::TUS_CHUNK_SIZE,
        52_428_800,
        "the server's TUS_CHUNK_SIZE"
    );
    const {
        assert!(
            tus::TUS_CHUNK_SIZE < 100_000_000,
            "under Cloudflare's 100 MB"
        )
    };
    assert_eq!(
        TusClient::new("9.9.9").unwrap().chunk_size(),
        tus::TUS_CHUNK_SIZE
    );
    assert_eq!(
        TusClient::new("9.9.9")
            .unwrap()
            .with_chunk_size(0)
            .chunk_size(),
        1
    );
}

#[tokio::test]
async fn a_file_of_several_chunks_goes_in_patches_of_one_chunk_at_most() {
    let (file, bytes) = random_file(2500);
    let server = Scripted::start(vec![
        Reply::new(201).header("Location", "/api/files/k1"),
        // Links on an answer that did not finish the upload are not links.
        links(Reply::new(204), "http://h/decoy-1").header("Upload-Offset", "1000"),
        links(Reply::new(204), "http://h/decoy-2").header("Upload-Offset", "2000"),
        links(Reply::new(200), "http://h/u/k1").header("Upload-Offset", "2500"),
    ])
    .await;
    let run = chunked_attempt(
        &server,
        &tus_file(file.path(), "k1.bin", 2500),
        1000,
        TusSession::default(),
    )
    .await;

    assert_eq!(
        run.result.outcome,
        TransferOutcome::Success,
        "{:?}",
        run.result
    );
    assert_eq!(run.result.view_url, "http://h/u/k1", "from the last answer");
    assert_eq!(run.result.copy_url, "http://h/u/k1.bin");
    assert_eq!(run.result.raw_url, "http://h/u/k1.bin");
    assert_eq!(run.result.status, Some(200));
    assert_eq!(methods(&run.seen), ["POST", "PATCH", "PATCH", "PATCH"]);
    assert_eq!(run.seen[0].header("upload-length"), Some("2500"));
    assert_eq!(
        patches(&run.seen),
        [(0, 1000, 1000), (1000, 1000, 1000), (2000, 500, 500)]
    );
    for request in &run.seen[1..] {
        assert_eq!(request.target, "/api/files/k1");
        assert_eq!(
            request.header("content-type"),
            Some("application/offset+octet-stream")
        );
    }
    assert!(run.seen[1].body == bytes[..1000]);
    assert!(run.seen[2].body == bytes[1000..2000]);
    assert!(run.seen[3].body == bytes[2000..]);

    let url = format!("{}/api/files/k1", server.origin);
    assert_eq!(
        run.saved,
        [
            (Some(url.clone()), 0),
            (Some(url.clone()), 1000),
            (Some(url.clone()), 2000)
        ],
        "the new session, then each chunk the server confirmed"
    );
    assert_eq!(
        run.session,
        TusSession {
            url: None,
            offset: 2500,
            folder_id: None
        }
    );
    assert_monotonic(&run.told, 2500);
    assert_eq!(run.told.first(), Some(&(0, 2500)));
    assert_eq!(run.told.last(), Some(&(2500, 2500)));
    for edge in [1000, 2000] {
        assert!(
            run.told.iter().any(|&(sent, _)| sent == edge),
            "told at the chunk edge {edge}: {:?}",
            run.told
        );
    }
}

#[tokio::test]
async fn files_up_to_one_chunk_go_in_one_patch_and_an_empty_one_in_an_empty_patch() {
    // Smaller than one chunk, and exactly one chunk: one PATCH with everything.
    for length in [999usize, 1000] {
        let (file, bytes) = random_file(length);
        let server = Scripted::start(vec![
            Reply::new(201).header("Location", "/api/files/one"),
            links(Reply::new(200), "http://h/u/one").header("Upload-Offset", length.to_string()),
        ])
        .await;
        let run = chunked_attempt(
            &server,
            &tus_file(file.path(), "one.bin", length as u64),
            1000,
            TusSession::default(),
        )
        .await;
        assert_eq!(run.result.outcome, TransferOutcome::Success, "{length}");
        assert_eq!(run.result.view_url, "http://h/u/one");
        assert_eq!(methods(&run.seen), ["POST", "PATCH"], "{length}");
        assert_eq!(patches(&run.seen), [(0, length as u64, length)], "{length}");
        assert!(run.seen[1].body == bytes);
        assert_eq!(run.saved.len(), 1, "only the new session: {:?}", run.saved);
        assert_monotonic(&run.told, length as u64);
        assert_eq!(run.told.last(), Some(&(length as u64, length as u64)));
    }

    // An empty file the server did not finish at creation: one empty PATCH finishes it.
    let empty = tempfile::NamedTempFile::new().unwrap();
    let server = Scripted::start(vec![
        Reply::new(201).header("Location", "/api/files/e0"),
        links(Reply::new(200), "http://h/u/e0").header("Upload-Offset", "0"),
    ])
    .await;
    let run = chunked_attempt(
        &server,
        &tus_file(empty.path(), "e0.txt", 0),
        1000,
        TusSession::default(),
    )
    .await;
    assert_eq!(
        run.result.outcome,
        TransferOutcome::Success,
        "{:?}",
        run.result
    );
    assert_eq!(run.result.view_url, "http://h/u/e0");
    assert_eq!(methods(&run.seen), ["POST", "PATCH"]);
    assert_eq!(run.seen[0].header("upload-length"), Some("0"));
    assert_eq!(patches(&run.seen), [(0, 0, 0)]);
    assert_eq!(run.told, [(0, 0)]);
    assert_eq!(run.session.url, None);
}

#[tokio::test]
async fn a_cut_in_the_second_chunk_resumes_from_the_end_of_the_first() {
    let (file, bytes) = random_file(2500);
    let upload = tus_file(file.path(), "cut.bin", 2500);
    let server = Scripted::start(vec![
        Reply::new(201).header("Location", "/api/files/c2"),
        Reply::new(204).header("Upload-Offset", "1000"),
        // The link dies 300 bytes into the second chunk.
        Reply::cut_after(300),
        // The next attempt: the server kept the first chunk.
        Reply::new(200)
            .header("Upload-Offset", "1000")
            .header("Upload-Length", "2500"),
        Reply::new(204).header("Upload-Offset", "2000"),
        links(Reply::new(200), "http://h/u/c2").header("Upload-Offset", "2500"),
    ])
    .await;

    let first = chunked_attempt(&server, &upload, 1000, TusSession::default()).await;
    assert_eq!(
        (
            first.result.outcome,
            first.result.reason,
            first.result.status
        ),
        (TransferOutcome::Retry, TransferReason::Unreachable, None),
        "{:?}",
        first.result
    );
    assert!(!first.result.detail.is_empty(), "the reason in words");
    assert_eq!(methods(&first.seen), ["POST", "PATCH", "PATCH"]);
    assert_eq!(first.seen[2].header("upload-offset"), Some("1000"));
    assert_eq!(first.seen[2].body.len(), 300, "cut mid-body");
    let url = format!("{}/api/files/c2", server.origin);
    assert_eq!(
        first.session,
        TusSession {
            url: Some(url.clone()),
            offset: 1000,
            folder_id: None
        },
        "the end of the first chunk, confirmed"
    );
    assert_eq!(
        first.saved.last(),
        Some(&(Some(url.clone()), 1000)),
        "saved before the second chunk went"
    );
    assert_monotonic(&first.told, 2500);

    // A restart in between: the session goes through JSON.
    let saved = serde_json::to_string(&first.session).unwrap();
    let session: TusSession = serde_json::from_str(&saved).unwrap();
    let second = chunked_attempt(&server, &upload, 1000, session).await;
    assert_eq!(
        second.result.outcome,
        TransferOutcome::Success,
        "{:?}",
        second.result
    );
    assert_eq!(second.result.view_url, "http://h/u/c2");
    assert_eq!(methods(&second.seen), ["HEAD", "PATCH", "PATCH"]);
    assert_eq!(second.seen[0].target, "/api/files/c2");
    assert_eq!(
        patches(&second.seen),
        [(1000, 1000, 1000), (2000, 500, 500)]
    );
    assert!(second.seen[1].body == bytes[1000..2000]);
    assert!(second.seen[2].body == bytes[2000..]);
    assert_eq!(
        second.told.first(),
        Some(&(1000, 2500)),
        "from the first chunk's end"
    );
    assert_eq!(second.told.last(), Some(&(2500, 2500)));
    assert_monotonic(&second.told, 2500);
    assert_eq!(second.saved, [(Some(url), 2000)]);
    assert_eq!(second.session.url, None);
}

#[tokio::test]
async fn bytes_in_memory_go_in_chunks_too() {
    let bytes = Arc::new(random_bytes(2100));
    let server = Scripted::start(vec![
        Reply::new(201).header("Location", "/api/files/m1"),
        Reply::new(204).header("Upload-Offset", "1000"),
        Reply::new(204).header("Upload-Offset", "2000"),
        links(Reply::new(200), "http://h/u/m1").header("Upload-Offset", "2100"),
    ])
    .await;
    let upload = TusFile::bytes("m1.png", Arc::clone(&bytes));
    assert_eq!(upload.length, 2100);
    assert_eq!(format!("{:?}", upload.source), "Bytes(2100 bytes)");
    let run = chunked_attempt(&server, &upload, 1000, TusSession::default()).await;
    assert_eq!(
        run.result.outcome,
        TransferOutcome::Success,
        "{:?}",
        run.result
    );
    assert_eq!(
        patches(&run.seen),
        [(0, 1000, 1000), (1000, 1000, 1000), (2000, 100, 100)]
    );
    let sent: Vec<u8> = run.seen[1..]
        .iter()
        .flat_map(|request| request.body.clone())
        .collect();
    assert!(sent == *bytes);
}

#[tokio::test]
async fn a_refusal_of_a_later_chunk_keeps_what_was_confirmed() {
    let (file, _) = random_file(2500);
    let server = Scripted::start(vec![
        Reply::new(201).header("Location", "/api/files/r1"),
        Reply::new(204).header("Upload-Offset", "1000"),
        Reply::new(503)
            .header("Retry-After", "9")
            .body("<html>busy</html>"),
    ])
    .await;
    let run = chunked_attempt(
        &server,
        &tus_file(file.path(), "r1.bin", 2500),
        1000,
        TusSession::default(),
    )
    .await;
    assert_eq!(
        summary(&run.result),
        (TransferOutcome::Retry, TransferReason::ServerError, "503")
    );
    assert_eq!(run.result.status, Some(503));
    assert_eq!(run.result.retry_after, Some(Duration::from_secs(9)));
    assert_eq!(run.session.offset, 1000);
    assert!(run.session.url.is_some());

    // A 204 that confirms less than the chunk: the link went, as with one PATCH.
    let server = Scripted::start(vec![
        Reply::new(201).header("Location", "/api/files/r2"),
        Reply::new(204).header("Upload-Offset", "1000"),
        Reply::new(204).header("Upload-Offset", "1500"),
    ])
    .await;
    let run = chunked_attempt(
        &server,
        &tus_file(file.path(), "r2.bin", 2500),
        1000,
        TusSession::default(),
    )
    .await;
    assert_eq!(
        summary(&run.result),
        (TransferOutcome::Retry, TransferReason::Unreachable, "")
    );
    assert_eq!(run.session.offset, 1500, "what the server confirmed");
    assert_eq!(methods(&run.seen), ["POST", "PATCH", "PATCH"]);
    assert_eq!(run.saved.len(), 2, "the session and the first chunk only");
}

#[tokio::test]
async fn a_cancel_between_chunks_keeps_the_session_and_the_confirmed_offset() {
    let (file, _) = random_file(2500);
    let server = Scripted::start(vec![
        Reply::new(201).header("Location", "/api/files/x1"),
        Reply::new(204).header("Upload-Offset", "1000"),
        Reply::silent(),
    ])
    .await;
    let client = TusClient::new("9.9.9").unwrap().with_chunk_size(1000);
    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    let mut session = TusSession::default();
    let result = client
        .send_with_checkpoints(
            &server.credential(),
            &tus_file(file.path(), "x1.bin", 2500),
            &mut session,
            |_, _| {},
            |session| {
                if session.offset == 1000 {
                    stopper.cancel();
                }
            },
            &cancel,
        )
        .await;
    assert_eq!(
        summary(&result),
        (TransferOutcome::Canceled, TransferReason::None, "")
    );
    assert_eq!(session.offset, 1000);
    assert!(session.url.is_some());
    assert_eq!(
        methods(&server.seen()),
        ["POST", "PATCH"],
        "no second chunk"
    );
}

/// Several real chunks to the reference server, and a cut in the second one resumed
/// from what the server holds.
#[tokio::test]
async fn tus_sends_several_chunks_to_the_reference_server_and_resumes_a_cut_one() {
    let Some(qa) = qa().await else { return };
    let (file, bytes) = random_file(10 * MIB as usize + 12345);
    let length = bytes.len() as u64;
    let mut cleanup = Cleanup::new(&qa);
    let name = cleanup.name("ut-tus-chunks", "bin");
    let upload = tus_file(file.path(), &name, length);
    let client = TusClient::new(VERSION).unwrap().with_chunk_size(4 * MIB);
    let mut session = TusSession::default();

    // The link dies 6 MiB in: in the second chunk.
    let cut = Proxy::start(Link::CutAfter(6 * MIB)).await;
    let mut saved = Vec::new();
    let result = client
        .send_with_checkpoints(
            &qa.via(&cut.origin),
            &upload,
            &mut session,
            |_, _| {},
            |session| saved.push(session.offset),
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(result.outcome, TransferOutcome::Retry, "{result:?}");
    assert_eq!(saved, [0, 4 * MIB], "the session, then the first chunk");
    assert_eq!(session.offset, 4 * MIB);
    drop(cut);

    let mut told = Vec::new();
    let result = client
        .send(
            &qa.me,
            &upload,
            &mut session,
            |sent, total| told.push((sent, total)),
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(result.outcome, TransferOutcome::Success, "{result:?}");
    let resumed_at = told.first().unwrap().0;
    assert!(
        (4 * MIB..6 * MIB).contains(&resumed_at),
        "resumed at {resumed_at}"
    );
    assert_monotonic(&told, length);
    assert_eq!(told.last(), Some(&(length, length)));
    assert_eq!(stored_sha256(&qa, &result.raw_url).await, sha256(&bytes));
    assert_eq!(purge(&qa, &name).await, 1);
}
