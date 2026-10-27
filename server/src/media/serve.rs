//! Port of `lib/media/serve.ts`: looking a public file up, deciding who is
//! looking at it, counting the view, and streaming its bytes (Range, cache
//! headers, Content-Disposition, nosniff).
//!
//! Plus the one deliberate difference from upstream in this module, security
//! fix 1 (PORT.md 12.1): the VIEW GRANT. Upstream let a request skip the view
//! counter when it looked like a player seek (`Range` not starting at 0), an
//! embedded subresource (`Sec-Fetch-Dest: image|video`) or a thumbnail that fell
//! back to the original (`?w=`), so a burn-after-view link could be read
//! forever with `Range: bytes=1-`. Here those shapes still skip the counter, but
//! only for a request that proves a view of this file was counted for it: a
//! signed, short-lived `ml_view_<id>` cookie minted by the request that counted
//! (the preview page, or the opening `/raw` request). See [`ViewBudget`] for the
//! exact rule.

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::Mac;
use indexmap::IndexMap;
use sqlx::PgPool;

use crate::api_token::{TokenAuth, TokenCapability, authenticate_api_token, bearer_from_request};
use crate::auth_helpers::RequestSession;
use crate::background::{BackgroundStyle, background_style};
use crate::db::models::{Columns, Upload};
use crate::db::types::JsDate;
use crate::http::cookies::{encode_uri_component, parse_cookies_next};
use crate::http::set_cookie::{SameSite, SetCookie};
use crate::media::mime::serve_decision;
use crate::media::paste_type::row_mime_type;
use crate::media::paths::code_from_media_code;
use crate::state::AppState;
use crate::storage::ByteRange;

/// `MediaLookup`
#[derive(Debug, Clone, PartialEq)]
pub struct MediaLookup {
    /// The row, with a paste's placeholder type resolved (`rowMimeType`).
    pub media: Upload,
    pub owner_user_code: String,
    pub owner_id: String,
    pub owner_theme: String,
    /// The owner's wallpaper, so a shared link is decorated the way its owner
    /// decorated their panel - the same promise `owner_theme` already makes.
    pub owner_background: Option<BackgroundStyle>,
    /// Hostname the owner generates links with (None = deployment default).
    pub owner_preferred_domain: Option<String>,
    /// Not upstream: the owner's file vault state, when there is one
    /// (`encrypting` | `on` | `decrypting`; docs/ENCRYPTION.md).
    pub owner_vault: Option<String>,
}

/// What a public `{userCode}/{code}` resolves to.
#[derive(Debug, Clone, PartialEq)]
pub enum Lookup {
    /// What `getMedia` returns.
    Live(MediaLookup),
    /// Everything `getMedia` checks passed except the view budget, which is
    /// spent EXACTLY (`views == max_views`): the file is gone for everybody,
    /// except a request holding the view grant of the view that spent it
    /// (security fix 1). A budget overspent by the owner lowering the cap
    /// (`views > max_views`) is [`Lookup::Missing`].
    Spent(MediaLookup),
    Missing,
}

impl Lookup {
    pub fn live(self) -> Option<MediaLookup> {
        match self {
            Lookup::Live(m) => Some(m),
            _ => None,
        }
    }
}

/// `getMedia(userCode, mediaCode)`: a live file by its public
/// `{userCode}/{code}`, honouring soft-delete, orphaning, the owner's current
/// or retired tag, expiry and the view budget. `None` is the 404 for all of
/// them. A malformed percent-escape in `media_code` is an error, as
/// `decodeURIComponent` throws upstream. Access control (unpublished/password)
/// is applied by callers with the requester's session.
pub async fn get_media(db: &PgPool, user_code: &str, media_code: &str) -> anyhow::Result<Option<MediaLookup>> {
    Ok(lookup_media(db, user_code, media_code).await?.live())
}

