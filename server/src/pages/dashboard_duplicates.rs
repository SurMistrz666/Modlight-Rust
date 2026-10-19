//! Loader of `app/(app)/dashboard/duplicates/page.tsx` (CONTRACT 6.14): the
//! account's duplicate scan.

use serde::Serialize;

use crate::data::duplicates::{DuplicateScan, scan_duplicates};
use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageResult};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DuplicatesProps {
    scan: DuplicateScan,
    user_code: String,
}

pub async fn load(ctx: PageCtx) -> PageResult {
    let session = ctx.require_user().await?;
    let scan = scan_duplicates(&ctx.state.db, &ctx.state.config, &session.user.id).await?;
    let title = ctx.t("profile").t("duplicates.page.metaTitle");
    PageData::new(
        &DuplicatesProps { scan, user_code: session.user.user_code.clone().unwrap_or_default() },
        PageMeta::titled(title),
    )
}
