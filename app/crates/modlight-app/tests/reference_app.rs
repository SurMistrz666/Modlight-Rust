//! The app against the reference server at http://127.0.0.1:8420, headless: sign in with
//! a token, load the library, make, rename and delete a folder, upload a file, move it
//! to the trash, restore it, delete it for good, sign out. Every step goes through the
//! same code the window runs; no window opens and nothing touches the clipboard.
//!
//! The token comes from `_wzorzec/qa-accounts.env` (or `MODLIGHT_QA_ENV`). Without it,
//! or without a server, the test says so and passes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use modlight_app::app::{Mode, ModlightApp};
use modlight_app::dialogs::confirm::ConfirmOutcome;
use modlight_app::dialogs::folder_delete::FolderDeleteChoice;
use modlight_app::dialogs::prompt::PromptOutcome;
use modlight_app::signin::SignInAction;
use modlight_app::state::{AppState, Env};
use modlight_core::account::{AccountStore, FileCredentialStore};
use modlight_core::config::AppConfig;

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

/// Step the app until `done` says so, or fail after `seconds`.
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

fn unique(prefix: &str) -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("{prefix} {millis}")
}

#[test]
fn the_app_signs_in_and_manages_the_library_on_the_reference_server() {
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
    let config = AppConfig::load(profile.path());
    let account = AccountStore::load(
        profile.path(),
        Box::new(FileCredentialStore::new(profile.path().join("credentials"))),
    );
    assert!(!account.is_signed_in());
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

    // Sign in with the token, as the manual-token box does.
    {
        let state = &mut harness.state_mut().state;
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
        let state = &harness.state().state;
        assert_eq!(state.account.host(), SERVER);
        assert!(!state.account.display_name().is_empty());
        let stored = std::fs::read_to_string(profile.path().join("credentials")).unwrap();
        assert!(!stored.is_empty(), "the token is kept in the QA profile");
        let uploaders =
            std::fs::read_to_string(profile.path().join("UploadersConfig.json")).unwrap();
        assert!(
            !uploaders.contains(&token),
            "the account file never holds the token"
        );
    }
    wait(&mut harness, "the library", 20, |s| {
        s.space.loaded_once && !s.space.is_busy
    });
    assert!(
        !harness.state().state.space.has_error,
        "{}",
        harness.state().state.space.error_text
    );
    wait(&mut harness, "the storage chip", 20, |s| {
        s.strip.has_storage
    });

    // A folder: made, renamed, deleted.
    let name = unique("QA app folder");
    {
        let state = &mut harness.state_mut().state;
        state.ask_new_folder(None);
        assert!(state.windows.prompt.is_some());
        state.prompt_finished(PromptOutcome::Confirmed(name.clone()));
    }
    wait(&mut harness, "the new folder", 20, |s| {
        s.space.folders.iter().any(|f| f.name == name)
    });
    let id = harness
        .state()
        .state
        .space
        .folders
        .iter()
        .find(|f| f.name == name)
        .unwrap()
        .id
        .clone();
    let renamed = format!("{name} renamed");
    {
        let state = &mut harness.state_mut().state;
        state.ask_rename_folder(&id);
        state.prompt_finished(PromptOutcome::Confirmed(renamed.clone()));
    }
    wait(&mut harness, "the rename", 20, |s| {
        s.space.folders.iter().any(|f| f.name == renamed)
    });

    // A file: uploaded into the folder, found there, trashed, restored, deleted for good.
    let file = profile.path().join("qa-pixel.png");
    {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([90, 87, 233, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        std::fs::write(&file, png).unwrap();
        let state = &mut harness.state_mut().state;
        state.space_navigate_to(Some(id.clone()));
    }
    wait(&mut harness, "the folder listing", 20, |s| {
        s.space.loaded_once
            && !s.space.is_busy
            && s.space.current_folder.as_deref() == Some(id.as_str())
    });
    harness
        .state_mut()
        .state
        .upload_paths_to(vec![file.clone()], Some(id.clone()));
    wait(&mut harness, "the upload", 30, |s| {
        s.space.files.iter().any(|f| f.file.name == "qa-pixel.png")
    });
    let upload_id = harness
        .state()
        .state
        .space
        .files
        .iter()
        .find(|f| f.file.name == "qa-pixel.png")
        .unwrap()
        .file
        .id
        .clone();
    assert!(
        harness
            .state()
            .state
            .toasts
            .iter()
            .any(|t| !t.failed && !t.link.is_empty()),
        "a notice with the link"
    );

    {
        let state = &mut harness.state_mut().state;
        state.ask_trash(std::slice::from_ref(&upload_id), false);
        assert!(
            state.windows.confirm.is_some(),
            "the trash is asked about first"
        );
        state.confirm_finished(ConfirmOutcome::Yes);
    }
    wait(&mut harness, "the trash", 20, |s| {
        s.space.file(&upload_id).is_none()
    });

    {
        let state = &mut harness.state_mut().state;
        state.space_select_view(7);
    }
    wait(&mut harness, "the trash listing", 20, |s| {
        s.space.trashed && s.space.loaded_once && !s.space.is_busy
    });
    assert!(
        harness.state().state.space.file(&upload_id).is_some(),
        "the file is in the trash"
    );
    harness
        .state_mut()
        .state
        .restore_files(std::slice::from_ref(&upload_id));
    wait(&mut harness, "the restore", 20, |s| {
        s.space.file(&upload_id).is_none()
    });
    {
        let state = &mut harness.state_mut().state;
        state.space_navigate_to(Some(id.clone()));
    }
    wait(&mut harness, "the folder again", 20, |s| {
        s.space.file(&upload_id).is_some()
    });
    {
        let state = &mut harness.state_mut().state;
        state.ask_trash(std::slice::from_ref(&upload_id), true);
        state.confirm_finished(ConfirmOutcome::Yes);
    }
    wait(&mut harness, "the permanent delete", 20, |s| {
        s.space.file(&upload_id).is_none()
    });

    {
        let state = &mut harness.state_mut().state;
        state.ask_delete_folder(&id);
        assert!(state.windows.folder_delete.is_some());
        state.folder_delete_finished(FolderDeleteChoice::KeepContents);
    }
    wait(&mut harness, "the folder delete", 20, |s| {
        !s.space.folders.iter().any(|f| f.id == id)
    });
    assert!(
        harness.state().state.space.current_folder.is_none(),
        "the open folder went away, so the view went up"
    );

    // Sign out: the token goes, the host stays.
    harness.state_mut().state.sign_out();
    harness.step();
    let state = &harness.state().state;
    assert!(!state.account.is_signed_in());
    assert_eq!(state.account.host(), SERVER);
    assert!(!state.space.has_account);
    assert!(
        std::fs::read_to_string(profile.path().join("credentials"))
            .map(|s| s.trim().is_empty())
            .unwrap_or(true),
        "the token is gone from the profile"
    );
}

/// Delete every upload whose name is in `names`, in the library and in the trash.
fn delete_uploads_named(state: &AppState, names: &[String]) {
    let Some(me) = state.credential() else { return };
    let api = state.api.clone();
    let names = names.to_vec();
    let _ = std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        runtime.block_on(async {
            for name in &names {
                let mut ids = Vec::new();
                for trashed in [false, true] {
                    if let Ok(page) = api
                        .get_uploads(
                            &me,
                            &modlight_api::UploadQuery {
                                page: 1,
                                per_page: 120,
                                search: Some(name.clone()),
                                trashed,
                                ..Default::default()
                            },
                        )
                        .await
                    {
                        ids.extend(
                            page.items
                                .unwrap_or_default()
                                .into_iter()
                                .filter(|row| row.filename.as_deref() == Some(name.as_str()))
                                .filter_map(|row| row.id),
                        );
                    }
                }
                if !ids.is_empty() {
                    let _ = api.delete_uploads(&me, &ids, "trash").await;
                    let _ = api.delete_uploads(&me, &ids, "permanent").await;
                }
            }
        });
    })
    .join();
}

