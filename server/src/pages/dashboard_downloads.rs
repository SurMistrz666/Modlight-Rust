//! Not upstream: the loader of `/dashboard/downloads`, Downloads (PL "Do
//! pobrania"): the desktop app's builds for macOS and Windows
//! (`crate::downloads`, docs/SKELETON.md section 9, item 45, CONTRACT.md 6.28).
//!
//! Props are exactly what `GET /api/downloads` answers the same account
//! (`DownloadsList`), so the page renders on its first paint and the SPA
//! types one shape. Behind the `(app)` layout's sign-in like every dashboard
//! page; an administrator also gets `publishToken` (whether
//! `RELEASE_PUBLISH_TOKEN` is set) for the publishing guide.

use crate::downloads;
use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageResult};

pub async fn load(ctx: PageCtx) -> PageResult {
    let session = ctx.require_user().await?;
    let config = &ctx.state.config;
    let props = downloads::list(&ctx.state.db, session.is_admin(), config.release_publish_token.is_set()).await?;
    let title = ctx.t("downloads").t("metaTitle");
    PageData::new(&props, PageMeta::titled(title))
}
