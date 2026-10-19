//! Loader of `app/(app)/dashboard/links/page.tsx` (CONTRACT 6.15): the
//! account's short links (expired ones included), newest first.

use serde::Serialize;

use crate::data::user_prefs::{get_user_prefs, may_create_root_short_link};
use crate::db::models::ShortLink;
use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageResult};
use crate::shortlinks::{LinkView, list_short_links};

/// The page's mapping of a row (no password hash, no owner id).
pub fn link_view(l: &ShortLink) -> LinkView {
    LinkView {
        id: l.id,
        code: l.code.clone(),
        target: l.target.clone(),
        clicks: l.clicks,
        has_password: l.password_hash.as_deref().is_some_and(|h| !h.is_empty()),
        last_used_at: l.last_used_at.map(|d| d.to_iso_string()),
        expires_at: l.expires_at.map(|d| d.to_iso_string()),
        created_at: l.created_at.to_iso_string(),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LinksProps {
    links: Vec<LinkView>,
    user_code: Option<String>,
    can_use_root: bool,
}

pub async fn load(ctx: PageCtx) -> PageResult {
    let session = ctx.require_user().await?;
    let db = &ctx.state.db;
    let (rows, prefs) = tokio::try_join!(list_short_links(db, &session.user.id), get_user_prefs(db, &session.user.id))?;
    let props = LinksProps {
        links: rows.iter().map(link_view).collect(),
        user_code: prefs.as_ref().and_then(|p| p.user_code.clone()),
        can_use_root: prefs
            .as_ref()
            .is_some_and(|p| may_create_root_short_link(p.role.as_deref(), p.allow_root_short_links)),
    };
    PageData::new(&props, PageMeta::titled(ctx.t("links").t("metaTitle")))
}
