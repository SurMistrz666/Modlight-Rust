//! Port of `app/actions/pastes.ts`: create a text/code paste, stored as a
//! text "upload" so it reuses links, expiry, password and viewing.
//!
//! The steps run in upstream's order, side effects included: the bytes are
//! written to storage before the title, expiry and password are read, so an
//! argument of the wrong type there leaves the stored object behind and
//! throws, exactly as upstream did.

use bytes::Bytes;
use serde_json::{Value, json};

use crate::db::types::JsDate;
use crate::js_value::{is_nullish, prop, to_number, truthy};
use crate::rpc::{ActionCtx, ActionError, ActionResult, RpcRegistry};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("pastes", "createPaste", create_paste);
}

/// `content.length` ceiling, in UTF-16 units (unreachable over RPC: the 1 MiB
/// body limit trips first, as Next's did).
const MAX_PASTE_LENGTH: usize = 5_000_000;

/// View budgets are an int4; a million opens is far beyond what "burn after
/// N views" is for, and the cap keeps a hand-crafted call inside the column.
const MAX_VIEW_BUDGET: f64 = 1_000_000.0;

/// JavaScript's `Date` range: a time value past it is an Invalid Date.
const JS_DATE_MAX_MS: f64 = 8.64e15;

/// `normaliseMaxViews(input)`: null when there is no cap.
fn normalise_max_views(input: Option<&Value>) -> Option<i64> {
    if is_nullish(input) {
        return None;
    }
    let n = to_number(input);
    if !n.is_finite() {
        return None;
    }
    let floored = n.floor();
    if floored < 1.0 {
        return None;
    }
    Some(floored.min(MAX_VIEW_BUDGET) as i64)
}

/// Lifetime in minutes: `expiresInMinutes` when it is a finite positive
/// number (no coercion: `Number.isFinite`), else `floor(expiresInDays) * 1440`
/// when that is truthy (coerced), else none. NaN and 0 mean none; a negative
/// value is kept (an already-expired paste, as upstream).
fn lifetime_minutes(input: Option<&Value>) -> f64 {
    let minutes = prop(input, "expiresInMinutes");
    if let Some(Value::Number(n)) = minutes
        && let Some(m) = n.as_f64()
        && m.is_finite()
        && m > 0.0
    {
        return m.floor();
    }
    let days = prop(input, "expiresInDays");
    if truthy(days) {
        return to_number(days).floor() * 1440.0;
    }
    f64::NAN
}

/// `(title?.trim() || "paste").replace(/[^a-zA-Z0-9._-]+/g, "-").slice(0, 80) || "paste"`
fn file_stem(title: Option<&Value>) -> Result<String, ActionError> {
    let trimmed = match title {
        _ if is_nullish(title) => String::new(),
        Some(Value::String(s)) => crate::config::js_trim(s).to_string(),
        _ => return Err(ActionError::thrown("input.title?.trim is not a function")),
    };
    let base = if trimmed.is_empty() { "paste".to_string() } else { trimmed };
    let mut out = String::new();
    let mut in_run = false;
    for c in base.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push('-');
            in_run = true;
        }
    }
    // Every character left is ASCII, so 80 code units are 80 bytes.
    out.truncate(80);
    Ok(if out.is_empty() { "paste".to_string() } else { out })
}

