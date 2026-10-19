//! Port of `lib/media/embed.ts`: what a shared link says when a chat client
//! unfurls it.
//!
//! Pure on purpose (no database, no request, no config read): the whole point
//! of this module is that the card can be asserted in a unit test, because the
//! old one was assembled inline and quietly said the product name four times
//! and the file name twice.
//!
//! The rules it encodes:
//!  - the brand appears EXACTLY once, as og:site_name;
//!  - the file name appears EXACTLY once, as og:title;
//!  - the description carries facts the title cannot (type, dimensions, size),
//!    or the opening of the text for a paste;
//!  - twitter:* mirrors Open Graph, declared once so nothing generates a second.
//!
//! String lengths and slices are JavaScript's (UTF-16 code units), so a title
//! or a snippet is cut exactly where upstream cut it.

use crate::format::human_file_size;
use crate::media::card::{CARD_HEIGHT, CARD_WIDTH, CardSubject, card_choice_for, card_url};
use crate::media::mime::{MediaKind, effective_type, is_displayable_image, media_kind};
use crate::pages::meta::{Alternates, Field, OgMedia, OpenGraph, PageMeta, Robots, Title, Twitter};

/// Discord and friends stop proxying a still somewhere around this size.
const EMBED_PROXY_LIMIT: i64 = 8 * 1024 * 1024;

/// Mirrors MAX_TRANSFORM_BYTES in the raw route: past this it declines to
/// rasterise anything and hands back the original.
const RAW_TRANSFORM_LIMIT: i64 = 32 * 1024 * 1024;

/// Widest still an unfurler will show; anything larger is scaled, never cropped.
const EMBED_IMAGE_WIDTH: i64 = 1200;

/// Longest file name kept whole in a title before the middle is elided.
const MAX_TITLE_LENGTH: usize = 100;

/// Longest paste excerpt used as a description (Discord truncates near 300).
const MAX_SNIPPET_LENGTH: usize = 180;

/// `EmbedMedia`
#[derive(Debug, Clone, PartialEq)]
pub struct EmbedMedia {
    pub filename: String,
    pub ext: String,
    pub mime_type: String,
    pub size: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    /// uploads.kind: "file" | "paste".
    pub kind: String,
    pub published: bool,
    pub has_password: bool,
    /// The file carries a view cap (burn-after-N-views). It suppresses og:image
    /// for the same reason a password does: an unfurler copies the picture onto
    /// its own CDN and serves it publicly forever, and the fetch itself would
    /// spend one of the views the sender meant for a person.
    pub has_view_cap: bool,
}

/// `EmbedContext`
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EmbedContext {
    /// Site name, one word, same for every hostname of this deployment.
    pub brand: String,
    /// Product name. Used only for the title of a protected file.
    pub app_name: String,
    /// Canonical preview page URL.
    pub page: String,
    /// Raw file URL on the owner's hostname.
    pub raw: String,
    /// oEmbed discovery href.
    pub oembed_href: String,
    /// Where the machine-readable description of THIS file lives
    /// (`/api/link-info?url=…`). Emitted only on a card.
    pub link_info_href: Option<String>,
    /// Origin the type card is served from.
    pub card_origin: String,
    /// First bytes of a text file or paste, when they were cheap to read.
    pub snippet: Option<String>,
}

/// `OgImage`
#[derive(Debug, Clone, PartialEq)]
pub struct OgImage {
    pub url: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub mime: Option<String>,
    pub alt: String,
}

/// `OgVideo`
#[derive(Debug, Clone, PartialEq)]
pub struct OgVideo {
    pub url: String,
    pub mime: String,
    pub width: i64,
    pub height: i64,
}

/// `OgCard`
#[derive(Debug, Clone, PartialEq)]
pub struct OgCard {
    /// "website" | "video.other"
    pub og_type: &'static str,
    pub title: String,
    pub site_name: String,
    pub url: String,
    pub description: String,
    pub image: Option<OgImage>,
    pub video: Option<OgVideo>,
}

/// `EmbedPlan`
#[derive(Debug, Clone, PartialEq)]
pub enum EmbedPlan {
    /// Password-gated: not even the file name may leak into a title.
    Protected { document_title: String },
    /// Unpublished: reachable by its owner, never advertised.
    Unlisted { document_title: String },
    Card {
        document_title: String,
        canonical: String,
        oembed_href: String,
        link_info_href: Option<String>,
        /// "summary" | "summary_large_image"
        twitter_card: &'static str,
        og: Box<OgCard>,
    },
}

