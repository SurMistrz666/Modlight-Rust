//! Not upstream: the grant that opens an account's encrypted files (the server's
//! `docs/ENCRYPTION.md`, section 6) and where this computer keeps it.
//!
//! * It is kept like the token, behind a [`CredentialStore`] (the login Keychain on a
//!   Mac, Credential Manager on Windows, a private file of a QA profile), in an item of
//!   its own ([`GRANT_ITEM`]), and only when the person ticks "Remember on this computer
//!   for 30 days" (ticked by default). Otherwise it lives in memory for this run only,
//!   and an older remembered grant is removed.
//! * It is bound to the account it opens (host and tag), as the token is bound to its
//!   host: another account never gets it, and asking for another account's grant
//!   forgets it. One that has run out is never used.
//! * It is forgotten on sign-out, on a switch to another account, on a `423` that shows
//!   it no longer works, and on "Lock files" ([`GrantKeeper::forget`]).
//!
//! Nothing here logs the grant; [`UnlockGrant`]'s `Debug` hides it.

use chrono::{DateTime, Utc};
use modlight_api::UnlockGrant;
use serde::{Deserialize, Serialize};

use crate::account::{CredentialStore, MemoryCredentialStore, same_host};

/// The credential store item the grant is kept in (beside the token's `api-token`).
pub const GRANT_ITEM: &str = "vault-grant";

/// The QA profile's file for the grant (beside its `credentials`).
pub const GRANT_FILE: &str = "vault-grant";

/// What the credential store holds: the grant, the account it opens, until when.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StoredGrant {
    host: String,
    tag: String,
    grant: String,
    #[serde(default)]
    expires_at: Option<DateTime<Utc>>,
}

/// The grant in use, and whose it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptGrant {
    /// The server, as the account's host is written.
    pub host: String,
    /// The account on that server.
    pub tag: String,
    pub grant: UnlockGrant,
    /// Kept on this computer (`true`) or for this run only.
    pub remembered: bool,
}

impl KeptGrant {
    /// Whether this grant belongs to `host`'s account `tag`.
    pub fn opens(&self, host: &str, tag: &str) -> bool {
        same_host(&self.host, host) && self.tag == tag
    }
}

/// The grant of the signed-in account, in memory and (when remembered) on this computer.
pub struct GrantKeeper {
    store: Box<dyn CredentialStore>,
    kept: Option<KeptGrant>,
    /// The store has been read this run: it is asked at most once (and never again after
    /// a grant was kept or forgotten here).
    read: bool,
}

impl GrantKeeper {
    pub fn new(store: Box<dyn CredentialStore>) -> GrantKeeper {
        GrantKeeper {
            store,
            kept: None,
            read: false,
        }
    }

    /// A keeper that never writes anywhere (previews, tests, headless runs).
    pub fn in_memory() -> GrantKeeper {
        GrantKeeper::new(Box::new(MemoryCredentialStore::default()))
    }

    /// The grant in use, if any.
    pub fn kept(&self) -> Option<&KeptGrant> {
        self.kept.as_ref()
    }

    /// The grant that opens `host`'s account `tag`, when this computer has one that has
    /// not run out by `now`.
    ///
    /// A grant held for another account, or one that has run out, is forgotten here and
    /// on this computer; so is a remembered one that names another account or cannot be
    /// read. (Revoking it on the server is the caller's: it holds the credential.)
    pub fn grant_for(&mut self, host: &str, tag: &str, now: DateTime<Utc>) -> Option<UnlockGrant> {
        if let Some(kept) = &self.kept {
            if kept.opens(host, tag) && !kept.grant.has_expired(now) {
                return Some(kept.grant.clone());
            }
            self.forget();
            return None;
        }
        if self.read {
            return None;
        }
        self.read = true;
        let secret = self.store.load()?;
        let usable = serde_json::from_str::<StoredGrant>(&secret)
            .ok()
            .map(|stored| KeptGrant {
                grant: UnlockGrant::new(stored.grant, stored.expires_at),
                host: stored.host,
                tag: stored.tag,
                remembered: true,
            })
            .filter(|kept| {
                kept.opens(host, tag)
                    && !kept.grant.has_expired(now)
                    && modlight_api::is_plausible_token(&kept.grant.value)
            });
        match usable {
            Some(kept) => {
                let grant = kept.grant.clone();
                self.kept = Some(kept);
                Some(grant)
            }
            None => {
                if let Err(error) = self.store.delete() {
                    log::warn!(
                        "Modlight: a file password grant that is no use here could not be removed: {error}"
                    );
                }
                None
            }
        }
    }

