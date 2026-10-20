//! The Server tab's words (`AdminDiskViewModel`): the disk report of `/api/admin/disk`
//! turned into what the screen says, so the arithmetic and the wording are tested apart
//! from the drawing.

use chrono::{DateTime, Utc};

use modlight_api::{DiskAccount, DiskFile, DiskReport};

use crate::format::{age_of, cloud_size};
use crate::l10n::{Arg, format, s, t};

/// `AdminDiskState`.
#[derive(Clone, Debug, PartialEq)]
pub enum AdminState {
    Idle,
    Loading,
    Ready,
    SignedOut,
    Unauthorized,
    Refused,
    NotAllowed,
    Unsupported,
    TimedOut,
    /// The sentence of `ModlightSignIn_Unreachable`.
    Failed(String),
}

impl AdminState {
    pub fn is_problem(&self) -> bool {
        !matches!(
            self,
            AdminState::Idle | AdminState::Loading | AdminState::Ready
        )
    }

    /// `CanRetry`.
    pub fn can_retry(&self) -> bool {
        matches!(self, AdminState::Failed(_) | AdminState::TimedOut)
    }

    /// `ProblemTitle`.
    pub fn title(&self) -> &'static str {
        match self {
            AdminState::SignedOut => t(s::Admin_SignedOutTitle),
            AdminState::Unauthorized => t(s::Admin_UnauthorizedTitle),
            AdminState::Refused => t(s::Admin_RefusedTitle),
            AdminState::NotAllowed => t(s::Admin_NotAllowedTitle),
            AdminState::Unsupported => t(s::Admin_UnsupportedTitle),
            AdminState::TimedOut => t(s::Admin_TimedOutTitle),
            AdminState::Failed(_) => t(s::Admin_FailedTitle),
            _ => "",
        }
    }

    /// `ProblemBody`.
    pub fn body(&self, host: &str) -> String {
        match self {
            AdminState::SignedOut => t(s::Admin_SignedOutBody).to_string(),
            AdminState::Unauthorized => t(s::Admin_UnauthorizedBody).to_string(),
            AdminState::Refused => t(s::Admin_RefusedBody).to_string(),
            AdminState::NotAllowed => t(s::Admin_NotAllowedBody).to_string(),
            AdminState::Unsupported => format(
                t(s::Admin_UnsupportedBody),
                &[Arg::from(modlight_api::extras::DISK_PATH)],
            ),
            AdminState::TimedOut => format(
                t(s::Admin_TimedOutBody),
                &[
                    Arg::from(host),
                    Arg::from(modlight_api::extras::DISK_TIMEOUT.as_secs() as i64),
                ],
            ),
            AdminState::Failed(sentence) => sentence.clone(),
            _ => String::new(),
        }
    }

    /// `ProblemIcon`.
    pub fn icon(&self) -> char {
        use crate::icons as i;
        match self {
            AdminState::SignedOut | AdminState::Unauthorized => i::CIRCLE_USER,
            AdminState::Refused | AdminState::NotAllowed => i::SHIELD,
            AdminState::Unsupported => i::CIRCLE_HELP,
            _ => i::TRIANGLE_ALERT,
        }
    }
}

/// `Pressure`: how close the disk is to full.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pressure {
    Ok,
    Low,
    Critical,
}

/// A legend entry of the machine card (`AdminLegendViewModel`).
#[derive(Clone, Debug, PartialEq)]
pub struct Legend {
    /// "live", "trash", "other", "free" or "app".
    pub key: &'static str,
    pub label: String,
    pub value: String,
    pub hint: String,
}

/// A row with a share bar (`AdminShareViewModel`).
#[derive(Clone, Debug, PartialEq)]
pub struct Share {
    pub label: String,
    pub hint: String,
    pub value: String,
    /// 0 to 100.
    pub percent: f64,
}

/// A file row (`AdminFileViewModel`).
#[derive(Clone, Debug, PartialEq)]
pub struct FileRow {
    pub name: String,
    pub meta: String,
    pub size: String,
}

