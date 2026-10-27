//! Loaders of `app/[userCode]/[mediaCode]/page.tsx` (the public media viewer,
//! CONTRACT 6.27) and `app/[userCode]/page.tsx` (tagless media links
//! `/{code}.{ext}`, CONTRACT 6.26): the password gate, the encrypted viewer or
//! the media viewer, and `generateMetadata`'s Open Graph / Twitter / oEmbed
//! card (`media::embed`).
//!
//! Security fix 1 on this page (see `media::serve::ViewBudget`): a counted view
//! of a file with a view cap mints the view grant that the page's own
//! `<img>`/`<video>` and the player's seeks present to `/raw`; a preview bot,
//! which is not counted (as upstream), is not handed the content of a capped
//! file either (no inlined text, no render ticket), and the card of a capped
//! text file carries no excerpt of it.

use serde::Serialize;

use crate::background::BackgroundStyle;
use crate::crypto::envelope::is_encrypted_upload;
use crate::data::uploads::{FileItem, to_file_items};
use crate::data::user_prefs::{get_owner_password_version, resolve_tagless_owner_tag};
use crate::db::models::Upload;
use crate::db::types::JsDate;
use crate::http::cookies::encode_uri_component;
use crate::media::bots::{is_direct_media_bot, is_preview_bot};
use crate::media::embed::{EmbedContext, EmbedMedia, build_embed_plan, embed_metadata};
use crate::media::mime::{MediaKind, media_kind};
use crate::media::part_a::{
    InlineTextPlan, TIMELINE_TRACK_ACTION, can_timeline_preview, diagnose_playback, inline_text_plan, playback_source,
    shows_source,
};
use crate::media::paths::code_from_media_code;
use crate::media::render_ticket::{can_render_inline, mint_render_ticket};
use crate::media::serve::{Lookup, MediaLookup, consume_view, lookup_media, view_grant_cookie, viewer_relation};
use crate::media::urls::{download_url, media_action_path, public_url, raw_url, render_path, user_base_url};
use crate::naming::is_reserved_tag;
use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageError, PageResult, SearchParams};
use crate::password::{MediaUnlockSubject, is_media_unlocked};
use crate::rpc::tagged::Num;

/// Discord (and friends) stop proxying media somewhere around this size.
const EMBED_PROXY_LIMIT: i64 = 8 * 1024 * 1024;

/// Width asked of the still shown before a video plays: one of the sizes /raw
/// renders, large enough to fill the stage on a laptop.
const POSTER_WIDTH: u32 = 800;

/// The snippet read for a text file's card.
const SNIPPET_BYTES: u64 = 8 * 1024;

// ── Props ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlaybackOut {
    faults: Vec<&'static str>,
    edge: Num,
    bits_per_pixel: Option<Num>,
    starvation_factor: Option<Num>,
    repairable: bool,
    repair_edge: Num,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlaybackRepair {
    item: FileItem,
    user_code: String,
    app_url: String,
}

/// `MediaViewerProps`, every key present, in upstream's order.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ViewerProps {
    filename: String,
    mime_type: String,
    ext: String,
    size: i64,
    views: i32,
    downloads: i32,
    created_at: JsDate,
    width: Option<i32>,
    height: Option<i32>,
    frames: Option<i32>,
    public_url: String,
    raw_url: String,
    download_url: String,
    copy_raw: bool,
    can_encode: bool,
    sign_in_url: Option<String>,
    text_content: Option<String>,
    text_unavailable: Option<&'static str>,
    render_path: Option<String>,
    timeline_track: Option<String>,
    poster_url: Option<String>,
    playback: Option<PlaybackOut>,
    playback_repair: Option<PlaybackRepair>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "view", rename_all = "camelCase")]
enum View {
    #[serde(rename = "gate", rename_all = "camelCase")]
    Gate { media_id: String, on_default_password: bool },
    #[serde(rename = "encrypted", rename_all = "camelCase")]
    Encrypted { raw_path: String, cipher_size: i64, created_at: String, views: i32, downloads: i32 },
    #[serde(rename = "media")]
    Media { viewer: Box<ViewerProps> },
    /// Not upstream: the owner's file vault asks for the file password
    /// (docs/ENCRYPTION.md). Nothing about the file: not even its name.
    #[serde(rename = "vault", rename_all = "camelCase")]
    Vault { media_id: String },
}

/// `MediaPageProps`: the owner's look, then the view.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaPageProps {
    owner_theme: String,
    owner_background: Option<BackgroundStyle>,
    #[serde(flatten)]
    view: View,
}

// ── Metadata ────────────────────────────────────────────────────────────────

