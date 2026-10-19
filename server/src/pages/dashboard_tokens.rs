//! Loader of `app/(app)/dashboard/tokens/page.tsx`: Integrations (CONTRACT 6.19).
//!
//! Upstream: `requireUser()`, then `myTokens()` and `myUrlPrefs()` (each runs
//! `requireUser()` again; the session read is memoised per request here), and
//! `<AppCard>`, which awaited `currentRelease()` itself.
//!
//! Everything generated on this page (the .sxcu RequestURL, the curl snippets,
//! the bash uploader) points at the hostname the user chose in their profile,
//! so what ShareX posts to matches the links it gets back: `appUrl` is
//! `myUrlPrefs().baseUrl` = `userBaseUrl(user.preferred_domain)`.
//!
//! Never sent: `token_hash`, `user_id` (upstream strips them before the client
//! component).

use serde::Serialize;
use uuid::Uuid;

use crate::api_token::list_api_tokens;
use crate::media::urls::user_base_url;
use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageResult};
use crate::releases::{ReleaseManifest, current_release};

/// One row of `<TokenManager tokens>`: `myTokens()` without the secrets.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenView {
    pub id: Uuid,
    pub name: String,
    pub scope: String,
    pub preview: String,
    /// `createdAt.toISOString()`
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
}

/// `{userCode, name, email}` of the session user.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokensUser {
    pub user_code: Option<String>,
    pub name: Option<String>,
    pub email: String,
}

/// `TokensPageProps`
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokensProps {
    pub tokens: Vec<TokenView>,
    pub app_url: String,
    pub username: String,
    pub user: TokensUser,
    pub release: Option<ReleaseManifest>,
}

/// A friendly handle for naming the downloaded .sxcu file:
/// `name?.split(" ")[0] || email.split("@")[0] || userCode || "user"`.
pub fn friendly_handle(name: &str, email: &str, user_code: Option<&str>) -> String {
    let first_word = name.split(' ').next().unwrap_or("");
    if !first_word.is_empty() {
        return first_word.to_string();
    }
    let local_part = email.split('@').next().unwrap_or("");
    if !local_part.is_empty() {
        return local_part.to_string();
    }
    match user_code {
        Some(code) if !code.is_empty() => code.to_string(),
        _ => "user".to_string(),
    }
}

/// `myUrlPrefs().preferredDomain`: `row?.preferredDomain ?? null`.
async fn preferred_domain(ctx: &PageCtx, user_id: &str) -> anyhow::Result<Option<String>> {
    let row: Option<Option<String>> =
        sqlx::query_scalar("SELECT \"preferred_domain\" FROM \"user\" WHERE \"user\".\"id\" = $1 LIMIT 1")
            .bind(user_id)
            .fetch_optional(&ctx.state.db)
            .await?;
    Ok(row.flatten())
}

pub async fn load(ctx: PageCtx) -> PageResult {
    let session = ctx.require_user().await?;
    let user = &session.user;

    // Strip secrets: never ship tokenHash to the client.
    let tokens = list_api_tokens(&ctx.state.db, &user.id)
        .await?
        .into_iter()
        .map(|t| TokenView {
            id: t.id,
            name: t.name,
            scope: t.scope,
            preview: t.preview,
            created_at: t.created_at.to_iso_string(),
            last_used_at: t.last_used_at.map(|d| d.to_iso_string()),
            revoked_at: t.revoked_at.map(|d| d.to_iso_string()),
        })
        .collect();
    let app_url = user_base_url(&ctx.state.config, preferred_domain(&ctx, &user.id).await?.as_deref());
    let release = current_release(&ctx.state.db, &ctx.state.config).await?;

    let props = TokensProps {
        tokens,
        app_url,
        username: friendly_handle(&user.name, &user.email, user.user_code.as_deref()),
        user: TokensUser {
            user_code: user.user_code.clone(),
            name: Some(user.name.clone()),
            email: user.email.clone(),
        },
        release,
    };
    PageData::new(&props, PageMeta::titled("Integrations"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_friendly_handle_falls_back_in_upstream_order() {
        assert_eq!(friendly_handle("Alicja Nowak", "a@x.test", Some("al")), "Alicja");
        // `split(" ")` splits on U+0020 only, and a leading space leaves "" first.
        assert_eq!(friendly_handle(" Alicja", "ala@x.test", Some("al")), "ala");
        assert_eq!(friendly_handle("Alicja\u{a0}Nowak", "a@x.test", None), "Alicja\u{a0}Nowak");
        assert_eq!(friendly_handle("", "@x.test", Some("al")), "al");
        assert_eq!(friendly_handle("", "@x.test", Some("")), "user");
        assert_eq!(friendly_handle("", "", None), "user");
    }
}
