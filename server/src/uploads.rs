//! Port of `lib/uploads.ts`: what can be done to an upload after it exists,
//! with no idea who is logged in.
//!
//! Each function takes the ACTOR instead of calling `getSession()`, so the
//! panel's server actions and `app/api/uploads/route.ts` run the same code.
//! The rules that must not diverge: slug normalisation and the clash check
//! for a vanity code; `passwordVersion` cleared whenever a per-file password
//! is set; the view-cap floor; and the audit entry an administrator's touch on
//! somebody else's file leaves.

use serde_json::{Map, Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::models::{Columns, JsDate, Upload};
use crate::media::actions::{DeletableMedia, delete_media_bulk};
use crate::storage::StorageDriver;
use crate::upload::folder_target::is_uuid;

/// Why an operation was refused (`UploadIssue`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadIssue {
    NotFound,
    InvalidCode,
    CodeTooLong,
    CodeReserved,
    CodeTaken,
    InvalidDate,
    InvalidViews,
    PasswordTooLong,
}

impl UploadIssue {
    pub fn as_str(self) -> &'static str {
        match self {
            UploadIssue::NotFound => "not-found",
            UploadIssue::InvalidCode => "invalid-code",
            UploadIssue::CodeTooLong => "code-too-long",
            UploadIssue::CodeReserved => "code-reserved",
            UploadIssue::CodeTaken => "code-taken",
            UploadIssue::InvalidDate => "invalid-date",
            UploadIssue::InvalidViews => "invalid-views",
            UploadIssue::PasswordTooLong => "password-too-long",
        }
    }
}

/// `UploadOpError`
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct UploadOpError {
    pub reason: UploadIssue,
    pub message: String,
}

impl UploadOpError {
    fn new(reason: UploadIssue, message: impl Into<String>) -> UploadOpError {
        UploadOpError { reason, message: message.into() }
    }
}

fn not_found() -> UploadOpError {
    UploadOpError::new(UploadIssue::NotFound, "Not found")
}

/// A refusal or an unexpected failure.
#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    #[error(transparent)]
    Op(#[from] UploadOpError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<sqlx::Error> for UploadError {
    fn from(e: sqlx::Error) -> Self {
        UploadError::Other(e.into())
    }
}

pub type UploadResult<T> = Result<T, UploadError>;

/// `UploadRecord = typeof uploads.$inferSelect`
pub type UploadRecord = Upload;

/// Who is asking, and how far their reach goes. `ip` is what the audit entry
/// records (`clientIp(await headers())` upstream).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UploadActor {
    pub user_id: String,
    /// Recorded on an administrative touch, never used to decide anything.
    pub email: Option<String>,
    pub is_admin: bool,
    pub ip: String,
}

/// Ceiling on how many rows one call may touch.
pub const UPLOAD_BATCH_MAX: usize = 500;
/// `uploads.code` is a varchar(64).
pub const VANITY_CODE_MAX: usize = 64;
/// Longest password accepted (scrypt is linear in the input).
pub const UPLOAD_PASSWORD_MAX: usize = 1024;

