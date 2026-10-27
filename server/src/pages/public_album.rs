//! Loader of `app/a/[userCode]/[slug]/page.tsx` (CONTRACT 6.23): a shared
//! folder's public gallery (`getPublicFolder`, retired tags included), in the
//! owner's skin and wallpaper, with a card whose picture is the newest
//! displayable image.

use serde::Serialize;

use crate::background::{BackgroundStyle, background_style};
use crate::data::folders::get_public_folder;
use crate::i18n::format::Arg;
use crate::media::mime::is_displayable_image;
use crate::media::urls::{raw_url, user_base_url};
use crate::pages::public_profile::{PublicTileItem, gallery_meta};
use crate::pages::{PageCtx, PageData, PageError, PageResult};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Album {
    name: String,
    slug: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AlbumOwner {
    name: Option<String>,
    theme: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AlbumProps {
    album: Album,
    owner: AlbumOwner,
    owner_code: String,
    owner_base: String,
    background: Option<BackgroundStyle>,
    tiles: Vec<PublicTileItem>,
    /// Not upstream: the owner's file vault (docs/ENCRYPTION.md); absent for
    /// an account without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    vault: Option<crate::vault::access::GalleryVault>,
}

/// `/a/[userCode]/[slug]`
pub async fn load(ctx: PageCtx) -> PageResult {
    // The page's spelling (CONTRACT 5.3).
    let user_code = crate::http::cookies::encode_uri_component(ctx.param("userCode").unwrap_or(""));
    let slug = crate::http::cookies::encode_uri_component(ctx.param("slug").unwrap_or(""));
    let Some(data) = get_public_folder(&ctx.state.db, &user_code, &slug).await? else {
        return Err(PageError::NotFound);
    };
    let t = ctx.t("public");
    let owner = &data.owner;
    let owner_code = owner.user_code.clone().unwrap_or_else(|| user_code.clone());
    let owner_base = user_base_url(&ctx.state.config, owner.preferred_domain.as_deref());

    // generateMetadata: no brand in the description; og:site_name is the one
    // place the card names us.
    let title = data.folder.name.clone();
    let owner_name = if owner.name.is_empty() { t.t("album.anonymous") } else { owner.name.clone() };
    let description = t.t_with(
        "album.metaDescription",
        &[("count", Arg::Num(data.items.len() as f64)), ("owner", owner_name.as_str().into())],
    );
    let url = format!("{owner_base}/a/{owner_code}/{}", data.folder.slug);
    // Not upstream: an encrypted account's album asks for the file password
    // and never offers a picture to an unfurler.
    let vault = crate::vault::access::gallery_vault(&ctx.state, &ctx.headers, &owner.id, &owner_code).await?;
    let locked = vault.as_ref().is_some_and(|v| v.locked);
    let og_image = data
        .items
        .iter()
        .filter(|_| vault.is_none())
        .find(|i| is_displayable_image(Some(&i.mime_type), Some(&i.ext)))
        .map(|i| format!("{}?w=1200&h=630", raw_url(&owner_code, &i.code, &i.ext, &owner_base)));
    let meta = gallery_meta(&ctx, title, description, url, "website", og_image);

    let background =
        background_style(owner.user_code.as_deref(), owner.background_key.as_deref(), Some(owner.background_dim));
    // Only what a tile needs crosses into the client bundle.
    let tiles = data
        .items
        .iter()
        .filter(|_| !locked)
        .map(|i| PublicTileItem {
            id: i.id,
            code: i.code.clone(),
            ext: i.ext.clone(),
            filename: i.filename.clone(),
            mime_type: i.mime_type.clone(),
            size: i.size,
            blurhash: i.blurhash.clone(),
        })
        .collect();
    let props = AlbumProps {
        album: Album { name: data.folder.name.clone(), slug: data.folder.slug.clone() },
        owner: AlbumOwner { name: Some(owner.name.clone()), theme: owner.theme.clone() },
        owner_code,
        owner_base,
        background: background.clone(),
        tiles,
        vault,
    };
    Ok(PageData::new(&props, meta)?.with_skin(owner.theme.as_deref(), background))
}
