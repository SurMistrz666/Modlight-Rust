//! Port of `lib/shortlinks.ts`: minting short links, and resolving them.
//!
//! Minting has two callers (the panel's action and `POST /api/shortlinks`).
//! [`normalize_target`] is a SECURITY CONTROL, not a convenience: a shortener
//! that will redirect to a `javascript:` URL is an XSS vector on whichever
//! domain serves it, so there is exactly one implementation and both entry
//! points go through it.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;

use crate::config::{Config, js_trim};
use crate::db::models::{Columns, JsDate, ShortLink};
use crate::naming::{SHORT_CODE_MAX, scoped_short_code};

/// Why a short link could not be created (`ShortLinkIssue`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortLinkIssue {
    InvalidUrl,
    InvalidCode,
    CodeTaken,
    CodeTooLong,
    RootNotAllowed,
    NoTag,
    Error,
}

impl ShortLinkIssue {
    pub fn as_str(self) -> &'static str {
        match self {
            ShortLinkIssue::InvalidUrl => "invalid-url",
            ShortLinkIssue::InvalidCode => "invalid-code",
            ShortLinkIssue::CodeTaken => "code-taken",
            ShortLinkIssue::CodeTooLong => "code-too-long",
            ShortLinkIssue::RootNotAllowed => "root-not-allowed",
            ShortLinkIssue::NoTag => "no-tag",
            ShortLinkIssue::Error => "error",
        }
    }
}

/// `CreateShortLinkResult`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateShortLinkResult {
    Ok { code: String, url: String },
    Refused(ShortLinkIssue),
}

impl CreateShortLinkResult {
    /// `{ ok: true, code, url }` / `{ ok: false, reason }`
    pub fn to_json(&self) -> Value {
        match self {
            CreateShortLinkResult::Ok { code, url } => serde_json::json!({ "ok": true, "code": code, "url": url }),
            CreateShortLinkResult::Refused(r) => serde_json::json!({ "ok": false, "reason": r.as_str() }),
        }
    }
}

/// `CreateShortLinkInput`, as the raw values the caller sent (the action
/// passes the client's object through; the HTTP route validated types first).
#[derive(Debug, Clone, Default)]
pub struct CreateShortLinkInput {
    pub target: Value,
    pub custom_code: Option<Value>,
    pub expires_in_days: Option<Value>,
    pub password: Option<Value>,
    /// Prefix the link with the owner's tag; defaults to true.
    pub use_user_code: Option<Value>,
}

/// Longest raw target this will even try to parse.
const TARGET_MAX: usize = 4096;
/// Longest raw custom code accepted before slugifying.
const CUSTOM_CODE_MAX: usize = 256;
/// Longest expiry a caller may ask for, in days (10 years).
const EXPIRY_MAX_DAYS: f64 = 3650.0;
const MS_PER_DAY: f64 = 86_400_000.0;

/// `/^([a-zA-Z][a-zA-Z0-9+.-]*):/`: the scheme a value starts with, if any.
fn scheme_of(input: &str) -> Option<&str> {
    let b = input.as_bytes();
    if b.is_empty() || !b[0].is_ascii_alphabetic() {
        return None;
    }
    let mut i = 1;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'+' | b'.' | b'-')) {
        i += 1;
    }
    if i < b.len() && b[i] == b':' { Some(&input[..i]) } else { None }
}

/// Accept what people actually type: a bare host is promoted to `https://`.
/// Returns the normalised absolute URL, or `None` when it cannot be made into
/// one (any non-web scheme is refused outright).
pub fn normalize_target(raw: &str) -> Option<String> {
    // Length before trim (UTF-16 units, as `.length`).
    if raw.encode_utf16().count() > TARGET_MAX {
        return None;
    }
    let input = js_trim(raw);
    if input.is_empty() {
        return None;
    }
    let scheme = scheme_of(input);
    if let Some(s) = scheme {
        let s = s.to_ascii_lowercase();
        if s != "http" && s != "https" {
            return None;
        }
    }
    let candidate = if scheme.is_some() { input.to_string() } else { format!("https://{input}") };
    let url = url::Url::parse(&candidate).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    // A hostname with no dot is almost certainly a typo rather than an
    // intranet name.
    let host = url.host_str().unwrap_or("");
    if !host.contains('.') && host != "localhost" {
        return None;
    }
    Some(url.to_string())
}

