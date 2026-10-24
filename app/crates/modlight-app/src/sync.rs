//! Keeping this installation in step with the account: the skin (`ModlightTheme`) at
//! sign-in and start, pushed when changed here; the settings document
//! (`ModlightSettingsSync`) on its timers.

use std::time::Instant;

use modlight_api::Credential;
use modlight_core::sync::document;
use modlight_core::sync::theme::{ThemeChange, push_skin, sync_skin};
use modlight_core::sync::{SyncApply, SyncReport, SyncState, apply_report, run_round};

use crate::state::AppState;

impl AppState {
    /// Bring the skin and the account's into agreement, once (`ModlightTheme.SyncAsync`).
    pub fn sync_skin(&mut self) {
        let Some(credential) = self.credential() else {
            return;
        };
        let api = self.api.clone();
        let options = self.options.clone();
        self.rt.spawn(async move {
            let change = sync_skin(&api, &credential, &options).await;
            Box::new(move |state: &mut AppState, ctx: &egui::Context| {
                state.apply_theme_change(&change, ctx)
            })
        });
    }

    /// The skin was picked here: tell the server (`ModlightTheme.PushAsync`).
    pub fn push_skin(&mut self) {
        let Some(credential) = self.credential() else {
            return;
        };
        let api = self.api.clone();
        let options = self.options.clone();
        self.rt.spawn(async move {
            let change = push_skin(&api, &credential, &options).await;
            Box::new(move |state: &mut AppState, ctx: &egui::Context| {
                state.apply_theme_change(&change, ctx)
            })
        });
    }

    fn apply_theme_change(&mut self, change: &ThemeChange, ctx: &egui::Context) {
        if change.is_empty() {
            return;
        }
        change.apply(&mut self.options);
        self.refresh_theme(Some(ctx));
        self.save_config();
    }

    /// Per frame: run a settings-sync round when one is due (`ModlightSettingsSync`).
    pub fn tick_settings_sync(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if let Some(wait) = self.schedule.wait(now) {
            ctx.request_repaint_after(wait.max(std::time::Duration::from_millis(50)));
        }
        if self.sync_running {
            return;
        }
        let Some(reason) = self.schedule.due(now) else {
            return;
        };
        let Some(credential) = self.credential() else {
            return;
        };
        if self.sync.refused {
            return;
        }
        let tag = self.account.account().tag.clone();
        let local = document::build(&self.config);
        let marker = self.config.cloud_marker();
        let mut sync = self.sync.clone();
        let backup = self.config.backup_folder();
        let api = self.api.clone();
        self.sync_running = true;
        self.rt.spawn(async move {
            let report = run_round(
                &api,
                &credential,
                &tag,
                &local,
                &marker,
                &mut sync,
                backup.as_deref(),
                reason,
            )
            .await;
            Box::new(move |state: &mut AppState, ctx: &egui::Context| {
                state.settings_round_done(&credential, sync, report, ctx)
            })
        });
    }

    /// A round finished: keep what it learned and apply what it decided, on this thread.
    pub(crate) fn settings_round_done(
        &mut self,
        credential: &Credential,
        sync: SyncState,
        report: SyncReport,
        ctx: &egui::Context,
    ) {
        self.sync_running = false;
        for line in &report.log {
            log::debug!("{line}");
        }
        // Everything the round learned is about the credential it talked with. Signed
        // out since, or signed in with another token (another account, even on the same
        // server): its refusal must not stop the new account's sync, and its document
        // and marker belong to the account that went away. The sign-in asked for a round
        // of its own, which runs next.
        if self.credential().as_ref() != Some(credential) {
            return;
        }
        self.sync = sync;
        let editor_open = self.windows.settings.is_some();
        let adopted = matches!(report.apply, SyncApply::Adopt { .. });
        if !apply_report(&mut self.config, &report.apply, editor_open) {
            return;
        }
        if adopted {
            // The adopted document can carry the skin, the language and the library's
            // switches: read them back as the rest of the app sees them. Hotkeys, watch
            // folders and upload limits follow too.
            self.apply_settings_document(Some(ctx));
        }
        if !matches!(report.apply, SyncApply::None)
            && let Err(error) = self.config.save()
        {
            log::warn!("Modlight: settings could not be saved: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use modlight_core::account::{AccountStore, StoredAccount};
    use modlight_core::config::CloudMarker;
    use modlight_core::sync::{SyncAction, SyncPlan};
    use serde_json::json;

    use super::*;
    use crate::qa::{Look, state};

    fn adopt_report(language: &str) -> SyncReport {
        SyncReport {
            plan: Some(SyncPlan {
                action: SyncAction::Adopt,
                version: 7,
                reason: "test",
            }),
            apply: SyncApply::Adopt {
                document: json!({"schema": 1, "app": {"Language": language}})
                    .as_object()
                    .unwrap()
                    .clone(),
                marker: CloudMarker {
                    account: "https://files.example.com\nalex".into(),
                    version: 7,
                    digest: "d".into(),
                },
            },
            log: Vec::new(),
        }
    }

    /// A round for one account that finishes after somebody else signed in, on the same
    /// server: it must neither stop the new account's sync with its refusal nor put the
    /// old account's settings and marker on this machine.
    #[test]
    fn a_round_for_an_account_that_went_away_changes_nothing() {
        let ctx = egui::Context::default();
        let mut app = state(&Look::default(), true);
        app.config.set("Language", json!("English"));
        let old = app.credential().unwrap();

        // Another account on the same server signs in while the round is out.
        app.account = AccountStore::in_memory(
            StoredAccount {
                host: old.host.clone(),
                tag: "sam".into(),
                ..StoredAccount::default()
            },
            "another-token",
        );
        app.sync.refused = false;
        app.sync_running = true;

        let refused = SyncState {
            refused: true,
            oversize_reported: false,
        };
        app.settings_round_done(&old, refused, adopt_report("Polish"), &ctx);
        assert!(!app.sync_running);
        assert!(
            !app.sync.refused,
            "the old token's refusal does not stop the new account's sync"
        );
        assert_eq!(app.config.get("Language"), Some(&json!("English")));
        assert_eq!(app.config.cloud_marker(), CloudMarker::default());

        // The same round for the account still signed in applies.
        let current = app.credential().unwrap();
        app.settings_round_done(&current, SyncState::default(), adopt_report("Polish"), &ctx);
        assert_eq!(app.config.get("Language"), Some(&json!("Polish")));
        assert_eq!(app.config.cloud_marker().version, 7);
    }
}