    /// Keep a grant the server just gave for `host`'s account `tag`: in memory for this
    /// run, and on this computer too when `remember`. Without `remember`, a grant
    /// remembered before is removed from this computer.
    ///
    /// The grant is in use even when the store refuses it (a locked Keychain); the error
    /// says it will not outlast this run.
    pub fn keep(
        &mut self,
        host: &str,
        tag: &str,
        grant: UnlockGrant,
        remember: bool,
    ) -> Result<(), String> {
        self.read = true;
        let stored = remember.then(|| StoredGrant {
            host: host.to_string(),
            tag: tag.to_string(),
            grant: grant.value.clone(),
            expires_at: grant.expires_at,
        });
        self.kept = Some(KeptGrant {
            host: host.to_string(),
            tag: tag.to_string(),
            grant,
            remembered: remember,
        });
        match stored {
            Some(stored) => {
                let text = serde_json::to_string(&stored).map_err(|e| e.to_string())?;
                self.store.save(&text)
            }
            None => self.store.delete(),
        }
    }

    /// Forget the grant here and on this computer. Returns what was in use, so the
    /// caller can revoke it on the server.
    pub fn forget(&mut self) -> Option<KeptGrant> {
        self.read = true;
        if let Err(error) = self.store.delete() {
            log::warn!("Modlight: the file password grant could not be removed: {error}");
        }
        self.kept.take()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use chrono::{Duration, TimeZone};

    use super::*;
    use crate::account::FileCredentialStore;

    const HOST: &str = "https://files.example.com";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 27, 12, 0, 0).unwrap()
    }

    fn grant(value: &str, days: i64) -> UnlockGrant {
        UnlockGrant::new(value, Some(now() + Duration::days(days)))
    }

    /// A store whose contents a test can look at, shared with a "later run".
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Option<String>>>);

    impl CredentialStore for Shared {
        fn load(&self) -> Option<String> {
            self.0.lock().unwrap().clone()
        }
        fn save(&self, token: &str) -> Result<(), String> {
            *self.0.lock().unwrap() = Some(token.to_string());
            Ok(())
        }
        fn delete(&self) -> Result<(), String> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    impl Shared {
        fn held(&self) -> Option<String> {
            self.0.lock().unwrap().clone()
        }
    }

    #[test]
    fn a_remembered_grant_outlasts_the_run_for_its_account_only() {
        let store = Shared::default();
        let mut keeper = GrantKeeper::new(Box::new(store.clone()));
        keeper
            .keep(HOST, "alex", grant("id.secret-a", 30), true)
            .unwrap();
        let written = store.held().expect("kept on this computer");
        assert!(written.contains("id.secret-a"));
        assert!(written.contains("\"Tag\":\"alex\""));
        assert!(keeper.kept().unwrap().remembered);

        // The next run, same account: the grant is back.
        let mut later = GrantKeeper::new(Box::new(store.clone()));
        assert_eq!(
            later.grant_for(HOST, "alex", now()).map(|g| g.value),
            Some("id.secret-a".into())
        );
        // The same origin written another way is the same server.
        let mut later = GrantKeeper::new(Box::new(store.clone()));
        assert!(
            later
                .grant_for("HTTPS://files.example.com/", "alex", now())
                .is_some()
        );

        // Another account on the same server never gets it, and it is forgotten.
        let mut other = GrantKeeper::new(Box::new(store.clone()));
        assert_eq!(other.grant_for(HOST, "sam", now()), None);
        assert_eq!(store.held(), None, "another account's grant is removed");
    }

    #[test]
    fn another_server_never_gets_the_grant() {
        let store = Shared::default();
        let mut keeper = GrantKeeper::new(Box::new(store.clone()));
        keeper
            .keep(HOST, "alex", grant("id.secret-a", 30), true)
            .unwrap();
        let mut later = GrantKeeper::new(Box::new(store.clone()));
        assert_eq!(
            later.grant_for("https://other.example.com", "alex", now()),
            None
        );
        assert_eq!(store.held(), None);
    }

    #[test]
    fn without_remember_it_lives_for_this_run_only() {
        let store = Shared::default();
        let mut keeper = GrantKeeper::new(Box::new(store.clone()));
        // Remembered first, then unlocked again without "remember".
        keeper
            .keep(HOST, "alex", grant("id.remembered", 30), true)
            .unwrap();
        keeper
            .keep(HOST, "alex", grant("id.session", 30), false)
            .unwrap();
        assert_eq!(store.held(), None, "nothing stays on this computer");
        assert_eq!(
            keeper.grant_for(HOST, "alex", now()).map(|g| g.value),
            Some("id.session".into()),
            "in use for this run"
        );
        assert!(!keeper.kept().unwrap().remembered);
        let mut later = GrantKeeper::new(Box::new(store.clone()));
        assert_eq!(later.grant_for(HOST, "alex", now()), None);
    }

    #[test]
    fn a_grant_that_ran_out_is_not_used() {
        let store = Shared::default();
        let mut keeper = GrantKeeper::new(Box::new(store.clone()));
        keeper
            .keep(HOST, "alex", grant("id.secret-a", 30), true)
            .unwrap();
        let mut later = GrantKeeper::new(Box::new(store.clone()));
        assert_eq!(
            later.grant_for(HOST, "alex", now() + Duration::days(31)),
            None
        );
        assert_eq!(store.held(), None);
        // In memory too.
        keeper
            .keep(HOST, "alex", grant("id.session", 1), false)
            .unwrap();
        assert_eq!(
            keeper.grant_for(HOST, "alex", now() + Duration::days(2)),
            None
        );
        assert!(keeper.kept().is_none());
    }

    #[test]
    fn forgetting_clears_memory_and_this_computer() {
        let store = Shared::default();
        let mut keeper = GrantKeeper::new(Box::new(store.clone()));
        keeper
            .keep(HOST, "alex", grant("id.secret-a", 30), true)
            .unwrap();
        let forgotten = keeper.forget().expect("the grant that was in use");
        assert_eq!(forgotten.grant.value, "id.secret-a");
        assert!(keeper.kept().is_none());
        assert_eq!(store.held(), None);
        assert_eq!(keeper.grant_for(HOST, "alex", now()), None);
        assert!(keeper.forget().is_none());
    }

    /// Switching accounts: the first account's grant is forgotten when the second one's
    /// is asked for, and is handed back for revoking.
    #[test]
    fn a_held_grant_of_another_account_is_forgotten() {
        let store = Shared::default();
        let mut keeper = GrantKeeper::new(Box::new(store.clone()));
        keeper
            .keep(HOST, "alex", grant("id.secret-a", 30), true)
            .unwrap();
        assert!(!keeper.kept().unwrap().opens(HOST, "sam"));
        assert_eq!(keeper.grant_for(HOST, "sam", now()), None);
        assert!(keeper.kept().is_none());
        assert_eq!(store.held(), None);
    }

    #[test]
    fn an_unreadable_item_is_removed() {
        let store = Shared::default();
        store.save("not json").unwrap();
        let mut keeper = GrantKeeper::new(Box::new(store.clone()));
        assert_eq!(keeper.grant_for(HOST, "alex", now()), None);
        assert_eq!(store.held(), None);
        // A value that could not travel in a header is not used either.
        store
            .save(r#"{"Host":"https://files.example.com","Tag":"alex","Grant":"two\nlines"}"#)
            .unwrap();
        let mut keeper = GrantKeeper::new(Box::new(store.clone()));
        assert_eq!(keeper.grant_for(HOST, "alex", now()), None);
    }

    /// QA profiles keep it as they keep the token: a file only this user reads.
    #[test]
    fn a_qa_profile_keeps_it_in_a_private_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(GRANT_FILE);
        let mut keeper = GrantKeeper::new(Box::new(FileCredentialStore::new(path.clone())));
        keeper
            .keep(HOST, "alex", grant("id.secret-a", 30), true)
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let mut later = GrantKeeper::new(Box::new(FileCredentialStore::new(path.clone())));
        assert!(later.grant_for(HOST, "alex", now()).is_some());
        later.forget();
        assert!(!path.exists());
    }

    /// The store is asked once a run, not on every look.
    #[test]
    fn the_store_is_read_once() {
        #[derive(Default)]
        struct Counting(Arc<Mutex<usize>>);
        impl CredentialStore for Counting {
            fn load(&self) -> Option<String> {
                *self.0.lock().unwrap() += 1;
                None
            }
            fn save(&self, _: &str) -> Result<(), String> {
                Ok(())
            }
            fn delete(&self) -> Result<(), String> {
                Ok(())
            }
        }
        let reads = Arc::new(Mutex::new(0));
        let mut keeper = GrantKeeper::new(Box::new(Counting(reads.clone())));
        for _ in 0..5 {
            assert_eq!(keeper.grant_for(HOST, "alex", now()), None);
        }
        assert_eq!(*reads.lock().unwrap(), 1);
    }

    #[test]
    fn the_grant_is_never_printed() {
        let mut keeper = GrantKeeper::in_memory();
        keeper
            .keep(HOST, "alex", grant("id.secret-a", 30), true)
            .unwrap();
        assert!(!format!("{:?}", keeper.kept()).contains("secret-a"));
    }
}
