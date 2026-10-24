//! Settings sync rounds (`modlight_core::sync::run_round`) against a stand-in for
//! `/api/client-settings` that stores the document the way the real server does: in a
//! Postgres `jsonb` column, printed back through JavaScript's `JSON.parse` and
//! `JSON.stringify`. So what comes back is the same content as what was sent, but with
//! every object's members in jsonb's order (shorter names first, then by bytes) and every
//! number as JavaScript prints it (`3.0` comes back as `3`). The reference server on
//! 127.0.0.1:8420 answers `{"app":...,"schema":1}` for a document PUT as
//! `{"schema":1,"app":...}`.
//!
//! Hermetic: the stand-in runs on 127.0.0.1 inside the test.

use std::sync::{Arc, Mutex};

use modlight_api::{Api, Credential};
use modlight_core::config::AppConfig;
use modlight_core::sync::document;
use modlight_core::sync::{SyncAction, SyncState, apply_report, run_round};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const TOKEN: &str = "settings-sync-token-0123456789";
const TAG: &str = "SyncTag";

/// What the stand-in holds: the row of one account.
#[derive(Default)]
struct Row {
    version: i64,
    settings: Map<String, Value>,
    puts: u32,
}

/// jsonb's member order (`lengthCompareJsonbString`: length first, then bytes) and
/// JavaScript's numbers, all the way down.
fn as_stored(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.len().cmp(&b.len()).then(a.as_bytes().cmp(b.as_bytes())));
            let mut out = Map::new();
            for key in keys {
                out.insert(key.clone(), as_stored(&map[key]));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(as_stored).collect()),
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 9_007_199_254_740_992.0 => {
                json!(f as i64)
            }
            _ => value.clone(),
        },
        other => other.clone(),
    }
}

fn shape(row: &Row) -> String {
    json!({
        "client": "desktop",
        "version": row.version,
        "settings": as_stored(&Value::Object(row.settings.clone())),
        "updatedAt": null,
        "limitBytes": 262144,
    })
    .to_string()
}

