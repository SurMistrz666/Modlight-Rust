//! Loader of `app/(app)/dashboard/paste/page.tsx` (CONTRACT 6.16): no data,
//! only metadata (the editor is client-side).

use crate::pages::meta::{Field, PageMeta};
use crate::pages::{PageCtx, PageData, PageResult};

pub async fn load(ctx: PageCtx) -> PageResult {
    let t = ctx.t("paste");
    let meta = PageMeta { description: Field::Set(t.t("metaDescription")), ..PageMeta::titled(t.t("metaTitle")) };
    PageData::new(&serde_json::json!({}), meta)
}
