//! The whole app, headless (`Mode::Headless`, egui_kittest), with an account whose files
//! are encrypted on the server, against a stand-in server on 127.0.0.1: the pictures are
//! refused with 423 and the banner comes up over the files (nobody is signed out); a
//! wrong password is said in the banner; the right one, typed into the banner's field and
//! sent with Enter, unlocks: the grant is kept in the QA profile (remembered by default),
//! the pictures load, and the account menu offers "Lock files", which locks again and
//! revokes the grant on the server. No window opens and nothing leaves the machine.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui_kittest::kittest::Queryable;
use modlight_app::app::{Mode, ModlightApp};
use modlight_app::commands::Command;
use modlight_app::menus::Entry;
use modlight_app::state::{AppState, Env};
use modlight_core::account::{AccountStore, FileCredentialStore, StoredAccount};
use modlight_core::config::AppConfig;
use modlight_core::l10n::{port, t};
use modlight_core::vault::{GRANT_FILE, GrantKeeper};

const TOKEN: &str = "token-of-alex-0123456789";
const GRANT: &str =
    "6f1c2a9e-0000-4000-8000-000000000001.c2VjcmV0LW9mLWFsZXgtMDEyMzQ1Njc4OWFiY2RlZg";
const PASSWORD: &str = "correct horse battery staple";

/// A tiny PNG, so the pictures decode.
fn png() -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(8, 6, image::Rgba([90, 87, 233, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

struct Server {
    origin: String,
    /// "METHOD path" of every request, with " [grant]" when it carried one.
    log: Arc<Mutex<Vec<String>>>,
    revoked: Arc<Mutex<bool>>,
}

/// The account `alex` with two pictures, whose files only [`GRANT`] opens.
fn server() -> Server {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let revoked = Arc::new(Mutex::new(false));
    let (kept, gone, base) = (log.clone(), revoked.clone(), origin.clone());
    let picture = png();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut data = Vec::new();
            let mut buffer = [0u8; 8192];
            let head_end = loop {
                if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    break Some(at + 4);
                }
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break None,
                    Ok(n) => data.extend_from_slice(&buffer[..n]),
                }
            };
            let Some(head_end) = head_end else { continue };
            let head = String::from_utf8_lossy(&data[..head_end]).to_string();
            let mut lines = head.split("\r\n");
            let first = lines.next().unwrap_or("").to_string();
            let headers: HashMap<String, String> = lines
                .filter_map(|l| l.split_once(':'))
                .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
                .collect();
            let length: usize = headers
                .get("content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            while data.len() < head_end + length {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => data.extend_from_slice(&buffer[..n]),
                }
            }
            let body = String::from_utf8_lossy(&data[head_end..]).to_string();
            let mut parts = first.split(' ');
            let method = parts.next().unwrap_or("").to_string();
            let target = parts.next().unwrap_or("").to_string();
            let path = target.split('?').next().unwrap_or("").to_string();
            let grant = headers.get("x-modlight-unlock").cloned();
            let open = grant.as_deref() == Some(GRANT) && !*gone.lock().unwrap();
            kept.lock().unwrap().push(format!(
                "{method} {path}{}",
                if grant.is_some() { " [grant]" } else { "" }
            ));
            let authorised = headers.get("x-api-key").map(String::as_str) == Some(TOKEN);
            let json = |text: String| ("200 OK", "application/json", text.into_bytes());
            let (status, kind, payload): (&str, &str, Vec<u8>) = if !authorised {
                (
                    "401 Unauthorized",
                    "application/json",
                    br#"{"message":"Token not specified or not found."}"#.to_vec(),
                )
            } else {
                match (method.as_str(), path.as_str()) {
                    ("GET", "/api/me") => json(
                        r#"{"tag":"alex","name":"Alex Rivera","email":"alex@example.com","quota":{"used":1000,"max":10000000}}"#.into(),
                    ),
                    ("GET", "/api/folders") => json(r#"{"rootValue":"root","folders":[]}"#.into()),
                    ("GET", "/api/uploads") => json(format!(
                        r#"{{"page":1,"perPage":60,"total":2,"rootValue":"root","items":[{}, {}]}}"#,
                        row(&base, "1"),
                        row(&base, "2")
                    )),
                    ("GET", "/api/encryption") => json(format!(
                        r#"{{"state":"on","unlocked":{open},"progress":{{"done":2,"total":2}}}}"#
                    )),
                    ("POST", "/api/encryption/unlock") => {
                        if body.contains(PASSWORD) {
                            *gone.lock().unwrap() = false;
                            json(format!(
                                r#"{{"grant":"{GRANT}","expiresAt":"2026-11-26T12:00:00Z"}}"#
                            ))
                        } else {
                            (
                                "401 Unauthorized",
                                "application/json",
                                br#"{"error":"wrong_password"}"#.to_vec(),
                            )
                        }
                    }
                    ("POST", "/api/encryption/lock") => {
                        if grant.as_deref() == Some(GRANT) {
                            *gone.lock().unwrap() = true;
                        }
                        ("204 No Content", "text/plain", Vec::new())
                    }
                    ("GET", p) if p.starts_with("/alex/") => {
                        if open {
                            ("200 OK", "image/png", picture.clone())
                        } else {
                            (
                                "423 Locked",
                                "application/json",
                                br#"{"error":"files_locked","owner":"alex"}"#.to_vec(),
                            )
                        }
                    }
                    _ => ("404 Not Found", "application/json", b"{}".to_vec()),
                }
            };
            let mut reply = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n",
                payload.len()
            );
            if status.starts_with("423") {
                reply.push_str("X-Modlight-Locked: 1\r\n");
            }
            reply.push_str("\r\n");
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.write_all(&payload);
        }
    });
    Server {
        origin,
        log,
        revoked,
    }
}