#[test]
fn the_upload_windows_drive_the_task_flow_on_the_reference_server() {
    use modlight_app::dialogs::upload_confirm::UploadConfirmOutcome;

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
    let config = AppConfig::load(profile.path());
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
        // The sign-in window closes itself after a token sign-in; make sure.
        let state = &mut harness.state_mut().state;
        if state.windows.sign_in.is_some() {
            state.sign_in_action(SignInAction::Close);
        }
    }

    // Eleven files at once: more than UploadManager uploads without asking.
    let stamp = unique("qa-app").replace(' ', "-");
    let names: Vec<String> = (0..11).map(|i| format!("{stamp}-{i:02}.txt")).collect();
    let mut files = Vec::new();
    for name in &names {
        let path = profile.path().join(name);
        std::fs::write(&path, format!("file {name}\n")).unwrap();
        files.push(path);
    }
    let mut uploaded = names.clone();
    {
        let state = &mut harness.state_mut().state;
        state.upload_paths_to(files.clone(), None);
        assert!(
            state.uploads.windows.multi_upload.is_some(),
            "eleven files are asked about first"
        );
        assert!(state.uploads.tasks.tasks.is_empty(), "nothing started yet");
        state.multi_upload_finished(UploadConfirmOutcome {
            go_on: true,
            dont_show_again: false,
        });
    }
    wait(&mut harness, "eleven uploads", 60, |s| {
        names.iter().all(|name| {
            s.upload_tab
                .history
                .iter()
                .any(|row| &row.name == name && row.has_link())
        })
    });

    // The paste window: a title and a language make the file name.
    let paste_title = format!("{stamp}-notes");
    let paste_name;
    {
        let state = &mut harness.state_mut().state;
        state.open_paste_window();
        let window = state
            .uploads
            .windows
            .paste
            .as_mut()
            .expect("the paste window");
        window.model.title = paste_title.clone();
        window.model.content = "# Notes\n\nfrom the paste window\n".into();
        let (bytes, name) = window.model.try_send().expect("a paste to send");
        paste_name = name.clone();
        state.paste_finished(modlight_app::dialogs::paste::PasteOutcome::Send(
            bytes, name,
        ));
        assert!(state.uploads.windows.paste.is_none());
    }
    uploaded.push(paste_name.clone());
    assert_eq!(paste_name, format!("{paste_title}.txt"));
    wait(&mut harness, "the paste", 30, |s| {
        s.upload_tab
            .history
            .iter()
            .any(|row| row.name == paste_name && row.has_link())
    });
    {
        let state = &harness.state().state;
        assert!(
            state.toasts.iter().any(|t| !t.failed && !t.link.is_empty()),
            "finished uploads say so in the corner"
        );
        assert!(
            profile.path().join("History.db").is_file(),
            "the history is written"
        );
        assert!(harness.state().copied.iter().any(|c| c.starts_with("http")));
    }

    // The history store has them, read back as the Upload tab reads it at start.
    {
        let state = &mut harness.state_mut().state;
        state.refresh_upload_history();
    }
    wait(&mut harness, "the history read back", 20, |s| {
        !s.upload_tab.loading_history
            && s.upload_tab
                .history
                .iter()
                .any(|row| row.name == paste_name)
    });

    delete_uploads_named(&harness.state().state, &uploaded);
}