/// Everything the report says, in words.
#[derive(Clone, Debug, PartialEq)]
pub struct DiskView {
    pub checked: String,
    pub has_filesystem: bool,
    pub source: String,
    pub used: String,
    pub of_total: String,
    pub free_badge: String,
    pub pressure: Pressure,
    pub pressure_note: String,
    /// The stacked bar's four weights: live, trash, other, free (bytes).
    pub weights: [i64; 4],
    pub legend: Vec<Legend>,
    pub insights: String,
    pub trash_desc: String,
    pub trash_held: String,
    pub trash_held_hint: String,
    pub trash_due_soon_label: String,
    pub trash_due_soon: String,
    pub trash_due_soon_hint: String,
    pub trash_oldest: String,
    pub trash_overdue: Option<String>,
    pub kinds: Vec<Share>,
    pub accounts: Vec<Share>,
    pub biggest: Vec<FileRow>,
    pub cold: Vec<FileRow>,
    pub cold_title: String,
    pub cold_summary: String,
    pub cold_empty: String,
}

/// `Share(part, total)`.
pub fn share(part: i64, total: i64) -> f64 {
    if total > 0 {
        (part as f64 * 100.0 / total as f64).clamp(0.0, 100.0)
    } else {
        0.0
    }
}

/// `PressureOf`.
pub fn pressure_of(free: i64, total: i64, min_free: i64) -> Pressure {
    if total <= 0 || free < 0 {
        return Pressure::Ok;
    }
    let ratio = free as f64 / total as f64;
    if ratio <= 0.05 || (min_free > 0 && free <= min_free) {
        return Pressure::Critical;
    }
    if ratio <= 0.10 || (min_free > 0 && free <= min_free * 2) {
        return Pressure::Low;
    }
    Pressure::Ok
}

fn note_for(pressure: Pressure, min_free: i64) -> String {
    let note = match pressure {
        Pressure::Ok => return String::new(),
        Pressure::Critical => t(s::Admin_CriticalNote),
        Pressure::Low => t(s::Admin_LowNote),
    };
    if min_free > 0 {
        format!(
            "{note} {}",
            format(t(s::Admin_ReserveNote), &[Arg::from(cloud_size(min_free))])
        )
    } else {
        note.to_string()
    }
}

/// `PluralForm`: one, few (2 to 4, not 12 to 14), many.
pub fn plural_form(
    count: i64,
    one: &'static str,
    few: &'static str,
    many: &'static str,
) -> &'static str {
    if count == 1 {
        return one;
    }
    let last_two = count.abs() % 100;
    let last = count.abs() % 10;
    if (2..=4).contains(&last) && !(12..=14).contains(&last_two) {
        few
    } else {
        many
    }
}

pub fn files_text(count: i64) -> String {
    format(
        plural_form(
            count,
            t(s::Admin_FilesOne),
            t(s::Admin_FilesFew),
            t(s::Admin_FilesMany),
        ),
        &[Arg::from(count)],
    )
}

fn views_text(count: i64) -> String {
    format(
        plural_form(
            count,
            t(s::Admin_ViewsOne),
            t(s::Admin_ViewsFew),
            t(s::Admin_ViewsMany),
        ),
        &[Arg::from(count)],
    )
}

fn kind_label(kind: Option<&str>) -> &'static str {
    match kind {
        Some("image") => t(s::Admin_KindImage),
        Some("video") => t(s::Admin_KindVideo),
        Some("audio") => t(s::Admin_KindAudio),
        Some("text") => t(s::Admin_KindText),
        Some("pdf") => t(s::Admin_KindPdf),
        Some("archive") => t(s::Admin_KindArchive),
        _ => t(s::Admin_KindOther),
    }
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|v| !v.trim().is_empty())
}

