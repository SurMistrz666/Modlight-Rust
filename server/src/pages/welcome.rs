//! Loader of `app/welcome/page.tsx`: the marketing page (CONTRACT 6.2).
//!
//! `appName` / `appUrl` come from the root layout's config; the page itself
//! only needs to know whether the visitor is signed in (the call to action
//! says "Open the panel" instead of "Sign in"). No metadata of its own: the
//! root default title.

use serde::Serialize;

use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageResult};

/// `WelcomePageProps`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WelcomeProps {
    pub is_authed: bool,
}

pub async fn load(ctx: PageCtx) -> PageResult {
    let is_authed = ctx.get_session().await?.is_some();
    PageData::new(&WelcomeProps { is_authed }, PageMeta::default())
}