/// `buildEmbedPlan(media, ctx)`: decide every tag a preview page will emit.
pub fn build_embed_plan(media: &EmbedMedia, ctx: &EmbedContext) -> EmbedPlan {
    // The title is the one thing a browser shows before anything is unlocked,
    // and "raport-finansowy-2026.xlsx" often says more than the contents would.
    if media.has_password {
        return EmbedPlan::Protected { document_title: ctx.app_name.clone() };
    }
    if !media.published {
        return EmbedPlan::Unlisted { document_title: media.filename.clone() };
    }

    let title = elide_middle(&media.filename, MAX_TITLE_LENGTH);
    let document_title = format!("{title} \u{b7} {}", ctx.brand);
    // The extension travels with the type everywhere: an upload stored as
    // application/octet-stream is still an image, and its card should say so.
    let kind = media_kind(Some(&media.mime_type), Some(&media.ext));
    let is_image = is_displayable_image(Some(&media.mime_type), Some(&media.ext));
    let is_text = media.kind == "paste" || kind == MediaKind::Text || media.mime_type.starts_with("text/");

    let card = |twitter_card: &'static str, og: OgCard| EmbedPlan::Card {
        document_title: document_title.clone(),
        canonical: ctx.page.clone(),
        oembed_href: ctx.oembed_href.clone(),
        link_info_href: ctx.link_info_href.clone().filter(|h| !h.is_empty()),
        twitter_card,
        og: Box::new(og),
    };

    if is_image {
        return card(
            "summary_large_image",
            OgCard {
                og_type: "website",
                title: title.clone(),
                site_name: ctx.brand.clone(),
                url: ctx.page.clone(),
                description: dimensions_description(media),
                image: (!media.has_view_cap).then(|| embed_image(media, &ctx.raw, &title)),
                video: None,
            },
        );
    }

    if kind == MediaKind::Video {
        // Not "player": that card needs an HTTPS iframe player on X's allowlist,
        // and we hand it a bare .mp4, so it renders nothing at all.
        return card(
            "summary",
            OgCard {
                og_type: "video.other",
                title: title.clone(),
                site_name: ctx.brand.clone(),
                url: ctx.page.clone(),
                description: dimensions_description(media),
                // No og:image on purpose: a "still" URL for a video would have to
                // be a frame the poster path draws, never the clip itself.
                image: None,
                video: (!media.has_view_cap).then(|| OgVideo {
                    url: ctx.raw.clone(),
                    mime: media.mime_type.clone(),
                    width: media.width.unwrap_or(1280),
                    height: media.height.unwrap_or(720),
                }),
            },
        );
    }

    // Everything else: archives, code, documents, pastes, audio, binaries, and
    // the image formats a browser refuses to render. They get a drawn card - one
    // family, one glyph per category. `summary_large_image` is not decoration
    // here: at `summary` Discord files the picture into an 80x80 tile.
    let choice = card_choice_for(CardSubject { ext: &media.ext, mime_type: &media.mime_type, kind: &media.kind });
    let description = if is_text {
        snippet_description(ctx.snippet.as_deref()).unwrap_or_else(|| size_description(media))
    } else {
        size_description(media)
    };
    card(
        if choice.is_some() { "summary_large_image" } else { "summary" },
        OgCard {
            og_type: "website",
            title,
            site_name: ctx.brand.clone(),
            url: ctx.page.clone(),
            description,
            image: choice.map(|choice| OgImage {
                url: card_url(&ctx.card_origin, &choice),
                width: Some(i64::from(CARD_WIDTH)),
                height: Some(i64::from(CARD_HEIGHT)),
                mime: Some("image/png".to_string()),
                // The spoken equivalent of the glyph, for a screen reader.
                alt: long_type(media),
            }),
            video: None,
        },
    )
}

/// What Next prints for an `og:image` / `twitter:image` URL: the URL parsed
/// and serialised again (WHATWG). `secure_url` and the video URLs go out as
/// given.
fn normalized_url(url: &str) -> String {
    url::Url::parse(url).map(|u| u.to_string()).unwrap_or_else(|_| url.to_string())
}

