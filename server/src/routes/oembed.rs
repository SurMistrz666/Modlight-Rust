//! Port of `app/oembed/route.ts`: the oEmbed provider endpoint
//! (https://oembed.com), `GET /oembed?url={publicUrl}&format=json`. Rich
//! clients discover it through the `<link rel="alternate"
//! type="application/json+oembed">` the media page's metadata emits.
//!
//! Security fix 6 (PORT.md 12.6): a published file with a view cap is described
//! the way `lib/media/embed.ts` describes it on the page - by name, never by its
//! bytes. Upstream handed out the raw URL (and the dimensions, and for a video a
//! player pointing at the raw URL), which an unfurler fetches, keeps and serves
//! publicly forever, spending one of the views on the way. Here such a file is
//! a `link` with its title and provider and nothing that points at the file.

use axum::Router;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::get;
use serde_json::{Map, Value, json};

use crate::api_json::json_response;
use crate::error::AppResult;
use crate::media::mime::{MediaKind, media_kind};
use crate::media::serve::get_media;
use crate::media::urls::{raw_url, user_base_url};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/oembed", get(oembed))
}

fn error(status: StatusCode, message: &str) -> Response {
    json_response(status, &json!({ "error": message }), None)
}

/// `oembedJson(body)`
fn oembed_json(body: Map<String, Value>) -> Response {
    let mut res = json_response(StatusCode::OK, &Value::Object(body), Some("public, max-age=86400, s-maxage=86400"));
    res.headers_mut().insert("access-control-allow-origin", HeaderValue::from_static("*"));
    res
}

/// `escapeAttr(s)`
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

async fn oembed(State(state): State<AppState>, RawQuery(query): RawQuery) -> AppResult<Response> {
    let params: Vec<(String, String)> =
        form_urlencoded::parse(query.as_deref().unwrap_or("").as_bytes()).into_owned().collect();
    let get = |key: &str| params.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    let target = get("url");
    let format = get("format").unwrap_or("json");

    if format != "json" {
        return Ok(error(StatusCode::NOT_IMPLEMENTED, "Only json format is supported"));
    }
    let Some(target) = target.filter(|t| !t.is_empty()) else {
        return Ok(error(StatusCode::BAD_REQUEST, "Missing url parameter"));
    };

    // Parse {userCode}/{code}.{ext} out of the supplied public URL; trailing
    // /raw, /download etc. are tolerated - only the first two segments count.
    let Ok(parsed) = url::Url::parse(target) else {
        return Ok(error(StatusCode::BAD_REQUEST, "Invalid url parameter"));
    };
    let mut segments = parsed.path().split('/').filter(|s| !s.is_empty());
    let (Some(user_code), Some(media_code)) = (segments.next(), segments.next()) else {
        return Ok(error(StatusCode::BAD_REQUEST, "Invalid url parameter"));
    };

    let Some(found) = get_media(&state.db, user_code, media_code).await? else {
        return Ok(error(StatusCode::NOT_FOUND, "Not found"));
    };
    let media = &found.media;
    // Don't leak embeds for private or password-protected media.
    if !media.published || media.password_hash.as_deref().is_some_and(|h| !h.is_empty()) {
        return Ok(error(StatusCode::NOT_FOUND, "Not found"));
    }

    // Same hostname the owner's other links use, so the embed and the link the
    // viewer clicked agree with each other.
    let owner_base = user_base_url(&state.config, found.owner_preferred_domain.as_deref());
    let raw = raw_url(&found.owner_user_code, &media.code, &media.ext, &owner_base);
    let kind = media_kind(Some(&media.mime_type), Some(&media.ext));

    // `author_name` is deliberately absent; the provider name is the brand,
    // exactly as og:site_name says it.
    let mut body = Map::new();
    body.insert("version".into(), json!("1.0"));
    body.insert("title".into(), json!(media.filename));
    body.insert("provider_name".into(), json!(state.config.embed_brand));
    body.insert("provider_url".into(), json!(owner_base));
    body.insert("cache_age".into(), json!(86400));

    // Security fix 6: a capped file is a card without its bytes, as on the page.
    if media.max_views.is_some() {
        body.insert("type".into(), json!("link"));
        return Ok(oembed_json(body));
    }

    match kind {
        MediaKind::Image => {
            body.insert("type".into(), json!("photo"));
            body.insert("url".into(), json!(raw));
            body.insert("width".into(), json!(media.width.unwrap_or(1200)));
            body.insert("height".into(), json!(media.height.unwrap_or(630)));
        }
        MediaKind::Video => {
            let (w, h) = (media.width.unwrap_or(1280), media.height.unwrap_or(720));
            body.insert("type".into(), json!("video"));
            body.insert("width".into(), json!(w));
            body.insert("height".into(), json!(h));
            // No thumbnail_url: the thumbnail endpoint would stream the clip.
            body.insert(
                "html".into(),
                json!(format!(
                    "<video src=\"{}\" controls width=\"{w}\" height=\"{h}\" style=\"max-width:100%;border-radius:12px\"></video>",
                    escape_attr(&raw)
                )),
            );
        }
        // Everything else degrades to a link card.
        _ => {
            body.insert("type".into(), json!("link"));
            body.insert("url".into(), json!(raw));
        }
    }
    Ok(oembed_json(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_are_escaped_like_upstream() {
        assert_eq!(escape_attr("a&b\"c<d>"), "a&amp;b&quot;c&lt;d>");
    }
}
