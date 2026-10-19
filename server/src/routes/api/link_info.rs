//! Port of `app/api/link-info/route.ts`: `GET /api/link-info?url=<any
//! Modlight URL>`, what is behind a link, in JSON, from the database row.
//!
//! Three rules shape it (upstream's own):
//! 1. It consumes nothing: no view, no download, no one-time link burned, no
//!    unlock cookie read or issued, no short-link click counted.
//! 2. It is not an oracle: an anonymous caller learns nothing a `HEAD` on the
//!    raw URL plus the preview page's meta tags would not already say. Every
//!    kind of absence is the same 404 body; a gated file gives up only that
//!    it is gated (not even its extension).
//! 3. It answers from the row: no storage, no libvips, no ffmpeg.

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use axum::routing::get;
use serde_json::{Map, Value, json};

use crate::api_caller::{ApiAuth, ApiCaller, resolve_api_caller};
use crate::api_token::TokenCapability;
use crate::auth_helpers::Sessions;
use crate::config::js_trim;
use crate::data::user_prefs::resolve_tagless_owner_tag;
use crate::db::models::{Columns, ShortLink};
use crate::db::types::JsDate;
use crate::error::{AppError, AppResult};
use crate::format::human_file_size;
use crate::http::cookies::encode_uri_component;
use crate::link_info::parse::{LinkHosts, LinkTarget, MediaRef, MediaShape, ParseRefusal, Parsed, parse_link_url};
use crate::media::av_targets::{AvCost, AvSourceSize, av_targets_for};
use crate::media::convert_targets::convert_targets_for;
use crate::media::mime::{MediaKind, effective_type, media_kind, serve_decision};
use crate::media::motion_preview::{
    AV_MAX_INPUT_BYTES, MAX_TRANSFORM_BYTES, MOTION_PARAM, MotionCandidate, can_motion_preview, clamp_motion_edge,
};
use crate::media::serve::{MediaLookup, cache_headers_for, get_media};
use crate::media::urls::{download_url, public_url, raw_url, user_base_url};
use crate::naming::split_short_code;
use crate::state::AppState;

/// Thumbnail size the panel's grid uses, one the raw route snaps to.
const THUMB_PX: u32 = 480;

/// `MAX_INLINE_TEXT` of `lib/media/source-preview.ts` (512 KB): the largest
/// text the preview page renders inline.
pub const MAX_INLINE_TEXT: i64 = 512 * 1024;

fn motion_px() -> u32 {
    clamp_motion_edge(Some(THUMB_PX)).unwrap_or(THUMB_PX)
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/link-info", get(link_info))
}

/// `json(body, status)`: `JSON.stringify(body, null, 2)`, per-caller headers.
fn json_out(status: StatusCode, body: &Value) -> Response {
    let text = serde_json::to_string_pretty(body).unwrap_or_else(|_| "null".into());
    let mut res = Response::new(axum::body::Body::from(text));
    *res.status_mut() = status;
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    h.insert(header::VARY, HeaderValue::from_static("Authorization, Cookie"));
    res
}

/// An answer before it is serialised (a short link embeds its target's).
type Answer = (StatusCode, Value);

fn refusal(reason: &str, message: &str, input: Option<&str>) -> Answer {
    let status = if reason == "not-found" { StatusCode::NOT_FOUND } else { StatusCode::BAD_REQUEST };
    let mut body = Map::new();
    body.insert("ok".into(), json!(false));
    body.insert("reason".into(), json!(reason));
    body.insert("message".into(), json!(message));
    if let Some(input) = input.filter(|i| !i.is_empty()) {
        body.insert("input".into(), json!(input));
    }
    (status, Value::Object(body))
}

/// The single answer for "there is no file here".
fn not_found(input: &str) -> Answer {
    refusal(
        "not-found",
        "No file answers that link, or it is not available to you. It may never have existed, or it may be unlisted, expired or spent.",
        Some(input),
    )
}

