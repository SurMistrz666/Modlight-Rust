//! The storage chip and the account button at the right end of the tab strip
//! (`AccountStripViewModel.cs`), refreshed from `/api/me` at most every 20 seconds.

use std::time::{Duration, Instant};

use modlight_api::{AccountInfo, ApiError, Credential};
use modlight_core::format::cloud_size;
use modlight_core::l10n::{Arg, format, s, t};
use modlight_core::messages;

use crate::state::AppState;

/// How long a storage reading stays fresh.
pub const FRESH_FOR: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccountStrip {
    /// Whose reading this is (host + token); a different account clears it.
    pub identity: String,
    pub read_at: Option<Instant>,
    pub in_flight: Option<u64>,
    /// Whose reading the request in flight is asking for (host + token).
    pub asking: String,
    pub generation: u64,
    pub has_storage: bool,
    pub has_storage_bar: bool,
    pub has_storage_problem: bool,
    pub storage_text: String,
    pub storage_tooltip: String,
    pub storage_percent: f32,
}

impl AccountStrip {
    pub fn clear_storage(&mut self) {
        self.identity.clear();
        self.read_at = None;
        self.storage_text.clear();
        self.storage_tooltip.clear();
        self.storage_percent = 0.0;
        self.has_storage_bar = false;
        self.has_storage_problem = false;
        self.has_storage = false;
    }

    /// A fresh `/api/me` answer.
    pub fn show(&mut self, identity: String, info: &AccountInfo) {
        self.identity = identity;
        self.read_at = Some(Instant::now());
        let Some(quota) = &info.quota else {
            self.clear_storage();
            return;
        };
        let used = cloud_size(quota.used);
        match quota.max.filter(|max| !quota.unlimited && *max > 0) {
            None => {
                self.storage_text = format(
                    t(s::MainWindow_StorageUnlimited),
                    &[Arg::from(used.as_str())],
                );
                self.storage_tooltip = format(
                    t(s::SignInWindow_QuotaUnlimited),
                    &[Arg::from(used.as_str())],
                );
                self.storage_percent = 0.0;
                self.has_storage_bar = false;
            }
            Some(max) => {
                let remaining = quota.remaining.unwrap_or((max - quota.used).max(0));
                self.storage_text = format(
                    t(s::MainWindow_StorageUsed),
                    &[Arg::from(used.as_str()), Arg::from(cloud_size(max))],
                );
                self.storage_tooltip = format(
                    t(s::SignInWindow_QuotaUsed),
                    &[
                        Arg::from(used.as_str()),
                        Arg::from(cloud_size(max)),
                        Arg::from(cloud_size(remaining)),
                    ],
                );
                self.storage_percent = (quota.used as f32 * 100.0 / max as f32).min(100.0);
                self.has_storage_bar = true;
            }
        }
        self.has_storage_problem = false;
        self.has_storage = true;
    }

    /// The answer could not be had: say so instead of a number.
    pub fn show_problem(&mut self, identity: String, reason: String) {
        self.identity = identity;
        self.read_at = None;
        self.storage_text = t(s::MainWindow_StorageUnavailable).to_string();
        self.storage_tooltip = reason;
        self.storage_percent = 0.0;
        self.has_storage_bar = false;
        self.has_storage_problem = true;
        self.has_storage = true;
    }
}

fn identity_of(credential: &Credential) -> String {
    format!("{}\n{}", credential.host, credential.token)
}

