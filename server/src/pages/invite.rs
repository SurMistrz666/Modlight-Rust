//! Loader of `app/(auth)/invite/[token]/page.tsx` (CONTRACT 6.9): the landing
//! page for the link an administrator mails when creating an account.
//!
//! The token is resolved before anything is rendered, so an expired or
//! already used invitation says so instead of presenting a form that cannot
//! work. It is only READ here (an expired one, or one whose account is gone,
//! is deleted by `readInvite`); it is consumed when the password is submitted.

use serde::Serialize;

use crate::actions::invite::{InvitePreview, preview_invite_for};
use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageResult};

/// `InvitePageProps`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InviteProps {
    pub token: String,
    pub preview: Option<InvitePreview>,
}

pub async fn load(ctx: PageCtx) -> PageResult {
    // `params.token` in the page spelling: encodeURIComponent(decoded segment).
    let token = crate::http::cookies::encode_uri_component(ctx.param("token").unwrap_or_default());
    let preview = preview_invite_for(&ctx.state.db, &token).await?;
    PageData::new(&InviteProps { token, preview }, PageMeta::titled(ctx.t("auth").t("invite.title")))
}