/// Days from now to an absolute expiry, or `None` for "never". Non-numbers,
/// non-finite and non-positive values mean "never"; clamped to 10 years.
pub fn expiry_from_days(days: Option<&Value>) -> Option<JsDate> {
    let d = days.and_then(|v| match v {
        Value::Number(n) => n.as_f64(),
        _ => crate::rpc::tagged::raw_num(v).filter(|_| v.is_object()),
    })?;
    if !d.is_finite() || d <= 0.0 {
        return None;
    }
    let ms = JsDate::now().millis() as f64 + d.min(EXPIRY_MAX_DAYS) * MS_PER_DAY;
    JsDate::from_millis(ms.trunc() as i64)
}

/// Absolute short link: SHORTLINK_URL + "/" + code when a dedicated shortener
/// domain is configured, else APP_URL + "/s/" + code.
pub fn short_link_url(config: &Config, stored: &str) -> String {
    if config.short_url != config.app_url {
        return format!("{}/{stored}", config.short_url);
    }
    format!("{}/s/{stored}", config.app_url)
}

async fn is_code_taken(db: &PgPool, stored: &str) -> sqlx::Result<bool> {
    let hit: Option<uuid::Uuid> =
        sqlx::query_scalar(r#"select "id" from "short_links" where "short_links"."code" = $1 limit 1"#)
            .bind(stored)
            .fetch_optional(db)
            .await?;
    Ok(hit.is_some())
}

/// Random leaf, unique within the scope it will be stored under.
async fn unique_code(db: &PgPool, tag: Option<&str>) -> anyhow::Result<String> {
    for _ in 0..50 {
        let c = crate::ids::human_code(7);
        if !is_code_taken(db, &scoped_short_code(tag, &c)).await? {
            return Ok(c);
        }
    }
    anyhow::bail!("Could not allocate a unique code")
}

/// A thrown error (`TypeError` on a value of the wrong type, the code
/// allocator giving up, the database failing).
pub type Thrown = anyhow::Error;

/// `x?.trim()` on a raw value: `None` for null/undefined, the trimmed string,
/// or the TypeError a non-string raises.
fn optional_trim(value: Option<&Value>, what: &str) -> Result<Option<String>, Thrown> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(v) if crate::rpc::tagged::is_undefined(v) => Ok(None),
        Some(Value::String(s)) => Ok(Some(js_trim(s).to_string())),
        Some(_) => Err(anyhow::anyhow!("input.{what}?.trim is not a function")),
    }
}

