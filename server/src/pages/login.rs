//! Loader of `app/(auth)/login/page.tsx` (CONTRACT 6.4).
//!
//! No gate: a signed-in visitor still sees the form. The `next` parameter was
//! sanitised on the server upstream (`/x` but not `//x`, else `/dashboard`);
//! the SPA does the same in the browser, so the server reads no query here.
//! The only server-side fact is whether the "create an account" link is shown.

use serde::Serialize;

use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageResult};
use crate::registration::registration_page_state;

/// `{ open }` of `registrationPageState()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RegistrationOpen {
    pub open: bool,
}

/// `LoginPageProps`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LoginProps {
    pub registration: RegistrationOpen,
}

pub async fn load(ctx: PageCtx) -> PageResult {
    let config = &ctx.state.config;
    let state = registration_page_state(&ctx.state.db, &config.auth.secret, config.auth.registration_enabled).await?;
    let meta = PageMeta::titled(ctx.t("auth").t("login.title"));
    PageData::new(&LoginProps { registration: RegistrationOpen { open: state.open } }, meta)
}
