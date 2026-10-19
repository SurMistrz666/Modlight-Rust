//! Port of the reminder half of `lib/data/duplicates.ts` (`duplicateReminder`,
//! `reminderDue`) and of `components/duplicates/snooze.ts` (`isSnoozed`): what
//! the panel shell's DuplicateReminderSlot decides on every page load. The
//! scan, trash and keep operations belong to the library port.

use sqlx::PgPool;

/// More than this many redundant copies, or more than a gibibyte reclaimable
/// (the same 1024^3 `humanFileSize` prints as "1 GB"), and the reminder speaks.
pub const REMINDER_MIN_REDUNDANT: i64 = 10;
pub const REMINDER_MIN_BYTES: i64 = 1024 * 1024 * 1024;

/// "Not now" for the reminder: a cookie, it lapses, and it says nothing about
/// the files at all.
pub const SNOOZE_COOKIE: &str = "ml_dupes_snooze";
pub const SNOOZE_DAYS: i64 = 7;

/// `isSnoozed(value, now)`: anything unparseable reads as "not snoozed" (a
/// hand-typed cookie must not silence the reminder forever).
pub fn is_snoozed(value: Option<&str>, now_ms: i64) -> bool {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return false;
    };
    let until = crate::config::js_number(value);
    until.is_finite() && until > now_ms as f64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DuplicateTotals {
    pub groups: i64,
    pub redundant: i64,
    pub reclaimable: i64,
}

/// Pure, so the rule can be read (and tested) without a database.
pub fn reminder_due(totals: &DuplicateTotals) -> bool {
    totals.redundant > REMINDER_MIN_REDUNDANT || totals.reclaimable > REMINDER_MIN_BYTES
}

/// The reminder's aggregate: the duplicate sets of one owner, minus the ones
/// waved off (`duplicate_dismissals`), in ONE statement that returns one row.
/// Shares the CTE with the full scan so the two never disagree about what a
/// duplicate is.
pub async fn duplicate_reminder(db: &PgPool, user_id: &str) -> sqlx::Result<(DuplicateTotals, bool)> {
    let row: (Option<i32>, Option<i32>, Option<i64>) = sqlx::query_as(
        r#"with dupes as (
             select "checksum" as checksum, "owner_id" as owner_id, count(*)::int as copies,
                    sum("size")::bigint as bytes, max("size")::bigint as keep
             from "uploads"
             where "owner_id" = $1 and "deleted_at" is null
               and "checksum" is not null and length("checksum") > 0
             group by "checksum", "owner_id"
             having count(*) > 1)
           select count(*)::int as groups,
                  coalesce(sum(d.copies - 1), 0)::int as redundant,
                  coalesce(sum(d.bytes - d.keep), 0)::bigint as reclaimable
           from dupes d
           where not exists (select 1 from "duplicate_dismissals"
                             where "owner_id" = d.owner_id and "checksum" = d.checksum)"#,
    )
    .bind(user_id)
    .fetch_one(db)
    .await?;
    let totals = DuplicateTotals {
        groups: i64::from(row.0.unwrap_or(0)),
        redundant: i64::from(row.1.unwrap_or(0)),
        reclaimable: row.2.unwrap_or(0),
    };
    let due = reminder_due(&totals);
    Ok((totals, due))
}

#[cfg(test)]
mod tests {
    use super::*;

    // tests/unit/duplicate-reminder.test.ts (rule and snooze)
    #[test]
    fn reminder_rule_and_snooze() {
        let t = |redundant, reclaimable| DuplicateTotals { groups: 1, redundant, reclaimable };
        assert!(!reminder_due(&t(10, REMINDER_MIN_BYTES)));
        assert!(reminder_due(&t(11, 0)));
        assert!(reminder_due(&t(0, REMINDER_MIN_BYTES + 1)));
        assert!(!is_snoozed(None, 1000));
        assert!(!is_snoozed(Some(""), 1000));
        assert!(!is_snoozed(Some("abc"), 1000));
        assert!(!is_snoozed(Some("999"), 1000));
        assert!(is_snoozed(Some("1001"), 1000));
        assert!(!is_snoozed(Some("Infinity"), 1000));
    }
}