/// `embedMetadata(plan)`: the plan as the page's metadata.
pub fn embed_metadata(plan: &EmbedPlan) -> PageMeta {
    let (document_title, canonical, oembed_href, link_info_href, twitter_card, og) = match plan {
        EmbedPlan::Protected { document_title } | EmbedPlan::Unlisted { document_title } => {
            // `absolute` here too: a plain string still runs through the root
            // layout's "%s - <appName>" template. The root layout's product
            // blurb would otherwise describe every gated file.
            return PageMeta {
                title: Title::Absolute(document_title.clone()),
                description: Field::Clear,
                application_name: Field::Clear,
                robots: Some(Robots { index: false, follow: false }),
                ..Default::default()
            };
        }
        EmbedPlan::Card { document_title, canonical, oembed_href, link_info_href, twitter_card, og } => {
            (document_title, canonical, oembed_href, link_info_href, twitter_card, og)
        }
    };

    let mut types = vec![("application/json+oembed".to_string(), oembed_href.clone())];
    if let Some(href) = link_info_href {
        // A distinct media type from the oEmbed one, so the two links cannot be
        // confused by a consumer that reads only `type`.
        types.push(("application/json".to_string(), href.clone()));
    }

    let images: Vec<OgMedia> = og
        .image
        .iter()
        .map(|image| {
            // Next's order: secure_url, type, width, height, alt.
            let mut props = vec![("secure_url".to_string(), image.url.clone())];
            if let Some(t) = &image.mime {
                props.push(("type".to_string(), t.clone()));
            }
            if let Some(w) = image.width.filter(|w| *w != 0) {
                props.push(("width".to_string(), w.to_string()));
            }
            if let Some(h) = image.height.filter(|h| *h != 0) {
                props.push(("height".to_string(), h.to_string()));
            }
            props.push(("alt".to_string(), image.alt.clone()));
            OgMedia { url: normalized_url(&image.url), props }
        })
        .collect();
    let videos: Vec<OgMedia> = og
        .video
        .iter()
        .map(|video| {
            let mut props =
                vec![("secure_url".to_string(), video.url.clone()), ("type".to_string(), video.mime.clone())];
            if video.width != 0 {
                props.push(("width".to_string(), video.width.to_string()));
            }
            if video.height != 0 {
                props.push(("height".to_string(), video.height.to_string()));
            }
            OgMedia { url: video.url.clone(), props }
        })
        .collect();

    PageMeta {
        // `absolute` skips the root layout's "%s - <appName>" template, so the
        // tab and the card agree instead of naming two different things.
        title: Title::Absolute(document_title.clone()),
        // Explicitly cleared, not merely omitted: metadata is INHERITED from the
        // root layout, so leaving these out published the product blurb.
        description: Field::Clear,
        application_name: Field::Clear,
        robots: None,
        alternates: Some(Alternates { canonical: Some(canonical.clone()), types }),
        open_graph: Some(OpenGraph {
            og_type: Some(og.og_type.to_string()),
            title: Some(og.title.clone()),
            description: Some(og.description.clone()),
            url: Some(og.url.clone()),
            site_name: Some(og.site_name.clone()),
            images,
            videos,
        }),
        // Stated explicitly, because Next fills the twitter:* family in from
        // openGraph whether or not this object exists; the values deliberately
        // mirror Open Graph, so no consumer sees two different stories.
        twitter: Some(Twitter {
            card: Some(twitter_card.to_string()),
            title: Some(og.title.clone()),
            description: Some(og.description.clone()),
            images: og.image.as_ref().map(|i| vec![normalized_url(&i.url)]),
        }),
        extra: Vec::new(),
    }
}

// ── Descriptions ────────────────────────────────────────────────────────────

/// "PNG · 1920x1080 · 240.0 KB" - facts the title and the picture don't carry.
fn dimensions_description(media: &EmbedMedia) -> String {
    let mut parts = vec![short_type(media)];
    if let (Some(w), Some(h)) = (media.width.filter(|w| *w != 0), media.height.filter(|h| *h != 0)) {
        parts.push(format!("{w}x{h}"));
    }
    parts.push(human_file_size(media.size as f64, 1));
    parts.join(" \u{b7} ")
}