async fn unique_code(db: &sqlx::PgPool) -> Result<String, ActionError> {
    for _ in 0..50 {
        let c = crate::ids::human_code(10);
        let hit: Option<uuid::Uuid> =
            sqlx::query_scalar(r#"select "id" from "uploads" where "uploads"."code" = $1 limit 1"#)
                .bind(&c)
                .fetch_optional(db)
                .await?;
        if hit.is_none() {
            return Ok(c);
        }
    }
    Err(ActionError::thrown("Could not allocate a unique code"))
}

async fn create_paste(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let input = ctx.args.raw(0);
    if is_nullish(input) {
        return Err(ActionError::thrown("Cannot read properties of undefined (reading 'content')"));
    }

    // `input.content ?? ""`, then `.trim()` (a TypeError on a non-string).
    let content = match prop(input, "content") {
        c if is_nullish(c) => String::new(),
        Some(Value::String(s)) => s.clone(),
        _ => return Err(ActionError::thrown("content.trim is not a function")),
    };
    if crate::config::js_trim(&content).is_empty() {
        return Err(ActionError::thrown("Paste is empty"));
    }
    if content.encode_utf16().count() > MAX_PASTE_LENGTH {
        return Err(ActionError::thrown("Paste is too large (max ~5MB)"));
    }

    // `(input.language || "text").toLowerCase()`. Only the closed LANG_EXT
    // list maps to an extension; any other name is "txt". Deliberate
    // deviation (SKELETON.md section 9): upstream's object lookup also found
    // "constructor" and "__proto__", wrote the bytes under a junk key and threw.
    let language = prop(input, "language");
    let lang = if truthy(language) {
        match language {
            Some(Value::String(s)) => s.to_lowercase(),
            _ => return Err(ActionError::thrown("(input.language || \"text\").toLowerCase is not a function")),
        }
    } else {
        "text".to_string()
    };
    let ext = crate::paste_languages::lang_ext(&lang).unwrap_or("txt");
    let buf = Bytes::from(content.into_bytes());
    let size = buf.len() as i64;

    let max_quota = session.user.max_quota;
    let used = session.user.used_quota.unwrap_or(0);
    if let Some(max) = max_quota
        && max != -1
        && used + size > max
    {
        return Err(ActionError::thrown("Quota exceeded"));
    }

    let code = unique_code(&ctx.state.db).await?;
    let user_code = session.user.user_code.clone().unwrap_or_else(|| "u".to_string());
    let key = crate::media::urls::storage_key_for(&user_code, &code, ext);
    let checksum = crate::media::derived_cache::content_hash(&buf);
    let storage = ctx.state.storage.get()?;
    // Not upstream: an account with file encryption on stores the paste
    // encrypted with its public key (docs/ENCRYPTION.md section 5).
    let sealer =
        crate::vault::write::sealer_for(&ctx.state.db, &ctx.state.config.auth.secret, &session.user.id).await?;
    let encryption = match &sealer {
        Some(sealer) => Some(sealer.write_bytes(storage.as_ref(), &key, buf.clone()).await?),
        None => {
            let body = buf.clone();
            storage.write(&key, Box::pin(futures_util::stream::once(async move { Ok(body) }))).await?;
            None
        }
    };

    let filename = format!("{}.{ext}", file_stem(prop(input, "title"))?);

    let minutes = lifetime_minutes(input);
    let expires_at = if minutes.is_nan() || minutes == 0.0 {
        None
    } else {
        let ms = JsDate::now().millis() as f64 + minutes * 60_000.0;
        if !ms.is_finite() || ms.abs() > JS_DATE_MAX_MS {
            // An Invalid Date: drizzle's `toISOString()` throws.
            return Err(ActionError::thrown("Invalid time value"));
        }
        JsDate::from_millis(ms as i64)
    };

    // A password typed here is a decision about THIS paste: it wins and
    // stores no generation, which is what stops an account-wide "remember for
    // 30 days" consent from opening it. Otherwise the account default applies.
    let password = prop(input, "password");
    let chosen = if truthy(password) {
        match password {
            Some(Value::String(p)) => Some(crate::password::hash_password(p).await?),
            _ => return Err(ActionError::thrown("The \"password\" argument must be of type string")),
        }
    } else {
        None
    };
    let (password_hash, password_version) = match chosen {
        Some(hash) => (Some(hash), None),
        None => {
            let stamp = crate::data::user_prefs::default_password_stamp(&ctx.state.db, &session.user.id).await?;
            (stamp.password_hash, stamp.password_version)
        }
    };
    let max_views = normalise_max_views(prop(input, "maxViews"));

    let mime_type =
        crate::media::paste_type::paste_mime_type(Some(&lang)).unwrap_or_else(|| "text/plain; charset=utf-8".into());
    let published = !session.user.hide_uploads.unwrap_or(false);
    let mut tx = ctx.state.db.begin().await?;
    if let Err(err) =
        crate::vault::write::confirm_vault_for_write(&mut tx, &session.user.id, encryption.is_some()).await
    {
        let _ = storage.delete(&key).await;
        return Err(err.into());
    }
    sqlx::query(
        r#"insert into "uploads" ("owner_id", "code", "filename", "ext", "kind", "language", "mime_type", "size",
             "checksum", "storage_key", "published", "delete_token", "expires_at", "password_hash",
             "password_version", "max_views", "encryption")
           values ($1, $2, $3, $4, 'paste', $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)"#,
    )
    .bind(&session.user.id)
    .bind(&code)
    .bind(&filename)
    .bind(ext)
    .bind(&lang)
    .bind(&mime_type)
    .bind(size)
    .bind(&checksum)
    .bind(&key)
    .bind(published)
    .bind(crate::ids::secure_token(16))
    .bind(expires_at)
    .bind(&password_hash)
    .bind(password_version)
    .bind(max_views.map(|v| v as i32))
    .bind(&encryption)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    // drizzle's `$onUpdate` stamps `updated_at` on every user update.
    sqlx::query(
        r#"update "user" set "used_quota" = "user"."used_quota" + $1, "updated_at" = $2 where "user"."id" = $3"#,
    )
    .bind(size)
    .bind(JsDate::now())
    .bind(&session.user.id)
    .execute(&ctx.state.db)
    .await?;

    ctx.revalidate_path("/dashboard");
    // Same hostname the user's uploads are handed out on.
    let prefs = crate::data::user_prefs::get_user_prefs(&ctx.state.db, &session.user.id).await?;
    let base = crate::media::urls::user_base_url(
        &ctx.state.config,
        prefs.as_ref().and_then(|p| p.preferred_domain.as_deref()),
    );
    Ok(json!({
        "code": code,
        "url": crate::media::urls::public_url(&user_code, &code, ext, &base),
        "rawUrl": crate::media::urls::raw_url(&user_code, &code, ext, &base),
        "expiresAt": expires_at.map(|d| d.to_iso_string()),
        "maxViews": max_views,
        "hasPassword": password_hash.is_some(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_budgets() {
        assert_eq!(normalise_max_views(None), None);
        assert_eq!(normalise_max_views(Some(&json!(null))), None);
        assert_eq!(normalise_max_views(Some(&json!(3.9))), Some(3));
        assert_eq!(normalise_max_views(Some(&json!("7"))), Some(7));
        assert_eq!(normalise_max_views(Some(&json!(0.5))), None);
        assert_eq!(normalise_max_views(Some(&json!(-4))), None);
        assert_eq!(normalise_max_views(Some(&json!(5e9))), Some(1_000_000));
        assert_eq!(normalise_max_views(Some(&json!({"$num": "Infinity"}))), None);
        assert_eq!(normalise_max_views(Some(&json!(true))), Some(1));
    }

    #[test]
    fn lifetimes() {
        let l = |v: Value| lifetime_minutes(Some(&v));
        assert_eq!(l(json!({ "expiresInMinutes": 90.7 })), 90.0);
        assert_eq!(l(json!({ "expiresInMinutes": 0, "expiresInDays": 2 })), 2880.0);
        assert_eq!(l(json!({ "expiresInMinutes": "5", "expiresInDays": "1" })), 1440.0);
        assert_eq!(l(json!({ "expiresInDays": -1 })), -1440.0);
        assert_eq!(l(json!({ "expiresInDays": 0.5 })), 0.0);
        assert!(l(json!({ "expiresInDays": "abc" })).is_nan());
        assert!(l(json!({})).is_nan());
    }

    #[test]
    fn file_names() {
        assert_eq!(file_stem(None).unwrap(), "paste");
        assert_eq!(file_stem(Some(&json!("  "))).unwrap(), "paste");
        assert_eq!(file_stem(Some(&json!("My notes: v2!"))).unwrap(), "My-notes-v2-");
        assert_eq!(file_stem(Some(&json!("Zażółć gęślą"))).unwrap(), "Za-g-l-");
        assert_eq!(file_stem(Some(&json!("x".repeat(100)))).unwrap().len(), 80);
        assert!(file_stem(Some(&json!(5))).is_err());
    }
}