async fn link_info(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, _) = req.into_parts();
    let input = form_urlencoded::parse(parts.uri.query().unwrap_or("").as_bytes())
        .find(|(k, _)| k == "url")
        .map(|(_, v)| v.into_owned());
    let Some(input) = input.filter(|i| !js_trim(i).is_empty()) else {
        return Ok(json_out(StatusCode::BAD_REQUEST, &usage(&state)));
    };

    let parsed = match parse_link_url(&LinkHosts::of(&state.config), Some(&input)) {
        Parsed::Ok(p) => *p,
        Parsed::Refused { reason, message } => {
            let (status, body) = refusal(reason.as_str(), &message, Some(&input));
            return Ok(json_out(status, &body));
        }
        Parsed::Malformed => return Err(AppError::internal(anyhow::anyhow!("URIError: URI malformed"))),
    };

    // Token first, session second; a token that cannot read describes links
    // as a stranger would (this endpoint has an anonymous mode by design).
    let caller = match resolve_api_caller(&state, &session, &parts.headers, TokenCapability::Read).await? {
        ApiAuth::Ok { caller, .. } => Some(caller),
        ApiAuth::Refused { .. } => None,
    };

    let (status, body) = describe(&state, &parsed.target, caller.as_ref(), &input, parsed.warnings).await?;
    Ok(json_out(status, &body))
}

async fn describe(
    state: &AppState,
    target: &LinkTarget,
    caller: Option<&ApiCaller>,
    input: &str,
    warnings: Vec<String>,
) -> anyhow::Result<Answer> {
    let app = &state.config.app_url;
    Ok(match target {
        LinkTarget::Media { primary, fallback } => {
            describe_media(state, primary, fallback.as_ref(), caller, input, warnings).await?
        }
        LinkTarget::ShortLink { stored, .. } => describe_short_link(state, stored, caller, input, warnings).await?,
        LinkTarget::FolderGallery { tag, slug, .. } => (
            StatusCode::OK,
            json!({
                "ok": true,
                "kind": "folder-gallery",
                "status": "unchecked",
                "input": input,
                "warnings": warnings,
                "gallery": { "tag": tag, "slug": slug },
                "links": { "page": format!("{app}/a/{tag}/{slug}"), "zip": format!("{app}/a/{tag}/{slug}/zip") },
                "notes": [
                    "A shared folder gallery. Its files are ordinary uploads, so each tile on the page has its own link this endpoint can describe.",
                    "Whether the folder exists and is published was not checked, because that answer belongs to the page itself and probing it here would make this a folder oracle.",
                    "The zip link streams the whole folder. It has no size limit, so check the page first.",
                ],
            }),
        ),
        LinkTarget::UserGallery { tag } => (
            StatusCode::OK,
            json!({
                "ok": true,
                "kind": "user-gallery",
                "status": "unchecked",
                "input": input,
                "warnings": warnings,
                "gallery": { "tag": tag },
                "links": { "page": format!("{app}/u/{tag}") },
                "notes": [
                    "A public profile gallery. It answers 404 unless the account turned public profiles on, and it lists at most 200 published files.",
                    "Whether this account exists was not checked here.",
                ],
            }),
        ),
        LinkTarget::AppRoute { path, label } => (
            StatusCode::OK,
            json!({
                "ok": true,
                "kind": "app-route",
                "status": "unchecked",
                "input": input,
                "warnings": warnings,
                "route": { "path": path, "label": label },
                "notes": [format!(
                    "That URL is {label}. There is no upload behind it, so there is nothing to describe from the database."
                )],
            }),
        ),
    })
}

/// One reading of a URL to a row, honouring the tagless form.
async fn lookup(state: &AppState, r: &MediaRef) -> anyhow::Result<Option<MediaLookup>> {
    if let Some(tag) = &r.tag {
        return get_media(&state.db, tag, &r.code).await;
    }
    match resolve_tagless_owner_tag(&state.db, &r.code).await? {
        Some(tag) => get_media(&state.db, &tag, &r.code).await,
        None => Ok(None),
    }
}

/// `thumbUrlFor(kind, size, raw)`: only where `/raw?w=` really returns a tile.
fn thumb_url_for(kind: MediaKind, size: i64, raw: &str) -> Option<String> {
    let fits = match kind {
        MediaKind::Video => size <= AV_MAX_INPUT_BYTES,
        MediaKind::Image => size <= MAX_TRANSFORM_BYTES,
        _ => false,
    };
    fits.then(|| format!("{raw}?w={THUMB_PX}&h={THUMB_PX}"))
}

