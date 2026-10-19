//! Loaders of `app/s/[code]/unlock/page.tsx` and
//! `app/s/[code]/[name]/unlock/page.tsx` (CONTRACT 6.24, 6.25): the password
//! gate of a short link, dressed in its owner's theme and wallpaper. No
//! session is read; expiry is not checked here.

use serde::Serialize;

use crate::background::{BackgroundStyle, background_style};
use crate::naming::scoped_short_code;
use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageError, PageResult};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UnlockProps {
    code: String,
    owner_theme: Option<String>,
    background: Option<BackgroundStyle>,
}

/// `generateMetadata`: the same for every outcome.
fn meta(ctx: &PageCtx) -> PageMeta {
    PageMeta::titled(ctx.t("public").t("shortlink.metaTitle")).noindex()
}

async fn gate(ctx: &PageCtx, code: String) -> PageResult {
    type Row = (Option<String>, Option<String>, Option<String>, Option<String>, Option<i32>);
    let row: Option<Row> = sqlx::query_as(
        r#"select "short_links"."password_hash", "user"."theme", "user"."user_code", "user"."background_key", "user"."background_dim"
           from "short_links" left join "user" on "user"."id" = "short_links"."owner_id"
           where "short_links"."code" = $1 limit 1"#,
    )
    .bind(&code)
    .fetch_optional(&ctx.state.db)
    .await?;
    let Some((hash, theme, user_code, background_key, background_dim)) = row else {
        return Err(PageError::NotFound);
    };
    if hash.as_deref().is_none_or(str::is_empty) {
        return Err(PageError::redirect(format!("/s/{code}")));
    }
    let background = background_style(user_code.as_deref(), background_key.as_deref(), background_dim);
    PageData::new(&UnlockProps { code, owner_theme: theme, background }, meta(ctx))
}

/// `/s/[code]/unlock`
pub async fn load(ctx: PageCtx) -> PageResult {
    let code = ctx.param("code").unwrap_or("").to_string();
    gate(&ctx, code).await
}

/// `/s/[code]/[name]/unlock`: the stored code is `{tag}/{name}`.
pub async fn load_scoped(ctx: PageCtx) -> PageResult {
    let code = scoped_short_code(ctx.param("code"), ctx.param("name").unwrap_or(""));
    gate(&ctx, code).await
}
