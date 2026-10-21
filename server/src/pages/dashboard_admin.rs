//! Loader of `app/(app)/dashboard/admin/page.tsx` (CONTRACT 6.20).
//!
//! `requireAdmin()` (signed out -> `/login`, not an admin -> `/dashboard`),
//! then, in parallel as upstream: `isRegistrationEnabled()`,
//! `registrationKeyState()`, `toMailStatus(loadMailSettings(true))`,
//! `loadGlobalStats()` (page-local upstream, ported here) and
//! `serverStorageUsage()`; plus `diskInsights()`, which upstream's
//! `<ServerDisk>` awaited itself while the page rendered.
//!
//! Props (`AdminPageProps` in the SPA): `{ session: { user: { id, email } },
//! regEnabled, keyState, mail, stats, disk, diskInsights, defaultQuota,
//! turnstile }` (`turnstile` is not upstream: the optional Cloudflare
//! Turnstile card's `TurnstileStatus`, read through like `mail`).
//! Metadata `{ title: "Admin" }` (not translated). No database writes; the
//! process-wide mail snapshot is refreshed (read through, not from the 30 s
//! snapshot: an operator who has just saved SMTP settings must not be shown
//! the previous ones).

use serde::Serialize;
use sqlx::PgPool;

use crate::data::admin::{ServerStorageUsage, server_storage_usage};
use crate::data::admin_disk::{DiskInsights, disk_insights};
use crate::db::types::JsDate;
use crate::format::human_file_size;
use crate::mail_settings::{MailStatus, load_mail_settings, to_mail_status};
use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageResult};
use crate::quota::is_unlimited;
use crate::registration::{RegistrationKeyState, registration_key_state};
use crate::settings::is_registration_enabled;
use crate::turnstile::{TurnstileStatus, to_status};

/// `GlobalStats` (components/admin/StatsCards.tsx), in the order
/// `loadGlobalStats` builds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalStats {
    pub total_files: i64,
    pub total_storage: String,
    pub total_views: i64,
    pub users: i64,
    pub admins: i64,
    pub banned: i64,
    pub orphan_files: i64,
    pub orphan_storage: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdminSessionUser {
    pub id: String,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdminSession {
    pub user: AdminSessionUser,
}

/// `AdminPageProps` (the SPA's `pages/(app)/dashboard/admin/page.tsx`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminPageProps {
    /// `session.user.id` and `session.user.email`, never more of the session.
    pub session: AdminSession,
    pub reg_enabled: bool,
    pub key_state: RegistrationKeyState,
    pub mail: MailStatus,
    pub stats: GlobalStats,
    pub disk: ServerStorageUsage,
    pub disk_insights: DiskInsights,
    pub default_quota: Option<String>,
    /// Not upstream: the optional Cloudflare Turnstile card (src/turnstile.rs).
    pub turnstile: TurnstileStatus,
}

/// Two aggregate queries instead of shipping the whole user table to the
/// browser to be counted there. `size` and the quota columns are bigint, so
/// every sum is read back as a number rather than trusted as-is.
///
/// The file aggregate is split by owner presence in one pass: orphans (owner
/// deleted, files kept) must not inflate "files stored", but the operator
/// still needs to see that they exist; a second scan just to count them
/// would be waste.
pub async fn load_global_stats(db: &PgPool) -> sqlx::Result<GlobalStats> {
    let files = sqlx::query_as::<_, (Option<i32>, Option<i64>, Option<i32>, Option<i32>, Option<i64>)>(
        r#"SELECT (count(*) filter (where "owner_id" is not null))::int,
                  coalesce(sum("size") filter (where "owner_id" is not null), 0)::bigint,
                  coalesce(sum("views"), 0)::int,
                  (count(*) filter (where "owner_id" is null))::int,
                  coalesce(sum("size") filter (where "owner_id" is null), 0)::bigint
           FROM "uploads" WHERE "deleted_at" IS NULL"#,
    )
    .fetch_optional(db);
    let accounts = sqlx::query_as::<_, (Option<i32>, Option<i32>, Option<i32>)>(
        r#"SELECT count(*)::int, (count(*) filter (where "role" = 'admin'))::int,
                  (count(*) filter (where "banned"))::int
           FROM "user""#,
    )
    .fetch_optional(db);
    let (files, accounts) = tokio::try_join!(files, accounts)?;
    let (file_count, size, views, orphan_files, orphan_size) = files.unwrap_or((None, None, None, None, None));
    let (total, admins, banned) = accounts.unwrap_or((None, None, None));
    Ok(GlobalStats {
        total_files: i64::from(file_count.unwrap_or(0)),
        total_storage: human_file_size(size.unwrap_or(0) as f64, 1),
        total_views: i64::from(views.unwrap_or(0)),
        users: i64::from(total.unwrap_or(0)),
        admins: i64::from(admins.unwrap_or(0)),
        banned: i64::from(banned.unwrap_or(0)),
        orphan_files: i64::from(orphan_files.unwrap_or(0)),
        orphan_storage: human_file_size(orphan_size.unwrap_or(0) as f64, 1),
    })
}

pub async fn load(ctx: PageCtx) -> PageResult {
    let session = ctx.require_admin().await?;
    let state = &ctx.state;
    let (db, config) = (&state.db, &state.config);

    let (reg_enabled, key_state, mail, stats, disk, insights, turnstile) = tokio::try_join!(
        is_registration_enabled(db, config.auth.registration_enabled),
        registration_key_state(db, &config.auth.secret),
        load_mail_settings(state, true),
        load_global_stats(db),
        server_storage_usage(db, config),
        disk_insights(db, config, JsDate::now()),
        state.turnstile.load(db, config, true),
    )?;

    let default_quota = if is_unlimited(Some(config.default_user_quota)) {
        None
    } else {
        Some(human_file_size(config.default_user_quota as f64, 1))
    };

    let props = AdminPageProps {
        session: AdminSession {
            user: AdminSessionUser { id: session.user.id.clone(), email: session.user.email.clone() },
        },
        reg_enabled,
        key_state,
        mail: to_mail_status(&mail, config),
        stats,
        disk,
        disk_insights: insights,
        default_quota,
        turnstile: to_status(&turnstile, config),
    };
    PageData::new(&props, PageMeta::titled("Admin"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_stats_keep_upstreams_key_order() {
        let stats = GlobalStats {
            total_files: 1,
            total_storage: "1.5 KB".into(),
            total_views: 2,
            users: 3,
            admins: 1,
            banned: 0,
            orphan_files: 4,
            orphan_storage: "0 B".into(),
        };
        let v = serde_json::to_value(&stats).unwrap();
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            vec![
                "totalFiles",
                "totalStorage",
                "totalViews",
                "users",
                "admins",
                "banned",
                "orphanFiles",
                "orphanStorage"
            ]
        );
    }
}
