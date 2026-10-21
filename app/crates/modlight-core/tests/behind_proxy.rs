//! A 250 MiB file through a proxy that refuses any request body over 100 MB, as
//! Cloudflare's does on the Free and Pro plans: the reason uploads go in 50 MiB chunks.
//!
//! Ignored by default, because it needs such a proxy in front of a Modlight server. Run
//! it by hand with the proxy's origin and a file holding an API token (scope `write`) of
//! an account on that server; the token is never printed:
//!
//! ```sh
//! MODLIGHT_LIMITED_ORIGIN=http://127.0.0.1:8453 \
//! MODLIGHT_LIMITED_TOKEN_FILE=/path/to/token \
//! cargo test -p modlight-core --test behind_proxy -- --ignored --nocapture
//! ```
//!
//! nginx makes such a proxy with `client_max_body_size 100m;` and
//! `proxy_request_buffering off;`. The test first checks that the proxy really refuses
//! the file in one request (the live `POST /upload` and a tus upload with a single
//! `PATCH`, which is what upstream sends), then sends it the app's way twice: through the
//! task flow (the live upload, which goes over tus above one chunk) and over tus as the
//! offline queue sends it, and compares what the server stored with the file. Everything
//! uploaded is deleted for good at the end.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use modlight_api::{
    Api, Credential, LiveOutcome, LiveUpload, TUS_CHUNK_SIZE, TransferOutcome, TransferReason,
    TusClient, TusFile, TusSession, UploadQuery, UploadSource,
};
use modlight_core::tasks::context::TaskContext;
use modlight_core::tasks::host::{TaskAnswer, TaskChannel, TaskMessage, TaskQuestion};
use modlight_core::tasks::info::TaskStatus;
use modlight_core::tasks::manager::TaskManager;
use modlight_core::tasks::settings::{AppUploadSettings, TaskSettings};
use modlight_core::tasks::upload_manager;
use modlight_core::tasks::worker::TaskState;
use modlight_core::upload::queue::{AccountSlot, QueueAccount, UploadQueue};
use modlight_core::upload::store::QueueStore;
use modlight_core::upload::transport::ApiTransport;
use rand::RngCore;
use sha2::Digest;
use tokio_util::sync::CancellationToken;

const MIB: u64 = 1024 * 1024;
const SIZE: u64 = 250 * MIB;
const VERSION: &str = "0.7.7";

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

fn limited() -> (String, Credential) {
    let origin = std::env::var("MODLIGHT_LIMITED_ORIGIN")
        .expect("MODLIGHT_LIMITED_ORIGIN: the proxy's origin");
    let token_file = std::env::var("MODLIGHT_LIMITED_TOKEN_FILE")
        .expect("MODLIGHT_LIMITED_TOKEN_FILE: a file holding an API token");
    let token = std::fs::read_to_string(token_file).expect("the token file");
    let origin = origin.trim_end_matches('/').to_string();
    let credential = Credential::new(&origin, token.trim());
    (origin, credential)
}

/// `SIZE` random bytes in a file, and their SHA-256.
fn random_file(dir: &Path, name: &str) -> (PathBuf, String) {
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).unwrap();
    let mut hasher = sha2::Sha256::new();
    let mut piece = vec![0u8; MIB as usize];
    for _ in 0..SIZE / MIB {
        rand::rng().fill_bytes(&mut piece);
        hasher.update(&piece);
        file.write_all(&piece).unwrap();
    }
    file.sync_all().unwrap();
    (path, hex::encode(hasher.finalize()))
}

fn sha256_of(path: &Path) -> String {
    let mut file = std::fs::File::open(path).unwrap();
    let mut hasher = sha2::Sha256::new();
    std::io::copy(&mut file, &mut hasher).unwrap();
    hex::encode(hasher.finalize())
}

/// Delete every upload called `name` for good; how many there were.
async fn purge(api: &Api, me: &Credential, name: &str) -> usize {
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
    match api.delete_uploads(me, &ids, "permanent").await {
        Ok(gone) => gone.affected as usize,
        Err(_) => 0,
    }
}

/// One file through the task flow, the way the window drives it: the finished task.
fn through_the_task_flow(
    runtime: &tokio::runtime::Runtime,
    profile: &Path,
    origin: &str,
    me: &Credential,
    file: &Path,
) -> TaskState {
    let api = Api::new(VERSION).unwrap();
    let tus = TusClient::new(VERSION).unwrap();
    let slot: AccountSlot = Arc::new(RwLock::new(Some(QueueAccount {
        host: origin.to_string(),
        token: me.token.clone(),
        tag: String::new(),
        display_name: "QA".into(),
    })));
    let queue = UploadQueue::new(
        QueueStore::new(profile),
        slot.clone(),
        Arc::new(ApiTransport::with_tus(api.clone(), tus.clone())),
        Arc::new(|| true),
    );
    let (channel, receiver) = TaskChannel::new(Arc::new(|| {}));
    let ctx = TaskContext {
        api,
        tus,
        account: slot,
        queue,
        channel,
        runtime: runtime.handle().clone(),
        settings: Arc::new(RwLock::new(AppUploadSettings::default())),
        personal_folder: profile.to_path_buf(),
        auto_increment: Arc::new(AtomicI64::new(0)),
        strip_png_color_space: Arc::new(AtomicBool::new(false)),
    };
    let mut tasks = TaskManager::new(None);
    let settings = TaskSettings::default();
    upload_manager::upload_file(&mut tasks, file, &settings, &ctx);
    let deadline = Instant::now() + Duration::from_secs(600);
    loop {
        assert!(Instant::now() < deadline, "the task did not end in 600 s");
        let Ok(message) = receiver.recv_timeout(Duration::from_millis(50)) else {
            continue;
        };
        match message {
            TaskMessage::Event { task, event } => {
                let outcome = tasks.handle_event(task, event, &ctx);
                if outcome.finished
                    && let Some(task) = tasks.find(task)
                {
                    assert!(
                        ctx.queue.snapshot().is_empty(),
                        "nothing was handed to the queue"
                    );
                    return task.snapshot();
                }
            }
            TaskMessage::Ask {
                question, reply, ..
            } => {
                let answer = match question {
                    TaskQuestion::LargeFile { .. } => TaskAnswer::LargeFile {
                        should_continue: true,
                        dont_show_again: false,
                    },
                    other => panic!("unexpected question {other:?}"),
                };
                let _ = reply.send(answer);
            }
            TaskMessage::Effect(_) => {}
        }
    }
}

