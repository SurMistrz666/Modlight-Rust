//! One round of settings sync (`ModlightSettingsSync.SyncAsync`), and the timing rules
//! of when rounds run.
//!
//! The round talks to the server and decides; it does not touch the live configuration.
//! It returns what should happen to it ([`SyncReport::apply`]), and the caller applies
//! that on the thread that owns the settings, where it can also see whether a settings
//! window is open (an adopt then stands down and the marker does NOT move).

use std::path::Path;
use std::time::Duration;

use modlight_api::{Api, Credential, SettingsAnswer};
use serde_json::{Map, Value};

use super::document::{self, SchemaVerdict};
use super::plan::{self, SyncAction, SyncPlan};
use crate::config::{AppConfig, CloudMarker};

/// How long after a configuration save to send it (restarted by every save).
pub const DEBOUNCE: Duration = Duration::from_secs(20);
/// The periodic verification that catches the OTHER machine's change.
pub const INTERVAL: Duration = Duration::from_secs(15 * 60);
/// The first run after start, deliberately not immediate.
pub const STARTUP_DELAY: Duration = Duration::from_secs(45);

/// What survives between rounds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncState {
    /// The server refused this credential (401/403/413): stop until the next sign-in.
    pub refused: bool,
    /// "Bigger than the server accepts" has been said once already.
    pub oversize_reported: bool,
}