/// [`get_media`] that also tells a spent budget apart from a missing file.
pub async fn lookup_media(db: &PgPool, user_code: &str, media_code: &str) -> anyhow::Result<Lookup> {
    let code = code_from_media_code(media_code).ok_or_else(|| anyhow::anyhow!("URIError: URI malformed"))?;
    let sql = format!(
        r#"select {} from {} where ("uploads"."code" = $1 and "uploads"."deleted_at" is null) limit 1"#,
        Upload::COLUMNS,
        Upload::TABLE
    );
    let Some(media) = sqlx::query_as::<_, Upload>(&sql).bind(&code).fetch_optional(db).await? else {
        return Ok(Lookup::Missing);
    };
    // Orphaned file (its account was deleted with "keep the files"). The public
    // URL is /{tag}/{code} and the tag belongs to the owner - with no owner there
    // is nothing to match it against, so the file is simply not on the web.
    let Some(owner_id) = media.owner_id.clone() else {
        return Ok(Lookup::Missing);
    };
    type OwnerRow = (String, Option<String>, Option<String>, Option<String>, Option<String>, i32, Option<String>);
    let owner: Option<OwnerRow> = sqlx::query_as(
        r#"select "id", "user_code", "theme", "preferred_domain", "background_key", "background_dim",
                  (select "state" from "file_vaults" where "file_vaults"."user_id" = "user"."id") as "vault"
           from "user" where "user"."id" = $1 limit 1"#,
    )
    .bind(&owner_id)
    .fetch_optional(db)
    .await?;
    let Some((id, owner_code, theme, preferred_domain, background_key, background_dim, vault)) = owner else {
        return Ok(Lookup::Missing);
    };
    // The tag in the URL must belong to the owner - either their current one or a
    // tag they used previously. Without the alias check, letting a user rename
    // their tag would 404 every link they had ever shared.
    if owner_code.as_deref() != Some(user_code) {
        let alias: Option<String> = sqlx::query_scalar(
            r#"select "user_id" from "user_code_aliases" where "user_code_aliases"."code" = $1 limit 1"#,
        )
        .bind(user_code)
        .fetch_optional(db)
        .await?;
        if alias.as_deref() != Some(id.as_str()) {
            return Ok(Lookup::Missing);
        }
    }
    // expiry / burn-after-views
    if media.expires_at.is_some_and(|e| e.millis() <= JsDate::now().millis()) {
        return Ok(Lookup::Missing);
    }
    let spent = match media.max_views {
        Some(max) if media.views > max => return Ok(Lookup::Missing),
        Some(max) => media.views >= max,
        None => false,
    };
    // A paste's stored type was a placeholder, so every reader of this row is
    // given the type its language declares. Nothing changes on the wire:
    // `stream_media` still runs it through `serve_decision`.
    let mime = row_mime_type(Some(&media.kind), media.language.as_deref(), &media.mime_type);
    let found = MediaLookup {
        owner_background: background_style(owner_code.as_deref(), background_key.as_deref(), Some(background_dim)),
        owner_user_code: owner_code.unwrap_or_else(|| user_code.to_string()),
        owner_id: id,
        owner_theme: theme.unwrap_or_else(|| "modern".to_string()),
        owner_preferred_domain: preferred_domain,
        media: Upload {
            mime_type: mime,
            vault_gated: vault.as_deref().and_then(crate::vault::VaultState::parse).is_some_and(|s| s.is_gated()),
            ..media
        },
        owner_vault: vault,
    };
    Ok(if spent { Lookup::Spent(found) } else { Lookup::Live(found) })
}

// ── Who is looking ──────────────────────────────────────────────────────────

/// `{ privileged, isOwner }`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewerRelation {
    /// The owner or an administrator: may see an unpublished file, and neither
    /// spends its view budget.
    pub privileged: bool,
    /// The person whose file this is (the playback note reads this alone).
    pub is_owner: bool,
}

const NOBODY: ViewerRelation = ViewerRelation { privileged: false, is_owner: false };

fn relate(viewer_id: &str, role: Option<&str>, owner_id: &str) -> ViewerRelation {
    let is_owner = viewer_id == owner_id;
    ViewerRelation { privileged: is_owner || role == Some("admin"), is_owner }
}

