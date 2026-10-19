//! Loader of `app/(auth)/register/page.tsx` (CONTRACT 6.5).
//!
//! A closed registration renders the "closed" card rather than a 404, so the
//! link from /login never dead-ends. Only the three booleans leave the
//! server: never the registration key or its ciphertext.

use serde::Serialize;

use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageResult};
use crate::registration::registration_page_state;

/// `RegistrationPageState` as the page receives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterState {
    pub bootstrap: bool,
    pub open: bool,
    pub key_required: bool,
}

/// `RegisterPageProps`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RegisterProps {
    pub state: RegisterState,
}

pub async fn load(ctx: PageCtx) -> PageResult {
    let config = &ctx.state.config;
    let s = registration_page_state(&ctx.state.db, &config.auth.secret, config.auth.registration_enabled).await?;
    let state = RegisterState { bootstrap: s.bootstrap, open: s.open, key_required: s.key_required };
    PageData::new(&RegisterProps { state }, PageMeta::titled(ctx.t("auth").t("signUp")))
}
