//! Loader of `app/(auth)/2fa/page.tsx` (CONTRACT 6.8).
//!
//! No gate: the pending two-factor state lives in Better Auth's own cookie and
//! is checked by the verify endpoints. `next` is sanitised in the browser.

use serde_json::json;

use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageResult};

pub async fn load(ctx: PageCtx) -> PageResult {
    PageData::new(&json!({}), PageMeta::titled(ctx.t("auth").t("twoFactor.title")))
}