/// `viewerRelation(ownerId)`.
///
/// An API token is the account holder: the desktop client draws its library
/// from thumbnails and sends no cookies at all, so the token is checked FIRST.
/// SCOPE: `read` - an `upload`-scoped token (the one in a ShareX config) is an
/// anonymous visitor here. Without a token, the cheap path: no cookie whose name
/// contains `session_token` means no session lookup at all.
pub async fn viewer_relation(
    db: &PgPool,
    headers: &HeaderMap,
    session: &RequestSession,
    owner_id: &str,
) -> anyhow::Result<ViewerRelation> {
    if let Some(token) = bearer_from_request(headers) {
        return Ok(match authenticate_api_token(db, Some(&token), TokenCapability::Read).await? {
            TokenAuth::Ok { user, .. } => relate(&user.id, user.role.as_deref(), owner_id),
            _ => NOBODY,
        });
    }
    if !parse_cookies_next(headers).keys().any(|name| name.contains("session_token")) {
        return Ok(NOBODY);
    }
    let Some(s) = session.get().await? else {
        return Ok(NOBODY);
    };
    Ok(relate(&s.user.id, s.user.role.as_deref(), owner_id))
}

/// `isPrivilegedViewer(ownerId)`
pub async fn is_privileged_viewer(
    db: &PgPool,
    headers: &HeaderMap,
    session: &RequestSession,
    owner_id: &str,
) -> anyhow::Result<bool> {
    Ok(viewer_relation(db, headers, session, owner_id).await?.privileged)
}

// ── Counting ────────────────────────────────────────────────────────────────

