//! Captures from the fake desktop through the whole task flow to the reference server at
//! http://127.0.0.1:8420 and back: a full-screen capture and a region capture are
//! started by simulated hotkey presses, saved to the screenshots folder, uploaded,
//! downloaded again and compared byte for byte, and then deleted from the server.
//!
//! Nothing asks macOS for anything: the pictures come from `FakeSource`, the hotkeys
//! from the fake backend, and no window opens. Without `_wzorzec/qa-accounts.env` or a
//! running server the test says so and passes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use modlight_app::app::{Mode, ModlightApp};
use modlight_app::signin::SignInAction;
use modlight_app::state::{AppState, Env};
use modlight_core::account::{AccountStore, FileCredentialStore};
use modlight_core::capture::geometry::Point;
use modlight_core::capture::region::Button;
use modlight_core::config::AppConfig;
use modlight_core::hotkeys::HotkeyType;
use serde_json::json;

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

fn server_up() -> bool {
    std::net::TcpStream::connect_timeout(
        &"127.0.0.1:8420".parse().unwrap(),
        Duration::from_millis(500),
    )
    .is_ok()
}

type Harness<'a> = egui_kittest::Harness<'a, ModlightApp>;

fn wait(harness: &mut Harness<'_>, what: &str, seconds: u64, done: impl Fn(&AppState) -> bool) {
    let started = Instant::now();
    loop {
        harness.step();
        if done(&harness.state().state) {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(seconds),
            "timed out waiting for: {what}"
        );
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// Run `work` on its own runtime (the app's runtime belongs to the harness).
fn block_on<T: Send + 'static>(work: impl std::future::Future<Output = T> + Send + 'static) -> T {
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(work)
    })
    .join()
    .unwrap()
}

/// The uploads whose name starts with `prefix`.
fn uploads_named(state: &AppState, prefix: &str) -> Vec<modlight_api::Upload> {
    let me = state.credential().expect("signed in");
    let api = state.api.clone();
    let prefix = prefix.to_string();
    block_on(async move {
        let page = api
            .get_uploads(
                &me,
                &modlight_api::UploadQuery {
                    page: 1,
                    per_page: 120,
                    search: Some(prefix.clone()),
                    ..Default::default()
                },
            )
            .await
            .expect("the library");
        page.items
            .unwrap_or_default()
            .into_iter()
            .filter(|row| {
                row.filename
                    .as_deref()
                    .is_some_and(|n| n.starts_with(&prefix))
            })
            .collect()
    })
}

fn files_in(folder: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(prefix))
            {
                found.push(path);
            }
        }
    }
    found
}

