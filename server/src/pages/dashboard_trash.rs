//! Loader of `app/(app)/dashboard/trash/page.tsx` (CONTRACT 6.13): the
//! newest 120 trashed files with their purge countdown, and the totals of
//! the WHOLE trash (the list stops at 120).

use serde::Serialize;

use crate::data::uploads::{FileItem, ListParams, file_item, list_uploads};
use crate::data::user_prefs::get_user_prefs;
use crate::db::types::JsDate;
use crate::maintenance::trash::trash_days_left;
use crate::media::urls::user_base_url;
use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageResult};
use crate::uploads::{TrashTotals, UploadActor, trash_totals_for};

/// `TrashItem`: a FileItem with its countdown.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashItem {
    #[serde(flatten)]
    pub item: FileItem,
    pub days_left: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrashProps {
    tag: String,
    link_base: String,
    totals: Option<TrashTotals>,
    items: Vec<TrashItem>,
}

pub async fn load(ctx: PageCtx) -> PageResult {
    let session = ctx.require_user().await?;
    let user = &session.user;
    let db = &ctx.state.db;
    let tag = user.user_code.clone().unwrap_or_default();
    let actor =
        UploadActor { user_id: user.id.clone(), email: Some(user.email.clone()), is_admin: false, ip: ctx.client_ip() };

    let (result, prefs, totals) = tokio::try_join!(
        async {
            list_uploads(
                db,
                &ListParams {
                    user_id: user.id.clone(),
                    trashed: true,
                    per_page: Some(120),
                    sort: Some("newest".into()),
                    ..Default::default()
                },
            )
            .await
            .map_err(anyhow::Error::from)
        },
        async { get_user_prefs(db, &user.id).await.map_err(anyhow::Error::from) },
        async {
            if tag.is_empty() {
                return Ok(None);
            }
            trash_totals_for(db, &actor, &tag).await.map(Some).map_err(|e| anyhow::anyhow!("{e}"))
        },
    )?;

    let link_base = user_base_url(&ctx.state.config, prefs.as_ref().and_then(|p| p.preferred_domain.as_deref()));
    // One clock for the whole page.
    let now = JsDate::now().millis();
    let items = result
        .items
        .iter()
        .map(|u| TrashItem { item: file_item(u, None), days_left: trash_days_left(u.deleted_at, now) })
        .collect();
    PageData::new(&TrashProps { tag, link_base, totals, items }, PageMeta::default())
}
