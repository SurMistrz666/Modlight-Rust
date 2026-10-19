//! Loader of `app/(app)/dashboard/upload/page.tsx` (CONTRACT 6.17): no data,
//! only metadata (the uploaders are client-side).

use crate::pages::meta::{Field, PageMeta};
use crate::pages::{PageCtx, PageData, PageResult};

pub async fn load(ctx: PageCtx) -> PageResult {
    let t = ctx.t("upload");
    let meta = PageMeta { description: Field::Set(t.t("page.metaDescription")), ..PageMeta::titled(t.t("page.title")) };
    PageData::new(&serde_json::json!({}), meta)
}
