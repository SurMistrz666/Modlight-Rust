//! Port of `lib/data/admin-files.ts`: an administrator looking at another
//! account's library.
//!
//! Not impersonation: the signed-in account stays the administrator, so every
//! write is attributed to them. The visit itself is audited
//! (`admin.user.files_viewed`), deduplicated over ten minutes per target so
//! paging does not bury the log.

use serde::Serialize;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::avatar::avatar_url;
use crate::config::Config;
use crate::data::uploads::{FileItem, ListParams, list_uploads, to_file_items};
use crate::db::types::JsDate;
use crate::maintenance::trash::trash_days_left;
use crate::media::urls::user_base_url;
use crate::pages::dashboard_trash::TrashItem;

/// Paging through somebody's library should not write one entry per page.
const VIEW_DEDUP_MS: i64 = 10 * 60 * 1000;

/// `AdminFileTarget`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminFileTarget {
    pub id: String,
    pub name: String,
    pub email: String,
    pub tag: String,
    pub avatar: Option<String>,
    pub role: Option<String>,
    pub banned: bool,
    /// The TARGET's link base, not the administrator's.
    pub link_base: String,
    pub copy_raw: bool,
    pub used_quota: i64,
    pub max_quota: i64,
}

/// `AdminFilesView`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminFilesView {
    pub target: AdminFileTarget,
    pub items: Vec<FileItem>,
    pub trash_items: Option<Vec<TrashItem>>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    /// Live files of the account, whatever the filter.
    pub live_total: i64,
}

/// `AdminFilesParams`
#[derive(Debug, Clone, Default)]
pub struct AdminFilesParams {
    pub page: Option<f64>,
    pub per_page: Option<i64>,
    pub search: Option<String>,
    pub sort: Option<String>,
    pub kind: Option<String>,
    pub trashed: bool,
}

type TargetRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<bool>,
    Option<String>,
    Option<bool>,
    Option<i64>,
    Option<i64>,
);

async fn load_target(db: &PgPool, config: &Config, user_id: &str) -> sqlx::Result<Option<AdminFileTarget>> {
    let row: Option<TargetRow> = sqlx::query_as(
        r#"select "id", "name", "email", "user_code", "image", "role", "banned", "preferred_domain", "copy_raw",
                  "used_quota", "max_quota"
           from "user" where "user"."id" = $1 limit 1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(id, name, email, tag, image, role, banned, domain, copy_raw, used, max)| AdminFileTarget {
        avatar: avatar_url(tag.as_deref(), image.as_deref()),
        tag: tag.unwrap_or_default(),
        id,
        name,
        email,
        role,
        banned: banned.unwrap_or(false),
        link_base: user_base_url(config, domain.as_deref()),
        copy_raw: copy_raw.unwrap_or(false),
        used_quota: used.unwrap_or(0),
        max_quota: max.unwrap_or(-1),
    }))
}

/// Audit the visit unless the same administrator looked at the same account
/// in the last ten minutes.
async fn record_visit(db: &PgPool, admin_id: &str, target: &AdminFileTarget, ip: &str) -> sqlx::Result<()> {
    let since = JsDate::from_millis(JsDate::now().millis() - VIEW_DEDUP_MS).unwrap_or_else(JsDate::now);
    let recent: Option<Uuid> = sqlx::query_scalar(
        r#"select "id" from "audit_log"
           where ("audit_log"."user_id" = $1 and "audit_log"."action" = 'admin.user.files_viewed'
                  and "audit_log"."created_at" > $2 and "audit_log"."detail" ->> 'targetUserId' = $3)
           order by "audit_log"."created_at" desc limit 1"#,
    )
    .bind(admin_id)
    .bind(since)
    .bind(&target.id)
    .fetch_optional(db)
    .await?;
    if recent.is_some() {
        return Ok(());
    }
    crate::audit::record_audit(
        db,
        admin_id,
        "admin.user.files_viewed",
        json!({ "by": admin_id, "targetUserId": target.id, "targetEmail": target.email }),
        ip,
    )
    .await;
    Ok(())
}

/// `adminUserFiles(adminId, targetId, params)`: `None` when the account does
/// not exist (no audit row then).
pub async fn admin_user_files(
    db: &PgPool,
    config: &Config,
    admin_id: &str,
    target_id: &str,
    params: &AdminFilesParams,
    ip: &str,
) -> anyhow::Result<Option<AdminFilesView>> {
    let Some(target) = load_target(db, config, target_id).await? else {
        return Ok(None);
    };
    record_visit(db, admin_id, &target, ip).await?;

    let result = list_uploads(
        db,
        &ListParams {
            user_id: target_id.to_string(),
            page: params.page,
            per_page: params.per_page,
            search: params.search.clone(),
            sort: params.sort.clone(),
            kind: params.kind.clone(),
            trashed: params.trashed,
            ..Default::default()
        },
    )
    .await?;
    let owned: Vec<(Uuid, String)> =
        sqlx::query_as(r#"select "id", "name" from "folders" where "folders"."owner_id" = $1"#)
            .bind(target_id)
            .fetch_all(db)
            .await?;
    let folder_names = owned.into_iter().collect();
    let live: i32 = sqlx::query_scalar(
        r#"select count(*)::int from "uploads" where ("uploads"."owner_id" = $1 and "uploads"."deleted_at" is null)"#,
    )
    .bind(target_id)
    .fetch_one(db)
    .await?;

    let items = to_file_items(&result.items, &folder_names);
    let now = JsDate::now().millis();
    let trash_items = params.trashed.then(|| {
        items
            .iter()
            .zip(&result.items)
            .map(|(item, row)| TrashItem { item: item.clone(), days_left: trash_days_left(row.deleted_at, now) })
            .collect()
    });
    Ok(Some(AdminFilesView {
        target,
        items,
        trash_items,
        total: result.total,
        page: result.page,
        per_page: result.per_page,
        live_total: i64::from(live),
    }))
}