/// "ZIP archive · 24.8 MB" for everything without a picture to show.
fn size_description(media: &EmbedMedia) -> String {
    format!("{} \u{b7} {}", long_type(media), human_file_size(media.size as f64, 1))
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn from_utf16(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

/// `snippetDescription(snippet)`: the opening of a paste, whitespace collapsed.
/// None when there is nothing to show.
fn snippet_description(snippet: Option<&str>) -> Option<String> {
    let snippet = snippet.filter(|s| !s.is_empty())?;
    // A snippet is cut at a byte offset, so it can end mid character; control
    // characters would survive into the tag as garbage; /\s+/g collapses.
    let mut flat = String::with_capacity(snippet.len());
    let mut in_space = false;
    for c in snippet.chars() {
        if c == '\u{fffd}' {
            continue;
        }
        let c = if c <= '\u{1f}' || c == '\u{7f}' { ' ' } else { c };
        if crate::config::is_js_whitespace(c) {
            if !in_space {
                flat.push(' ');
                in_space = true;
            }
        } else {
            flat.push(c);
            in_space = false;
        }
    }
    let flat = crate::config::js_trim(&flat);
    if flat.is_empty() {
        return None;
    }
    let units = utf16(flat);
    if units.len() > MAX_SNIPPET_LENGTH {
        let head = from_utf16(&units[..MAX_SNIPPET_LENGTH]);
        let trimmed = head.trim_end_matches(crate::config::is_js_whitespace);
        Some(format!("{trimmed}..."))
    } else {
        Some(flat.to_string())
    }
}

/// Extension in caps ("PNG"), falling back to the MIME subtype.
fn short_type(media: &EmbedMedia) -> String {
    if !media.ext.is_empty() {
        return media.ext.to_uppercase();
    }
    let sub = media.mime_type.split('/').nth(1).unwrap_or(&media.mime_type);
    sub.split('+').next().unwrap_or("").to_uppercase()
}

/// `LONG_TYPES` (exact keys; see the note on prototype names in `media::card`).
fn long_type_by_ext(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "pdf" => "PDF document",
        "zip" => "ZIP archive",
        "rar" => "RAR archive",
        "7z" => "7z archive",
        "tar" => "TAR archive",
        "gz" | "tgz" => "GZIP archive",
        "mp3" => "MP3 audio",
        "wav" => "WAV audio",
        "flac" => "FLAC audio",
        "ogg" => "OGG audio",
        "opus" => "Opus audio",
        "m4a" => "M4A audio",
        "txt" => "Text file",
        "md" => "Markdown document",
        "json" => "JSON file",
        "csv" => "CSV spreadsheet",
        "html" => "HTML document",
        // Stays "image": an SVG is a drawing its author made, even though we
        // serve the bytes as inert text.
        "svg" => "SVG image",
        "doc" | "docx" => "Word document",
        "xls" | "xlsx" => "Excel spreadsheet",
        "ppt" | "pptx" => "PowerPoint presentation",
        _ => return None,
    })
}

/// A readable label ("PDF document"), or just the extension when unknown.
fn long_type(media: &EmbedMedia) -> String {
    if let Some(by_ext) = long_type_by_ext(&media.ext.to_lowercase()) {
        return by_ext.to_string();
    }
    match media_kind(Some(&media.mime_type), Some(&media.ext)) {
        MediaKind::Audio => format!("{} audio", short_type(media)),
        MediaKind::Archive => format!("{} archive", short_type(media)),
        _ => short_type(media),
    }
}

// ── Image ───────────────────────────────────────────────────────────────────

/// One og:image, never two. Oversized stills are scaled with `fit=inside` (the
/// raw route's own thumbnailer, which returns WebP) so the whole frame survives.
fn embed_image(media: &EmbedMedia, raw: &str, alt: &str) -> OgImage {
    let width = media.width.unwrap_or(0);
    let height = media.height.unwrap_or(0);
    let too_wide = width > EMBED_IMAGE_WIDTH;
    let too_heavy = media.size > EMBED_PROXY_LIMIT;
    // The type an unfurler is TOLD, resolved the way every other decision on this
    // page resolves it (an ordinary .avif stored as octet-stream is an image).
    let declared = effective_type(Some(&media.mime_type), Some(&media.ext));

    // Above the raw route's own transform ceiling the `?w=` parameter is ignored
    // and the original bytes are streamed instead, so asking for a resize here
    // would advertise a picture that never arrives.
    if media.size > RAW_TRANSFORM_LIMIT || (!too_wide && !too_heavy) {
        return OgImage {
            url: raw.to_string(),
            width: media.width,
            height: media.height,
            mime: Some(declared),
            alt: alt.to_string(),
        };
    }

    let url = format!("{raw}?w={EMBED_IMAGE_WIDTH}&fit=inside");
    if too_wide && height > 0 {
        let scaled = crate::config::js_round((height * EMBED_IMAGE_WIDTH) as f64 / width as f64) as i64;
        return OgImage {
            url,
            width: Some(EMBED_IMAGE_WIDTH),
            height: Some(scaled.max(1)),
            mime: Some("image/webp".to_string()),
            alt: alt.to_string(),
        };
    }
    // Heavy but not wide: the thumbnailer keeps the original dimensions.
    OgImage {
        url,
        width: media.width,
        height: media.height,
        mime: Some("image/webp".to_string()),
        alt: alt.to_string(),
    }
}

// ── Title ───────────────────────────────────────────────────────────────────