async fn describe_media(
    state: &AppState,
    primary: &MediaRef,
    fallback: Option<&MediaRef>,
    caller: Option<&ApiCaller>,
    input: &str,
    mut warnings: Vec<String>,
) -> anyhow::Result<Answer> {
    // Tagged reading first, tagless second: the order the app resolves in.
    let mut r = primary;
    let mut found = lookup(state, primary).await?;
    if found.is_none()
        && let Some(fb) = fallback
        && let Some(alt) = lookup(state, fb).await?
    {
        r = fb;
        found = Some(alt);
        warnings.push("Read as a tagless action URL, not as /{tag}/{code}. Note that the redirect the app issues for this shape drops the query string, so a resize or a conversion target sent to it is lost. Use the tagged form to keep query parameters.".into());
    }
    let Some(found) = found else {
        return Ok(not_found(input));
    };
    let media = &found.media;
    let privileged = caller.is_some_and(|c| c.id == found.owner_id || c.role.as_deref() == Some("admin"));
    if !media.published && !privileged {
        return Ok(not_found(input));
    }

    let base = user_base_url(&state.config, found.owner_preferred_domain.as_deref());
    let has_password = media.password_hash.as_deref().is_some_and(|h| !h.is_empty());
    let status = if !media.published {
        "unlisted"
    } else if has_password {
        "password-required"
    } else {
        "public"
    };
    let tag = &found.owner_user_code;
    // A gated file gives up its EXTENSION too (the name can say everything).
    let gated = status == "password-required" && !privileged;
    let ext = if gated { "" } else { media.ext.as_str() };
    let page = public_url(tag, &media.code, ext, &base);
    let raw = raw_url(tag, &media.code, ext, &base);

    if gated {
        return Ok((
            StatusCode::OK,
            json!({
                "ok": true,
                "kind": "media",
                "status": status,
                "privileged": false,
                "input": input,
                "warnings": warnings,
                "normalizedUrl": page,
                "requestShape": r.shape.as_str(),
                "file": null,
                "links": { "page": page },
                "bytes": { "url": raw, "anonymous": false },
                "notes": [
                    "This file is password protected. Nothing about it is described here - not its name, type, size or dimensions - because its own preview page withholds all of that as well.",
                    "The preview page answers 200 with a password form. The raw and download URLs answer 401 until the form is cleared in a browser.",
                    "Sign in as the owner (API token or session) to see the full description.",
                ],
            }),
        ));
    }

    let mime = Some(media.mime_type.as_str());
    let file_ext = Some(media.ext.as_str());
    let kind_type = effective_type(mime, file_ext);
    let decision = serve_decision(mime, file_ext);
    let kind = media_kind(mime, file_ext);

    // The checksum is published exactly where the serving layer publishes it.
    let serve_headers = cache_headers_for(media);
    let public_bytes = !serve_headers.get("Cache-Control").is_some_and(|c| c.starts_with("private"));
    let etag = serve_headers.get("ETag").cloned();
    let is_text = media.kind == "paste" || kind == MediaKind::Text || media.mime_type.starts_with("text/");

    let mut notes: Vec<&str> = vec![
        "The extension in a Modlight URL is decorative: the lookup key is the code alone, so the same row answers with any extension or with none.",
    ];
    if r.shape == MediaShape::Preview {
        notes.push("The URL you asked about is the preview PAGE and returns HTML. The bytes are at the raw URL below.");
    }
    if !public_bytes {
        notes.push("The serving layer marks this link private and no-store, which means it carries at least one restriction: it may be time limited or view limited. Fetching the bytes may consume it. Which restriction applies is not disclosed to an anonymous caller.");
    }
    let thumbnail = thumb_url_for(kind, media.size, &raw);
    let candidate = MotionCandidate { mime_type: mime, ext: file_ext, size: media.size, frames: media.frames };
    let motion_thumbnail = can_motion_preview(candidate).then(|| {
        let px = motion_px();
        format!("{raw}?w={px}&h={px}&{MOTION_PARAM}=1")
    });
    if motion_thumbnail.is_some() {
        notes.push("This file moves, so a moving tile is offered as well: motionThumbnail is an animated WebP of frames sampled across the whole file, not a clip of its opening seconds. Like the still thumbnail it is a preview, so fetching it spends no view and burns no one-time link.");
    }

    let checksum = match &etag {
        Some(e) => Some(e.replace('"', "")),
        None if privileged => media.checksum.clone(),
        None => None,
    };
    let size_human = human_file_size(media.size as f64, 1);

    let mut body = Map::new();
    let mut put = |k: &str, v: Value| {
        body.insert(k.to_string(), v);
    };
    put("ok", json!(true));
    put("kind", json!("media"));
    put("status", json!(status));
    put("privileged", json!(privileged));
    put("input", json!(input));
    put("warnings", json!(warnings));
    put("normalizedUrl", json!(page));
    put("requestShape", json!(r.shape.as_str()));
    put("resolvedAs", json!({ "tag": tag, "code": media.code, "tagless": r.tag.is_none() }));
    put(
        "file",
        json!({
            "filename": media.filename,
            "ext": media.ext,
            "type": kind_type,
            "servedType": decision.content_type,
            "kind": kind.as_str(),
            "size": media.size,
            "sizeHuman": size_human,
            "width": media.width,
            "height": media.height,
            "checksum": checksum,
            "isPaste": media.kind == "paste",
        }),
    );
    put(
        "links",
        json!({
            "page": page,
            "raw": raw,
            "download": download_url(tag, &media.code, &base),
            "thumbnail": thumbnail,
            "motionThumbnail": motion_thumbnail,
            "oembed": format!("{base}/oembed?url={}&format=json", encode_uri_component(&page)),
        }),
    );
    put(
        "bytes",
        json!({
            "url": raw,
            "size": media.size,
            "sizeHuman": size_human,
            "contentType": decision.content_type,
            "disposition": decision.disposition,
            "rangeSupported": true,
            "anonymous": media.published && !has_password,
            "cacheable": public_bytes,
            "etag": etag,
        }),
    );
    put(
        "text",
        if is_text {
            json!({
                "language": media.language,
                "url": raw,
                "inlineLimitBytes": MAX_INLINE_TEXT,
                "renderedInline": media.size <= MAX_INLINE_TEXT,
            })
        } else {
            Value::Null
        },
    );
    put("convert", convert_plan(&found, &base, tag));
    if privileged {
        put("owner", owner_block(&found));
    }
    put("notes", json!(notes));
    Ok((StatusCode::OK, Value::Object(body)))
}

