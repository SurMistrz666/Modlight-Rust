//! Loader of `app/(auth)/layout.tsx`: the centered, branded auth shell.
//!
//! Upstream has no gate here and reads no data: it renders `config.appName`
//! (the root layout's `config`), the `auth.layout.*` messages and the current
//! year, all of which the SPA has. The payload is the empty object, which the
//! SPA needs to be truthy to load the layout at all (CONTRACT 5.4, 5.9).

use serde_json::json;

use crate::pages::{LayoutResult, PageCtx};

pub async fn load(_ctx: PageCtx) -> LayoutResult {
    Ok(json!({}))
}
