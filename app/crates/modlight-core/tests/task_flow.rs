//! The task flow and the offline queue against the reference server on 127.0.0.1:8420,
//! the way the window drives them: `UploadManager` starts tasks, a host loop plays the
//! window (answers questions, collects effects), the queue sends over tus.
//!
//! Needs `_wzorzec/qa-accounts.env` and the server; without either every test prints
//! that it skipped and passes. Whatever a test uploads is deleted when it ends.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use modlight_api::{Api, Credential, UploadQuery};
use modlight_core::config::AppConfig;
use modlight_core::tasks::context::TaskContext;
use modlight_core::tasks::host::{TaskAnswer, TaskChannel, TaskEffect, TaskMessage, TaskQuestion};
use modlight_core::tasks::info::{DataType, TaskStatus};
use modlight_core::tasks::manager::TaskManager;
use modlight_core::tasks::settings::{AppUploadSettings, TaskSettings};
use modlight_core::tasks::upload_manager;
use modlight_core::tasks::worker::TaskState;
use modlight_core::upload::QueueReason;
use modlight_core::upload::queue::{AccountSlot, QueueAccount, QueueEvent, UploadQueue};
use modlight_core::upload::store::QueueStore;
use modlight_core::upload::transport::ApiTransport;
use sha2::Digest;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const SERVER: &str = "http://127.0.0.1:8420";
const SERVER_PORT: u16 = 8420;
const MIB: usize = 1024 * 1024;

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
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect(),
    )
}

/// The QA account, or `None` (and a line saying so) when the test cannot run.
fn qa(what: &str) -> Option<Credential> {
    let Some(env) = qa_env() else {
        eprintln!("{what}: skipped, no _wzorzec/qa-accounts.env");
        return None;
    };
    if std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{SERVER_PORT}").parse().unwrap(),
        Duration::from_millis(500),
    )
    .is_err()
    {
        eprintln!("{what}: skipped, nothing answers on {SERVER}");
        return None;
    }
    let token = env
        .get("QA_APP1_TOKEN_WRITE")
        .expect("QA_APP1_TOKEN_WRITE in qa-accounts.env")
        .clone();
    Some(Credential::new(SERVER, token))
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

fn unique(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}-{nanos:x}")
}

fn random_bytes(length: usize) -> Vec<u8> {
    use rand::RngCore;
    let mut bytes = vec![0u8; length];
    rand::rng().fill_bytes(&mut bytes);
    bytes
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

// ---------------------------------------------------------------------------------
// Cleaning up
// ---------------------------------------------------------------------------------

async fn delete_named(api: &Api, me: &Credential, name: &str) -> usize {
    let mut ids = Vec::new();
    for trashed in [false, true] {
        let Ok(page) = api
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
            .await
        else {
            continue;
        };
        ids.extend(
            page.items
                .unwrap_or_default()
                .into_iter()
                .filter(|row| row.filename.as_deref() == Some(name))
                .filter_map(|row| row.id),
        );
    }
    if ids.is_empty() {
        return 0;
    }
    let _ = api.delete_uploads(me, &ids, "trash").await;
    let _ = api.delete_uploads(me, &ids, "permanent").await;
    ids.len()
}

/// Upload names to delete when the test ends, however it ends.
struct Cleanup {
    me: Credential,
    names: Vec<String>,
}

impl Cleanup {
    fn new(me: &Credential) -> Cleanup {
        Cleanup {
            me: me.clone(),
            names: Vec::new(),
        }
    }

    fn add(&mut self, name: &str) {
        self.names.push(name.to_string());
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let me = self.me.clone();
        let names = std::mem::take(&mut self.names);
        let _ = std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async {
                let Ok(api) = Api::new("0.7.7") else { return };
                for name in &names {
                    delete_named(&api, &me, name).await;
                }
            });
        })
        .join();
    }
}

// ---------------------------------------------------------------------------------
// The window's side, played by the test
// ---------------------------------------------------------------------------------

struct Host {
    runtime: tokio::runtime::Runtime,
    ctx: TaskContext,
    receiver: Receiver<TaskMessage>,
    tasks: TaskManager,
    profile: PathBuf,
    effects: Vec<TaskEffect>,
    finished: Vec<TaskState>,
    questions: Vec<String>,
}

