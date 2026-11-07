//! The whole app, headless (`Mode::Headless`, egui_kittest), with the upload page of the
//! settings open, against a stand-in server on 127.0.0.1: the account's "Remove metadata
//! from files on upload" switch is read from the server when the settings open, a click
//! sends it (`PUT /api/strip-metadata`) and shows what the server stored, a refusal puts
//! the switch back and says why in the server's words, and signed out it is greyed with
//! the reason and sends nothing. No window opens and nothing leaves the machine.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui_kittest::kittest::{NodeT, Queryable};
use modlight_app::app::{Mode, ModlightApp};
use modlight_app::commands::Command;
use modlight_app::state::{AppState, Env};
use modlight_core::account::{AccountStore, StoredAccount};
use modlight_core::config::AppConfig;
use modlight_core::l10n::{port, t};

const TOKEN: &str = "token-of-alex-0123456789";

struct Server {
    origin: String,
    /// "METHOD path body" of every request.
    log: Arc<Mutex<Vec<String>>>,
    /// The switch as the stand-in keeps it.
    value: Arc<Mutex<bool>>,
    /// Refuse the next change with this message.
    refuse: Arc<Mutex<Option<&'static str>>>,
}

fn server() -> Server {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let value = Arc::new(Mutex::new(false));
    let refuse: Arc<Mutex<Option<&'static str>>> = Arc::new(Mutex::new(None));
    let (kept, held, refusing) = (log.clone(), value.clone(), refuse.clone());
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
            let path = parts
                .next()
                .unwrap_or("")
                .split('?')
                .next()
                .unwrap_or("")
                .to_string();
            kept.lock()
                .unwrap()
                .push(format!("{method} {path} {body}").trim_end().to_string());
            let authorised = headers.get("x-api-key").map(String::as_str) == Some(TOKEN);
            let (status, payload): (&str, String) = if !authorised {
                (
                    "401 Unauthorized",
                    r#"{"message":"Token not specified or not found."}"#.into(),
                )
            } else {
                match (method.as_str(), path.as_str()) {
                    ("GET", "/api/strip-metadata") => ("200 OK", format!(r#"{{"enabled":{}}}"#, held.lock().unwrap())),
                    ("PUT", "/api/strip-metadata") => match refusing.lock().unwrap().take() {
                        Some(message) => ("500 Internal Server Error", format!(r#"{{"message":"{message}"}}"#)),
                        None => {
                            let on = body.contains(r#""enabled":true"#);
                            *held.lock().unwrap() = on;
                            ("200 OK", format!(r#"{{"enabled":{on}}}"#))
                        }
                    },
                    ("GET", "/api/me") => (
                        "200 OK",
                        r#"{"tag":"alex","name":"Alex Rivera","email":"alex@example.com","quota":{"used":1000,"max":10000000}}"#
                            .into(),
                    ),
                    ("GET", "/api/folders") => ("200 OK", r#"{"rootValue":"root","folders":[]}"#.into()),
                    ("GET", "/api/uploads") => {
                        ("200 OK", r#"{"page":1,"perPage":60,"total":0,"rootValue":"root","items":[]}"#.into())
                    }
                    ("GET", "/api/encryption") => {
                        ("200 OK", r#"{"state":"off","unlocked":false,"progress":{"done":0,"total":0}}"#.into())
                    }
                    _ => ("404 Not Found", "{}".into()),
                }
            };
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    Server {
        origin,
        log,
        value,
        refuse,
    }
}

type Harness<'a> = egui_kittest::Harness<'a, ModlightApp>;

fn wait(harness: &mut Harness<'_>, what: &str, done: impl Fn(&AppState) -> bool) {
    let started = Instant::now();
    loop {
        harness.step();
        if done(&harness.state().state) {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "timed out waiting for: {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn switch(state: &AppState) -> modlight_app::settings::StripSwitch {
    state
        .windows
        .settings
        .as_ref()
        .map(|w| w.strip.clone())
        .unwrap_or_default()
}

fn puts(server: &Server) -> Vec<String> {
    server
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|l| l.starts_with("PUT"))
        .cloned()
        .collect()
}

#[test]
fn the_switch_follows_the_server_and_goes_back_when_refused() {
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
        AppConfig::load(profile.path()),
        account,
    );
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_eframe(|cc| ModlightApp::new(cc, state, Mode::Headless));
    harness.run_steps(2);

    // Opening the upload page reads the switch.
    harness
        .state_mut()
        .state
        .show_settings(Some("application.upload"));
    wait(&mut harness, "the switch from the server", |s| {
        switch(s).enabled == Some(false)
    });
    harness.run_steps(3);
    let label = t(port::StripMetadataSwitch);

    // On, by a click.
    harness.get_by_label(label).click();
    harness.run_steps(1);
    assert!(
        switch(&harness.state().state).busy || switch(&harness.state().state).enabled == Some(true)
    );
    wait(&mut harness, "the change stored", |s| {
        let sw = switch(s);
        sw.enabled == Some(true) && !sw.busy
    });
    assert!(*server.value.lock().unwrap(), "the server holds it");
    assert_eq!(
        puts(&server),
        vec![r#"PUT /api/strip-metadata {"enabled":true}"#.to_string()]
    );
    assert_eq!(switch(&harness.state().state).error, None);

    // Off, refused by the server: back on, and the server's words on the page.
    *server.refuse.lock().unwrap() = Some("database unavailable");
    harness.run_steps(3);
    harness.get_by_label(label).click();
    wait(&mut harness, "the refusal", |s| {
        switch(s).error.is_some() && !switch(s).busy
    });
    let after = switch(&harness.state().state);
    assert_eq!(after.enabled, Some(true), "the switch goes back");
    let error = after.error.unwrap();
    assert!(error.contains("database unavailable"), "{error}");
    assert!(
        *server.value.lock().unwrap(),
        "nothing changed on the server"
    );
    harness.run_steps(3);
    assert!(
        harness
            .query_by_label_contains("database unavailable")
            .is_some(),
        "the reason is on the page"
    );

    // Signed out: greyed, with the reason, and a click sends nothing.
    harness.state_mut().state.push(Command::SignOut);
    wait(&mut harness, "signed out", |s| !s.account.is_signed_in());
    harness.run_steps(3);
    let sent = puts(&server).len();
    assert!(
        harness
            .query_by_label_contains(t(port::StripMetadataSignedOut))
            .is_some()
    );
    let node = harness.get_by_label(label);
    assert!(
        node.accesskit_node().is_disabled(),
        "greyed while signed out"
    );
    node.click();
    harness.run_steps(5);
    assert_eq!(
        puts(&server).len(),
        sent,
        "nothing is sent while signed out"
    );
}
