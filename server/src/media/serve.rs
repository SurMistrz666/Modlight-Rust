//! Port of the lookup half of `lib/media/serve.ts`: `getMedia` and
//! `cacheHeadersFor`, the two pure reads `/api/link-info` answers from.
//!
//! PARTIAL by design: the serving half (`streamMedia`, `viewerRelation`,
//! `consumeView`, `isPrivilegedViewer`, range parsing) belongs to the media
//! serving port, which extends this module.

use indexmap::IndexMap;
use sqlx::PgPool;

use crate::background::{BackgroundStyle, background_style};
use crate::db::models::{Columns, Upload};
use crate::db::types::JsDate;
use crate::media::paste_type::row_mime_type;
use crate::media::paths::code_from_media_code;

/// `MediaLookup`
#[derive(Debug, Clone, PartialEq)]
pub struct MediaLookup {
    /// The row, with a paste's placeholder type resolved (`rowMimeType`).
    pub media: Upload,
    pub owner_user_code: String,
    pub owner_id: String,
    pub owner_theme: String,
    pub owner_background: Option<BackgroundStyle>,
    pub owner_preferred_domain: Option<String>,
}

/// `getMedia(userCode, mediaCode)`: a live file by its public
/// `{userCode}/{code}`, honouring soft-delete, orphaning, the owner's current
/// or retired tag, expiry and the view budget. `None` is the 404 for all of
/// them. A malformed percent-escape in `media_code` is an error, as
/// `decodeURIComponent` throws upstream.
pub async fn get_media(db: &PgPool, user_code: &str, media_code: &str) -> anyhow::Result<Option<MediaLookup>> {
    let code = code_from_media_code(media_code).ok_or_else(|| anyhow::anyhow!("URIError: URI malformed"))?;
    let sql = format!(
        r#"select {} from {} where ("uploads"."code" = $1 and "uploads"."deleted_at" is null) limit 1"#,
        Upload::COLUMNS,
        Upload::TABLE
    );
    let Some(media) = sqlx::query_as::<_, Upload>(&sql).bind(&code).fetch_optional(db).await? else {
        return Ok(None);
    };
    // An orphaned file has no tag to match: it is off the web.
    let Some(owner_id) = media.owner_id.clone() else {
        return Ok(None);
    };
    type OwnerRow = (String, Option<String>, Option<String>, Option<String>, Option<String>, i32);
    let owner: Option<OwnerRow> = sqlx::query_as(
        r#"select "id", "user_code", "theme", "preferred_domain", "background_key", "background_dim"
           from "user" where "user"."id" = $1 limit 1"#,
    )
    .bind(&owner_id)
    .fetch_optional(db)
    .await?;
    let Some((id, owner_code, theme, preferred_domain, background_key, background_dim)) = owner else {
        return Ok(None);
    };
    if owner_code.as_deref() != Some(user_code) {
        let alias: Option<String> = sqlx::query_scalar(
            r#"select "user_id" from "user_code_aliases" where "user_code_aliases"."code" = $1 limit 1"#,
        )
        .bind(user_code)
        .fetch_optional(db)
        .await?;
        if alias.as_deref() != Some(id.as_str()) {
            return Ok(None);
        }
    }
    if media.expires_at.is_some_and(|e| e.millis() <= JsDate::now().millis()) {
        return Ok(None);
    }
    if media.max_views.is_some_and(|max| media.views >= max) {
        return Ok(None);
    }
    let mime = row_mime_type(Some(&media.kind), media.language.as_deref(), &media.mime_type);
    Ok(Some(MediaLookup {
        owner_background: background_style(owner_code.as_deref(), background_key.as_deref(), Some(background_dim)),
        owner_user_code: owner_code.unwrap_or_else(|| user_code.to_string()),
        owner_id: id,
        owner_theme: theme.unwrap_or_else(|| "modern".to_string()),
        owner_preferred_domain: preferred_domain,
        media: Upload { mime_type: mime, ..media },
    }))
}

/// `cacheHeadersFor(media)`: nothing restricted lands in a shared cache; a
/// public file gets its checksum as the ETag and `immutable` unless it was
/// ever converted in place.
pub fn cache_headers_for(media: &Upload) -> IndexMap<&'static str, String> {
    let mut headers = IndexMap::new();
    let restricted = !media.published
        || media.password_hash.as_deref().is_some_and(|h| !h.is_empty())
        || media.expires_at.is_some()
        || media.max_views.is_some();
    if restricted {
        headers.insert("Cache-Control", "private, no-store".to_string());
        headers.insert("Vary", "Cookie".to_string());
        return headers;
    }
    if let Some(sum) = media.checksum.as_deref().filter(|c| !c.is_empty()) {
        headers.insert("ETag", format!("\"{sum}\""));
    }
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