impl Host {
    /// The task machinery over `profile`, signed in as `account` at `host`.
    fn new(profile: &Path, account: Option<(&str, &Credential)>) -> Host {
        let runtime = runtime();
        let api = Api::new("0.7.7").unwrap();
        let slot: AccountSlot = Arc::new(RwLock::new(account.map(|(host, me)| QueueAccount {
            host: host.to_string(),
            token: me.token.clone(),
            tag: String::new(),
            display_name: "QA".into(),
        })));
        let queue = UploadQueue::new(
            QueueStore::new(profile),
            slot.clone(),
            Arc::new(ApiTransport::new(api.clone())),
            Arc::new(|| true),
        );
        let (channel, receiver) = TaskChannel::new(Arc::new(|| {}));
        let ctx = TaskContext {
            api,
            account: slot,
            queue,
            channel,
            runtime: runtime.handle().clone(),
            settings: Arc::new(RwLock::new(AppUploadSettings::default())),
            personal_folder: profile.to_path_buf(),
            auto_increment: Arc::new(AtomicI64::new(0)),
            strip_png_color_space: Arc::new(AtomicBool::new(false)),
        };
        Host {
            runtime,
            ctx,
            receiver,
            tasks: TaskManager::new(None),
            profile: profile.to_path_buf(),
            effects: Vec::new(),
            finished: Vec::new(),
            questions: Vec::new(),
        }
    }

    fn settings(&self) -> TaskSettings {
        TaskSettings::defaults_from(&AppConfig::load(&self.profile))
    }

    /// Play the window until `count` tasks ended.
    fn run_until_finished(&mut self, count: usize, seconds: u64) {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while self.finished.len() < count {
            assert!(
                Instant::now() < deadline,
                "only {} of {count} tasks ended in {seconds} s",
                self.finished.len()
            );
            let Ok(message) = self.receiver.recv_timeout(Duration::from_millis(50)) else {
                continue;
            };
            match message {
                TaskMessage::Event { task, event } => {
                    let outcome = self.tasks.handle_event(task, event, &self.ctx);
                    self.effects.extend(outcome.effects);
                    if outcome.finished
                        && let Some(task) = self.tasks.find(task)
                    {
                        self.finished.push(task.snapshot());
                    }
                }
                TaskMessage::Ask {
                    question, reply, ..
                } => {
                    let answer = match question {
                        TaskQuestion::LargeFile { .. } => {
                            self.questions.push("large file".into());
                            TaskAnswer::LargeFile {
                                should_continue: true,
                                dont_show_again: false,
                            }
                        }
                        TaskQuestion::SignIn => {
                            self.questions.push("sign in".into());
                            TaskAnswer::SignIn(false)
                        }
                        TaskQuestion::BeforeUpload(info) => {
                            self.questions.push("before upload".into());
                            TaskAnswer::BeforeUpload {
                                accepted: true,
                                settings: Box::new(info.settings.clone()),
                            }
                        }
                        TaskQuestion::FileExists { path } => {
                            self.questions.push("file exists".into());
                            TaskAnswer::FileExists(Some(path))
                        }
                        TaskQuestion::SaveAs { folder, file_name } => {
                            self.questions.push("save as".into());
                            TaskAnswer::SaveAs(Some(folder.join(format!("saved-{file_name}"))))
                        }
                        TaskQuestion::Annotate(image) => {
                            self.questions.push("annotate".into());
                            TaskAnswer::Annotated(Some(image))
                        }
                        TaskQuestion::Beautify(image) => {
                            self.questions.push("beautify".into());
                            TaskAnswer::Annotated(Some(image))
                        }
                    };
                    let _ = reply.send(answer);
                }
                TaskMessage::Effect(effect) => self.effects.push(effect),
            }
        }
    }