/// What "Download as" offers for this file, from the row alone.
fn convert_plan(found: &MediaLookup, base: &str, tag: &str) -> Value {
    let media = &found.media;
    let frames = media.frames.map(f64::from);
    let mime = Some(media.mime_type.as_str());
    let ext = Some(media.ext.as_str());
    let mut targets: Vec<Value> = convert_targets_for(mime, ext, frames)
        .into_iter()
        .map(|o| json!({ "to": o.to.as_str(), "requiresOwner": false, "motion": o.motion.map(|m| m.as_str()) }))
        .collect();
    let size = AvSourceSize { width: media.width.map(f64::from), height: media.height.map(f64::from) };
    targets.extend(av_targets_for(mime, ext, frames, Some(&size)).into_iter().map(|o| {
        json!({
            "to": o.to.as_str(),
            "requiresOwner": o.cost == AvCost::Encode,
            "motion": null,
            "yields": o.yields.as_str(),
        })
    }));
    let convert_base = format!("{base}/{tag}/{}/convert", media.code);
    json!({
        "urlTemplate": format!("{convert_base}?to={{target}}"),
        "dryRunTemplate": format!("{convert_base}?to={{target}}&check=1"),
        "targets": targets,
    })
}

/// The owner's own view of their own row (never `delete_token` or
/// `storage_key`).
fn owner_block(found: &MediaLookup) -> Value {
    let m = &found.media;
    json!({
        "id": m.id,
        "ownerId": found.owner_id,
        "ownerTag": found.owner_user_code,
        "published": m.published,
        "createdAt": m.created_at,
        "updatedAt": m.updated_at,
        "views": m.views,
        "downloads": m.downloads,
        "lastAccessedAt": m.last_accessed_at,
        "expiresAt": m.expires_at,
        "maxViews": m.max_views,
        "hasPassword": m.password_hash.as_deref().is_some_and(|h| !h.is_empty()),
        "passwordVersion": m.password_version,
        "folderId": m.folder_id,
        "checksum": m.checksum,
        "blurhash": m.blurhash,
        "frames": m.frames,
        "convertedAt": m.converted_at,
        "originalExt": m.original_ext,
    })
}