fn account_label(row: &DiskAccount) -> String {
    if row.rest {
        let form = plural_form(
            row.accounts,
            t(s::Admin_AccountsRestOne),
            t(s::Admin_AccountsRestFew),
            t(s::Admin_AccountsRestMany),
        );
        return format(form, &[Arg::from(row.accounts)]);
    }
    if row.orphaned {
        return t(s::Admin_AccountsOrphaned).to_string();
    }
    non_empty(&row.name)
        .or_else(|| non_empty(&row.email))
        .or_else(|| non_empty(&row.tag))
        .map(str::to_string)
        .unwrap_or_else(|| t(s::Admin_AccountsUnknown).to_string())
}

fn owner_label(file: &DiskFile) -> String {
    non_empty(&file.owner_name)
        .or_else(|| non_empty(&file.owner_tag))
        .map(str::to_string)
        .unwrap_or_else(|| t(s::Admin_AccountsOrphaned).to_string())
}

fn name_of(file: &DiskFile) -> String {
    non_empty(&file.filename)
        .or_else(|| non_empty(&file.code))
        .map(str::to_string)
        .unwrap_or_else(|| "?".to_string())
}

impl DiskView {
    /// `Rebuild`: `clock` is the reader's own time of day ("t" pattern).
    pub fn build(r: &DiskReport, now: DateTime<Utc>, clock: &str) -> DiskView {
        let driver = r.driver.as_deref().unwrap_or("").to_uppercase();
        let app_bytes = r.live_bytes + r.trashed_bytes;
        let trash_hint = format(t(s::Admin_TrashHint), &[Arg::from(r.trash_retention_days)]);
        let disk = r.disk.as_ref().filter(|d| d.total_bytes > 0);
        let mut legend = vec![
            Legend {
                key: "live",
                label: t(s::Admin_Live).into(),
                value: cloud_size(r.live_bytes),
                hint: files_text(r.live_files),
            },
            Legend {
                key: "trash",
                label: t(s::Admin_Trash).into(),
                value: cloud_size(r.trashed_bytes),
                hint: trash_hint,
            },
        ];
        let (source, used, of_total, free_badge, pressure, pressure_note, weights);
        match disk {
            Some(disk) => {
                let total = disk.total_bytes;
                let free = disk.free_bytes.max(0);
                let used_bytes = disk.used_bytes.max(0);
                // Clamped: the two halves come from statfs and a sum of recorded sizes.
                let other = (used_bytes - app_bytes).max(0);
                source = t(s::Admin_DescLocal).to_string();
                used = cloud_size(used_bytes);
                of_total = format(t(s::Admin_OfTotal), &[Arg::from(cloud_size(total))]);
                free_badge = format(
                    t(s::Admin_FreeBadge),
                    &[Arg::from(share(free, total).round() as i64)],
                );
                pressure = pressure_of(free, total, r.min_free_bytes);
                pressure_note = note_for(pressure, r.min_free_bytes);
                weights = [r.live_bytes.max(0), r.trashed_bytes.max(0), other, free];
                legend.push(Legend {
                    key: "other",
                    label: t(s::Admin_Other).into(),
                    value: cloud_size(other),
                    hint: t(s::Admin_OtherHint).into(),
                });
                legend.push(Legend {
                    key: "free",
                    label: t(s::Admin_Free).into(),
                    value: cloud_size(free),
                    hint: t(s::Admin_FreeHint).into(),
                });
            }
            None => {
                source = if driver.is_empty() || r.is_local {
                    t(s::Admin_DescUnmeasured).to_string()
                } else {
                    format(t(s::Admin_DescObject), &[Arg::from(driver.as_str())])
                };
                used = cloud_size(app_bytes);
                of_total = String::new();
                free_badge = String::new();
                pressure = Pressure::Ok;
                pressure_note = String::new();
                weights = [0, 0, 0, 1];
                legend.push(Legend {
                    key: "app",
                    label: t(s::Admin_AppTotal).into(),
                    value: cloud_size(app_bytes),
                    hint: files_text(r.live_files + r.trashed_files),
                });
            }
        }
        let insights = if r.is_local && r.disk.is_some() {
            t(s::Admin_InsightsDesc).to_string()
        } else {
            format(
                t(s::Admin_InsightsDescObject),
                &[Arg::from(driver.as_str())],
            )
        };

        let trash = r.trash.clone().unwrap_or_default();
        let trash_overdue = (trash.overdue_files > 0).then(|| {
            format(
                t(s::Admin_TrashOverdue),
                &[
                    Arg::from(cloud_size(trash.overdue_bytes)),
                    Arg::from(files_text(trash.overdue_files)),
                    Arg::from(r.trash_retention_days),
                ],
            )
        });

        let kinds = r
            .kinds
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|kind| Share {
                label: kind_label(kind.kind.as_deref()).into(),
                hint: files_text(kind.files),
                value: cloud_size(kind.bytes),
                percent: share(kind.bytes, r.kind_bytes),
            })
            .collect();