/// `consumeView(media, { download })`: count one access, atomically refusing to
/// go past `maxViews`.
///
/// Burn-after-view has to be enforced by the database, not by a read followed by
/// a write: two concurrent requests for a one-time link would both read views=0
/// and both be served. `UPDATE … WHERE views < max_views RETURNING` lets exactly
/// one of them win. Returns false when the budget is already spent (→ 404).
pub async fn consume_view(db: &PgPool, media: &Upload, download: bool) -> anyhow::Result<bool> {
    let set = if download {
        r#""views" = "uploads"."views" + 1, "downloads" = "uploads"."downloads" + 1, "last_accessed_at" = $1"#
    } else {
        r#""views" = "uploads"."views" + 1, "last_accessed_at" = $1"#
    };
    let now = JsDate::now();
    // Uncapped file: the counter is statistics, so don't make the client wait on
    // a write before its bytes start flowing.
    if media.max_views.is_none() {
        let (db, id) = (db.clone(), media.id);
        let sql = format!(r#"update "uploads" set {set} where "uploads"."id" = $2"#);
        tokio::spawn(async move {
            let _ = sqlx::query(&sql).bind(now).bind(id).execute(&db).await;
        });
        return Ok(true);
    }
    let sql = format!(
        r#"update "uploads" set {set} where ("uploads"."id" = $2 and ("uploads"."max_views" is null or "uploads"."views" < "uploads"."max_views")) returning "id""#
    );
    let row: Option<uuid::Uuid> = sqlx::query_scalar(&sql).bind(now).bind(media.id).fetch_optional(db).await?;
    Ok(row.is_some())
}

// ── Security fix 1: the view grant ──────────────────────────────────────────

/// How long a counted view keeps covering the requests that belong to it (a
/// player's seeks, the page's own `<img>`/`<video>`, the second half of an
/// encrypted download). Long enough to watch a feature-length video with
/// pauses; it is never extended by the requests it covers.
pub const VIEW_GRANT_TTL: i64 = 6 * 60 * 60;

/// `ml_view_<uploadId>`
pub fn view_grant_cookie_name(media_id: &str) -> String {
    format!("ml_view_{media_id}")
}

fn grant_signature(secret: &str, media_id: &str, exp: i64) -> String {
    let message = ["view", media_id, &exp.to_string()].map(encode_uri_component).join("|");
    let mut mac = <hmac::Hmac<sha2::Sha256> as Mac>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(message.as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

/// `<exp>.<HMAC-SHA256(secret, "view|<id>|<exp>")>`, the same shape as the
/// unlock tokens of `lib/password.ts`, under a scope of its own.
pub fn mint_view_grant(secret: &str, media_id: &str) -> String {
    let exp = JsDate::now().millis().div_euclid(1000) + VIEW_GRANT_TTL;
    format!("{exp}.{}", grant_signature(secret, media_id, exp))
}

pub fn verify_view_grant(secret: &str, token: Option<&str>, media_id: &str) -> bool {
    let Some((raw_exp, sig)) = token.and_then(|t| t.split_once('.')) else {
        return false;
    };
    let Ok(exp) = raw_exp.parse::<i64>() else {
        return false;
    };
    if exp <= 0 || exp.saturating_mul(1000) <= JsDate::now().millis() {
        return false;
    }
    crate::auth::crypto::constant_time_eq(sig.as_bytes(), grant_signature(secret, media_id, exp).as_bytes())
}

/// The grant cookie: httpOnly, path `/`, `SameSite=Lax`, secure in production.
pub fn view_grant_cookie(secret: &str, media_id: &str, is_prod: bool) -> SetCookie {
    SetCookie::new(view_grant_cookie_name(media_id), mint_view_grant(secret, media_id))
        .http_only(true)
        .path("/")
        .same_site(SameSite::Lax)
        .secure(is_prod)
        .max_age(VIEW_GRANT_TTL)
}

/// Does this request carry the grant of a counted view of `media_id`?
pub fn has_view_grant(secret: &str, headers: &HeaderMap, media_id: &str) -> bool {
    let jar = parse_cookies_next(headers);
    verify_view_grant(secret, jar.get(&view_grant_cookie_name(media_id)).map(String::as_str), media_id)
}

/// Security fix 1, the exact rule. For a file WITH a view cap, a request by
/// anybody but its owner or an administrator:
///
/// 1. a request upstream counts (the opening request: no `Range` or one from
///    byte 0, no `Sec-Fetch-Dest: image|video`, no `w`/`h`/`motion`; a preview
///    bot counts too, as upstream already did for capped files) consumes a
///    view, or is refused with 404 when none is left - unchanged;
/// 2. a request upstream did NOT count (a seek, an embedded subresource, a
///    thumbnail or moving-tile request, a timeline sprite sheet) is served
///    without consuming only when it carries the view grant of this file, and
///    even when that grant's view was the last one (`views == max_views`);
///    without the grant it consumes a view exactly like case 1. This applies
///    whenever the response carries the file's content: its stored bytes
///    (`/raw`, `/raw.<ext>`, `/download`, whole or ranged, a thumbnail request
///    falling back to the original) or a picture made of them (a resized still,
///    which at up to 1200 px is the whole of most screenshots, a moving tile,
///    a timeline sprite sheet). A refusal (415/413/503) carries nothing
///    and spends nothing; the timeline's cue track (`timeline.vtt`, times and
///    rectangles, no pixels) keeps upstream's exemption;
/// 3. every view counted under 1 or 2 mints a grant for this file
///    (`ml_view_<id>`, [`VIEW_GRANT_TTL`]), and so does the preview page's
///    counted view, which is what keeps the page's own `<img>`/`<video>`, a
///    player's seeks and an encrypted file's second request working.
///
/// Files without a cap keep upstream's rules exactly (the counter is
/// statistics there).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewBudget {
    /// Nothing to count: privileged viewer, or an uncapped request upstream
    /// does not count.
    Free,
    /// Count it (uncapped: statistics; capped: refuse with 404 when spent).
    Consume,
    /// Capped, a shape upstream exempts, and the grant is present: serve.
    Granted,
}

/// Decide [`ViewBudget`] for a request that will deliver the stored bytes.
pub fn view_budget(media: &Upload, privileged: bool, upstream_counts: bool, has_grant: bool) -> ViewBudget {
    if privileged {
        return ViewBudget::Free;
    }
    if upstream_counts {
        return ViewBudget::Consume;
    }
    if media.max_views.is_none() {
        return ViewBudget::Free;
    }
    if has_grant { ViewBudget::Granted } else { ViewBudget::Consume }
}

// ── Caching ─────────────────────────────────────────────────────────────────

/// `cacheHeadersFor(media)`: nothing restricted lands in a shared cache; a
/// public file gets its checksum as the ETag and `immutable` unless it was
/// ever converted in place.
pub fn cache_headers_for(media: &Upload) -> IndexMap<&'static str, String> {
    let mut headers = IndexMap::new();
    let restricted = !media.published
        || media.password_hash.as_deref().is_some_and(|h| !h.is_empty())
        || media.expires_at.is_some()
        || media.max_views.is_some()
        // Not upstream: behind a file password of the whole account.
        || media.is_vaulted();
    if restricted {
        headers.insert("Cache-Control", "private, no-store".to_string());
        headers.insert("Vary", "Cookie".to_string());
        return headers;
    }
    // A validator, always. The checksum changes with the bytes.
    if let Some(sum) = media.checksum.as_deref().filter(|c| !c.is_empty()) {
        headers.insert("ETag", format!("\"{sum}\""));
    }
    // `immutable` is a promise that these bytes will never change at this URL,
    // and in-place conversion is the one thing in this app that breaks it.
    headers.insert(
        "Cache-Control",
        if media.converted_at.is_some() {
            "public, max-age=300, must-revalidate"
        } else {
            "public, max-age=31536000, immutable"
        }
        .to_string(),
    );
    headers
}

// ── Streaming ───────────────────────────────────────────────────────────────

/// `parseRange(header, size)`: one `bytes=a-b`, `bytes=a-` or `bytes=-n`
/// range; anything else (several ranges, a range past the end) is no range
/// at all, and the whole file is sent with 200 - upstream never answers 416.
pub fn parse_range(header: Option<&str>, size: u64) -> Option<ByteRange> {
    let h = crate::config::js_trim(header?);
    let rest = h.strip_prefix("bytes=")?;
    let (a, b) = rest.split_once('-')?;
    if !a.bytes().all(|c| c.is_ascii_digit()) || !b.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    // Number(...) of a digit string, then the comparisons upstream makes.
    let num = |s: &str| crate::config::js_number(s);
    let size = size as f64;
    let (start, end) = if a.is_empty() {
        // suffix: last N bytes
        let suffix = num(b);
        if !suffix.is_finite() || suffix <= 0.0 {
            return None;
        }
        ((size - suffix).max(0.0), size - 1.0)
    } else {
        let start = num(a);
        let end = if b.is_empty() { size - 1.0 } else { num(b).min(size - 1.0) };
        (start, end)
    };
    if !start.is_finite() || start > end || start < 0.0 {
        return None;
    }
    Some(ByteRange { start: start as u64, end: end as u64 })
}

/// A request header as Node hands it to a route handler: repeated headers
/// joined with ", " (Range and Sec-Fetch-Dest are not among the ones Node
/// keeps only the first of), bytes read as latin1.
pub fn joined_header(headers: &HeaderMap, name: &str) -> Option<String> {
    let mut values = headers.get_all(name).iter().peekable();
    values.peek()?;
    Some(values.map(|v| v.as_bytes().iter().map(|&b| b as char).collect::<String>()).collect::<Vec<_>>().join(", "))
}

/// A header Node keeps only the first of (`user-agent`).
pub fn first_header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name).map(|v| v.as_bytes().iter().map(|&b| b as char).collect())
}

