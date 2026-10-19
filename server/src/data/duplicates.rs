//! Port of the reminder half of `lib/data/duplicates.ts` (`duplicateReminder`,
//! `duplicateReminderQuery`, `reminderDue`) and of
//! `components/duplicates/snooze.ts` (`isSnoozed`, `snoozeUntil`,
//! `snoozeCookie`): what the panel shell's DuplicateReminderSlot decides on
//! every page load. The scan, trash and keep operations belong to the library
//! port.

use sqlx::PgPool;

/// More than this many redundant copies, or more than a gibibyte reclaimable
/// (the same 1024^3 `humanFileSize` prints as "1 GB"), and the reminder speaks.
pub const REMINDER_MIN_REDUNDANT: i64 = 10;
pub const REMINDER_MIN_BYTES: i64 = 1024 * 1024 * 1024;

/// "Not now" for the reminder: a cookie, it lapses, and it says nothing about
/// the files at all.
pub const SNOOZE_COOKIE: &str = "ml_dupes_snooze";

/// How long "not now" lasts. Long enough not to be a nag, short enough to return.
pub const SNOOZE_DAYS: i64 = 7;

const DAY_MS: i64 = 86_400_000;

/// `snoozeUntil(now)`: the instant a snooze started `now` stops holding.
pub fn snooze_until(now_ms: i64) -> i64 {
    now_ms + SNOOZE_DAYS * DAY_MS
}

/// `snoozeCookie(now)`: the cookie the strip writes, as a `document.cookie`
/// string. `Max-Age` matches the value inside it, so a browser that keeps the
/// cookie and a server that reads it agree on when the week is up.
pub fn snooze_cookie(now_ms: i64) -> String {
    format!("{SNOOZE_COOKIE}={}; Path=/; Max-Age={}; SameSite=Lax", snooze_until(now_ms), SNOOZE_DAYS * 86_400)
}

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

/// `duplicateReminderQuery(userId)`: the reminder's aggregate, the user id
/// bound as `$1`. The duplicate sets of one owner, minus the ones waved off
/// (`duplicate_dismissals`), in ONE statement that returns one row and never
/// touches the uploads table twice. Shares the CTE (`dupesCte`) with the full
/// scan so the two never disagree about what a duplicate is.
///
/// `not exists` rather than a LEFT JOIN because here the dismissed sets are
/// not wanted at all, and an anti-join says that in one clause.
pub fn duplicate_reminder_query() -> &'static str {
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
                             where "owner_id" = d.owner_id and "checksum" = d.checksum)"#
}

/// `duplicateReminder(userId)`: what the reminder needs to decide whether to
/// speak, and what to say. One query, no rows.
pub async fn duplicate_reminder(db: &PgPool, user_id: &str) -> sqlx::Result<(DuplicateTotals, bool)> {
    let row: (Option<i32>, Option<i32>, Option<i64>) =
        sqlx::query_as(duplicate_reminder_query()).bind(user_id).fetch_one(db).await?;
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

    /// The reminder SQL, whitespace-flattened.
    fn reminder_sql() -> String {
        duplicate_reminder_query().split_whitespace().collect::<Vec<_>>().join(" ")
    }

    // tests/unit/duplicate-reminder.test.ts: "the reminder aggregate"
    #[test]
    fn the_aggregate_leaves_out_every_set_the_owner_has_waved_off() {
        // This single clause is "count the next threshold from zero".
        let text = reminder_sql();
        assert!(text.contains("not exists"));
        assert!(text.contains(r#""duplicate_dismissals""#));
        assert!(text.contains(r#""checksum" = d.checksum"#));
    }

    #[test]
    fn the_aggregate_matches_a_dismissal_on_the_owner_as_well_as_on_the_bytes() {
        // A checksum is a global fact about bytes; matching on it alone would
        // let one account silence another account's reminder.
        assert!(reminder_sql().contains(r#""owner_id" = d.owner_id"#));
    }

    #[test]
    fn the_aggregate_carries_the_same_safety_clauses_as_the_full_scan() {
        let text = reminder_sql();
        assert!(text.contains(r#""checksum" is not null"#));
        assert!(text.contains(r#"length("checksum") > 0"#));
        assert!(text.contains(r#""deleted_at" is null"#));
        assert!(text.contains("having count(*) > 1"));
    }

    #[test]
    fn the_aggregate_scopes_to_one_account_as_a_bound_parameter_and_asks_for_no_rows() {
        let text = reminder_sql();
        assert!(text.contains(r#""owner_id" = $1"#));
        assert!(!text.contains("$2"));
        // One row out, whatever the library holds: this runs on every panel load.
        assert!(text.contains("count(*)::int as groups"));
        assert!(!text.to_lowercase().contains("limit"));
    }

    #[test]
    fn the_aggregate_never_casts_a_byte_total_to_int4() {
        assert!(reminder_sql().contains("::bigint as reclaimable"));
    }

    // tests/unit/duplicate-reminder.test.ts: "when the reminder is allowed to speak"
    #[test]
    fn says_nothing_at_either_threshold_exactly_and_speaks_one_over() {
        let t = |redundant, reclaimable| DuplicateTotals { groups: 1, redundant, reclaimable };
        assert!(!reminder_due(&t(REMINDER_MIN_REDUNDANT, 0)));
        assert!(!reminder_due(&t(0, REMINDER_MIN_BYTES)));
        assert!(reminder_due(&t(REMINDER_MIN_REDUNDANT + 1, 0)));
        assert!(reminder_due(&t(0, REMINDER_MIN_BYTES + 1)));
        // A gigabyte the way the panel prints one.
        assert_eq!(REMINDER_MIN_BYTES, 1024_i64.pow(3));
        // Quiet on an empty library.
        assert!(!reminder_due(&t(0, 0)));
    }

    // tests/unit/duplicate-reminder.test.ts: "the snooze"
    #[test]
    fn the_snooze_holds_for_exactly_as_long_as_it_says_then_lapses() {
        let now = 1_787_140_800_000; // Date.UTC(2026, 7, 19, 12, 0, 0)
        let value = snooze_until(now).to_string();
        assert!(is_snoozed(Some(&value), now));
        assert!(is_snoozed(Some(&value), now + SNOOZE_DAYS * 86_400_000 - 1));
        assert!(!is_snoozed(Some(&value), now + SNOOZE_DAYS * 86_400_000));
    }

    #[test]
    fn the_snooze_treats_anything_unreadable_as_not_snoozed() {
        let now = 1_787_140_800_000;
        assert!(!is_snoozed(None, now));
        assert!(!is_snoozed(Some(""), now));
        assert!(!is_snoozed(Some("soon"), now));
        assert!(!is_snoozed(Some("Infinity"), now));
    }

    #[test]
    fn the_snooze_cookie_max_age_agrees_with_the_instant_inside_it() {
        let now = 1_787_140_800_000;
        let cookie = snooze_cookie(now);
        assert!(cookie.starts_with(&format!("{SNOOZE_COOKIE}={};", snooze_until(now))));
        assert!(cookie.contains(&format!("Max-Age={}", SNOOZE_DAYS * 86_400)));
        assert!(cookie.contains("Path=/"));
        assert!(cookie.contains("SameSite=Lax"));
    }
}
