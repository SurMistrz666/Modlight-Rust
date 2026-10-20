//! The Download tab's flow, headless, with a FAKE yt-dlp: a shell script this test
//! writes, which records its argument vector, prints upstream-shaped progress and marker
//! lines and writes a file. The file then goes up through the task flow to the
//! reference server on 127.0.0.1:8420 and is deleted afterwards.
//!
//! The upload half needs `_wzorzec/qa-accounts.env` and the server; without either it
//! says so and passes. Nothing here runs a real yt-dlp or touches the internet.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use modlight_api::{Api, Credential, UploadQuery};
use modlight_core::config::AppConfig;
use modlight_core::download::cookies::{NetscapeCookie, YouTubeCookies};
use modlight_core::download::item::{Delivery, DownloadQueue, State};
use modlight_core::download::run::{self, RunEvent, RunSpec};
use modlight_core::download::{
    AudioFormat, Destination, DownloadFormat, DownloadLink, MediaKind, Quality,
};
use modlight_core::tasks::context::TaskContext;
use modlight_core::tasks::host::{TaskAnswer, TaskChannel, TaskMessage};
use modlight_core::tasks::info::TaskStatus;
use modlight_core::tasks::manager::TaskManager;
use modlight_core::tasks::settings::{AppUploadSettings, TaskSettings};
use modlight_core::tasks::upload_manager;
use modlight_core::tasks::worker::TaskState;
use modlight_core::upload::queue::{AccountSlot, QueueAccount, UploadQueue};
use modlight_core::upload::store::QueueStore;
use modlight_core::upload::transport::ApiTransport;
use tokio_util::sync::CancellationToken;

const SERVER: &str = "http://127.0.0.1:8420";

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

fn qa(what: &str) -> Option<Credential> {
    let Some(env) = qa_env() else {
        eprintln!("{what}: upload half skipped, no _wzorzec/qa-accounts.env");
        return None;
    };
    if std::net::TcpStream::connect_timeout(
        &"127.0.0.1:8420".parse().unwrap(),
        Duration::from_millis(500),
    )
    .is_err()
    {
        eprintln!("{what}: upload half skipped, nothing answers on {SERVER}");
        return None;
    }
    Some(Credential::new(
        SERVER,
        env.get("QA_APP1_TOKEN_WRITE")
            .expect("QA_APP1_TOKEN_WRITE in qa-accounts.env")
            .clone(),
    ))
}

fn unique(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}-{nanos:x}")
}