#[test]
fn captures_from_the_fake_desktop_reach_the_reference_server_and_come_back() {
    let Some(env) = qa_env() else {
        eprintln!("skipped: no _wzorzec/qa-accounts.env");
        return;
    };
    if !server_up() {
        eprintln!("skipped: the reference server is not running");
        return;
    }
    let token = env
        .get("QA_APP1_TOKEN_WRITE")
        .expect("QA_APP1_TOKEN_WRITE in qa-accounts.env")
        .clone();
    let profile = tempfile::tempdir().unwrap();
    let stamp = format!(
        "qacap{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );
    // Save and upload, nothing on the clipboard, no notices, no sounds; a name the
    // test can find again.
    let mut config = AppConfig::load(profile.path());
    config.set(
        "DefaultTaskSettings",
        json!({
            "AfterCaptureJob": "SaveImageToFile, UploadImageToHost",
            "AfterUploadJob": "None",
            "GeneralSettings": {
                "PlaySoundAfterCapture": false,
                "PlaySoundAfterUpload": false,
                "PlaySoundAfterAction": false,
                "ShowToastNotificationAfterTaskCompleted": false
            },
            // A selection over a window is named with the window pattern (`GetFileName`).
            "UploadSettings": {
                "NameFormatPattern": format!("{stamp}-%i"),
                "NameFormatPatternActiveWindow": format!("{stamp}-%pn-%i")
            },
            "CaptureSettings": {"ShowCursor": false, "ScreenshotDelay": 0.0}
        }),
    );
    let account = AccountStore::load(
        profile.path(),
        Box::new(FileCredentialStore::new(profile.path().join("credentials"))),
    );
    let state = AppState::new(
        Env {
            profile: profile.path().to_path_buf(),
            qa: true,
            frozen_now: None,
            machine: "QA Mac".into(),
            system_language: "en".into(),
            system_locale: "en_US".into(),
            reduce_motion: true,
            system_dark: Some(true),
        },
        config,
        account,
    );
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1040.0, 720.0))
        .build_eframe(|cc| ModlightApp::new(cc, state, Mode::Headless));
    {
        let state = &mut harness.state_mut().state;
        state.captures.source = Arc::new(modlight_app::qa_capture::qa_desktop());
        state.show_sign_in();
        let window = state.windows.sign_in.as_mut().unwrap();
        window.host = SERVER.into();
        window.manual_token = token.clone();
        state.sign_in_action(SignInAction::UseToken);
    }
    wait(&mut harness, "the sign-in", 20, |s| {
        s.account.is_signed_in()
    });
    {
        let state = &mut harness.state_mut().state;
        if state.windows.sign_in.is_some() {
            state.sign_in_action(SignInAction::Close);
        }
    }
    let index_of = |state: &AppState, job: HotkeyType| {
        state
            .captures
            .hotkeys
            .hotkeys
            .iter()
            .position(|h| h.job() == job)
            .expect("a default hotkey for the job")
    };

    // 1. Print Screen's hotkey: the whole fake desktop.
    {
        let state = &mut harness.state_mut().state;
        let index = index_of(state, HotkeyType::PrintScreen);
        state.captures.simulated.push(index);
    }
    let first = format!("{stamp}-1");
    wait(&mut harness, "the full-screen upload", 60, |s| {
        !uploads_named(s, &first).is_empty()
    });

    // 2. The region hotkey: the overlay opens on the fake desktop and a selection is
    // dragged out on it, as the pointer would.
    {
        let state = &mut harness.state_mut().state;
        let index = index_of(state, HotkeyType::RectangleRegion);
        state.captures.simulated.push(index);
    }
    wait(&mut harness, "the region overlay", 20, |s| {
        s.captures.overlay.is_some()
    });
    {
        let state = &mut harness.state_mut().state;
        let session = &mut state.captures.overlay.as_mut().unwrap().session;
        session.pointer_pressed(Button::Left, Point::new(100.0, 80.0), 1);
        session.pointer_moved(Point::new(300.0, 230.0));
        session.pointer_released(Button::Left, Point::new(300.0, 230.0));
    }
    // Over the fake Notes window: the window pattern, with the second number.
    let second = format!("{stamp}-Notes-2");
    wait(&mut harness, "the region upload", 60, |s| {
        !uploads_named(s, &second).is_empty()
    });

    // Back: each upload, downloaded again, is the file saved in the screenshots folder.
    let screenshots = profile.path().join("Screenshots");
    let state = &harness.state().state;
    let credential = state.credential().unwrap();
    let mut ids = Vec::new();
    for (name, size) in [(&first, (960u32, 600u32)), (&second, (200, 150))] {
        let rows = uploads_named(state, name);
        assert_eq!(rows.len(), 1, "one upload named {name}");
        let row = rows[0].clone();
        ids.push(row.id.clone().unwrap());
        let saved = files_in(&screenshots, name);
        assert_eq!(saved.len(), 1, "one saved file named {name}");
        let local = std::fs::read(&saved[0]).unwrap();
        let picture = image::load_from_memory(&local).unwrap();
        assert_eq!(
            (picture.width(), picture.height()),
            size,
            "the picture of {name}"
        );
        let back = profile.path().join(format!("{name}-back"));
        let api = state.api.clone();
        let link = row.raw_url.clone().expect("the raw link");
        let me = credential.clone();
        let target = back.clone();
        block_on(async move { api.download_to_file(&me, &link, &target).await })
            .expect("the download");
        assert_eq!(
            std::fs::read(&back).unwrap(),
            local,
            "{name} came back unchanged"
        );
        assert_eq!(row.size as usize, local.len());
    }

    // And gone again.
    let api = state.api.clone();
    let me = credential.clone();
    let removed = ids.clone();
    block_on(async move {
        let _ = api.delete_uploads(&me, &removed, "trash").await;
        api.delete_uploads(&me, &removed, "permanent").await
    })
    .expect("the cleanup");
    assert!(
        uploads_named(state, &stamp).is_empty(),
        "nothing of this run is left on the server"
    );
}

/// Removes what interrupted runs of the test above left behind: uploads of the last
/// hours whose names it gave (`qacap...`) or that a region over the fake Notes window
/// was named before the test set the window pattern (`Notes_` and ten characters).
#[test]
#[ignore = "run by hand after an interrupted run"]
fn remove_leftover_capture_uploads() {
    let Some(env) = qa_env() else { return };
    if !server_up() {
        return;
    }
    let token = env.get("QA_APP1_TOKEN_WRITE").unwrap().clone();
    let api = modlight_api::Api::new(env!("CARGO_PKG_VERSION")).unwrap();
    let me = modlight_api::Credential::new(SERVER, token);
    let removed = block_on(async move {
        let cutoff = chrono::Utc::now() - chrono::Duration::hours(6);
        let mut ids = Vec::new();
        for trashed in [false, true] {
            for search in ["qacap", "Notes_"] {
                let page = api
                    .get_uploads(
                        &me,
                        &modlight_api::UploadQuery {
                            page: 1,
                            per_page: 120,
                            search: Some(search.into()),
                            trashed,
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap();
                for row in page.items.unwrap_or_default() {
                    let name = row.filename.clone().unwrap_or_default();
                    let ours = name.starts_with("qacap")
                        || (name.starts_with("Notes_")
                            && name.len() == "Notes_".len() + 10 + ".png".len());
                    if ours && row.created_at.is_some_and(|at| at > cutoff) {
                        ids.extend(row.id);
                    }
                }
            }
        }
        if !ids.is_empty() {
            let _ = api.delete_uploads(&me, &ids, "trash").await;
            let _ = api.delete_uploads(&me, &ids, "permanent").await;
        }
        ids.len()
    });
    eprintln!("removed {removed} leftover upload(s)");
}
