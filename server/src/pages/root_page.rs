//! Loader of `app/page.tsx` (CONTRACT 6.1).
//!
//! Modlight is a closed tool: accounts are created by an administrator, not by
//! signing up. So the root URL is not a marketing page: it goes straight to
//! where a visitor actually needs to be.
//!
//! The marketing page still exists at /welcome for anyone who wants to link to
//! it.

use crate::pages::{PageCtx, PageError, PageResult};

pub async fn load(ctx: PageCtx) -> PageResult {
    // A session lookup failure is an error (no catch upstream).
    let session = ctx.get_session().await?;
    Err(PageError::redirect(if session.is_some() { "/dashboard" } else { "/login" }))
}