/// `readSnippet(storageKey, size)`: the first few kilobytes, never throws.
async fn read_snippet(ctx: &PageCtx, storage_key: &str, size: i64) -> Option<String> {
    let storage = ctx.state.storage.get().ok()?;
    let bytes = if size as u64 <= SNIPPET_BYTES {
        storage.read_buffer(storage_key).await.ok()?
    } else {
        use futures_util::StreamExt;
        let mut stream = storage
            .read_stream(storage_key, Some(crate::storage::ByteRange { start: 0, end: SNIPPET_BYTES - 1 }))
            .await
            .ok()?;
        let mut out = Vec::new();
        while let Some(chunk) = stream.next().await {
            out.extend_from_slice(&chunk.ok()?);
        }
        out.into()
    };
    // Buffer#toString("utf8"): invalid bytes become U+FFFD, a BOM is kept.
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// `generateMetadata`, computed from the row the page looked up (CONTRACT
/// 6.27: before this request's view is counted, so the view that spends the
/// last of a budget still gets its card).
async fn media_metadata(ctx: &PageCtx, found: &MediaLookup) -> PageMeta {
    let media = &found.media;
    let config = &ctx.state.config;
    let owner_base = user_base_url(config, found.owner_preferred_domain.as_deref());
    let page = public_url(&found.owner_user_code, &media.code, &media.ext, &owner_base);
    let raw = raw_url(&found.owner_user_code, &media.code, &media.ext, &owner_base);
    // Not upstream: an account's file password hides a file like its own does.
    let has_password = media.password_hash.as_deref().is_some_and(|h| !h.is_empty()) || media.is_vaulted();
    let hidden = has_password || !media.published;
    // Security fix 1: the opening of a capped file is its content (a short paste
    // is ALL of it), so its card describes it by type and size instead.
    let snippet = if !hidden && media.max_views.is_none() && shows_source(media) && media.size > 0 {
        read_snippet(ctx, &media.storage_key, media.size).await
    } else {
        None
    };
    embed_metadata(&build_embed_plan(
        &EmbedMedia {
            filename: media.filename.clone(),
            ext: media.ext.clone(),
            mime_type: media.mime_type.clone(),
            size: media.size,
            width: media.width.map(i64::from),
            height: media.height.map(i64::from),
            kind: media.kind.clone(),
            published: media.published,
            has_password,
            has_view_cap: media.max_views.is_some(),
        },
        &EmbedContext {
            brand: config.embed_brand.clone(),
            app_name: config.app_name.clone(),
            oembed_href: format!("{owner_base}/oembed?url={}&format=json", encode_uri_component(&page)),
            link_info_href: Some(format!("{owner_base}/api/link-info?url={}", encode_uri_component(&page))),
            card_origin: owner_base.clone(),
            page,
            raw,
            snippet,
        },
    ))
}

// ── Page ────────────────────────────────────────────────────────────────────

/// `MEDIA_ACTIONS`: `/^(raw(\.[a-z0-9]+)?|download|convert|timeline\.(vtt|jpg))$/i`
fn is_media_action(second: &str) -> bool {
    let s = second.to_ascii_lowercase();
    if s == "raw" || s == "download" || s == "convert" || s == "timeline.vtt" || s == "timeline.jpg" {
        return true;
    }
    s.strip_prefix("raw.").is_some_and(|ext| !ext.is_empty() && ext.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// Next's `searchParams` object, keys in JavaScript property order: integer-like
/// keys first, ascending, then the rest in the order they first appeared.
fn query_object(search: &SearchParams) -> Vec<(String, Vec<String>)> {
    let mut entries: Vec<(String, Vec<String>)> = Vec::new();
    for (k, v) in &search.0 {
        match entries.iter_mut().find(|(key, _)| key == k) {
            Some((_, values)) => values.push(v.clone()),
            None => entries.push((k.clone(), vec![v.clone()])),
        }
    }
    let index = |k: &str| -> Option<u32> {
        if k == "0" {
            return Some(0);
        }
        if k.starts_with('0') || k.is_empty() || !k.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        k.parse::<u32>().ok().filter(|n| *n < u32::MAX)
    };
    let (mut ints, rest): (Vec<_>, Vec<_>) = entries.into_iter().partition(|(k, _)| index(k).is_some());
    ints.sort_by_key(|(k, _)| index(k));
    ints.into_iter().chain(rest).collect()
}

/// `taglessActionRedirect(first, second, query)`: /{code}/raw ->
/// /{ownerTag}/{code}/raw, the query rebuilt from the parsed params so nothing
/// a caller appended can steer the target.
async fn tagless_action_redirect(
    ctx: &PageCtx,
    first: &str,
    second: &str,
    search: &SearchParams,
) -> Result<Option<String>, PageError> {
    if !is_media_action(second) {
        return Ok(None);
    }
    let Some(code) = code_from_media_code(first) else {
        return Err(PageError::Internal(anyhow::anyhow!("URIError: URI malformed")));
    };
    let Some(owner_tag) = resolve_tagless_owner_tag(&ctx.state.db, &code).await? else {
        return Ok(None);
    };
    let mut qs = form_urlencoded::Serializer::new(String::new());
    for (k, values) in query_object(search) {
        for v in values {
            qs.append_pair(&k, &v);
        }
    }
    let qs = qs.finish();
    let suffix = if qs.is_empty() { String::new() } else { format!("?{qs}") };
    Ok(Some(format!("/{owner_tag}/{code}/{second}{suffix}")))
}

/// `MediaPage({ params: { userCode, mediaCode }, searchParams })`, with the
/// page's spelling of both params (CONTRACT 5.3: encoded).
async fn media_page(ctx: &PageCtx, user_code: &str, media_code: &str, search: &SearchParams) -> PageResult {
    let found = match lookup_media(&ctx.state.db, user_code, media_code).await? {
        Lookup::Live(found) => found,
        // The page always counts a view, so a spent budget is a missing file.
        Lookup::Spent(_) | Lookup::Missing => {
            // Might be a tagless action (/{code}/raw) rather than a missing file.
            if let Some(canonical) = tagless_action_redirect(ctx, user_code, media_code, search).await? {
                return Err(PageError::redirect(canonical));
            }
            return Err(PageError::NotFound);
        }
    };
    let meta = media_metadata(ctx, &found).await;
    let state = &ctx.state;
    let media = &found.media;
    let skin = |data: PageData| data.with_skin(Some(&found.owner_theme), found.owner_background.clone());

    // Access control: unpublished media is visible only to owner/admin, who are
    // also exempt from the view counter and the burn-after-view budget.
    let rel = viewer_relation(&state.db, &ctx.headers, &ctx.session, &found.owner_id).await?;
    let privileged = rel.privileged;
    if !media.published && !privileged {
        return Err(PageError::NotFound);
    }

    // Not upstream: the owner's file vault (docs/ENCRYPTION.md). The unlock
    // page for everybody without a grant, the owner and administrators
    // included; no view is counted. With a grant, `keyed` reads the plaintext.
    let Some(keyed) =
        crate::vault::access::open_files(state, &ctx.headers, &found.owner_id, found.owner_vault.as_deref(), &[media])
            .await?
    else {
        let props = MediaPageProps {
            owner_theme: found.owner_theme.clone(),
            owner_background: found.owner_background.clone(),
            view: View::Vault { media_id: media.id.to_string() },
        };
        return Ok(skin(PageData::new(&props, meta)?));
    };

    let owner_base = user_base_url(&state.config, found.owner_preferred_domain.as_deref());
    let page = public_url(&found.owner_user_code, &media.code, &media.ext, &owner_base);
    let raw = raw_url(&found.owner_user_code, &media.code, &media.ext, &owner_base);
    let dl = download_url(&found.owner_user_code, &media.code, &owner_base);

    let ua = crate::media::serve::first_header(&ctx.headers, "user-agent").unwrap_or_default();
    let is_bot = is_preview_bot(Some(&ua));
    let kind = media_kind(Some(&media.mime_type), Some(&media.ext));
    let has_password = media.password_hash.as_deref().is_some_and(|h| !h.is_empty());

    // Send the media-capable unfurlers to the bytes themselves. Never for
    // protected, unlisted or one-shot links.
    if is_bot
        && is_direct_media_bot(Some(&ua))
        && media.published
        && !has_password
        && !media.is_vaulted()
        && media.max_views.is_none()
        && matches!(kind, MediaKind::Image | MediaKind::Video | MediaKind::Audio)
    {
        // Oversized stills would silently fail to proxy, so those get a 1200px
        // WebP render (uncropped). Video/audio always go to the original.
        let too_big = kind == MediaKind::Image && media.size > EMBED_PROXY_LIMIT;
        return Err(PageError::redirect(if too_big { format!("{raw}?w=1200&fit=inside") } else { raw }));
    }

    // Password gate: only a valid signed unlock token counts.
    if has_password && !privileged {
        let jar = ctx.cookies();
        let unlocked = is_media_unlocked(
            &state.config.auth.secret,
            &MediaUnlockSubject {
                media_id: media.id.to_string(),
                owner_id: media.owner_id.clone(),
                password_version: media.password_version.map(i64::from),
            },
            |name| jar.get(name).cloned(),
        );
        if !unlocked {
            // Only a file stamped with the owner's CURRENT default may offer
            // "remember for 30 days".
            let owner_version = get_owner_password_version(&state.db, &found.owner_id).await?;
            let on_default = media.password_version.is_some() && media.password_version == owner_version;
            let props = MediaPageProps {
                owner_theme: found.owner_theme.clone(),
                owner_background: found.owner_background.clone(),
                view: View::Gate { media_id: media.id.to_string(), on_default_password: on_default },
            };
            return Ok(skin(PageData::new(&props, meta)?));
        }
    }

    // End-to-end encrypted upload: everything else happens in the visitor's
    // browser. The view is NOT counted here: the viewer's first ranged read of
    // the container is the request that counts.
    if is_encrypted_upload(Some(&media.mime_type), Some(&media.ext)) {
        let props = MediaPageProps {
            owner_theme: found.owner_theme.clone(),
            owner_background: found.owner_background.clone(),
            view: View::Encrypted {
                // Relative on purpose: the viewer fetches with the page's own origin.
                raw_path: raw_url(&found.owner_user_code, &media.code, &media.ext, ""),
                cipher_size: media.size,
                created_at: media.created_at.to_iso_string(),
                views: media.views,
                downloads: media.downloads,
            },
        };
        return Ok(skin(PageData::new(&props, meta)?));
    }

    // Owner's copyRaw preference; non-essential, so any error reads as false.
    let copy_raw: bool =
        sqlx::query_scalar::<_, Option<bool>>(r#"select "copy_raw" from "user" where "user"."id" = $1 limit 1"#)
            .bind(&found.owner_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten()
            .flatten()
            .unwrap_or(false);

    // Count the view once the gate has been cleared; crawlers are excluded.
    let mut counted = false;
    if !privileged && !is_bot {
        if !consume_view(&state.db, media, false).await? {
            return Err(PageError::NotFound);
        }
        counted = true;
    }
    // Security fix 1: what this view entitles the page's own requests to.
    if counted && media.max_views.is_some() {
        ctx.session.response_cookies().push(view_grant_cookie(
            &state.config.auth.secret,
            &media.id.to_string(),
            state.config.is_prod,
        ));
    }
    // Security fix 1: an uncounted visitor of a capped file (a preview bot) gets
    // the page without the file's content.
    let withhold = media.max_views.is_some() && !privileged && !counted;

    // Source preview: read server-side (bounded) and passed as a plain string.
    let mut text_content = None;
    let mut text_unavailable = None;
    if shows_source(media) && !withhold {
        if inline_text_plan(media.size) == InlineTextPlan::TooLarge {
            text_unavailable = Some("too-large");
        } else {
            match read_text(&keyed, media).await {
                Some(text) => text_content = Some(text),
                None => text_unavailable = Some("unreadable"),
            }
        }
    }

    // The ticket for the sandboxed render is minted only once every gate above
    // has been cleared.
    let render_href = (!withhold && can_render_inline(&media.mime_type, Some(&media.ext), media.size)).then(|| {
        render_path(
            &found.owner_user_code,
            &media.code,
            &mint_render_ticket(&state.config.auth.secret, &media.id.to_string(), None),
        )
    });

    // One entry pointing at the same file on the hostname where a session can
    // exist, for a stranger on a secondary hostname.
    let app_host = url::Url::parse(&state.config.app_url).map(|u| host_of(&u)).unwrap_or_default().to_lowercase();
    let request_host = crate::media::serve::first_header(&ctx.headers, "host").unwrap_or_default().to_lowercase();
    let sign_in_url = (!privileged && !request_host.is_empty() && request_host != app_host)
        .then(|| public_url(&found.owner_user_code, &media.code, &media.ext, &state.config.app_url));

    let timeline_track = can_timeline_preview(&media.mime_type, &media.ext, media.size)
        .then(|| media_action_path(&found.owner_user_code, &media.code, TIMELINE_TRACK_ACTION));
    let poster_url = (kind == MediaKind::Video)
        .then(|| format!("{}?w={POSTER_WIDTH}", media_action_path(&found.owner_user_code, &media.code, "raw")));

    // `isOwner`, not `privileged`: an administrator cannot change how somebody
    // else records.
    let playback = if rel.is_owner { diagnose_playback(&playback_source(media)) } else { None };
    let playback_repair = playback.as_ref().map(|_| PlaybackRepair {
        item: to_file_items(std::slice::from_ref(media), &Default::default()).remove(0),
        user_code: found.owner_user_code.clone(),
        app_url: owner_base.clone(),
    });
    let playback = playback.map(|d| PlaybackOut {
        faults: d.faults.iter().map(|f| f.as_str()).collect(),
        edge: Num(d.edge),
        bits_per_pixel: d.bits_per_pixel.map(Num),
        starvation_factor: d.starvation_factor.map(Num),
        repairable: d.repairable,
        repair_edge: Num(d.repair_edge),
    });

    let viewer = ViewerProps {
        filename: media.filename.clone(),
        mime_type: media.mime_type.clone(),
        ext: media.ext.clone(),
        size: media.size,
        views: media.views,
        downloads: media.downloads,
        created_at: media.created_at,
        width: media.width,
        height: media.height,
        frames: media.frames,
        public_url: page,
        raw_url: raw,
        download_url: dl,
        copy_raw,
        // `privileged` is exactly the test the convert route applies.
        can_encode: privileged,
        sign_in_url,
        text_content,
        text_unavailable,
        render_path: render_href,
        timeline_track,
        poster_url,
        playback,
        playback_repair,
    };
    let props = MediaPageProps {
        owner_theme: found.owner_theme.clone(),
        owner_background: found.owner_background.clone(),
        view: View::Media { viewer: Box::new(viewer) },
    };
    Ok(skin(PageData::new(&props, meta)?))
}

/// WHATWG `url.host`: the hostname plus a non-default port.
fn host_of(u: &url::Url) -> String {
    match (u.host_str(), u.port()) {
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_string(),
        _ => String::new(),
    }
}

/// The whole object as text (`buf.toString("utf8")`), None when storage fails.
async fn read_text(state: &crate::state::AppState, media: &Upload) -> Option<String> {
    let storage = state.storage.get().ok()?;
    let buf = storage.read_buffer(&media.storage_key).await.ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// The page's spelling of a param: `encodeURIComponent(decoded)`.
fn page_spelling(ctx: &PageCtx, name: &str) -> String {
    encode_uri_component(ctx.param(name).unwrap_or(""))
}

/// `/[userCode]/[mediaCode]`
pub async fn load(ctx: PageCtx) -> PageResult {
    let user_code = page_spelling(&ctx, "userCode");
    let media_code = page_spelling(&ctx, "mediaCode");
    let search = ctx.search.clone();
    media_page(&ctx, &user_code, &media_code, &search).await
}

/// `resolveTag(segment)`: the owner's tag when this segment is a tagless media
/// link (owners who opted in with `skip_tag_in_url`), otherwise None.
async fn resolve_tag(ctx: &PageCtx, segment: &str) -> Result<Option<String>, PageError> {
    if segment.is_empty() || segment.starts_with('.') {
        return Ok(None);
    }
    let Some(code) = code_from_media_code(segment) else {
        return Err(PageError::Internal(anyhow::anyhow!("URIError: URI malformed")));
    };
    if code.is_empty() || is_reserved_tag(segment) || is_reserved_tag(&code) {
        return Ok(None);
    }
    Ok(resolve_tagless_owner_tag(&ctx.state.db, &code).await?)
}

/// `/[userCode]`: rendering is delegated to the `/{tag}/{code}` page so view
/// counting, the password gate, expiry and the Open Graph tags have exactly one
/// implementation. No query travels into it.
pub async fn load_tagless(ctx: PageCtx) -> PageResult {
    let segment = page_spelling(&ctx, "userCode");
    let Some(tag) = resolve_tag(&ctx, &segment).await? else {
        return Err(PageError::NotFound);
    };
    media_page(&ctx, &tag, &segment, &SearchParams::default()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_actions_like_the_upstream_pattern() {
        for yes in ["raw", "RAW", "raw.png", "Raw.WebP", "download", "convert", "timeline.vtt", "TIMELINE.JPG"] {
            assert!(is_media_action(yes), "{yes}");
        }
        for no in ["raw.", "raw.p-g", "raws", "render", "timeline.png", "delete", "raw.ąb"] {
            assert!(!is_media_action(no), "{no}");
        }
    }

    #[test]
    fn the_query_travels_in_javascript_key_order() {
        let q = SearchParams::parse("w=64&10=a&2=b&w=128&b=1&01=c");
        let order: Vec<String> = query_object(&q).into_iter().map(|(k, _)| k).collect();
        assert_eq!(order, ["2", "10", "w", "b", "01"]);
        let w = query_object(&q).into_iter().find(|(k, _)| k == "w").unwrap().1;
        assert_eq!(w, ["64", "128"]);
    }
}
