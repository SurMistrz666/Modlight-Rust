//! Loader of `app/u/[userCode]/page.tsx` (CONTRACT 6.22): an account's public
//! gallery, only when its owner turned the public profile on, dressed in the
//! owner's skin and wallpaper. The first displayable image becomes the card's
//! picture.

use serde::Serialize;

use crate::avatar::avatar_url;
use crate::background::{BackgroundStyle, background_style};
use crate::media::mime::is_displayable_image;
use crate::media::urls::{raw_url, user_base_url};
use crate::pages::meta::{Alternates, Field, OgMedia, OpenGraph, PageMeta, Title, Twitter};
use crate::pages::{PageCtx, PageData, PageError, PageResult};

/// At most this many items are listed.
const ITEM_LIMIT: i64 = 200;

/// `PublicTileItem` (components/public/types.ts)
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PublicTileItem {
    pub id: uuid::Uuid,
    pub code: String,
    pub ext: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub blurhash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileOwner {
    name: String,
    user_code: String,
    theme: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileProps {
    owner: ProfileOwner,
    items: Vec<PublicTileItem>,
    owner_base: String,
    owner_avatar: Option<String>,
    background: Option<BackgroundStyle>,
    /// Not upstream: the owner's file vault (docs/ENCRYPTION.md); absent for
    /// an account without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    vault: Option<crate::vault::access::GalleryVault>,
}

type GalleryUser =
    (String, String, Option<String>, Option<String>, Option<String>, Option<bool>, Option<String>, i32, Option<String>);

struct Gallery {
    owner_id: String,
    name: String,
    user_code: String,
    theme: Option<String>,
    preferred_domain: Option<String>,
    background_key: Option<String>,
    background_dim: i32,
    image: Option<String>,
    items: Vec<PublicTileItem>,
}

/// `loadGallery(userCode)`: the user by tag (no alias fallback), only when the
/// public profile is on, and their PUBLIC, live, unprotected file uploads.
async fn load_gallery(ctx: &PageCtx, user_code: &str) -> Result<Option<Gallery>, PageError> {
    let owner: Option<GalleryUser> = sqlx::query_as(
        r#"select "id", "name", "user_code", "theme", "preferred_domain", "public_profile", "background_key", "background_dim", "image"
           from "user" where "user"."user_code" = $1 limit 1"#,
    )
    .bind(user_code)
    .fetch_optional(&ctx.state.db)
    .await?;
    let Some((id, name, Some(code), theme, preferred_domain, public_profile, background_key, background_dim, image)) =
        owner
    else {
        return Ok(None);
    };
    // Opt-in, and it has to be checked HERE: publishing someone's whole library
    // is a decision they make, not one the absence of a check makes for them.
    if public_profile != Some(true) {
        return Ok(None);
    }
    // The same gates the public folder gallery applies: a filename is
    // information too.
    let items: Vec<PublicTileItem> = sqlx::query_as(
        r#"select "id", "code", "ext", "filename", "mime_type", "size", "blurhash" from "uploads"
           where ("uploads"."owner_id" = $1 and "uploads"."published" = true and "uploads"."deleted_at" is null
                  and "uploads"."kind" = 'file' and "uploads"."password_hash" is null
                  and ("uploads"."expires_at" is null or "uploads"."expires_at" > now())
                  and ("uploads"."max_views" is null or "uploads"."views" < "uploads"."max_views"))
           order by "uploads"."created_at" desc limit $2"#,
    )
    .bind(&id)
    .bind(ITEM_LIMIT)
    .fetch_all(&ctx.state.db)
    .await?;
    Ok(Some(Gallery {
        owner_id: id,
        name,
        user_code: code,
        theme,
        preferred_domain,
        background_key,
        background_dim,
        image,
        items,
    }))
}

/// The card both public galleries emit (`openGraph.type` differs).
pub fn gallery_meta(
    ctx: &PageCtx,
    title: String,
    description: String,
    url: String,
    og_type: &str,
    og_image: Option<String>,
) -> PageMeta {
    let normalized = og_image.as_deref().map(|u| url::Url::parse(u).map(|p| p.to_string()).unwrap_or(u.to_string()));
    PageMeta {
        title: Title::Templated(title.clone()),
        description: Field::Set(description.clone()),
        alternates: Some(Alternates { canonical: Some(url.clone()), types: Vec::new() }),
        open_graph: Some(OpenGraph {
            og_type: Some(og_type.to_string()),
            title: Some(title.clone()),
            description: Some(description.clone()),
            url: Some(url),
            site_name: Some(ctx.state.config.embed_brand.clone()),
            images: normalized
                .iter()
                .map(|u| OgMedia {
                    url: u.clone(),
                    props: vec![
                        ("width".into(), "1200".into()),
                        ("height".into(), "630".into()),
                        ("alt".into(), title.clone()),
                    ],
                })
                .collect(),
            videos: Vec::new(),
        }),
        twitter: Some(Twitter {
            card: Some(if og_image.is_some() { "summary_large_image" } else { "summary" }.to_string()),
            title: Some(title),
            description: Some(description),
            images: normalized.map(|u| vec![u]),
        }),
        ..Default::default()
    }
}

/// `/u/[userCode]`
pub async fn load(ctx: PageCtx) -> PageResult {
    // The page's spelling (CONTRACT 5.3); tags are [a-z0-9-], where it is the
    // decoded one too.
    let user_code = crate::http::cookies::encode_uri_component(ctx.param("userCode").unwrap_or(""));
    let Some(g) = load_gallery(&ctx, &user_code).await? else {
        return Err(PageError::NotFound);
    };
    let t = ctx.t("public");
    let owner_base = user_base_url(&ctx.state.config, g.preferred_domain.as_deref());

    // generateMetadata: neither the title nor the description carries the brand.
    let title = t.t_with("profile.metaTitle", &[("name", g.name.as_str().into())]);
    let description = t.t_with("profile.metaDescription", &[("tag", g.user_code.as_str().into())]);
    let url = format!("{owner_base}/u/{}", g.user_code);
    // Not upstream: an encrypted account's gallery asks for the file password
    // and never offers a picture to an unfurler.
    let vault = crate::vault::access::gallery_vault(&ctx.state, &ctx.headers, &g.owner_id, &g.user_code).await?;
    let og_image = g
        .items
        .iter()
        .filter(|_| vault.is_none())
        .find(|i| is_displayable_image(Some(&i.mime_type), Some(&i.ext)))
        .map(|i| format!("{}?w=1200&h=630", raw_url(&g.user_code, &i.code, &i.ext, &owner_base)));
    let meta = gallery_meta(&ctx, title, description, url, "profile", og_image);

    let background = background_style(Some(&g.user_code), g.background_key.as_deref(), Some(g.background_dim));
    let locked = vault.as_ref().is_some_and(|v| v.locked);
    let props = ProfileProps {
        owner_avatar: avatar_url(Some(&g.user_code), g.image.as_deref()),
        owner: ProfileOwner { name: g.name, user_code: g.user_code, theme: g.theme.clone() },
        items: if locked { Vec::new() } else { g.items },
        owner_base,
        background: background.clone(),
        vault,
    };
    Ok(PageData::new(&props, meta)?.with_skin(g.theme.as_deref(), background))
}