/// Resolve a short link WITHOUT following it (no click counted).
async fn describe_short_link(
    state: &AppState,
    stored: &str,
    caller: Option<&ApiCaller>,
    input: &str,
    warnings: Vec<String>,
) -> anyhow::Result<Answer> {
    let sql =
        format!(r#"select {} from {} where "short_links"."code" = $1 limit 1"#, ShortLink::COLUMNS, ShortLink::TABLE);
    let Some(link) = sqlx::query_as::<_, ShortLink>(&sql).bind(stored).fetch_optional(&state.db).await? else {
        return Ok(not_found(input));
    };
    let owned = caller.is_some_and(|c| c.id == link.owner_id || c.role.as_deref() == Some("admin"));
    let (tag, code) = split_short_code(&link.code);
    let short = &state.config.short_url;
    let has_password = link.password_hash.as_deref().is_some_and(|h| !h.is_empty());

    let mut body = Map::new();
    body.insert("ok".into(), json!(true));
    body.insert("kind".into(), json!("shortlink"));
    body.insert("privileged".into(), json!(owned));
    body.insert("input".into(), json!(input));
    body.insert("warnings".into(), json!(warnings));
    body.insert("shortlink".into(), json!({ "code": code, "tag": tag, "stored": link.code }));
    body.insert(
        "links".into(),
        json!({ "canonical": format!("{short}/s/{}", link.code), "onShortDomain": format!("{short}/{}", link.code) }),
    );
    if owned {
        body.insert(
            "owner".into(),
            json!({
                "id": link.id,
                "ownerId": link.owner_id,
                "clicks": link.clicks,
                "lastUsedAt": link.last_used_at,
                "createdAt": link.created_at,
                "expiresAt": link.expires_at,
                "hasPassword": has_password,
                "target": link.target,
            }),
        );
    }

    // Expiry is publicly distinguishable already (the redirect answers 410).
    if link.expires_at.is_some_and(|e| e.millis() <= JsDate::now().millis()) {
        body.insert("status".into(), json!("expired"));
        body.insert("target".into(), Value::Null);
        body.insert("notes".into(), json!(["This short link has expired. Following it returns 410."]));
        return Ok((StatusCode::OK, Value::Object(body)));
    }

    if has_password {
        body.insert("status".into(), json!("password-required"));
        body.insert("target".into(), if owned { json!(link.target) } else { Value::Null });
        // Re-assigning keeps the key where `shared` put it, as in JavaScript.
        if let Some(Value::Object(links)) = body.get_mut("links") {
            links.insert("unlock".into(), json!(format!("{}/s/{}/unlock", state.config.app_url, link.code)));
        }
        body.insert(
            "notes".into(),
            json!([
                "This short link is password protected. Following it redirects to an unlock page instead of to its destination.",
                "The destination is withheld, because handing it over would be exactly the redirect the password was set to prevent.",
            ]),
        );
        return Ok((StatusCode::OK, Value::Object(body)));
    }

    body.insert("status".into(), json!("public"));
    body.insert("target".into(), json!(link.target));
    body.insert(
        "notes".into(),
        json!(["Following this link returns 302 to the target and counts one click. This description counted none."]),
    );

    // One hop, and one only: a link to a file on this deployment is described too.
    match parse_link_url(&LinkHosts::of(&state.config), Some(&link.target)) {
        Parsed::Ok(inner) => {
            if let LinkTarget::Media { primary, fallback } = &inner.target {
                let (_, info) =
                    Box::pin(describe_media(state, primary, fallback.as_ref(), caller, &link.target, Vec::new()))
                        .await?;
                body.insert("targetInfo".into(), info);
            }
        }
        Parsed::Malformed => anyhow::bail!("URIError: URI malformed"),
        Parsed::Refused { .. } => {}
    }
    Ok((StatusCode::OK, Value::Object(body)))
}

/// The answer to a call with no `url`: the endpoint explains itself.
fn usage(state: &AppState) -> Value {
    let app = &state.config.app_url;
    json!({
        "ok": false,
        "reason": ParseRefusal::MissingUrl.as_str(),
        "message": "Pass the URL to describe as the `url` query parameter.",
        "endpoint": {
            "method": "GET",
            "path": "/api/link-info",
            "parameters": { "url": "Any URL served by this deployment, absolute or as a bare path." },
            "example": format!("{app}/api/link-info?url={}", encode_uri_component(&format!("{app}/tag/code.png"))),
            "auth": "Optional. An API token (Authorization: Bearer, or X-API-Key) or a session cookie adds the owner's own fields for their own files.",
        },
        "describes": [
            "/{tag}/{code}[.ext] and /{code}[.ext] - the preview page",
            "/{tag}/{code}/raw[.ext], /download, /convert, /render - the file itself",
            "/s/{code} and /s/{tag}/{name} - short links, resolved without counting a click",
            "/a/{tag}/{slug} and /u/{tag} - shared galleries",
        ],
        "guarantees": [
            "Never counts a view or a download, never burns a one-time link, never follows a short link.",
            "Answers from the database row. It never reads the file.",
            "Tells an anonymous caller nothing that a HEAD on the raw URL would not.",
        ],
        "knownOrigins": state.config.app_domains,
    })
}
