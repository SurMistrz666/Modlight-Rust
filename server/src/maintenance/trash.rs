//! Port of `lib/maintenance/trash.ts`: trash retention. The constants and the
//! hook the job runner calls are here; the purge itself is NOT PORTED YET (the
//! uploads/library agent owns it: delete rows older than the retention window
//! in batches of 200, at most 250 batches, delete their bytes and derived
//! files, refund the owners' quota, keep rows storage refused to delete).

use std::time::Duration;

use crate::db::types::JsDate;
use crate::state::AppState;

/// How long a trashed item is kept before it is destroyed for good. Surfaced
/// in the UI verbatim, so this constant and that copy must stay in step.
pub const TRASH_RETENTION_DAYS: i64 = 30;
pub const TRASH_RETENTION_MS: i64 = TRASH_RETENTION_DAYS * 24 * 60 * 60 * 1000;

/// How often the server runs the sweep (`TRASH_SWEEP_INTERVAL_MS`).
pub const TRASH_SWEEP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// The first sweep waits this long after boot, so a boot loop can never turn
/// into a delete loop (server.ts `setTimeout(sweepTrash, 60_000)`).
pub const TRASH_FIRST_SWEEP_DELAY: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrashSweepResult {
    /// Rows permanently removed.
    pub deleted: u64,
    /// Bytes reclaimed (sum of the removed rows' sizes).
    pub bytes: u64,
    /// Rows left for the next sweep because storage refused to delete them.
    pub skipped: u64,
}

/// Days remaining before an item is purged; 0 once it is due.
pub fn trash_days_left(deleted_at: Option<JsDate>, now_ms: i64) -> Option<i64> {
    let stamp = deleted_at?.millis();
    let day = 24 * 60 * 60 * 1000;
    let left = stamp + TRASH_RETENTION_MS - now_ms;
    // Math.ceil for the division, then Math.max(0, ...)
    Some((left as f64 / day as f64).ceil().max(0.0) as i64)
}

/// `purgeExpiredTrash(now)`. NOT PORTED YET: purges nothing.
pub async fn purge_expired_trash(_state: &AppState, _now: JsDate) -> anyhow::Result<TrashSweepResult> {
    tracing::debug!("[trash] purge is not ported yet (lib/maintenance/trash.ts)");
    Ok(TrashSweepResult::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_left() {
        let now = 1_000_000_000_000;
        let deleted = JsDate::from_millis(now).unwrap();
        assert_eq!(trash_days_left(Some(deleted), now), Some(30));
        assert_eq!(trash_days_left(Some(deleted), now + 1), Some(30));
        assert_eq!(trash_days_left(Some(deleted), now + TRASH_RETENTION_MS + 5), Some(0));
        assert_eq!(trash_days_left(None, now), None);
    }
}