/// The WHERE clause every read and write is built from: ownership IN the query
/// (an administrator reaches every row; everyone else reaches their own).
fn scope(actor: &UploadActor, ids_param: usize, owner_param: usize) -> String {
    let mut s = format!(r#""uploads"."id" = any(${ids_param})"#);
    if !actor.is_admin {
        s.push_str(&format!(r#" and "uploads"."owner_id" = ${owner_param}"#));
    }
    format!("({s})")
}

/// Ids worth sending to Postgres: strings, uuid-shaped, unique, bounded.
pub fn clean_ids(ids: &[Value]) -> Vec<Uuid> {
    let mut seen: indexmap::IndexSet<String> = indexmap::IndexSet::new();
    for v in ids {
        if let Some(s) = v.as_str()
            && is_uuid(s)
        {
            seen.insert(s.to_string());
        }
        if seen.len() >= UPLOAD_BATCH_MAX {
            break;
        }
    }
    // Two spellings of one uuid (case) stay two entries, as in the JS Set.
    seen.iter().filter_map(|s| Uuid::parse_str(s).ok()).collect()
}

/// Record an administrative touch on ANOTHER account's file. No-op otherwise.
pub async fn audit_foreign_upload(
    db: &PgPool,
    actor: &UploadActor,
    media: &Upload,
    action: &str,
    detail: Map<String, Value>,
) {
    if media.owner_id.as_deref() == Some(actor.user_id.as_str()) {
        return;
    }
    let mut d = Map::new();
    d.insert("by".into(), json!(actor.user_id));
    d.insert("byEmail".into(), json!(actor.email));
    d.insert("uploadId".into(), json!(media.id));
    d.insert("code".into(), json!(media.code));
    d.insert("filename".into(), json!(media.filename));
    d.insert("ownerId".into(), json!(media.owner_id));
    d.extend(detail);
    crate::audit::record_audit(db, &actor.user_id, action, Value::Object(d), &actor.ip).await;
}

/// The rows out of `ids` this actor may act on. Silently drops the rest.
pub async fn actionable_uploads(db: &PgPool, actor: &UploadActor, ids: &[Value]) -> sqlx::Result<Vec<Upload>> {
    let clean = clean_ids(ids);
    if clean.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!("select {} from {} where {}", Upload::COLUMNS, Upload::TABLE, scope(actor, 1, 2));
    let mut q = sqlx::query_as::<_, Upload>(&sql).bind(&clean);
    if !actor.is_admin {
        q = q.bind(&actor.user_id);
    }
    q.fetch_all(db).await
}

/// One row, or a refusal that cannot be told apart from "no such id". Trashed
/// rows are included.
pub async fn get_actionable_upload(db: &PgPool, actor: &UploadActor, id: Option<&Value>) -> UploadResult<Upload> {
    let Some(id) = id.and_then(Value::as_str).filter(|s| is_uuid(s)).and_then(|s| Uuid::parse_str(s).ok()) else {
        return Err(not_found().into());
    };
    let sql = format!("select {} from {} where {} limit 1", Upload::COLUMNS, Upload::TABLE, scope(actor, 1, 2));
    let mut q = sqlx::query_as::<_, Upload>(&sql).bind(vec![id]);
    if !actor.is_admin {
        q = q.bind(&actor.user_id);
    }
    q.fetch_optional(db).await?.ok_or_else(|| not_found().into())
}

/// A column assignment of a patch.
#[derive(Debug, Clone)]
pub enum PatchValue {
    Bool(bool),
    Date(Option<JsDate>),
    Num(Option<f64>),
    Text(Option<String>),
}

/// Apply one patch to every row the actor may reach, and say what they now
/// look like (pre-images with the patch folded in).
async fn apply_to_uploads(
    db: &PgPool,
    actor: &UploadActor,
    ids: &[Value],
    patch: &[(&'static str, PatchValue)],
    action: &str,
    detail: Map<String, Value>,
) -> UploadResult<Vec<Upload>> {
    let rows = actionable_uploads(db, actor, ids).await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let updated_at = JsDate::now();
    let mut sets: Vec<String> = Vec::new();
    for (i, (column, _)) in patch.iter().enumerate() {
        sets.push(format!(r#""{column}" = ${}"#, i + 1));
    }
    let n = patch.len();
    sets.push(format!(r#""updated_at" = ${}"#, n + 1));
    let sql = format!(r#"update "uploads" set {} where {}"#, sets.join(", "), scope(actor, n + 2, n + 3));
    let mut q = sqlx::query(&sql);
    for (_, value) in patch {
        q = match value {
            PatchValue::Bool(b) => q.bind(*b),
            PatchValue::Date(d) => q.bind(*d),
            PatchValue::Num(n) => q.bind(*n),
            PatchValue::Text(t) => q.bind(t.clone()),
        };
    }
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    q = q.bind(updated_at).bind(&ids);
    if !actor.is_admin {
        q = q.bind(&actor.user_id);
    }
    q.execute(db).await?;
    for row in &rows {
        audit_foreign_upload(db, actor, row, action, detail.clone()).await;
    }
    Ok(rows
        .into_iter()
        .map(|mut r| {
            for (column, value) in patch {
                match (*column, value) {
                    ("published", PatchValue::Bool(b)) => r.published = *b,
                    ("expires_at", PatchValue::Date(d)) => r.expires_at = *d,
                    ("max_views", PatchValue::Num(v)) => r.max_views = v.map(|v| v as i32),
                    ("password_hash", PatchValue::Text(t)) => r.password_hash = t.clone(),
                    ("password_version", PatchValue::Num(v)) => r.password_version = v.map(|v| v as i32),
                    _ => {}
                }
            }
            r.updated_at = updated_at;
            r
        })
        .collect())
}

fn detail(pairs: Vec<(&str, Value)>) -> Map<String, Value> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// Publish or unpublish. May name many ids; unmatched ones are simply absent.
pub async fn set_published_for(
    db: &PgPool,
    actor: &UploadActor,
    ids: &[Value],
    published: bool,
) -> UploadResult<Vec<Upload>> {
    apply_to_uploads(
        db,
        actor,
        ids,
        &[("published", PatchValue::Bool(published))],
        "admin.upload.published",
        detail(vec![("published", json!(published))]),
    )
    .await
}

/// Postgres 23505: somebody claimed the code between our SELECT and UPDATE.
pub fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(e) if e.code().as_deref() == Some("23505"))
}

/// Point a file at a custom URL. The clash check looks at EVERY account's
/// codes, trash included; the unique index is checked again by Postgres.
pub async fn set_vanity_for(
    db: &PgPool,
    actor: &UploadActor,
    id: Option<&Value>,
    vanity: Option<&Value>,
) -> UploadResult<Upload> {
    let media = get_actionable_upload(db, actor, id).await?;
    let code = crate::ids::slugify(vanity.and_then(Value::as_str).unwrap_or(""));
    if code.is_empty() {
        return Err(UploadOpError::new(UploadIssue::InvalidCode, "Invalid vanity").into());
    }
    // A name the application already owns can never be served as a file.
    if crate::naming::is_reserved_tag(&code) {
        return Err(UploadOpError::new(UploadIssue::CodeReserved, format!("\"{code}\" is reserved by the app")).into());
    }
    if code.encode_utf16().count() > VANITY_CODE_MAX {
        return Err(UploadOpError::new(
            UploadIssue::CodeTooLong,
            format!("Custom URL must be at most {VANITY_CODE_MAX} characters"),
        )
        .into());
    }
    // Asking for the code it already has is not a clash with itself.
    if code == media.code {
        return Ok(media);
    }
    let clash: Option<Uuid> = sqlx::query_scalar(r#"select "id" from "uploads" where "uploads"."code" = $1 limit 1"#)
        .bind(&code)
        .fetch_optional(db)
        .await?;
    if clash.is_some_and(|c| c != media.id) {
        return Err(UploadOpError::new(UploadIssue::CodeTaken, "That URL is already taken").into());
    }
    let sql = format!(r#"update "uploads" set "code" = $1, "updated_at" = $2 where {}"#, scope(actor, 3, 4));
    let mut q = sqlx::query(&sql).bind(&code).bind(JsDate::now()).bind(vec![media.id]);
    if !actor.is_admin {
        q = q.bind(&actor.user_id);
    }
    if let Err(err) = q.execute(db).await {
        if is_unique_violation(&err) {
            return Err(UploadOpError::new(UploadIssue::CodeTaken, "That URL is already taken").into());
        }
        return Err(err.into());
    }
    audit_foreign_upload(
        db,
        actor,
        &media,
        "admin.upload.link_changed",
        detail(vec![("from", json!(media.code)), ("to", json!(code))]),
    )
    .await;
    Ok(Upload { code, ..media })
}

/// `parseExpiry(when)`: null, undefined and "" clear; a Date is used as is;
/// anything else goes through `new Date(value)`.
pub fn parse_expiry(when: Option<&Value>) -> Result<Option<JsDate>, UploadOpError> {
    let invalid = || UploadOpError::new(UploadIssue::InvalidDate, "Invalid date");
    match when {
        None | Some(Value::Null) => Ok(None),
        Some(v) if crate::rpc::tagged::is_undefined(v) => Ok(None),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(Value::String(s)) => crate::js_date::parse_js_date(s).map(Some).ok_or_else(invalid),
        Some(Value::Number(n)) => n
            .as_f64()
            .filter(|f| f.is_finite())
            .and_then(|f| JsDate::from_millis(f as i64))
            .map(Some)
            .ok_or_else(invalid),
        Some(Value::Object(m)) if m.len() == 1 && m.contains_key("$date") => {
            m["$date"].as_str().and_then(crate::js_date::parse_js_date).map(Some).ok_or_else(invalid)
        }
        // `new Date(true)` is 1 ms after the epoch; `new Date([])`, `new Date({})` are invalid.
        Some(Value::Bool(b)) => Ok(JsDate::from_millis(i64::from(*b))),
        Some(_) => Err(invalid()),
    }
}

/// Set or clear a time-based expiry (past dates are accepted).
pub async fn set_expiry_for(
    db: &PgPool,
    actor: &UploadActor,
    ids: &[Value],
    when: Option<&Value>,
) -> UploadResult<Vec<Upload>> {
    let expires_at = parse_expiry(when)?;
    apply_to_uploads(
        db,
        actor,
        ids,
        &[("expires_at", PatchValue::Date(expires_at))],
        "admin.upload.link_changed",
        detail(vec![("expiresAt", json!(expires_at.map(|d| d.to_iso_string())))]),
    )
    .await
}

/// Set a view cap (burn after N views), or null to clear. Floored at 1.
pub async fn set_max_views_for(
    db: &PgPool,
    actor: &UploadActor,
    ids: &[Value],
    max_views: Option<&Value>,
) -> UploadResult<Vec<Upload>> {
    let value = match max_views {
        None | Some(Value::Null) => None,
        Some(v) if crate::rpc::tagged::is_undefined(v) => None,
        Some(v) => {
            let n = crate::rpc::tagged::raw_num(v).filter(|_| v.is_number() || v.is_object());
            match n {
                Some(n) if n.is_finite() => Some(n.floor().max(1.0)),
                _ => return Err(UploadOpError::new(UploadIssue::InvalidViews, "View limit must be a number").into()),
            }
        }
    };
    let shown = match value {
        Some(v) if v <= i64::MAX as f64 => json!(v as i64),
        Some(v) => json!(v),
        None => Value::Null,
    };
    apply_to_uploads(
        db,
        actor,
        ids,
        &[("max_views", PatchValue::Num(value))],
        "admin.upload.link_changed",
        detail(vec![("maxViews", shown)]),
    )
    .await
}

/// Set or clear a per-file password (`null` or "" clears). `passwordVersion`
/// is cleared along with it, ALWAYS. Hashed once for the whole batch.
pub async fn set_password_for(
    db: &PgPool,
    actor: &UploadActor,
    ids: &[Value],
    password: Option<&Value>,
) -> UploadResult<Vec<Upload>> {
    let text = password.and_then(Value::as_str);
    if let Some(p) = text
        && p.encode_utf16().count() > UPLOAD_PASSWORD_MAX
    {
        return Err(UploadOpError::new(
            UploadIssue::PasswordTooLong,
            format!("Password must be at most {UPLOAD_PASSWORD_MAX} characters"),
        )
        .into());
    }
    let hash = match text.filter(|p| !p.is_empty()) {
        Some(p) => Some(crate::password::hash_password(p).await?),
        None => None,
    };
    let set = hash.is_some();
    apply_to_uploads(
        db,
        actor,
        ids,
        &[("password_hash", PatchValue::Text(hash)), ("password_version", PatchValue::Num(None))],
        "admin.upload.link_changed",
        detail(vec![("passwordSet", json!(set))]),
    )
    .await
}

/// What is sitting in one account's trash right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct TrashTotals {
    pub files: i64,
    pub bytes: i64,
}

/// `EmptyTrashResult`
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct EmptyTrashResult {
    pub files: i64,
    pub bytes: i64,
    pub failed: i64,
}

/// Rows read, and destroyed, per round trip.
const TRASH_PURGE_BATCH: i64 = 200;
/// Hard stop, so one call can never run away with the database.
const TRASH_PURGE_MAX_ROUNDS: usize = 500;

/// Whose trash a request is about, named by the tag. Retired tags are NOT
/// consulted; an empty tag, an unknown tag and another account's tag are ONE
/// answer for a non-administrator.
async fn trash_owner_id(db: &PgPool, actor: &UploadActor, owner_tag: &str) -> UploadResult<String> {
    if owner_tag.is_empty() {
        return Err(not_found().into());
    }
    let target: Option<String> = sqlx::query_scalar(r#"select "id" from "user" where "user"."user_code" = $1 limit 1"#)
        .bind(owner_tag)
        .fetch_optional(db)
        .await?;
    let Some(target) = target else {
        return Err(not_found().into());
    };
    if target != actor.user_id && !actor.is_admin {
        return Err(not_found().into());
    }
    Ok(target)
}

/// How much is in that trash, counted over the whole table.
pub async fn trash_totals_for(db: &PgPool, actor: &UploadActor, owner_tag: &str) -> UploadResult<TrashTotals> {
    let owner_id = trash_owner_id(db, actor, owner_tag).await?;
    let (files, bytes): (i64, String) = sqlx::query_as(
        r#"select count(*), coalesce(sum("uploads"."size"), 0)::text from "uploads"
           where ("uploads"."owner_id" = $1 and "uploads"."deleted_at" is not null)"#,
    )
    .bind(&owner_id)
    .fetch_one(db)
    .await?;
    Ok(TrashTotals { files, bytes: crate::config::js_number(&bytes) as i64 })
}

/// Permanently delete everything in that trash, now. Batched; rows storage
/// refused are parked in `stuck` and excluded from the next query.
pub async fn empty_trash_for(
    db: &PgPool,
    storage: &dyn StorageDriver,
    actor: &UploadActor,
    owner_tag: &str,
) -> UploadResult<EmptyTrashResult> {
    let owner_id = trash_owner_id(db, actor, owner_tag).await?;
    let mut stuck: Vec<Uuid> = Vec::new();
    let mut files = 0i64;
    let mut bytes = 0i64;
    for _ in 0..TRASH_PURGE_MAX_ROUNDS {
        let batch: Vec<DeletableMedia> = sqlx::query_as(
            r#"select "id", "owner_id", "size", "storage_key", "thumb_key" from "uploads"
               where ("uploads"."owner_id" = $1 and "uploads"."deleted_at" is not null
                      and (cardinality($2::uuid[]) = 0 or "uploads"."id" <> all($2::uuid[])))
               order by "uploads"."deleted_at" asc limit $3"#,
        )
        .bind(&owner_id)
        .bind(&stuck)
        .bind(TRASH_PURGE_BATCH)
        .fetch_all(db)
        .await?;
        if batch.is_empty() {
            break;
        }
        let result = delete_media_bulk(db, storage, &batch).await?;
        files += result.deleted;
        bytes += result.bytes;
        if result.failed > 0 {
            let ids: Vec<Uuid> = batch.iter().map(|r| r.id).collect();
            let survivors: Vec<Uuid> =
                sqlx::query_scalar(r#"select "id" from "uploads" where "uploads"."id" = any($1)"#)
                    .bind(&ids)
                    .fetch_all(db)
                    .await?;
            stuck.extend(survivors);
        }
    }
    if owner_id != actor.user_id {
        let detail = json!({
            "by": actor.user_id,
            "byEmail": actor.email,
            "ownerId": owner_id,
            "ownerTag": owner_tag,
            "files": files,
            "bytes": bytes,
            "failed": stuck.len(),
        });
        crate::audit::record_audit(db, &actor.user_id, "admin.trash.emptied", detail, &actor.ip).await;
    }
    Ok(EmptyTrashResult { files, bytes, failed: stuck.len() as i64 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_cleaned_like_upstream() {
        let a = "9d2e1a54-0000-4000-8000-000000000001";
        let ids = vec![json!(a), json!(a), json!(5), json!("nope"), json!(null)];
        assert_eq!(clean_ids(&ids).len(), 1);
        let many: Vec<Value> = (0..600).map(|i| json!(format!("9d2e1a54-0000-4000-8000-{i:012}"))).collect();
        assert_eq!(clean_ids(&many).len(), UPLOAD_BATCH_MAX);
    }

    #[test]
    fn expiry_parsing() {
        assert_eq!(parse_expiry(None).unwrap(), None);
        assert_eq!(parse_expiry(Some(&json!(""))).unwrap(), None);
        assert_eq!(parse_expiry(Some(&json!(null))).unwrap(), None);
        assert_eq!(
            parse_expiry(Some(&json!("2026-10-20T10:00:00.000Z"))).unwrap().unwrap().to_iso_string(),
            "2026-10-20T10:00:00.000Z"
        );
        assert_eq!(parse_expiry(Some(&json!("garbage"))).unwrap_err().message, "Invalid date");
        assert_eq!(parse_expiry(Some(&json!(0))).unwrap().unwrap().millis(), 0);
    }
}