impl AppState {
    /// `AccountStrip.Refresh`: re-read `/api/me` unless the reading is fresh.
    pub fn refresh_account_strip(&mut self) {
        let Some(credential) = self.credential() else {
            self.strip.in_flight = None;
            self.strip.clear_storage();
            self.media.avatar.clear();
            return;
        };
        let identity = identity_of(&credential);
        let changed = identity != self.strip.identity;
        if changed {
            self.strip.clear_storage();
        }
        if self.strip.in_flight.is_some() && self.strip.asking != identity {
            // The request still out is for the previous account: its answer is dropped
            // (`AccountStripViewModel.Owns`) and this one's is asked for now.
            self.strip.in_flight = None;
        }
        if self.strip.in_flight.is_some() {
            return;
        }
        if !changed
            && self.strip.has_storage
            && self
                .strip
                .read_at
                .is_some_and(|at| at.elapsed() < FRESH_FOR)
        {
            return;
        }
        self.strip.generation += 1;
        let generation = self.strip.generation;
        self.strip.in_flight = Some(generation);
        self.strip.asking = identity.clone();
        let api = self.api.clone();
        self.rt.spawn(async move {
            let answer = api.get_account(&credential).await;
            Box::new(move |state: &mut AppState, ctx: &egui::Context| {
                if state.strip.in_flight != Some(generation) {
                    return;
                }
                state.strip.in_flight = None;
                match answer {
                    Ok(Some(info)) => {
                        state.note_account(&credential, &info);
                        state.strip.show(identity, &info);
                        state.request_avatar(ctx);
                    }
                    Ok(None) => {
                        state.note_token_refused(&credential);
                        state
                            .strip
                            .show_problem(identity, t(s::ModlightSignIn_TokenRejected).to_string());
                    }
                    Err(error) => {
                        let reason = match &error {
                            ApiError::Unauthorized => {
                                t(s::ModlightSignIn_TokenRejected).to_string()
                            }
                            other => messages::unreachable(&credential.host, &other.to_string()),
                        };
                        state.strip.show_problem(identity, reason);
                    }
                }
            })
        });
    }

    /// The account button's label: the name, or "Sign in".
    pub fn account_label(&self) -> String {
        if !self.account.is_signed_in() {
            return t(s::MainWindow_SignIn).to_string();
        }
        let name = self.account.display_name();
        if name.trim().is_empty() {
            t(s::MainWindow_AccountUnnamed).to_string()
        } else {
            name
        }
    }

    pub fn account_tooltip(&self) -> String {
        if !self.account.is_signed_in() {
            return t(s::MainMenuBuilder_SignIn).to_string();
        }
        let label = self.account_label();
        let signed = format(
            t(s::MainMenuBuilder_SignedInAs),
            &[Arg::from(label.as_str())],
        );
        let host = self.account.host();
        let mut tip = if host.is_empty() {
            signed
        } else {
            format!("{signed}\n{host}")
        };
        // Not upstream: what this app can do with the account's encrypted files.
        match self.vault_badge() {
            Some(crate::vault::Badge::Unlocked) => {
                tip.push('\n');
                tip.push_str(t(modlight_core::l10n::port::VaultStateUnlocked));
            }
            Some(crate::vault::Badge::Locked) => {
                tip.push('\n');
                tip.push_str(t(modlight_core::l10n::port::VaultStateLocked));
            }
            None => {}
        }
        tip
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use modlight_core::account::{AccountStore, StoredAccount};

    use super::*;
    use crate::qa::{Look, state};

    /// An account whose server is a closed port on this machine: every request fails at
    /// once and nothing leaves the machine.
    fn account(tag: &str, token: &str) -> AccountStore {
        AccountStore::in_memory(
            StoredAccount {
                host: "http://127.0.0.1:9".into(),
                tag: tag.into(),
                ..StoredAccount::default()
            },
            token,
        )
    }

    /// Run what the background tasks handed back until `done` holds.
    fn settle(app: &mut AppState, ctx: &egui::Context, done: impl Fn(&AppState) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(app) && Instant::now() < deadline {
            for completion in app.rt.drain() {
                completion(app, ctx);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Another account signs in while the chip's request for the previous one is out:
    /// the new account's reading is asked for at once, and the old answer is not shown
    /// under the new name (`AccountStripViewModel.Owns`).
    #[test]
    fn a_reading_for_the_previous_account_is_dropped() {
        let ctx = egui::Context::default();
        let mut app = state(&Look::default(), true);
        app.account = account("first", "first-token");
        app.refresh_account_strip();
        let first = app.strip.in_flight;
        assert!(first.is_some());
        app.refresh_account_strip();
        assert_eq!(
            app.strip.in_flight, first,
            "the same account's request is not asked twice"
        );

        app.account = account("second", "second-token");
        let second = identity_of(&app.credential().unwrap());
        app.refresh_account_strip();

        settle(&mut app, &ctx, |app| {
            app.strip.in_flight.is_none() && app.strip.identity == second
        });
        assert!(app.strip.in_flight.is_none());
        assert_eq!(
            app.strip.identity, second,
            "the reading on screen is the signed-in account's"
        );
        assert!(
            app.strip.has_storage_problem,
            "the closed port answered nothing"
        );
    }
}