/// Write the fake yt-dlp. It copies `source` to `<-P folder>/<name>`, after printing
/// what the real one prints with the options the app passes.
fn fake_yt_dlp(dir: &Path, source: &Path, name: &str, fail: Option<&str>) -> (PathBuf, PathBuf) {
    let script = dir.join("yt-dlp");
    let log = dir.join("arguments.log");
    let failure = match fail {
        Some(line) => format!("echo '{line}' >&2\nexit 1\n"),
        None => String::new(),
    };
    let body = format!(
        r#"#!/bin/sh
: > '{log}'
for a in "$@"; do printf '%s\n' "$a" >> '{log}'; done
folder=""
previous=""
for a in "$@"; do
  if [ "$previous" = "-P" ]; then folder="$a"; fi
  previous="$a"
done
echo "[youtube] Extracting URL"
echo "@MT|Fake clip"
echo "@MP|$folder/{stem}.f137.mp4"
echo "@MR|720|1280"
{failure}echo "@ML|downloading|1000|4000|NA|2000.5|3"
echo "@ML|downloading|4000|4000|NA|2500|0"
echo "@ML|finished|4000|4000|NA|NA|NA"
echo "WARNING: this is only a test" >&2
cp '{source}' "$folder/{name}"
echo "@MF|$folder/{name}"
exit 0
"#,
        log = log.display(),
        source = source.display(),
        stem = name.trim_end_matches(".mp4"),
    );
    std::fs::write(&script, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (script, log)
}

fn spec(tool: &Path, link: &str, folder: &Path, cookies: Option<PathBuf>) -> RunSpec {
    RunSpec {
        tool: tool.to_path_buf(),
        link: DownloadLink::parse(link).unwrap(),
        folder: folder.to_path_buf(),
        format: DownloadFormat::new(MediaKind::Video, Quality::H1080, AudioFormat::M4a),
        ffmpeg: Some(PathBuf::from("/opt/fake/ffmpeg")),
        quickjs: None,
        cookies,
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn a_fake_yt_dlp_run_is_read_queued_and_uploaded() {
    let rt = runtime();
    let work = tempfile::tempdir().unwrap();
    let profile = work.path().join("profile");
    let downloads = work.path().join("Downloads");
    std::fs::create_dir_all(&profile).unwrap();
    let name = format!("{}.mp4", unique("Fake clip [qa-dl"));
    let name = name.replace(".mp4", "].mp4");
    let source = work.path().join("source.mp4");
    let bytes: Vec<u8> = (0..64 * 1024).map(|i| (i % 251) as u8).collect();
    std::fs::write(&source, &bytes).unwrap();
    let (tool, log) = fake_yt_dlp(work.path(), &source, &name, None);

    // Signed in to YouTube: the jar goes to a YouTube link, and only to it.
    let jar = YouTubeCookies::new(&profile);
    jar.save(&[NetscapeCookie {
        domain: ".youtube.com".into(),
        path: "/".into(),
        name: "SID".into(),
        value: "fixture".into(),
        secure: true,
        expires_unix: 0,
    }])
    .unwrap();
    let link = DownloadLink::parse("https://www.youtube.com/watch?v=-abc").unwrap();
    assert!(jar.file_for(&link).is_some());

    // The queue and the row, as the tab drives them.
    let mut queue = DownloadQueue::default();
    let id = queue.enqueue(
        link.clone(),
        downloads.clone(),
        false,
        Destination::Both,
        DownloadFormat::new(MediaKind::Video, Quality::H1080, AudioFormat::M4a),
        "1080p",
    );
    assert_eq!(queue.next_to_start(), Some(id));
    let cancel = queue.get_mut(id).unwrap().start().unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let outcome = rt.block_on(run::run(
        &spec(&tool, &link.url, &downloads, jar.file_for(&link)),
        &profile,
        Arc::new(move |e| sink.lock().unwrap().push(e)),
        &cancel,
    ));
    assert!(outcome.success, "{outcome:?}");
    assert_eq!(outcome.title.as_deref(), Some("Fake clip"));
    let file = outcome.file_path.clone().unwrap();
    assert_eq!(file, downloads.join(&name));
    assert_eq!(std::fs::read(&file).unwrap(), bytes);

    // What yt-dlp was given: a vector, `--` right before the URL, the jar, ffmpeg.
    let arguments: Vec<String> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(arguments[arguments.len() - 2], "--");
    assert_eq!(arguments[arguments.len() - 1], link.url);
    let at = |flag: &str| arguments.iter().position(|a| a == flag).unwrap();
    assert_eq!(
        PathBuf::from(&arguments[at("--cookies") + 1]),
        jar.file_path()
    );
    assert_eq!(arguments[at("--ffmpeg-location") + 1], "/opt/fake/ffmpeg");
    assert!(!arguments.iter().any(|a| a.starts_with("--exec")));

    // What the row heard, applied the way the tab applies it.
    {
        let item = queue.get_mut(id).unwrap();
        for event in events.lock().unwrap().iter() {
            match event {
                RunEvent::Progress(p) => item.apply(p, false),
                RunEvent::Title(t) => item.named(t),
                RunEvent::Format(f) => item.chose(f),
            }
        }
        assert_eq!(item.format_text, "720p");
        assert!(!item.note.is_empty(), "1080p was asked, 720p came");
        assert_eq!(
            item.state,
            State::Working,
            "the last progress line was a finish"
        );
    }
    let delivery = queue.get_mut(id).unwrap().finish(&outcome, false);
    assert_eq!(delivery, Some(Delivery::Upload(file.clone())));
    assert_eq!(queue.get(id).unwrap().state, State::Completed);

    // The upload through the task flow, when the reference server is there.
    let Some(me) = qa("download flow") else {
        return;
    };
    let uploaded = upload_and_delete(&profile, &me, &file);
    assert!(uploaded.starts_with("http"), "a link: {uploaded}");
}

#[test]
fn a_tiktok_link_never_gets_the_jar_and_a_failure_is_worded() {
    let rt = runtime();
    let work = tempfile::tempdir().unwrap();
    let profile = work.path().join("profile");
    let downloads = work.path().join("Downloads");
    let source = work.path().join("source.mp4");
    std::fs::write(&source, b"x").unwrap();
    let (tool, log) = fake_yt_dlp(
        work.path(),
        &source,
        "never.mp4",
        Some(
            "ERROR: [youtube] x: Private video. Sign in if you have been granted access to this video",
        ),
    );
    let jar = YouTubeCookies::new(&profile);
    jar.save(&[NetscapeCookie {
        domain: ".youtube.com".into(),
        path: "/".into(),
        name: "SID".into(),
        value: "fixture".into(),
        secure: true,
        expires_unix: 0,
    }])
    .unwrap();
    let tiktok = DownloadLink::parse("https://www.tiktok.com/@a/video/1").unwrap();
    assert_eq!(jar.file_for(&tiktok), None);
    let outcome = rt.block_on(run::run(
        &spec(&tool, &tiktok.url, &downloads, jar.file_for(&tiktok)),
        &profile,
        Arc::new(|_| {}),
        &CancellationToken::new(),
    ));
    assert!(!outcome.success);
    assert!(
        !outcome.needs_sign_in,
        "a TikTok link is not offered a YouTube sign-in"
    );
    assert!(!outcome.error.is_empty());
    let arguments = std::fs::read_to_string(&log).unwrap();
    assert!(!arguments.lines().any(|a| a == "--cookies"));

    let youtube = DownloadLink::parse("https://youtu.be/x").unwrap();
    let outcome = rt.block_on(run::run(
        &spec(&tool, &youtube.url, &downloads, jar.file_for(&youtube)),
        &profile,
        Arc::new(|_| {}),
        &CancellationToken::new(),
    ));
    assert!(outcome.needs_sign_in);
}

#[test]
fn a_cancelled_run_stops_at_once() {
    let rt = runtime();
    let work = tempfile::tempdir().unwrap();
    let tool = work.path().join("yt-dlp");
    std::fs::write(
        &tool,
        "#!/bin/sh\necho '@ML|downloading|1|100|NA|1|99'\nsleep 30\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    let started = Instant::now();
    let outcome = rt.block_on(async {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            stopper.cancel();
        });
        run::run(
            &spec(&tool, "https://example.com/v", &work.path().join("d"), None),
            work.path(),
            Arc::new(|_| {}),
            &cancel,
        )
        .await
    });
    assert!(outcome.canceled);
    assert!(started.elapsed() < Duration::from_secs(10));
}

/// Upload `file` through `UploadManager.UploadFile` as the tab does, wait for it, read
/// the link, and delete what went up.
fn upload_and_delete(profile: &Path, me: &Credential, file: &Path) -> String {
    let runtime = runtime();
    let api = Api::new("0.7.7").unwrap();
    let slot: AccountSlot = Arc::new(RwLock::new(Some(QueueAccount {
        host: SERVER.to_string(),
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
    let (channel, receiver): (TaskChannel, Receiver<TaskMessage>) =
        TaskChannel::new(Arc::new(|| {}));
    let ctx = TaskContext {
        api: api.clone(),
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
    let settings = TaskSettings::defaults_from(&AppConfig::load(profile));
    upload_manager::upload_file(&mut tasks, file, &settings, &ctx);
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut finished: Option<TaskState> = None;
    while finished.is_none() {
        assert!(Instant::now() < deadline, "the upload did not end in 60 s");
        let Ok(message) = receiver.recv_timeout(Duration::from_millis(50)) else {
            continue;
        };
        match message {
            TaskMessage::Event { task, event } => {
                let outcome = tasks.handle_event(task, event, &ctx);
                if outcome.finished {
                    finished = tasks.find(task).map(|t| t.snapshot());
                }
            }
            TaskMessage::Ask { reply, .. } => {
                let _ = reply.send(TaskAnswer::SignIn(false));
            }
            TaskMessage::Effect(_) => {}
        }
    }
    let state = finished.unwrap();
    let name = file.file_name().unwrap().to_string_lossy().to_string();
    // Delete whatever went up, by its name, before judging the outcome.
    runtime.block_on(async {
        for trashed in [false, true] {
            let Ok(page) = api
                .get_uploads(
                    me,
                    &UploadQuery {
                        page: 1,
                        per_page: 120,
                        search: Some(name.clone()),
                        trashed,
                        ..Default::default()
                    },
                )
                .await
            else {
                continue;
            };
            let ids: Vec<String> = page
                .items
                .unwrap_or_default()
                .into_iter()
                .filter(|row| row.filename.as_deref() == Some(name.as_str()))
                .filter_map(|row| row.id)
                .collect();
            if !ids.is_empty() {
                let _ = api.delete_uploads(me, &ids, "trash").await;
                let _ = api.delete_uploads(me, &ids, "permanent").await;
            }
        }
    });
    assert_eq!(
        state.status,
        TaskStatus::Completed,
        "{:?}",
        state.info.result.errors
    );
    state.info.result.url.clone()
}