/// Mint a short link for `owner_id`: the whole decision, minus how the caller
/// authenticated and whether a cache needs revalidating.
pub async fn create_short_link_for(
    db: &PgPool,
    config: &Config,
    owner_id: &str,
    input: &CreateShortLinkInput,
) -> Result<CreateShortLinkResult, Thrown> {
    use CreateShortLinkResult::Refused;
    let target = match &input.target {
        Value::String(s) => normalize_target(s),
        // `raw.length > TARGET_MAX` is false for a non-string, then `raw.trim()` throws.
        Value::Null => return Err(anyhow::anyhow!("Cannot read properties of null (reading 'length')")),
        v if crate::rpc::tagged::is_undefined(v) => {
            return Err(anyhow::anyhow!("Cannot read properties of undefined (reading 'length')"));
        }
        _ => return Err(anyhow::anyhow!("raw.trim is not a function")),
    };
    let Some(target) = target else {
        return Ok(Refused(ShortLinkIssue::InvalidUrl));
    };
    let Some(prefs) = crate::data::user_prefs::get_user_prefs(db, owner_id).await? else {
        return Ok(Refused(ShortLinkIssue::Error));
    };
    // Scope, enforced here rather than in the form.
    let wants_prefix = !matches!(input.use_user_code, Some(Value::Bool(false)));
    let tag: Option<String> = if wants_prefix {
        match prefs.user_code.clone().filter(|t| !t.is_empty()) {
            Some(t) => Some(t),
            None => return Ok(Refused(ShortLinkIssue::NoTag)),
        }
    } else {
        if !crate::data::user_prefs::may_create_root_short_link(prefs.role.as_deref(), prefs.allow_root_short_links) {
            return Ok(Refused(ShortLinkIssue::RootNotAllowed));
        }
        None
    };
    let custom = optional_trim(input.custom_code.as_ref(), "customCode")?.filter(|c| !c.is_empty());
    let code = match &custom {
        Some(c) => {
            if c.encode_utf16().count() > CUSTOM_CODE_MAX {
                return Ok(Refused(ShortLinkIssue::CodeTooLong));
            }
            let slug = crate::ids::slugify(c);
            if slug.is_empty() {
                return Ok(Refused(ShortLinkIssue::InvalidCode));
            }
            slug
        }
        None => unique_code(db, tag.as_deref()).await?,
    };
    let stored = scoped_short_code(tag.as_deref(), &code);
    // short_links.code is varchar(64); the scoped form has to fit.
    if stored.encode_utf16().count() > SHORT_CODE_MAX {
        return Ok(Refused(ShortLinkIssue::CodeTooLong));
    }
    if custom.is_some() && is_code_taken(db, &stored).await? {
        return Ok(Refused(ShortLinkIssue::CodeTaken));
    }
    let password = optional_trim(input.password.as_ref(), "password")?.filter(|p| !p.is_empty());
    let password_hash = match password {
        Some(p) => Some(crate::password::hash_password(&p).await?),
        None => None,
    };
    let expires_at = expiry_from_days(input.expires_in_days.as_ref());
    let inserted = sqlx::query(
        r#"insert into "short_links" ("owner_id", "code", "target", "expires_at", "password_hash") values ($1, $2, $3, $4, $5)"#,
    )
    .bind(owner_id)
    .bind(&stored)
    .bind(&target)
    .bind(expires_at)
    .bind(&password_hash)
    .execute(db)
    .await;
    if let Err(err) = inserted {
        // Unique index race: somebody claimed the code between check and insert.
        if crate::uploads::is_unique_violation(&err) {
            return Ok(Refused(ShortLinkIssue::CodeTaken));
        }
        return Ok(Refused(ShortLinkIssue::Error));
    }
    let url = short_link_url(config, &stored);
    Ok(CreateShortLinkResult::Ok { code: stored, url })
}

/// A plain-text response the way `new Response("text", {status})` builds one.
pub fn text_response(status: StatusCode, body: &str) -> Response {
    let mut res = (status, body.to_string()).into_response();
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain;charset=UTF-8"));
    res
}

/// `Response.redirect(url, 302)`: no body, `Location` serialised by the URL parser.
pub fn redirect_302(location: &str) -> Response {
    let mut res = StatusCode::FOUND.into_response();
    let href = url::Url::parse(location).map(|u| u.to_string()).unwrap_or_else(|_| location.to_string());
    if let Ok(v) = HeaderValue::from_str(&href) {
        res.headers_mut().insert(header::LOCATION, v);
    }
    res
}