pub use crate::http::web_response::{plain_text, put_headers};

/// `streamMedia(media, { rangeHeader, download })`: a streaming response for a
/// media file, with HTTP Range / 206 support.
pub async fn stream_media(state: &AppState, media: &Upload, range_header: Option<&str>, download: bool) -> Response {
    let storage = match state.storage.get() {
        Ok(s) => s,
        Err(err) => {
            tracing::error!(error = ?err, "storage unavailable");
            return plain_text(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error");
        }
    };
    let stat = match storage.stat(&media.storage_key).await {
        Ok(Some(stat)) => stat,
        Ok(None) => return plain_text(StatusCode::NOT_FOUND, "Not found"),
        Err(err) => {
            tracing::error!(error = ?err, "stat failed");
            return plain_text(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error");
        }
    };
    let size = stat.size;

    // The type we advertise is decided by the allowlist, never by the uploader's
    // MIME - combined with nosniff, that is what keeps an uploaded .html or .svg
    // from running script on our origin. The extension is passed too: plenty of
    // uploads arrive as application/octet-stream.
    let decided = serve_decision(Some(&media.mime_type), Some(&media.ext));
    let disposition = if download { "attachment" } else { decided.disposition };

    let mut pairs: Vec<(&str, String)> = vec![
        ("Content-Type", decided.content_type.clone()),
        ("X-Content-Type-Options", "nosniff".to_string()),
        ("Content-Disposition", format!("{disposition}; filename*=UTF-8''{}", encode_uri_component(&media.filename))),
    ];
    pairs.extend(cache_headers_for(media));

    let range = parse_range(range_header, size);
    let stream = match storage.read_stream(&media.storage_key, range).await {
        Ok(s) => s,
        Err(err) => {
            tracing::error!(error = ?err, "read failed");
            return plain_text(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error");
        }
    };
    let mut res = Response::new(Body::from_stream(stream));
    match range {
        Some(r) => {
            *res.status_mut() = StatusCode::PARTIAL_CONTENT;
            pairs.push(("Content-Range", format!("bytes {}-{}/{size}", r.start, r.end)));
            pairs.push(("Accept-Ranges", "bytes".to_string()));
            pairs.push(("Content-Length", (r.end - r.start + 1).to_string()));
        }
        None => {
            pairs.push(("Accept-Ranges", "bytes".to_string()));
            pairs.push(("Content-Length", size.to_string()));
        }
    }
    put_headers(&mut res, pairs);
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upload() -> Upload {
        serde_json::from_value(serde_json::json!({
            "id": "9d2e1a54-0000-4000-8000-000000000001", "ownerId": "u1", "code": "abc", "filename": "a.png",
            "ext": "png", "kind": "file", "language": null, "mimeType": "image/png", "size": 10,
            "storageKey": "t/abc.png", "checksum": "ff", "width": null, "height": null, "blurhash": null,
            "thumbKey": null, "published": true, "deleteToken": "d", "views": 0, "downloads": 0,
            "lastAccessedAt": null, "expiresAt": null, "maxViews": null, "passwordHash": null, "folderId": null,
            "deletedAt": null, "createdAt": "2026-01-01T00:00:00.000Z", "updatedAt": "2026-01-01T00:00:00.000Z",
            "passwordVersion": null, "frames": null, "convertedAt": null, "originalExt": null,
            "durationSeconds": null, "fps": null
        }))
        .unwrap()
    }

    #[test]
    fn ranges_like_upstream() {
        let r = |h: &str, size: u64| parse_range(Some(h), size).map(|r| (r.start, r.end));
        assert_eq!(r("bytes=0-", 10), Some((0, 9)));
        assert_eq!(r("bytes=0-0", 10), Some((0, 0)));
        assert_eq!(r("bytes=2-5", 10), Some((2, 5)));
        assert_eq!(r("bytes=2-500", 10), Some((2, 9)));
        assert_eq!(r("bytes=-3", 10), Some((7, 9)));
        assert_eq!(r("bytes=-30", 10), Some((0, 9)));
        assert_eq!(r("  bytes=1-  ", 10), Some((1, 9)));
        // No range at all: the whole file, never 416.
        assert_eq!(r("bytes=10-", 10), None);
        assert_eq!(r("bytes=5-2", 10), None);
        assert_eq!(r("bytes=-0", 10), None);
        assert_eq!(r("bytes=-", 10), None);
        assert_eq!(r("bytes=0-1,3-4", 10), None);
        assert_eq!(r("items=0-1", 10), None);
        assert_eq!(r("bytes=0-", 0), None);
        assert_eq!(r("bytes=-1", 0), None);
        assert_eq!(r("bytes=99999999999999999999999-", 10), None);
        assert_eq!(r("bytes=0-99999999999999999999999", 10), Some((0, 9)));
        assert_eq!(parse_range(None, 10), None);
    }

    #[test]
    fn cache_policy() {
        let open = upload();
        let h = cache_headers_for(&open);
        assert_eq!(h["ETag"], "\"ff\"");
        assert_eq!(h["Cache-Control"], "public, max-age=31536000, immutable");
        let converted = Upload { converted_at: Some(JsDate::now()), ..upload() };
        assert_eq!(cache_headers_for(&converted)["Cache-Control"], "public, max-age=300, must-revalidate");
        for restricted in [
            Upload { published: false, ..upload() },
            Upload { password_hash: Some("x".into()), ..upload() },
            Upload { expires_at: Some(JsDate::now()), ..upload() },
            Upload { max_views: Some(3), ..upload() },
        ] {
            let h = cache_headers_for(&restricted);
            assert_eq!(h["Cache-Control"], "private, no-store");
            assert_eq!(h["Vary"], "Cookie");
            assert!(!h.contains_key("ETag"));
        }
    }

    #[test]
    fn view_grants_are_bound_to_one_file_and_expire() {
        let secret = "grant-test-secret";
        let a = "9d2e1a54-0000-4000-8000-000000000001";
        let b = "9d2e1a54-0000-4000-8000-000000000002";
        let g = mint_view_grant(secret, a);
        assert!(verify_view_grant(secret, Some(&g), a));
        assert!(!verify_view_grant(secret, Some(&g), b));
        assert!(!verify_view_grant("other-secret", Some(&g), a));
        let (exp, sig) = g.split_once('.').unwrap();
        let later: i64 = exp.parse::<i64>().unwrap() + 3600;
        assert!(!verify_view_grant(secret, Some(&format!("{later}.{sig}")), a));
        let past = JsDate::now().millis() / 1000 - 1;
        let stale = format!("{past}.{}", grant_signature(secret, a, past));
        assert!(!verify_view_grant(secret, Some(&stale), a));
        for junk in ["", ".", "x.y", "-1.x", "1e3.x"] {
            assert!(!verify_view_grant(secret, Some(junk), a), "{junk}");
        }
        assert!(!verify_view_grant(secret, None, a));
        // The scope keeps it apart from an unlock token for the same file.
        let unlock = crate::password::mint_unlock_token(secret, crate::password::UnlockScope::Media, a, None);
        assert!(!verify_view_grant(secret, Some(&unlock), a));
    }

    #[test]
    fn the_budget_rule() {
        let open = upload();
        let capped = Upload { max_views: Some(1), ..upload() };
        // Privileged viewers never count.
        assert_eq!(view_budget(&capped, true, true, false), ViewBudget::Free);
        // What upstream counts is still counted, grant or not.
        assert_eq!(view_budget(&open, false, true, false), ViewBudget::Consume);
        assert_eq!(view_budget(&capped, false, true, true), ViewBudget::Consume);
        // Uncapped exempt shapes stay free (statistics only).
        assert_eq!(view_budget(&open, false, false, false), ViewBudget::Free);
        // Capped exempt shapes: free only with the grant, else they pay.
        assert_eq!(view_budget(&capped, false, false, true), ViewBudget::Granted);
        assert_eq!(view_budget(&capped, false, false, false), ViewBudget::Consume);
    }

    #[test]
    fn headers_as_node_reads_them() {
        use axum::http::HeaderValue;
        let mut h = HeaderMap::new();
        h.append("range", HeaderValue::from_static("bytes=0-1"));
        h.append("range", HeaderValue::from_static("bytes=2-3"));
        assert_eq!(joined_header(&h, "range").as_deref(), Some("bytes=0-1, bytes=2-3"));
        assert_eq!(joined_header(&h, "sec-fetch-dest"), None);
        h.append("user-agent", HeaderValue::from_bytes(b"caf\xe9").unwrap());
        assert_eq!(first_header(&h, "user-agent").as_deref(), Some("caf\u{e9}"));
    }
}
