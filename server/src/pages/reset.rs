//! Loader of `app/(auth)/reset/page.tsx` (CONTRACT 6.7).
//!
//! Better Auth's reset link lands here with `?token=` or
//! `?error=INVALID_TOKEN`; the SPA reads both in the browser, so the server
//! has nothing to load beyond the title.

use serde_json::json;

use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageResult};

pub async fn load(ctx: PageCtx) -> PageResult {
    PageData::new(&json!({}), PageMeta::titled(ctx.t("auth").t("reset.title")))
}