/// Resolve a stored short-link code to its redirect: 404 unknown, 410 expired,
/// the unlock page for a password-protected link without a valid token, else
/// a 302 to the target (the click is counted in the background).
pub async fn redirect_short_link(
    db: &PgPool,
    config: &Config,
    stored: &str,
    read_cookie: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Response> {
    let sql =
        format!(r#"select {} from {} where "short_links"."code" = $1 limit 1"#, ShortLink::COLUMNS, ShortLink::TABLE);
    let link: Option<ShortLink> = sqlx::query_as(&sql).bind(stored).fetch_optional(db).await?;
    let Some(link) = link else {
        return Ok(text_response(StatusCode::NOT_FOUND, "Not found"));
    };
    if link.expires_at.is_some_and(|e| e.millis() <= JsDate::now().millis()) {
        return Ok(text_response(StatusCode::GONE, "This link has expired"));
    }
    if link.password_hash.as_deref().is_some_and(|h| !h.is_empty()) {
        let id = link.id.to_string();
        let token = read_cookie(&crate::password::unlock_cookie_name(crate::password::UnlockScope::Shortlink, &id));
        if !crate::password::verify_unlock_token(
            &config.auth.secret,
            token.as_deref(),
            crate::password::UnlockScope::Shortlink,
            &id,
        ) {
            // `stored` may contain a slash; it is already a path.
            return Ok(redirect_302(&format!("{}/s/{stored}/unlock", config.app_url)));
        }
    }
    let pool = db.clone();
    let id = link.id;
    tokio::spawn(async move {
        let _ = sqlx::query(
            r#"update "short_links" set "clicks" = "short_links"."clicks" + 1, "last_used_at" = $1 where "short_links"."id" = $2"#,
        )
        .bind(JsDate::now())
        .bind(id)
        .execute(&pool)
        .await;
    });
    Ok(redirect_302(&link.target))
}

/// A short link as listed by the panel (`links` page, `listShortLinks` mapping).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkView {
    pub id: uuid::Uuid,
    pub code: String,
    pub target: String,
    pub clicks: i32,
    pub has_password: bool,
    pub last_used_at: Option<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
}

/// `SELECT * FROM short_links WHERE owner_id = $1 ORDER BY created_at DESC`.
pub async fn list_short_links(db: &PgPool, owner_id: &str) -> sqlx::Result<Vec<ShortLink>> {
    let sql = format!(
        r#"select {} from {} where "short_links"."owner_id" = $1 order by "short_links"."created_at" desc"#,
        ShortLink::COLUMNS,
        ShortLink::TABLE
    );
    sqlx::query_as(&sql).bind(owner_id).fetch_all(db).await
}

#[cfg(test)]
mod tests {
    //! tests/unit/shortlink-target.test.ts
    use super::*;

    #[test]
    fn targets() {
        assert_eq!(normalize_target("google.pl").as_deref(), Some("https://google.pl/"));
        assert_eq!(
            normalize_target("  https://Example.COM/a b?x=1  ").as_deref(),
            Some("https://example.com/a%20b?x=1")
        );
        assert_eq!(normalize_target("http://localhost:3000/x").as_deref(), Some("http://localhost:3000/x"));
        assert_eq!(normalize_target("javascript:alert(1)"), None);
        assert_eq!(normalize_target("JavaScript:alert(1)"), None);
        assert_eq!(normalize_target("data:text/html,x"), None);
        assert_eq!(normalize_target("ftp://example.com"), None);
        assert_eq!(normalize_target("intranet"), None);
        assert_eq!(normalize_target(""), None);
        assert_eq!(normalize_target("   "), None);
        assert_eq!(normalize_target(&format!("https://example.com/{}", "a".repeat(5000))), None);
        assert_eq!(normalize_target("www.google.pl/search?q=a").as_deref(), Some("https://www.google.pl/search?q=a"));
        for bad in [
            "  javascript:alert(1)",
            "\njavascript:alert(1)",
            "java\tscript:alert(1)",
            "java\nscript:alert(1)",
            "vbscript:x",
            "file:///etc/passwd",
            "notaurl",
        ] {
            assert_eq!(normalize_target(bad), None, "{bad:?}");
        }
        assert!(normalize_target(&format!("https://example.com/{}", "a".repeat(3000))).is_some());
        assert_eq!(normalize_target("http://example.com/x").as_deref(), Some("http://example.com/x"));
    }

    #[test]
    fn expiry_and_urls() {
        assert!(expiry_from_days(Some(&serde_json::json!(0))).is_none());
        assert!(expiry_from_days(Some(&serde_json::json!(-3))).is_none());
        assert!(expiry_from_days(Some(&serde_json::json!("5"))).is_none());
        assert!(expiry_from_days(None).is_none());
        let d = expiry_from_days(Some(&serde_json::json!(1e308))).unwrap();
        assert!(d.millis() - JsDate::now().millis() <= (3650.0 * MS_PER_DAY) as i64);
        let c = Config::from_env_map(&Default::default()).unwrap();
        assert_eq!(short_link_url(&c, "tag/x"), "http://localhost:8420/s/tag/x");
    }
}