/// What the round wants done to this installation.
#[derive(Clone, Debug, PartialEq)]
pub enum SyncApply {
    /// Nothing to do.
    None,
    /// Write the marker only.
    Remember(CloudMarker),
    /// Put the server's document on this machine, then write the marker. The caller
    /// must skip BOTH when a settings window is open.
    Adopt {
        document: Map<String, Value>,
        marker: CloudMarker,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SyncReport {
    pub plan: Option<SyncPlan>,
    pub apply: SyncApply,
    /// Lines for the debug log (never shown, never translated).
    pub log: Vec<String>,
}

impl SyncReport {
    fn new() -> Self {
        SyncReport {
            plan: None,
            apply: SyncApply::None,
            log: Vec::new(),
        }
    }
}

/// Run one round for the account `tag` on `credential.host`.
///
/// `local` is the document this machine would send right now, `marker` what the server
/// last confirmed. Backups of superseded documents go to `backup_folder`.
#[allow(clippy::too_many_arguments)]
pub async fn run_round(
    api: &Api,
    credential: &Credential,
    tag: &str,
    local: &Map<String, Value>,
    marker: &CloudMarker,
    state: &mut SyncState,
    backup_folder: Option<&Path>,
    reason: &str,
) -> SyncReport {
    let mut report = SyncReport::new();
    if state.refused {
        return report;
    }

    let remote = match fetch(api, credential, None, state, &mut report).await {
        Some(remote) => remote,
        None => return report,
    };

    let account_key = plan::account_key(&credential.host, tag);
    let local_digest = document::digest(Some(local));
    let verdict = document::read_schema(remote.version, &remote.settings);

    if verdict == SchemaVerdict::Unknown {
        report.log.push(
            "Modlight settings sync: the server holds a document this build does not understand. Nothing done."
                .into(),
        );
        return report;
    }

    let decided = if verdict == SchemaVerdict::Empty {
        SyncPlan {
            action: SyncAction::Push,
            version: remote.version,
            reason: "the stored row holds no document",
        }
    } else {
        plan::decide(
            Some(&account_key),
            Some(&marker.account),
            marker.version,
            Some(&marker.digest),
            Some(&local_digest),
            remote.version,
            Some(&document::digest(Some(&remote.settings))),
        )
    };
    report.log.push(format!(
        "Modlight settings sync ({reason}): {decided}, server v{}.",
        remote.version
    ));
    report.plan = Some(decided.clone());

    match decided.action {
        SyncAction::Nothing => {}
        SyncAction::Confirm => {
            report.apply = SyncApply::Remember(CloudMarker {
                account: account_key,
                version: remote.version,
                digest: local_digest,
            });
        }
        SyncAction::Adopt => {
            backup(backup_folder, local, "local");
            // The digest of what the SERVER confirmed, not of what this machine ends up
            // holding: a lossy apply then shows as a local change and is pushed once.
            let digest = document::digest(Some(&remote.settings));
            report.apply = SyncApply::Adopt {
                document: remote.settings.clone(),
                marker: CloudMarker {
                    account: account_key,
                    version: remote.version,
                    digest,
                },
            };
        }
        SyncAction::Seed | SyncAction::Push => {
            if decided.action == SyncAction::Push && verdict == SchemaVerdict::Known {
                backup(backup_folder, &remote.settings, "server");
            }
            let length = document::canonical_length(local) as i64;
            if remote.limit_bytes > 0 && length > remote.limit_bytes {
                if !state.oversize_reported {
                    state.oversize_reported = true;
                    report.log.push(format!(
                        "Modlight settings sync: the configuration is {length} bytes and the server accepts {}. Not sent.",
                        remote.limit_bytes
                    ));
                }
                return report;
            }
            state.oversize_reported = false;

            let body = Value::Object(local.clone());
            let Some(saved) = fetch(
                api,
                credential,
                Some((&body, decided.version)),
                state,
                &mut report,
            )
            .await
            else {
                return report;
            };
            if saved.conflict {
                report.log.push(format!(
                    "Modlight settings sync: another machine wrote first (now v{}). Deciding again next time.",
                    saved.version
                ));
                return report;
            }
            report.apply = SyncApply::Remember(CloudMarker {
                account: account_key,
                version: saved.version,
                digest: local_digest,
            });
        }
    }
    report
}

async fn fetch(
    api: &Api,
    credential: &Credential,
    body: Option<(&Value, i64)>,
    state: &mut SyncState,
    report: &mut SyncReport,
) -> Option<modlight_api::CloudDocument> {
    match api.client_settings(credential, body).await {
        Ok(SettingsAnswer::Document(document)) => Some(document),
        Ok(SettingsAnswer::Refused(413)) => {
            report
                .log
                .push("Modlight settings sync: the server refused the configuration as too large. Stopped.".into());
            state.refused = true;
            None
        }
        Ok(SettingsAnswer::Refused(status)) => {
            report.log.push(format!(
                "Modlight settings sync: the server refused this token ({status}). Stopped until the next sign-in."
            ));
            state.refused = true;
            None
        }
        Ok(SettingsAnswer::Failed(status)) => {
            report.log.push(format!(
                "Modlight settings sync: the server answered {status}."
            ));
            None
        }
        Err(error) => {
            report
                .log
                .push(format!("Modlight settings sync failed: {error}"));
            None
        }
    }
}

/// Keep a copy of a document that is about to be superseded: one file per side,
/// overwritten. Failure is ignored: a backup must not stop the sync it insures.
fn backup(folder: Option<&Path>, document: &Map<String, Value>, side: &str) {
    let Some(folder) = folder else { return };
    let path = folder.join(format!("CloudSettings-superseded-{side}.json"));
    if let Ok(text) = serde_json::to_vec_pretty(&Value::Object(document.clone())) {
        let _ = std::fs::create_dir_all(folder);
        let _ = std::fs::write(path, text);
    }
}

/// Apply a round's result to the configuration. Returns false when an adopt stood down
/// because a settings editor is open (then nothing at all is written).
pub fn apply_report(config: &mut AppConfig, apply: &SyncApply, editor_open: bool) -> bool {
    match apply {
        SyncApply::None => true,
        SyncApply::Remember(marker) => {
            config.set_cloud_marker(marker);
            true
        }
        SyncApply::Adopt { document, marker } => {
            if editor_open {
                return false;
            }
            document::apply_app(config, document.get("app").and_then(Value::as_object));
            if let Some(hotkeys) =
                document::read_hotkeys(document.get("hotkeys").and_then(Value::as_object))
            {
                document::apply_hotkeys(config, hotkeys);
            }
            config.set_cloud_marker(marker);
            true
        }
    }
}

/// When the next round is due, given the scheduling rules.
#[derive(Clone, Debug, Default)]
pub struct SyncSchedule {
    started: Option<std::time::Instant>,
    next_tick: Option<std::time::Instant>,
    debounce_until: Option<std::time::Instant>,
    kick: Option<&'static str>,
}

impl SyncSchedule {
    /// Begin: the first round after [`STARTUP_DELAY`], then every [`INTERVAL`].
    pub fn start(&mut self, now: std::time::Instant) {
        if self.started.is_none() {
            self.started = Some(now);
            self.next_tick = Some(now + STARTUP_DELAY);
        }
    }

    /// A token has just landed: run now, and realign the periodic check.
    pub fn signed_in(&mut self, now: std::time::Instant) {
        self.start(now);
        self.next_tick = Some(now + INTERVAL);
        self.kick = Some("signed in");
    }

    /// The configuration was saved: run once the saving stops.
    pub fn nudge(&mut self, now: std::time::Instant) {
        if self.started.is_some() {
            self.debounce_until = Some(now + DEBOUNCE);
        }
    }

    /// The reason a round should run now, if one should.
    pub fn due(&mut self, now: std::time::Instant) -> Option<&'static str> {
        if let Some(reason) = self.kick.take() {
            return Some(reason);
        }
        if self.debounce_until.is_some_and(|at| now >= at) {
            self.debounce_until = None;
            return Some("config saved");
        }
        if self.next_tick.is_some_and(|at| now >= at) {
            self.next_tick = Some(now + INTERVAL);
            return Some("timer");
        }
        None
    }

    /// How long until something is due (for a repaint timer).
    pub fn wait(&self, now: std::time::Instant) -> Option<Duration> {
        if self.kick.is_some() {
            return Some(Duration::ZERO);
        }
        [self.debounce_until, self.next_tick]
            .into_iter()
            .flatten()
            .map(|at| at.saturating_duration_since(now))
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Instant;

    #[test]
    fn the_schedule_follows_upstreams_timers() {
        let t0 = Instant::now();
        let mut schedule = SyncSchedule::default();
        schedule.nudge(t0);
        assert_eq!(schedule.due(t0 + DEBOUNCE), None, "nothing before start");
        schedule.start(t0);
        assert_eq!(schedule.due(t0), None);
        assert_eq!(schedule.due(t0 + STARTUP_DELAY), Some("timer"));
        assert_eq!(schedule.due(t0 + STARTUP_DELAY), None);
        schedule.nudge(t0 + Duration::from_secs(50));
        assert_eq!(schedule.due(t0 + Duration::from_secs(60)), None);
        assert_eq!(
            schedule.due(t0 + Duration::from_secs(70)),
            Some("config saved")
        );
        schedule.signed_in(t0 + Duration::from_secs(80));
        assert_eq!(
            schedule.due(t0 + Duration::from_secs(80)),
            Some("signed in")
        );
        assert!(schedule.wait(t0 + Duration::from_secs(80)).unwrap() > Duration::from_secs(800));
    }

    #[test]
    fn an_open_editor_blocks_an_adopt_entirely() {
        let mut config = AppConfig::in_memory();
        config.set("Language", json!("English"));
        let apply = SyncApply::Adopt {
            document: json!({"schema": 1, "app": {"Language": "Polish"}})
                .as_object()
                .unwrap()
                .clone(),
            marker: CloudMarker {
                account: "h\nT".into(),
                version: 3,
                digest: "d".into(),
            },
        };
        assert!(!apply_report(&mut config, &apply, true));
        assert_eq!(config.get("Language"), Some(&json!("English")));
        assert_eq!(config.cloud_marker(), CloudMarker::default());
        assert!(apply_report(&mut config, &apply, false));
        assert_eq!(config.get("Language"), Some(&json!("Polish")));
        assert_eq!(config.cloud_marker().version, 3);
    }
}
