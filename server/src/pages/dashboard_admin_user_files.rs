//! Loader of `app/(app)/dashboard/admin/users/[id]/files/page.tsx` (CONTRACT
//! 6.21): an administrator browsing another account's files. Unknown account:
//! `notFound()`. Every render of an existing account audits the visit
//! (deduplicated over ten minutes).

use serde::Serialize;

use crate::data::admin_files::{AdminFilesParams, AdminFilesView, admin_user_files};
use crate::pages::dashboard::{kind_param, page_param, per_page_param, search_param, sort_param, view_param};
use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageError, PageResult};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AdminUserFilesProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    search: Option<String>,
    sort: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    view: &'static str,
    trashed: bool,
    data: AdminFilesView,
}

pub async fn load(ctx: PageCtx) -> PageResult {
    let session = ctx.require_admin().await?;
    let id = ctx.param("id").unwrap_or("").to_string();
    let sp = &ctx.search;
    let search = search_param(sp);
    let sort = sort_param(sp);
    let kind = kind_param(sp);
    let view = view_param(sp);
    let trashed = sp.get("trashed") == Some("1");
    let params = AdminFilesParams {
        page: Some(page_param(sp.get("page"))),
        per_page: Some(per_page_param(sp.get("per"))),
        search: search.clone(),
        sort: Some(sort.clone()),
        kind: kind.clone(),
        trashed,
    };
    let data = admin_user_files(&ctx.state.db, &ctx.state.config, &session.user.id, &id, &params, &ctx.client_ip())
        .await?
        .ok_or(PageError::NotFound)?;
    let title = ctx.t("admin").t("files.pageTitle");
    PageData::new(&AdminUserFilesProps { search, sort, kind, view, trashed, data }, PageMeta::titled(title))
}