/// `elideMiddle(name, max)`: shorten from the middle so the extension survives.
/// Lengths and offsets in UTF-16 code units, as JavaScript counts them.
pub fn elide_middle(name: &str, max: usize) -> String {
    let units = utf16(name);
    let len = units.len();
    if len <= max {
        return name.to_string();
    }
    let dot = units.iter().rposition(|u| *u == u16::from(b'.'));
    let ext: &[u16] = match dot {
        Some(d) if d > 0 && len - d <= 12 => &units[d..],
        _ => &[],
    };
    let room = max as i64 - ext.len() as i64 - 3;
    if room < 8 {
        let keep = (max as i64 - 3).max(1) as usize;
        return format!("{}...", from_utf16(&units[..keep.min(len)]));
    }
    let room = room as usize;
    let head = (room as f64 * 0.6).ceil() as usize;
    let tail = room - head;
    let tail_start = len - ext.len() - tail;
    format!("{}...{}{}", from_utf16(&units[..head]), from_utf16(&units[tail_start..len - ext.len()]), from_utf16(ext))
}

#[cfg(test)]
mod tests {
    //! tests/unit/embed-tags.test.ts (`brandFromUrl` is pinned in `config`).
    use super::*;

    const BRAND: &str = "example";

    fn ctx() -> EmbedContext {
        EmbedContext {
            brand: BRAND.into(),
            app_name: "Modlight".into(),
            page: "https://h.example.com/alicja/kalit2.png".into(),
            raw: "https://h.example.com/alicja/kalit2/raw.png".into(),
            oembed_href: "https://h.example.com/oembed?url=x&format=json".into(),
            link_info_href: None,
            card_origin: "https://h.example.com".into(),
            snippet: None,
        }
    }

    fn media() -> EmbedMedia {
        EmbedMedia {
            filename: "screenshot.png".into(),
            ext: "png".into(),
            mime_type: "image/png".into(),
            size: 245_760,
            width: Some(1920),
            height: Some(1080),
            kind: "file".into(),
            published: true,
            has_password: false,
            has_view_cap: false,
        }
    }

    fn og(plan: &EmbedPlan) -> &OgCard {
        match plan {
            EmbedPlan::Card { og, .. } => og,
            _ => panic!("expected a card"),
        }
    }