    fn copied(&self) -> Vec<String> {
        self.effects
            .iter()
            .filter_map(|e| match e {
                TaskEffect::CopyText(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// Wait for the queue to report an outcome.
    fn queue_outcome(&self, seconds: u64) -> Box<modlight_core::upload::queue::QueueOutcome> {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        loop {
            for event in self.ctx.queue.drain_events() {
                if let QueueEvent::Finished(outcome) = event {
                    return outcome;
                }
            }
            assert!(
                Instant::now() < deadline,
                "the queue reported nothing in {seconds} s; it holds {:?}",
                self.ctx
                    .queue
                    .snapshot()
                    .iter()
                    .map(|i| (i.state, i.reason, i.detail.clone()))
                    .collect::<Vec<_>>()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

// ---------------------------------------------------------------------------------
// A forwarder whose link can be switched off, cut, or passed through
// ---------------------------------------------------------------------------------

const PASS: u8 = 0;
/// Accept and close at once: the server cannot be reached.
const DOWN: u8 = 1;
/// Close the link once `limit` bytes went towards the server.
const CUT: u8 = 2;

struct Switch {
    origin: String,
    mode: Arc<AtomicU8>,
    limit: Arc<AtomicU64>,
    forwarded: Arc<AtomicU64>,
    _runtime: tokio::runtime::Runtime,
}

impl Switch {
    fn start() -> Switch {
        let runtime = runtime();
        let mode = Arc::new(AtomicU8::new(PASS));
        let limit = Arc::new(AtomicU64::new(u64::MAX));
        let forwarded = Arc::new(AtomicU64::new(0));
        let listener = runtime
            .block_on(TcpListener::bind(("127.0.0.1", 0)))
            .unwrap();
        let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let (m, l, f) = (mode.clone(), limit.clone(), forwarded.clone());
        runtime.spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let (m, l, f) = (m.clone(), l.clone(), f.clone());
                tokio::spawn(async move { forward(client, m, l, f).await });
            }
        });
        Switch {
            origin,
            mode,
            limit,
            forwarded,
            _runtime: runtime,
        }
    }

    fn set(&self, mode: u8, limit: u64) {
        self.forwarded.store(0, Ordering::SeqCst);
        self.limit.store(limit, Ordering::SeqCst);
        self.mode.store(mode, Ordering::SeqCst);
    }

    fn forwarded(&self) -> u64 {
        self.forwarded.load(Ordering::SeqCst)
    }
}

async fn forward(
    client: TcpStream,
    mode: Arc<AtomicU8>,
    limit: Arc<AtomicU64>,
    forwarded: Arc<AtomicU64>,
) {
    if mode.load(Ordering::SeqCst) == DOWN {
        return;
    }
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
            let cap = if mode.load(Ordering::SeqCst) == CUT {
                limit.load(Ordering::SeqCst)
            } else {
                u64::MAX
            };
            let before = forwarded.fetch_add(read as u64, Ordering::SeqCst);
            let allowed = cap.saturating_sub(before).min(read as u64) as usize;
            if allowed > 0 && server_write.write_all(&buffer[..allowed]).await.is_err() {
                return;
            }
            if allowed < read {
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
    tokio::select! {
        _ = upstream => {}
        _ = downstream => {}
    }
}

// ---------------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------------

#[test]
fn a_small_file_goes_up_and_its_link_is_copied() {
    let Some(me) = qa("small upload") else { return };
    let mut cleanup = Cleanup::new(&me);
    let profile = tempfile::tempdir().unwrap();
    let mut host = Host::new(profile.path(), Some((SERVER, &me)));
    let name = format!("{}.bin", unique("qa-task-small"));
    cleanup.add(&name);
    let path = profile.path().join(&name);
    let bytes = random_bytes(200 * 1024);
    std::fs::write(&path, &bytes).unwrap();

    let settings = host.settings();
    upload_manager::upload_file(&mut host.tasks, &path, &settings, &host.ctx);
    host.run_until_finished(1, 60);

    let state = &host.finished[0];
    assert_eq!(
        state.status,
        TaskStatus::Completed,
        "{:?}",
        state.info.result.errors
    );
    let url = state.info.result.url.clone();
    assert!(url.starts_with("http"), "a link: {url}");
    assert_eq!(
        host.copied(),
        vec![url.clone()],
        "CopyURLToClipboard is on by default"
    );
    assert!(
        host.questions.is_empty(),
        "nothing to ask: {:?}",
        host.questions
    );
    assert!(host.ctx.queue.snapshot().is_empty(), "nothing queued");
    // The bytes the server keeps are the ones sent.
    let stored = profile.path().join("stored");
    host.runtime
        .block_on(
            host.ctx
                .api
                .download_to_file(&me, &state.info.result.thumbnail_url, &stored),
        )
        .unwrap();
    assert_eq!(sha256(&std::fs::read(&stored).unwrap()), sha256(&bytes));
}

#[test]
fn a_text_and_a_paste_go_up_as_files() {
    let Some(me) = qa("text and paste") else {
        return;
    };
    let mut cleanup = Cleanup::new(&me);
    let profile = tempfile::tempdir().unwrap();
    let mut host = Host::new(profile.path(), Some((SERVER, &me)));
    let settings = host.settings();

    // The text window: TextTaskSaveAsFile writes it to the screenshots folder first.
    let text = format!("Zażółć gęślą jaźń {}", unique("qa-text"));
    upload_manager::upload_text(&mut host.tasks, &text, &settings, false, &host.ctx);
    // The paste window: the content as a named file, no byte order mark.
    let paste_name = format!("{}.md", unique("qa-paste"));
    cleanup.add(&paste_name);
    let paste = "# Notes\n\n- one\n- two\n";
    upload_manager::upload_data(
        &mut host.tasks,
        DataType::File,
        paste.as_bytes().to_vec(),
        &paste_name,
        &settings,
        &host.ctx,
    );
    host.run_until_finished(2, 60);

    for state in &host.finished {
        cleanup.add(&state.info.file_name);
        assert_eq!(
            state.status,
            TaskStatus::Completed,
            "{:?}",
            state.info.result.errors
        );
        assert!(!state.info.result.url.is_empty());
    }
    let text_task = host
        .finished
        .iter()
        .find(|s| s.info.data_type == DataType::Text)
        .expect("the text task");
    let saved = std::fs::read(text_task.info.file_path()).expect("the text was saved as a file");
    assert_eq!(&saved[..3], b"\xEF\xBB\xBF", "saved with a byte order mark");
    let stored = profile.path().join("stored-text");
    host.runtime
        .block_on(
            host.ctx
                .api
                .download_to_file(&me, &text_task.info.result.thumbnail_url, &stored),
        )
        .unwrap();
    assert_eq!(
        std::fs::read(&stored).unwrap(),
        text.as_bytes(),
        "uploaded without the byte order mark"
    );
    let paste_task = host
        .finished
        .iter()
        .find(|s| s.info.file_name == paste_name)
        .expect("the paste task");
    let stored = profile.path().join("stored-paste");
    host.runtime
        .block_on(host.ctx.api.download_to_file(
            &me,
            &paste_task.info.result.thumbnail_url,
            &stored,
        ))
        .unwrap();
    assert_eq!(std::fs::read(&stored).unwrap(), paste.as_bytes());
}

#[test]
fn an_address_is_shortened() {
    let Some(me) = qa("shortener") else { return };
    let profile = tempfile::tempdir().unwrap();
    let mut host = Host::new(profile.path(), Some((SERVER, &me)));
    // A name of its own, and a link that expires on its own: the API has no way to
    // delete one.
    let code = unique("qa").replace('-', "");
    let mut settings = host.settings();
    settings.modlight_short_link = Some(modlight_api::ShortLinkOptions {
        custom_code: Some(code.clone()),
        expires_in_days: Some(1),
        ..Default::default()
    });
    let target = format!("https://example.com/{}", unique("qa-short"));
    upload_manager::shorten_url(&mut host.tasks, &target, &settings, &host.ctx);
    host.run_until_finished(1, 60);
    let done = &host.finished[0];
    assert_eq!(
        done.status,
        TaskStatus::Completed,
        "{}",
        done.info.result.errors
    );
    let short = done.info.result.shortened_url.clone();
    assert!(
        short.ends_with(&code),
        "the link has the name asked for: {short}"
    );
    assert_eq!(
        host.copied(),
        vec![short],
        "the short link is what gets copied"
    );

    // The same name again: the server refuses, and the sentence comes first.
    upload_manager::shorten_url(&mut host.tasks, &target, &settings, &host.ctx);
    host.run_until_finished(2, 60);
    let failed = &host.finished[1];
    assert!(failed.info.result.shortened_url.is_empty());
    let first = &failed.info.result.errors.errors[0].text;
    assert!(
        first.starts_with(modlight_core::l10n::t(
            modlight_core::l10n::uploaders::Modlight_Address_taken
        )),
        "the refusal reads as a sentence first: {first}"
    );
    // As upstream: the result still holds the address, so it is not an error, and
    // "copy the link" copies the address itself.
    assert_eq!(failed.status, TaskStatus::Completed);
    assert_eq!(host.copied()[1], target);
}

#[test]
fn signed_out_uploads_ask_for_a_sign_in_and_keep_the_file() {
    let Some(_me) = qa("signed out") else { return };
    let profile = tempfile::tempdir().unwrap();
    let mut host = Host::new(profile.path(), None);
    let path = profile.path().join("never-sent.bin");
    std::fs::write(&path, random_bytes(1024)).unwrap();
    let settings = host.settings();
    upload_manager::upload_file(&mut host.tasks, &path, &settings, &host.ctx);
    host.run_until_finished(1, 30);
    assert_eq!(host.questions, vec!["sign in".to_string()]);
    assert!(path.is_file(), "the file is untouched");
    assert!(host.copied().is_empty());
}

#[test]
fn an_upload_that_cannot_reach_the_server_is_queued_and_sent_after_a_restart() {
    let Some(me) = qa("offline queue") else {
        return;
    };
    let mut cleanup = Cleanup::new(&me);
    let switch = Switch::start();
    let profile = tempfile::tempdir().unwrap();
    let name = format!("{}.bin", unique("qa-queued"));
    cleanup.add(&name);
    let path = profile.path().join(&name);
    let bytes = random_bytes(300 * 1024);
    std::fs::write(&path, &bytes).unwrap();

    // The network is gone: the live upload gets no answer and the queue takes it.
    switch.set(DOWN, 0);
    {
        let mut host = Host::new(profile.path(), Some((&switch.origin, &me)));
        let settings = host.settings();
        upload_manager::upload_file(&mut host.tasks, &path, &settings, &host.ctx);
        host.run_until_finished(1, 60);
        let state = &host.finished[0];
        assert_eq!(state.status, TaskStatus::Stopped, "handed over, not failed");
        assert!(host.copied().is_empty());
        let items = host.ctx.queue.snapshot();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].file_name, name);
        assert!(!items[0].owns_source, "the file is read where it lies");
    }
    let saved = std::fs::read_to_string(profile.path().join("Queue.json")).unwrap();
    assert!(saved.contains(&name), "the queue is on disk");

    // The app starts again with the network back: a restored item is due at once
    // (`Revive`), and the queue sends it over tus.
    switch.set(PASS, 0);
    let host = Host::new(profile.path(), Some((&switch.origin, &me)));
    host.ctx.queue.initialize(host.runtime.handle());
    let outcome = host.queue_outcome(60);
    assert!(
        outcome.succeeded,
        "{:?}: {}",
        outcome.reason, outcome.detail
    );
    assert!(!outcome.link.is_empty());
    assert!(host.ctx.queue.snapshot().is_empty());
    let saved = std::fs::read_to_string(profile.path().join("Queue.json")).unwrap();
    assert!(!saved.contains(&name), "and gone from the file once sent");
    assert!(path.is_file(), "the person's own file is never deleted");
    let stored = profile.path().join("stored");
    host.runtime
        .block_on(
            host.ctx
                .api
                .download_to_file(&me, &outcome.raw_url, &stored),
        )
        .unwrap();
    assert_eq!(sha256(&std::fs::read(&stored).unwrap()), sha256(&bytes));
}

#[test]
fn a_large_queued_upload_is_cut_and_resumes_where_it_stopped_after_a_restart() {
    let Some(me) = qa("resumable queue") else {
        return;
    };
    let mut cleanup = Cleanup::new(&me);
    let switch = Switch::start();
    let profile = tempfile::tempdir().unwrap();
    let name = format!("{}.bin", unique("qa-resumed"));
    cleanup.add(&name);
    let path = profile.path().join(&name);
    let bytes = random_bytes(24 * MIB);
    std::fs::write(&path, &bytes).unwrap();

    // Queued while the server is unreachable.
    switch.set(DOWN, 0);
    {
        let mut host = Host::new(profile.path(), Some((&switch.origin, &me)));
        let settings = host.settings();
        upload_manager::upload_file(&mut host.tasks, &path, &settings, &host.ctx);
        host.run_until_finished(1, 60);
        assert_eq!(host.ctx.queue.snapshot().len(), 1);
    }

    // The link comes back but breaks 8 MiB into the transfer.
    switch.set(CUT, 8 * MIB as u64);
    {
        let host = Host::new(profile.path(), Some((&switch.origin, &me)));
        host.ctx.queue.initialize(host.runtime.handle());
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let item = host.ctx.queue.snapshot().into_iter().next();
            if let Some(item) = &item
                && item.reason == QueueReason::Unreachable
                && item.tus_url.is_some()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the cut was never noticed: {item:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let saved = std::fs::read_to_string(profile.path().join("Queue.json")).unwrap();
    assert!(saved.contains("\"TusUrl\""), "the session is kept on disk");

    // Another start, a good link: the transfer resumes from what the server holds.
    switch.set(PASS, 0);
    let host = Host::new(profile.path(), Some((&switch.origin, &me)));
    host.ctx.queue.initialize(host.runtime.handle());
    let outcome = host.queue_outcome(120);
    assert!(
        outcome.succeeded,
        "{:?}: {}",
        outcome.reason, outcome.detail
    );
    let sent = switch.forwarded();
    assert!(
        sent < (bytes.len() as u64) - 4 * MIB as u64,
        "only the rest went over the wire the second time ({sent} bytes)"
    );
    let stored = profile.path().join("stored");
    host.runtime
        .block_on(
            host.ctx
                .api
                .download_to_file(&me, &outcome.raw_url, &stored),
        )
        .unwrap();
    assert_eq!(sha256(&std::fs::read(&stored).unwrap()), sha256(&bytes));
}
