//! Keeping this installation in step with the account: the skin (`ModlightTheme`) at
//! sign-in and start, pushed when changed here; the settings document
//! (`ModlightSettingsSync`) on its timers.

use std::time::Instant;

use modlight_core::sync::document;
use modlight_core::sync::theme::{ThemeChange, push_skin, sync_skin};
use modlight_core::sync::{SyncApply, apply_report, run_round};

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
                state.sync_running = false;
                state.sync = sync;
                for line in &report.log {
                    log::debug!("{line}");
                }
                if state.credential().is_none_or(|c| c.host != credential.host) {
                    return;
                }
                let editor_open = state.windows.settings.is_some();
                let adopted = matches!(report.apply, SyncApply::Adopt { .. });
                if !apply_report(&mut state.config, &report.apply, editor_open) {
                    return;
                }
                if adopted {
                    // The adopted document can carry the skin, the language and the
                    // library's switches: read them back as the rest of the app sees them.
                    // Hotkeys, watch folders and upload limits follow too.
                    state.apply_settings_document(Some(ctx));
                }
                if !matches!(report.apply, SyncApply::None)
                    && let Err(error) = state.config.save()
                {
                    log::warn!("Modlight: settings could not be saved: {error}");
                }
            })
        });
    }
}
