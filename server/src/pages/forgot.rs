//! Loader of `app/(auth)/forgot/page.tsx` (CONTRACT 6.6).
//!
//! The form is only useful when mail can actually be sent; without it the page
//! says so instead of accepting an address and silently doing nothing.

use serde::Serialize;

use crate::mail_settings::load_mail_settings;
use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageResult};

/// `ForgotPageProps`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgotProps {
    pub mail_ready: bool,
}

pub async fn load(ctx: PageCtx) -> PageResult {
    let mail_ready = load_mail_settings(&ctx.state, false).await?.ready;
    PageData::new(&ForgotProps { mail_ready }, PageMeta::titled(ctx.t("auth").t("forgot.title")))
}