async fn serve(listener: TcpListener, row: Arc<Mutex<Row>>) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let row = row.clone();
        tokio::spawn(async move {
            let mut data = Vec::new();
            let mut buf = [0u8; 8192];
            let head_end = loop {
                let n = stream.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    return;
                }
                data.extend_from_slice(&buf[..n]);
                if let Some(p) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    break p;
                }
            };
            let head = String::from_utf8_lossy(&data[..head_end]).to_string();
            let header = |name: &str| {
                head.lines().find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
                })
            };
            let length: usize = header("content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            while data.len() < head_end + 4 + length {
                let n = stream.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
            }
            let body = data[head_end + 4..].to_vec();
            let line = head.lines().next().unwrap_or_default().to_string();

            let (status, text) = if header("x-api-key").as_deref() != Some(TOKEN) {
                (401, r#"{"message":"Unauthorized"}"#.to_string())
            } else if line.starts_with("GET /api/client-settings?client=desktop ") {
                (200, shape(&row.lock().unwrap()))
            } else if line.starts_with("PUT /api/client-settings?client=desktop ") {
                let envelope: Value = serde_json::from_slice(&body).unwrap();
                let expected = envelope["version"]
                    .as_i64()
                    .expect("a write states its version");
                let settings = envelope["settings"].as_object().unwrap().clone();
                let mut row = row.lock().unwrap();
                if expected != row.version {
                    (409, shape(&row))
                } else {
                    row.version += 1;
                    row.puts += 1;
                    // What the column keeps is the parsed document, printed back later.
                    row.settings = match as_stored(&Value::Object(settings)) {
                        Value::Object(map) => map,
                        _ => unreachable!(),
                    };
                    (200, shape(&row))
                }
            } else {
                (404, "{}".to_string())
            };
            let reason = match status {
                200 => "OK",
                401 => "Unauthorized",
                409 => "Conflict",
                _ => "Not Found",
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}

/// One installation: its settings and what survives between its rounds.
struct Machine {
    config: AppConfig,
    state: SyncState,
}

impl Machine {
    fn new(values: Value) -> Machine {
        let mut config = AppConfig::in_memory();
        if let Value::Object(map) = values {
            for (k, v) in map {
                config.set(&k, v);
            }
        }
        Machine {
            config,
            state: SyncState::default(),
        }
    }

    /// One round as the app runs it: build, ask, apply (no settings window open).
    async fn round(&mut self, api: &Api, credential: &Credential) -> Option<SyncAction> {
        let local = document::build(&self.config);
        let marker = self.config.cloud_marker();
        let report = run_round(
            api,
            credential,
            TAG,
            &local,
            &marker,
            &mut self.state,
            None,
            "test",
        )
        .await;
        assert!(
            apply_report(&mut self.config, &report.apply, false),
            "nothing blocks an apply here"
        );
        report.plan.map(|plan| plan.action)
    }
}

async fn stand_in() -> (Api, Credential, Arc<Mutex<Row>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let row = Arc::new(Mutex::new(Row::default()));
    tokio::spawn(serve(listener, row.clone()));
    (
        Api::new("0.7.7").unwrap(),
        Credential::new(origin, TOKEN),
        row,
    )
}

fn settings() -> Value {
    json!({
        "Language": "English",
        "ShowTray": true,
        "ModlightLibraryPanel": true,
        "DefaultTaskSettings": {
            "UseDefaultGeneralSettings": false,
            "GeneralSettings": {
                "PlaySoundAfterCapture": true,
                // What the Mac's number boxes write for a value with decimals.
                "ToastWindowDuration": 3.0
            },
            "AfterCaptureJob": "CopyImageToClipboard, SaveImageToFile, UploadImageToHost"
        }
    })
}

/// After a machine's settings reached the server, nothing more should move: the server
/// holds what the machine holds. A digest of the document's bytes never matches the
/// copy the server hands back (other member order, `3` for `3.0`), so every round used
/// to look like "the server moved" (adopt) and the next like "a local change" (push),
/// forever: a write to the account every other round.
#[tokio::test]
async fn a_machine_in_step_with_the_server_sends_nothing_more() {
    let (api, credential, row) = stand_in().await;
    let mut machine = Machine::new(settings());

    assert_eq!(
        machine.round(&api, &credential).await,
        Some(SyncAction::Seed)
    );
    assert_eq!(row.lock().unwrap().version, 1);

    for round in 2..=6 {
        let action = machine.round(&api, &credential).await;
        assert!(
            matches!(action, Some(SyncAction::Nothing | SyncAction::Confirm)),
            "round {round}: {action:?}; the server holds this machine's own document"
        );
    }
    assert_eq!(row.lock().unwrap().puts, 1, "only the seed was written");
    assert_eq!(row.lock().unwrap().version, 1);
}

/// A change made on one machine must reach the other, not be pushed over by it. The
/// second machine adopts the first one's document; when the first then changes
/// something, the second must take the change at its next round. With a digest of the
/// bytes, the machine that had just adopted saw its own (unchanged) settings as an
/// unsent change and pushed them over the other machine's.
#[tokio::test]
async fn a_change_made_elsewhere_is_taken_not_overwritten() {
    let (api, credential, row) = stand_in().await;
    let mut first = Machine::new(settings());
    let mut second = Machine::new(json!({ "Language": "English" }));

    assert_eq!(first.round(&api, &credential).await, Some(SyncAction::Seed));
    // A fresh installation takes the account's settings (the fresh-install guard).
    assert_eq!(
        second.round(&api, &credential).await,
        Some(SyncAction::Adopt)
    );
    assert_eq!(second.config.get("ShowTray"), Some(&json!(true)));

    // The first machine changes its language and sends it.
    first.config.set("Language", json!("Polish"));
    assert_eq!(first.round(&api, &credential).await, Some(SyncAction::Push));
    assert_eq!(
        row.lock().unwrap().settings["app"]["Language"],
        json!("Polish")
    );

    // The second machine has changed nothing since it adopted: it takes the change.
    assert_eq!(
        second.round(&api, &credential).await,
        Some(SyncAction::Adopt)
    );
    assert_eq!(
        row.lock().unwrap().settings["app"]["Language"],
        json!("Polish"),
        "the change made on the first machine is still the account's"
    );
    assert_eq!(second.config.get("Language"), Some(&json!("Polish")));

    // And from then on both are quiet.
    for _ in 0..3 {
        let a = first.round(&api, &credential).await;
        let b = second.round(&api, &credential).await;
        assert!(
            matches!(a, Some(SyncAction::Nothing | SyncAction::Confirm)),
            "{a:?}"
        );
        assert!(
            matches!(b, Some(SyncAction::Nothing | SyncAction::Confirm)),
            "{b:?}"
        );
    }
    assert_eq!(
        row.lock().unwrap().settings["app"]["Language"],
        json!("Polish")
    );
}

/// An installation updated from a build that wrote markers as digests of bytes: it had
/// adopted the account's settings (its marker is the digest of the server's copy as it
/// came back) and changed nothing since, while another machine sent a change. Its first
/// round after the update must take that change, not read its own unchanged settings as
/// a change and push them over it.
#[tokio::test]
async fn a_marker_from_an_earlier_build_does_not_push_over_a_newer_document() {
    use sha2::Digest as _;

    let (api, credential, row) = stand_in().await;
    let mut first = Machine::new(settings());
    assert_eq!(first.round(&api, &credential).await, Some(SyncAction::Seed));

    // The earlier build's state after adopting version 1: the content, and the digest
    // of the server's answer as it came back.
    let answer = as_stored(&Value::Object(row.lock().unwrap().settings.clone()));
    let mut second = Machine::new(Value::Object(answer["app"].as_object().unwrap().clone()));
    second
        .config
        .set_cloud_marker(&modlight_core::config::CloudMarker {
            account: modlight_core::sync::account_key(&credential.host, TAG),
            version: 1,
            digest: hex::encode(sha2::Sha256::digest(serde_json::to_vec(&answer).unwrap())),
        });

    first.config.set("Language", json!("Polish"));
    assert_eq!(first.round(&api, &credential).await, Some(SyncAction::Push));

    assert_eq!(
        second.round(&api, &credential).await,
        Some(SyncAction::Adopt)
    );
    assert_eq!(
        row.lock().unwrap().settings["app"]["Language"],
        json!("Polish")
    );
    assert_eq!(second.config.get("Language"), Some(&json!("Polish")));
}

/// The reference server on 127.0.0.1:8420 (`docs/PORT.md` section 4), with the QA
/// account of `_wzorzec/qa-accounts.env` (or the file `MODLIGHT_QA_ENV` names); skipped
/// without it. Writes nothing: a machine that takes the account's settings and changes
/// nothing must never push them back. With a digest of the bytes it pushed on its third
/// round, because the server's copy never matched its own.
#[tokio::test]
async fn against_the_reference_server_a_quiet_machine_writes_nothing() {
    let Some(token) = qa_token("QA_APP1_TOKEN_WRITE") else {
        eprintln!("reference server test skipped: no qa-accounts.env");
        return;
    };
    let api = Api::new("0.7.7").unwrap();
    let credential = Credential::new("http://127.0.0.1:8420", token);
    let mut machine = Machine::new(json!({}));
    for round in 1..=4 {
        let action = machine.round(&api, &credential).await;
        assert!(
            !matches!(action, Some(SyncAction::Push | SyncAction::Seed)),
            "round {round}: {action:?}; nothing changed on this machine"
        );
    }
}

fn qa_token(key: &str) -> Option<String> {
    let path = std::env::var_os("MODLIGHT_QA_ENV")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
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
    std::fs::read_to_string(path)
        .ok()?
        .lines()
        .find_map(|line| {
            let (k, v) = line.split_once('=')?;
            (k.trim() == key).then(|| v.trim().to_string())
        })
}
