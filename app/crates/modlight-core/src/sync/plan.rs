//! Which side wins, decided before a single byte moves (`ModlightSettingsPlan.cs`).
//!
//! Free of HTTP, JSON and config objects: a comparison between three short strings and
//! two integers, so every branch is tested without a network. The state it reads is what
//! the server last CONFIRMED (the marker), never a clock.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncAction {
    /// The two sides agree and the marker already says so.
    Nothing,
    /// Both sides hold the same document; only the marker is behind. Write it, send nothing.
    Confirm,
    /// The account has no stored settings. Send ours, and become the first copy.
    Seed,
    /// Take the server's document and put it on this machine.
    Adopt,
    /// Send this machine's document, replacing what the server holds.
    Push,
}

/// A decision, the version it is conditional on, and why it was reached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncPlan {
    pub action: SyncAction,
    /// What a write asserts it replaces (Seed, Push); what is being taken (Adopt, Confirm).
    pub version: i64,
    /// For the log; not shown to anybody.
    pub reason: &'static str,
}

impl fmt::Display for SyncPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} (v{}): {}", self.action, self.version, self.reason)
    }
}

/// Work out what to do with the document the server just reported.
#[allow(clippy::too_many_arguments)]
pub fn decide(
    account_key: Option<&str>,
    confirmed_account: Option<&str>,
    confirmed_version: i64,
    confirmed_digest: Option<&str>,
    local_digest: Option<&str>,
    remote_version: i64,
    remote_digest: Option<&str>,
) -> SyncPlan {
    let local = local_digest.unwrap_or("");
    let remote = remote_digest.unwrap_or("");

    // A marker written for another account or deployment is not evidence about this one.
    let marker_applies =
        account_key.is_some_and(|k| !k.is_empty()) && account_key == confirmed_account;
    let confirmed_at = if marker_applies { confirmed_version } else { 0 };
    let confirmed = if marker_applies {
        confirmed_digest.unwrap_or("")
    } else {
        ""
    };

    // Both sides hold the same bytes: at most the marker is behind.
    if remote_version > 0 && !local.is_empty() && local == remote {
        return if confirmed_at == remote_version && confirmed == remote {
            SyncPlan {
                action: SyncAction::Nothing,
                version: remote_version,
                reason: "already in step",
            }
        } else {
            SyncPlan {
                action: SyncAction::Confirm,
                version: remote_version,
                reason: "same document, marker behind",
            }
        };
    }

    // Nothing stored: whoever gets there first seeds, asserting there is still nothing.
    if remote_version == 0 {
        return SyncPlan {
            action: SyncAction::Seed,
            version: 0,
            reason: "nothing stored for this account",
        };
    }

    // THE FRESH-INSTALL GUARD: an installation never confirmed for this account cannot
    // tell settings somebody chose from defaults, so it never pushes. The cloud wins.
    if confirmed_at == 0 {
        return SyncPlan {
            action: SyncAction::Adopt,
            version: remote_version,
            reason: "this installation has never been confirmed",
        };
    }

    // Clean: everything held here was acknowledged, so the server is newer by construction.
    if local == confirmed {
        return SyncPlan {
            action: SyncAction::Adopt,
            version: remote_version,
            reason: "no local change, server moved",
        };
    }

    // Both changed: this machine wins (its change exists in one place only), against the
    // version just READ so a third machine's write in between is still refused.
    SyncPlan {
        action: SyncAction::Push,
        version: remote_version,
        reason: "local change the server never confirmed",
    }
}

/// The identity a marker belongs to: host AND tag, separated by a newline.
pub fn account_key(host: &str, tag: &str) -> String {
    format!("{host}\n{tag}")
}

/// A port of `ShareX.Tests/ModlightSettingsSyncTests.cs`, check for check.
#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> String {
        account_key("https://host.example.com", "ToqOse")
    }

    fn other() -> String {
        account_key("https://host.example.com", "someone-else")
    }

    const OURS: &str = "aaaa";
    const THEIRS: &str = "bbbb";
    const OLDER: &str = "cccc";

    #[test]
    fn fresh_install_never_pushes() {
        let plan = decide(
            Some(&account()),
            Some(""),
            0,
            Some(""),
            Some(OURS),
            7,
            Some(THEIRS),
        );
        assert_eq!(
            plan.action,
            SyncAction::Adopt,
            "a never-confirmed installation adopts instead of pushing"
        );
        assert_eq!(
            plan.version, 7,
            "and adopts the version the server actually reported"
        );
    }

    #[test]
    fn first_machine_seeds() {
        let plan = decide(
            Some(&account()),
            Some(""),
            0,
            Some(""),
            Some(OURS),
            0,
            Some(""),
        );
        assert_eq!(
            plan.action,
            SyncAction::Seed,
            "an empty account is seeded by whoever gets there first"
        );
        assert_eq!(
            plan.version, 0,
            "and the write asserts that nothing was stored"
        );
    }

    #[test]
    fn quiet_machine_adopts() {
        let a = account();
        let plan = decide(
            Some(&a),
            Some(&a),
            4,
            Some(OURS),
            Some(OURS),
            5,
            Some(THEIRS),
        );
        assert_eq!(
            plan.action,
            SyncAction::Adopt,
            "a clean machine adopts the newer document"
        );
        assert_eq!(plan.version, 5, "at the version it just read");
    }

    #[test]
    fn changed_machine_pushes() {
        let a = account();
        let quiet = decide(
            Some(&a),
            Some(&a),
            5,
            Some(OLDER),
            Some(OURS),
            5,
            Some(OLDER),
        );
        assert_eq!(
            quiet.action,
            SyncAction::Push,
            "an unsent local change is pushed"
        );
        assert_eq!(quiet.version, 5, "replacing the version it was based on");

        let contested = decide(
            Some(&a),
            Some(&a),
            5,
            Some(OLDER),
            Some(OURS),
            9,
            Some(THEIRS),
        );
        assert_eq!(
            contested.action,
            SyncAction::Push,
            "and still pushed when the server moved as well"
        );
        assert_eq!(
            contested.version, 9,
            "against the version just read, not the one confirmed"
        );
    }

    #[test]
    fn agreement_is_cheap() {
        let a = account();
        let behind = decide(
            Some(&a),
            Some(&a),
            4,
            Some(OLDER),
            Some(OURS),
            6,
            Some(OURS),
        );
        assert_eq!(
            behind.action,
            SyncAction::Confirm,
            "an identical document only moves the marker"
        );
        assert_eq!(behind.version, 6, "to the version the server holds");

        let settled = decide(Some(&a), Some(&a), 6, Some(OURS), Some(OURS), 6, Some(OURS));
        assert_eq!(
            settled.action,
            SyncAction::Nothing,
            "and nothing at all once the marker agrees"
        );
    }

    #[test]
    fn another_accounts_marker_is_not_evidence() {
        let a = account();
        let o = other();
        let plan = decide(
            Some(&a),
            Some(&o),
            12,
            Some(OURS),
            Some(OURS),
            3,
            Some(THEIRS),
        );
        assert_eq!(
            plan.action,
            SyncAction::Adopt,
            "a marker from another account is discarded, so the cloud wins"
        );

        let same = decide(
            Some(&a),
            Some(&a),
            12,
            Some(OURS),
            Some(THEIRS),
            3,
            Some(OLDER),
        );
        assert_eq!(
            same.action,
            SyncAction::Push,
            "while the same account's marker still counts"
        );
    }
}