    fn twitter_card(plan: &EmbedPlan) -> &'static str {
        match plan {
            EmbedPlan::Card { twitter_card, .. } => twitter_card,
            _ => panic!("expected a card"),
        }
    }

    fn card_text(plan: &EmbedPlan) -> Vec<String> {
        let og = og(plan);
        vec![og.site_name.clone(), og.title.clone(), og.description.clone()]
    }

    fn occurrences(haystack: &[String], needle: &str) -> usize {
        haystack.iter().map(|p| p.to_lowercase().matches(&needle.to_lowercase()).count()).sum()
    }

    #[test]
    fn says_nothing_at_all_about_a_password_protected_file() {
        let plan = build_embed_plan(&EmbedMedia { has_password: true, ..media() }, &ctx());
        assert_eq!(plan, EmbedPlan::Protected { document_title: "Modlight".into() });
        let meta = embed_metadata(&plan);
        assert!(meta.open_graph.is_none());
        assert_eq!(meta.robots, Some(Robots { index: false, follow: false }));
        assert_eq!(meta.title, Title::Absolute("Modlight".into()));
        for mime in ["image/png", "video/mp4", "text/plain", "application/zip"] {
            let m = EmbedMedia { mime_type: mime.into(), has_password: true, ..media() };
            assert!(matches!(build_embed_plan(&m, &ctx()), EmbedPlan::Protected { .. }));
        }
    }

    #[test]
    fn keeps_an_unpublished_file_out_of_every_index_and_card() {
        let plan = build_embed_plan(&EmbedMedia { published: false, ..media() }, &ctx());
        assert_eq!(plan, EmbedPlan::Unlisted { document_title: "screenshot.png".into() });
        let meta = embed_metadata(&plan);
        assert!(meta.open_graph.is_none());
        assert_eq!(meta.robots, Some(Robots { index: false, follow: false }));
    }

    #[test]
    fn names_the_brand_once_and_the_file_once() {
        let text = card_text(&build_embed_plan(&media(), &ctx()));
        assert_eq!(occurrences(&text, BRAND), 1);
        assert_eq!(occurrences(&text, "screenshot.png"), 1);
        assert_eq!(occurrences(&text, "Modlight"), 0);

        let zip = EmbedMedia {
            filename: "backup-2026-07-31.zip".into(),
            ext: "zip".into(),
            mime_type: "application/zip".into(),
            size: 26_000_000,
            width: None,
            height: None,
            ..media()
        };
        let text = card_text(&build_embed_plan(&zip, &ctx()));
        assert_eq!(occurrences(&text, BRAND), 1);
        assert_eq!(occurrences(&text, "backup-2026-07-31.zip"), 1);
        assert_eq!(text[2], "ZIP archive \u{b7} 24.8 MB");
    }

    #[test]
    fn puts_the_brand_in_og_site_name_not_in_the_title() {
        let plan = build_embed_plan(&media(), &ctx());
        assert_eq!(og(&plan).site_name, BRAND);
        assert_eq!(og(&plan).title, "screenshot.png");
        assert!(!og(&plan).description.contains("screenshot"));
    }

    #[test]
    fn descriptions_carry_facts() {
        let plan = build_embed_plan(&media(), &ctx());
        assert_eq!(og(&plan).description, "PNG \u{b7} 1920x1080 \u{b7} 240.0 KB");
        assert_eq!(twitter_card(&plan), "summary_large_image");

        let video = EmbedMedia {
            filename: "clip.mp4".into(),
            ext: "mp4".into(),
            mime_type: "video/mp4".into(),
            size: 13_000_000,
            ..media()
        };
        let plan = build_embed_plan(&video, &ctx());
        assert_eq!(og(&plan).og_type, "video.other");
        assert_eq!(og(&plan).description, "MP4 \u{b7} 1920x1080 \u{b7} 12.4 MB");
        assert_eq!(og(&plan).video.as_ref().unwrap().url, ctx().raw);
        assert!(og(&plan).image.is_none());
        assert_eq!(twitter_card(&plan), "summary");

        let bare = build_embed_plan(&EmbedMedia { width: None, height: None, ..media() }, &ctx());
        assert_eq!(og(&bare).description, "PNG \u{b7} 240.0 KB");
    }

    #[test]
    fn paste_descriptions_are_the_opening_of_the_text() {
        let paste = EmbedMedia {
            filename: "notes.txt".into(),
            ext: "txt".into(),
            mime_type: "text/plain".into(),
            kind: "paste".into(),
            size: 400,
            width: None,
            height: None,
            ..media()
        };
        let c = EmbedContext { snippet: Some("  const answer =\n\n\t42;   // computed slowly\n".into()), ..ctx() };
        assert_eq!(og(&build_embed_plan(&paste, &c)).description, "const answer = 42; // computed slowly");

        let long = EmbedMedia { filename: "long.txt".into(), ..paste.clone() };
        let c = EmbedContext { snippet: Some("a".repeat(500)), ..ctx() };
        let d = og(&build_embed_plan(&long, &c)).description.clone();
        assert!(d.encode_utf16().count() <= 184);
        assert!(d.ends_with("..."));

        let log = EmbedMedia {
            filename: "huge.log".into(),
            ext: "log".into(),
            mime_type: "text/plain".into(),
            kind: "file".into(),
            size: 4096,
            width: None,
            height: None,
            ..media()
        };
        let c = EmbedContext { snippet: Some("   \n\t  ".into()), ..ctx() };
        assert_eq!(og(&build_embed_plan(&log, &c)).description, "LOG \u{b7} 4.0 KB");

        // Replacement characters vanish, control characters become spaces.
        let c = EmbedContext { snippet: Some("a\u{fffd}b\u{0}c\u{7f}d".into()), ..ctx() };
        assert_eq!(og(&build_embed_plan(&log, &c)).description, "ab c d");
    }

    #[test]
    fn exactly_one_og_image() {
        let small = build_embed_plan(&EmbedMedia { width: Some(900), height: Some(600), ..media() }, &ctx());
        let img = og(&small).image.clone().unwrap();
        assert_eq!(img.url, ctx().raw);
        assert_eq!(img.width, Some(900));
        assert_eq!(img.mime.as_deref(), Some("image/png"));

        let wide = build_embed_plan(&EmbedMedia { width: Some(2400), height: Some(1200), ..media() }, &ctx());
        let img = og(&wide).image.clone().unwrap();
        assert_eq!(img.url, format!("{}?w=1200&fit=inside", ctx().raw));
        assert_eq!((img.width, img.height), (Some(1200), Some(600)));
        assert_eq!(img.mime.as_deref(), Some("image/webp"));

        let heavy = build_embed_plan(
            &EmbedMedia { width: Some(800), height: Some(600), size: 12 * 1024 * 1024, ..media() },
            &ctx(),
        );
        assert_eq!(og(&heavy).image.as_ref().unwrap().url, format!("{}?w=1200&fit=inside", ctx().raw));

        let huge =
            build_embed_plan(&EmbedMedia { width: Some(8000), height: Some(600), size: 40 << 20, ..media() }, &ctx());
        assert_eq!(og(&huge).image.as_ref().unwrap().url, ctx().raw);
    }

    #[test]
    fn a_view_cap_withholds_the_picture_and_the_player() {
        let capped = build_embed_plan(&EmbedMedia { has_view_cap: true, ..media() }, &ctx());
        assert!(og(&capped).image.is_none());
        assert_eq!(twitter_card(&capped), "summary_large_image");
        let video = EmbedMedia { ext: "mp4".into(), mime_type: "video/mp4".into(), has_view_cap: true, ..media() };
        assert!(og(&build_embed_plan(&video, &ctx())).video.is_none());
        // A type card is a drawing, not the file: it stays.
        let zip = EmbedMedia { ext: "zip".into(), mime_type: "application/zip".into(), has_view_cap: true, ..media() };
        assert!(og(&build_embed_plan(&zip, &ctx())).image.is_some());
    }

    #[test]
    fn the_type_card() {
        let plan_for = |filename: &str, ext: &str, mime: &str, kind: &str| {
            build_embed_plan(
                &EmbedMedia {
                    filename: filename.into(),
                    ext: ext.into(),
                    mime_type: mime.into(),
                    kind: kind.into(),
                    width: None,
                    height: None,
                    ..media()
                },
                &ctx(),
            )
        };
        let zip = plan_for("backup.zip", "zip", "application/zip", "file");
        let img = og(&zip).image.clone().unwrap();
        assert_eq!(img.url, "https://h.example.com/embed/v1/archive-ZIP.png");
        assert_eq!((img.width, img.height), (Some(1200), Some(630)));
        assert_eq!(img.mime.as_deref(), Some("image/png"));
        assert_eq!(img.alt, "ZIP archive");
        assert_eq!(twitter_card(&zip), "summary_large_image");

        assert_eq!(
            og(&plan_for("app.ts", "ts", "text/plain", "file")).image.as_ref().unwrap().url,
            "https://h.example.com/embed/v1/code-TS.png"
        );
        assert_eq!(
            og(&plan_for("notes.rs", "rs", "text/plain", "paste")).image.as_ref().unwrap().url,
            "https://h.example.com/embed/v1/text-RS.png"
        );
        let sheet = plan_for("salaries-2026.xlsx", "xlsx", "application/vnd.ms-excel", "file");
        let url = og(&sheet).image.as_ref().unwrap().url.clone();
        assert_eq!(url, "https://h.example.com/embed/v1/sheet-XLSX.png");
        assert!(!url.contains("salaries") && !url.contains("kalit2"));

        let pic = build_embed_plan(&media(), &ctx());
        assert!(og(&pic).image.as_ref().unwrap().url.starts_with(&ctx().raw));
        let clip = plan_for("clip.mp4", "mp4", "video/mp4", "file");
        assert!(og(&clip).image.is_none());
        assert_eq!(og(&clip).video.as_ref().unwrap().url, ctx().raw);

        for gate in [EmbedMedia { has_password: true, ..media() }, EmbedMedia { published: false, ..media() }] {
            let zip = EmbedMedia { ext: "zip".into(), mime_type: "application/zip".into(), ..gate };
            let meta = embed_metadata(&build_embed_plan(&zip, &ctx()));
            assert!(meta.open_graph.is_none() && meta.alternates.is_none());
        }
    }

    #[test]
    fn embed_metadata_shape() {
        let meta = embed_metadata(&build_embed_plan(&media(), &ctx()));
        let og = meta.open_graph.clone().unwrap();
        assert_eq!(og.images.len(), 1);
        assert_eq!(meta.description, Field::Clear);
        assert_eq!(meta.application_name, Field::Clear);
        assert_eq!(og.description.as_deref(), Some("PNG \u{b7} 1920x1080 \u{b7} 240.0 KB"));
        let tw = meta.twitter.clone().unwrap();
        assert_eq!(tw.card.as_deref(), Some("summary_large_image"));
        assert_eq!(tw.description, og.description);
        let alt = meta.alternates.clone().unwrap();
        assert_eq!(alt.canonical.as_deref(), Some(ctx().page.as_str()));
        assert_eq!(alt.types, vec![("application/json+oembed".to_string(), ctx().oembed_href)]);
        assert_eq!(meta.title, Title::Absolute("screenshot.png \u{b7} example".into()));
        assert!(meta.robots.is_none());
    }

    #[test]
    fn link_info_discovery() {
        const LINK_INFO: &str = "https://h.example.com/api/link-info?url=x";
        let c = EmbedContext { link_info_href: Some(LINK_INFO.into()), ..ctx() };
        let meta = embed_metadata(&build_embed_plan(&media(), &c));
        let types = meta.alternates.unwrap().types;
        assert_eq!(types[1], ("application/json".to_string(), LINK_INFO.to_string()));
        assert_eq!(types[0].0, "application/json+oembed");
        for gate in [EmbedMedia { has_password: true, ..media() }, EmbedMedia { published: false, ..media() }] {
            assert!(embed_metadata(&build_embed_plan(&gate, &c)).alternates.is_none());
        }
    }

    #[test]
    fn the_worked_example_of_the_contract() {
        // CONTRACT 6.27: brand example, the owner's host, alicja/kalit2.png 1920x1080.
        let c = EmbedContext {
            oembed_href:
                "https://h.example.com/oembed?url=https%3A%2F%2Fh.example.com%2Falicja%2Fkalit2.png&format=json".into(),
            link_info_href: Some(
                "https://h.example.com/api/link-info?url=https%3A%2F%2Fh.example.com%2Falicja%2Fkalit2.png".into(),
            ),
            ..ctx()
        };
        let head = crate::pages::meta::resolve(
            &crate::Config::from_env_map(&std::collections::HashMap::new()).unwrap(),
            &embed_metadata(&build_embed_plan(&media(), &c)),
            false,
        );
        assert_eq!(
            crate::pages::meta::render_head(&head),
            "<title>screenshot.png \u{b7} example</title>\
             <link rel=\"canonical\" href=\"https://h.example.com/alicja/kalit2.png\"/>\
             <link rel=\"alternate\" type=\"application/json+oembed\" href=\"https://h.example.com/oembed?url=https%3A%2F%2Fh.example.com%2Falicja%2Fkalit2.png&amp;format=json\"/>\
             <link rel=\"alternate\" type=\"application/json\" href=\"https://h.example.com/api/link-info?url=https%3A%2F%2Fh.example.com%2Falicja%2Fkalit2.png\"/>\
             <meta property=\"og:title\" content=\"screenshot.png\"/>\
             <meta property=\"og:description\" content=\"PNG \u{b7} 1920x1080 \u{b7} 240.0 KB\"/>\
             <meta property=\"og:url\" content=\"https://h.example.com/alicja/kalit2.png\"/>\
             <meta property=\"og:site_name\" content=\"example\"/>\
             <meta property=\"og:image\" content=\"https://h.example.com/alicja/kalit2/raw.png?w=1200&amp;fit=inside\"/>\
             <meta property=\"og:image:secure_url\" content=\"https://h.example.com/alicja/kalit2/raw.png?w=1200&amp;fit=inside\"/>\
             <meta property=\"og:image:type\" content=\"image/webp\"/>\
             <meta property=\"og:image:width\" content=\"1200\"/>\
             <meta property=\"og:image:height\" content=\"675\"/>\
             <meta property=\"og:image:alt\" content=\"screenshot.png\"/>\
             <meta property=\"og:type\" content=\"website\"/>\
             <meta name=\"twitter:card\" content=\"summary_large_image\"/>\
             <meta name=\"twitter:title\" content=\"screenshot.png\"/>\
             <meta name=\"twitter:description\" content=\"PNG \u{b7} 1920x1080 \u{b7} 240.0 KB\"/>\
             <meta name=\"twitter:image\" content=\"https://h.example.com/alicja/kalit2/raw.png?w=1200&amp;fit=inside\"/>\
             <link rel=\"icon\" href=\"/icon.svg\" sizes=\"any\" type=\"image/svg+xml\"/>\
             <link rel=\"apple-touch-icon\" href=\"/apple-icon.png\" sizes=\"180x180\" type=\"image/png\"/>"
        );
    }

    #[test]
    fn elide_middle_like_javascript() {
        assert_eq!(elide_middle("screenshot.png", 100), "screenshot.png");
        let long = format!("{}.pdf", "a".repeat(200));
        let out = elide_middle(&long, 100);
        assert!(out.len() <= 100 && out.ends_with(".pdf") && out.contains("..."));
        assert_eq!(out, format!("{}...{}.pdf", "a".repeat(56), "a".repeat(37)));
        assert!(elide_middle(&"b".repeat(300), 40).len() <= 40);
        // UTF-16 counting: an emoji is two units, and a cut through it leaves
        // a replacement character exactly where JavaScript left a lone surrogate.
        let emoji = "\u{1F600}".repeat(60);
        let out = elide_middle(&emoji, 100);
        assert!(out.encode_utf16().count() <= 100);
        // room < 8: plain truncation.
        let ext_heavy = format!("{}.abcdefghijk", "x".repeat(200));
        assert_eq!(elide_middle(&ext_heavy, 20), format!("{}...", "x".repeat(17)));
    }
}