#[test]
#[ignore = "needs a proxy with a 100 MB body limit in front of a Modlight server"]
fn a_250_mib_file_goes_up_through_a_proxy_that_refuses_bodies_over_100_mb() {
    let (origin, me) = limited();
    let runtime = runtime();
    let api = Api::new(VERSION).unwrap();
    let work = tempfile::tempdir().unwrap();
    let tag = modlight_api::signin::new_state()[..10].to_ascii_lowercase();
    let names: Vec<String> = ["one-post", "one-patch", "task", "queue"]
        .iter()
        .map(|what| format!("ut-cf-{what}-{tag}.bin"))
        .collect();
    let started = Instant::now();
    let (source, expected) = random_file(work.path(), "source.bin");
    eprintln!(
        "{SIZE} bytes of random data written, SHA-256 {expected} ({:?})",
        started.elapsed()
    );

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // 1. What upstream sends is refused: one POST /upload with the whole file...
        let live = runtime.block_on(api.upload(
            &me,
            LiveUpload::new(&names[0], UploadSource::File(source.clone())),
            |_, _| {},
            &CancellationToken::new(),
        ));
        match &live {
            LiveOutcome::Answered(answer) => {
                eprintln!("one POST /upload of the whole file: {}", answer.status);
                assert_eq!(answer.status, 413, "the proxy refuses the one request");
            }
            other => panic!("one POST /upload: {other:?}"),
        }
        // ...and one PATCH with the whole remainder (a chunk as large as the file).
        let single = TusClient::new(VERSION).unwrap().with_chunk_size(u64::MAX);
        let mut session = TusSession::default();
        let result = runtime.block_on(single.send(
            &me,
            &TusFile::file(&source, &names[1], SIZE),
            &mut session,
            |_, _| {},
            &CancellationToken::new(),
        ));
        eprintln!(
            "tus with one PATCH of the whole file: {:?} {:?} {:?}",
            result.outcome, result.reason, result.status
        );
        assert_eq!(
            (result.outcome, result.reason, result.status),
            (
                TransferOutcome::Blocked,
                TransferReason::TooLarge,
                Some(413)
            )
        );
        if let Some(url) = session.url.take() {
            let _ = runtime.block_on(single.abandon(&me, &url));
        }

        // 2. The live upload through the task flow: over tus, in 50 MiB chunks.
        let task_file = work.path().join(&names[2]);
        std::fs::hard_link(&source, &task_file).unwrap();
        let profile = work.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let clock = Instant::now();
        let state = through_the_task_flow(&runtime, &profile, &origin, &me, &task_file);
        assert_eq!(
            state.status,
            TaskStatus::Completed,
            "{:?}",
            state.info.result.errors
        );
        let raw = state.info.result.thumbnail_url.clone();
        eprintln!(
            "task flow: {:?} in {:?}, link {}",
            state.status,
            clock.elapsed(),
            state.info.result.url
        );
        let stored = work.path().join("stored-task");
        runtime
            .block_on(api.download_to_file(&me, &raw, &stored))
            .unwrap();
        let got = sha256_of(&stored);
        eprintln!("task flow: stored SHA-256 {got}");
        assert_eq!(got, expected, "the server holds the very bytes");
        std::fs::remove_file(&stored).unwrap();

        // 3. The offline queue's way: tus with the default chunk, every chunk confirmed.
        let tus = TusClient::new(VERSION).unwrap();
        assert_eq!(tus.chunk_size(), TUS_CHUNK_SIZE);
        let mut session = TusSession::default();
        let mut saved = Vec::new();
        let mut told: Vec<u64> = Vec::new();
        let clock = Instant::now();
        let result = runtime.block_on(tus.send_with_checkpoints(
            &me,
            &TusFile::file(&source, &names[3], SIZE),
            &mut session,
            |sent, _| told.push(sent),
            |session| saved.push(session.offset),
            &CancellationToken::new(),
        ));
        eprintln!(
            "queue's tus: {:?} in {:?}, confirmed offsets {saved:?}",
            result.outcome,
            clock.elapsed()
        );
        assert_eq!(result.outcome, TransferOutcome::Success, "{result:?}");
        assert_eq!(saved, [0, 50 * MIB, 100 * MIB, 150 * MIB, 200 * MIB]);
        assert!(told.windows(2).all(|pair| pair[0] <= pair[1]), "monotonic");
        assert_eq!(told.last(), Some(&SIZE));
        let stored = work.path().join("stored-queue");
        runtime
            .block_on(api.download_to_file(&me, &result.raw_url, &stored))
            .unwrap();
        let got = sha256_of(&stored);
        eprintln!("queue's tus: stored SHA-256 {got}");
        assert_eq!(got, expected);
    }));

    // Whatever happened, nothing stays on the server.
    for name in &names {
        let gone = runtime.block_on(purge(&api, &me, name));
        eprintln!("deleted {gone} upload(s) named {name}");
    }
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