        let rows = r.accounts.clone().unwrap_or_default();
        let total: i64 = rows
            .iter()
            .map(|row| row.live_bytes + row.trashed_bytes)
            .sum();
        let accounts = rows
            .iter()
            .map(|row| {
                let bytes = row.live_bytes + row.trashed_bytes;
                let hint = if row.trashed_bytes > 0 {
                    format!(
                        "{} \u{00B7} {}",
                        files_text(row.live_files),
                        format(
                            t(s::Admin_AccountsTrashed),
                            &[Arg::from(cloud_size(row.trashed_bytes))]
                        )
                    )
                } else {
                    files_text(row.live_files)
                };
                Share {
                    label: account_label(row),
                    hint,
                    value: cloud_size(bytes),
                    percent: share(bytes, total),
                }
            })
            .collect();

        let biggest = r
            .biggest
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|file| {
                let mut meta = owner_label(file);
                if file.created_at.is_some() {
                    meta.push_str(" \u{00B7} ");
                    meta.push_str(&format(
                        t(s::Admin_Uploaded),
                        &[Arg::from(age_of(file.created_at, now))],
                    ));
                }
                meta.push_str(" \u{00B7} ");
                meta.push_str(&views_text(file.views));
                FileRow {
                    name: name_of(file),
                    meta,
                    size: cloud_size(file.size),
                }
            })
            .collect();
        let cold = r
            .cold
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|file| {
                let when = match file.last_accessed_at {
                    None => t(s::Admin_NeverFetched).to_string(),
                    Some(_) => format(
                        t(s::Admin_LastFetched),
                        &[Arg::from(age_of(file.last_accessed_at, now))],
                    ),
                };
                FileRow {
                    name: name_of(file),
                    meta: format!("{} \u{00B7} {when}", owner_label(file)),
                    size: cloud_size(file.size),
                }
            })
            .collect();
        let cold_totals = r.cold_totals.clone().unwrap_or_default();

        DiskView {
            checked: format(t(s::Admin_Checked), &[Arg::from(clock)]),
            has_filesystem: disk.is_some(),
            source,
            used,
            of_total,
            free_badge,
            pressure,
            pressure_note,
            weights,
            legend,
            insights,
            trash_desc: format(t(s::Admin_TrashDesc), &[Arg::from(r.trash_retention_days)]),
            trash_held: cloud_size(trash.bytes),
            trash_held_hint: files_text(trash.files),
            trash_due_soon_label: format(
                t(s::Admin_TrashDueSoon),
                &[Arg::from(r.trash_due_soon_days)],
            ),
            trash_due_soon: cloud_size(trash.due_soon_bytes),
            trash_due_soon_hint: files_text(trash.due_soon_files),
            trash_oldest: match trash.oldest {
                None => t(s::Admin_TrashOldestNone).to_string(),
                Some(_) => age_of(trash.oldest, now),
            },
            trash_overdue,
            kinds,
            accounts,
            biggest,
            cold,
            cold_title: format(t(s::Admin_ColdTitle), &[Arg::from(cold_totals.days)]),
            cold_summary: format(
                t(s::Admin_ColdSummary),
                &[
                    Arg::from(cloud_size(cold_totals.bytes)),
                    Arg::from(files_text(cold_totals.files)),
                ],
            ),
            cold_empty: format(t(s::Admin_ColdEmpty), &[Arg::from(cold_totals.days)]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::l10n::{Language, set_thread_language};

    fn report() -> DiskReport {
        serde_json::from_value(serde_json::json!({
            "driver": "local",
            "local": true,
            "liveFiles": 120, "liveBytes": 4_000_000_000i64,
            "trashedFiles": 3, "trashedBytes": 1_000_000_000i64,
            "disk": {"totalBytes": 100_000_000_000i64, "freeBytes": 8_000_000_000i64, "usedBytes": 92_000_000_000i64},
            "minFreeBytes": 0,
            "kinds": [{"kind": "image", "files": 100, "bytes": 3_000_000_000i64}, {"kind": "zip?", "files": 20, "bytes": 1_000_000_000i64}],
            "kindBytes": 4_000_000_000i64,
            "accounts": [{"name": "Alex", "liveFiles": 2, "liveBytes": 300, "trashedBytes": 100}, {"rest": true, "accounts": 3, "liveBytes": 600}],
            "biggest": [{"filename": "", "code": "q8", "size": 10, "ownerTag": "alex", "views": 1}],
            "cold": [{"filename": "a.png", "size": 5}],
            "coldTotals": {"files": 1, "bytes": 5, "days": 90},
            "trash": {"files": 3, "bytes": 1000, "overdueFiles": 1, "overdueBytes": 10},
            "trashRetentionDays": 30, "trashDueSoonDays": 3
        }))
        .unwrap()
    }

    #[test]
    fn the_report_reads_like_upstream() {
        set_thread_language(Some(Language::English));
        let now = Utc::now();
        let view = DiskView::build(&report(), now, "9:41 AM");
        assert!(view.has_filesystem);
        assert_eq!(view.pressure, Pressure::Low);
        assert!(!view.pressure_note.is_empty());
        // Other is used minus what the app holds; free is clamped and in the bar.
        assert_eq!(
            view.weights,
            [4_000_000_000, 1_000_000_000, 87_000_000_000, 8_000_000_000]
        );
        assert_eq!(view.legend.len(), 4);
        assert_eq!(view.kinds[0].percent, 75.0);
        assert_eq!(view.kinds[1].label, t(s::Admin_KindOther));
        assert_eq!(view.accounts[0].label, "Alex");
        assert_eq!(view.accounts[0].percent, 40.0);
        assert!(view.accounts[0].hint.contains('\u{00B7}'));
        assert_eq!(view.biggest[0].name, "q8");
        assert!(view.biggest[0].meta.starts_with("alex"));
        assert!(view.cold[0].meta.contains(t(s::Admin_NeverFetched)));
        assert!(view.trash_overdue.is_some());
        assert!(view.checked.contains("9:41 AM"));
    }

    #[test]
    fn plurals_follow_polish_rules() {
        assert_eq!(plural_form(1, "one", "few", "many"), "one");
        assert_eq!(plural_form(3, "one", "few", "many"), "few");
        assert_eq!(plural_form(13, "one", "few", "many"), "many");
        assert_eq!(plural_form(22, "one", "few", "many"), "few");
        assert_eq!(plural_form(25, "one", "few", "many"), "many");
    }

    #[test]
    fn pressure_and_object_storage() {
        assert_eq!(pressure_of(4, 100, 0), Pressure::Critical);
        assert_eq!(pressure_of(50, 100, 30), Pressure::Low);
        assert_eq!(pressure_of(50, 100, 0), Pressure::Ok);
        let mut r = report();
        r.disk = None;
        r.is_local = false;
        r.driver = Some("s3".into());
        let view = DiskView::build(&r, Utc::now(), "");
        assert!(!view.has_filesystem);
        assert_eq!(view.legend.last().unwrap().key, "app");
        assert!(view.source.contains("S3"));
    }
}