/// One picture of the listing, its links on the stand-in.
fn row(base: &str, id: &str) -> String {
    format!(
        r#"{{"id":"{id}","code":"q{id}","filename":"picture-{id}.png","ext":"png","mimeType":"image/png","kind":"image","size":1234,"width":8,"height":6,"createdAt":"2026-10-27T10:00:00Z","folderId":null,"url":"{base}/alex/q{id}","rawUrl":"{base}/alex/q{id}.png","thumbUrl":"{base}/alex/q{id}.webp?w=480","copyUrl":"{base}/alex/q{id}"}}"#
    )
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
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn labels(entries: &[Entry]) -> Vec<String> {
    entries.iter().map(|e| e.label().to_string()).collect()
}

#[test]
fn the_banner_unlocks_the_files_and_lock_files_locks_them_again() {
    let server = server();
    let profile = tempfile::tempdir().unwrap();
    let account = AccountStore::in_memory(
        StoredAccount {
            host: server.origin.clone(),
            tag: "alex".into(),
            name: "Alex Rivera".into(),
            email: "alex@example.com".into(),
            avatar: String::new(),
            signed_in_at: Some(chrono::Utc::now()),
        },
        TOKEN,
    );
    let mut state = AppState::new(
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
        AppConfig::load(profile.path()),
        account,
    );
    let grant_file = profile.path().join(GRANT_FILE);
    state.vault.keeper = GrantKeeper::new(Box::new(FileCredentialStore::new(grant_file.clone())));
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1280.0, 760.0))
        .build_eframe(|cc| ModlightApp::new(cc, state, Mode::Headless));

    // Locked: the pictures were refused, the banner is up, the account is still signed in.
    wait(&mut harness, "the banner", 20, |s| {
        s.vault_shows_banner() && s.space.files.len() == 2
    });
    {
        let state = &harness.state().state;
        assert!(state.account.is_signed_in(), "a 423 never signs out");
        assert!(!state.space.has_error, "{}", state.space.error_text);
        assert!(state.media.thumbs.is_empty());
        assert!(
            labels(&modlight_app::shell::account_menu(state))
                .contains(&t(port::VaultUnlockFiles).to_string())
        );
    }

    // A wrong password, typed into the banner and sent with Enter.
    harness.run_steps(2);
    harness
        .get_by_role(egui::accesskit::Role::PasswordInput)
        .focus();
    harness.run_steps(2);
    harness
        .get_by_role(egui::accesskit::Role::PasswordInput)
        .type_text("not the password");
    harness.run_steps(2);
    assert_eq!(harness.state().state.vault.password, "not the password");
    harness.key_press(egui::Key::Enter);
    wait(&mut harness, "the refusal", 20, |s| {
        s.vault.problem == Some(modlight_app::vault::Problem::WrongPassword)
    });
    {
        let state = &harness.state().state;
        assert!(
            state.vault.password.is_empty(),
            "a refused password is cleared"
        );
        assert!(state.vault_shows_banner());
        assert!(state.account.is_signed_in());
    }

    // The right one.
    harness.run_steps(2);
    harness
        .get_by_role(egui::accesskit::Role::PasswordInput)
        .focus();
    harness.run_steps(2);
    harness
        .get_by_role(egui::accesskit::Role::PasswordInput)
        .type_text(PASSWORD);
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    wait(&mut harness, "the pictures", 20, |s| {
        !s.vault_shows_banner() && s.media.thumbs.len() == 2
    });
    {
        let state = &harness.state().state;
        assert!(state.vault.is_unlocked());
        assert!(state.vault.password.is_empty());
        let kept = std::fs::read_to_string(&grant_file).expect("remembered in the QA profile");
        assert!(kept.contains("\"Tag\":\"alex\""));
        assert!(
            labels(&modlight_app::shell::account_menu(state))
                .contains(&t(port::VaultLockFiles).to_string())
        );
    }
    assert!(
        server
            .log
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.starts_with("GET /alex/") && l.ends_with("[grant]")),
        "the pictures were asked for with the grant"
    );

    // "Lock files" from the account menu.
    harness.state_mut().state.push(Command::LockFiles);
    wait(&mut harness, "the lock", 20, |s| s.vault_shows_banner());
    {
        let state = &harness.state().state;
        assert!(
            state.media.thumbs.is_empty(),
            "the pictures leave the window"
        );
        assert!(!grant_file.exists(), "forgotten on this computer");
        assert!(state.account.is_signed_in());
    }
    let started = Instant::now();
    while !*server.revoked.lock().unwrap() && started.elapsed() < Duration::from_secs(10) {
        harness.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(*server.revoked.lock().unwrap(), "revoked on the server");
}
