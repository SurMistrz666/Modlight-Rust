//! Port of `app/actions/admin.ts`, the account-administration exports
//! (CONTRACT 7.4, module `admin`): `loadUserDetail`, `updateUserAccount`,
//! `setUserRole`, `setUserBanned`, `createAccount`, `resendInvite`,
//! `setUserEmailVerified`, `sendVerificationLink`.
//!
//! The other exports of admin.ts (`loadOrphans`, `deleteOrphan`,
//! `purgeAllOrphans`, `loadDeletionImpact`, `deleteUserAccount`,
//! `recalculateEveryQuota`, `runStorageScan`) are ported by srv-library and
//! will be added to this same file (and to [`register`]) at integration.
//!
//! The admin role is re-checked in every function below. The panel hides
//! these controls from everyone else, but a Server Action is a public POST
//! endpoint: the UI is never the authorisation boundary.
//!
//! Arguments arrive as tagged JSON from any caller, so the JavaScript reading
//! of them is reproduced ([`super::account::js`]): `patch.name.trim()` is a
//! TypeError for anything but a string, `!userId` is truthiness, a role or a
//! ban flag "changed" when it is not strictly equal to the stored value, a
//! permission flag goes to Postgres as node-postgres would send it, and an
//! audit detail is what `JSON.stringify` makes of the raw argument.

use futures_util::future::BoxFuture;
use serde::Serialize;
use serde_json::{Map, Value, json};
use sqlx::{PgPool, Postgres, QueryBuilder};

use super::account::js::{self, Js, js};
use super::account::set_user_tag_as_admin;
use super::registration::{NotSerializable, json_of};
use crate::admin_users::{
    AdminUserDetail, AdminUserRow, PASSWORD_MAX, PASSWORD_MIN, is_valid_email, is_valid_username,
};
use crate::auth::server::{
    ServerCall, ban_user, create_user, revoke_user_sessions, send_verification_email, set_role, unban_user,
};
use crate::auth::session::AuthSession;
use crate::auth_helpers::headers_without_session;
use crate::avatar::avatar_url;
use crate::config::{js_round, js_trim};
use crate::credentials::write_credential_password;
use crate::data::sessions::js_slice;
use crate::db::models::{Columns, User};
use crate::db::types::JsDate;
use crate::invites::{create_invite, has_pending_invite, revoke_invites};
use crate::mail::{MailMessage, send_mail};
use crate::mail_settings::mail_ready;
use crate::mail_templates::{
    MailLocale, MailTexts, RenderedMail, pick_mail_locale, render_account_invited, render_email_changed,
    render_password_changed,
};
use crate::quota::{UNLIMITED_QUOTA, is_safe_integer};
use crate::rpc::{ActionCtx, ActionError, ActionResult, RpcRegistry};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("admin", "loadUserDetail", load_user_detail);
    reg.register("admin", "updateUserAccount", update_user_account);
    reg.register("admin", "setUserRole", set_user_role);
    reg.register("admin", "setUserBanned", set_user_banned);
    reg.register("admin", "createAccount", create_account);
    reg.register("admin", "resendInvite", resend_invite);
    reg.register("admin", "setUserEmailVerified", set_user_email_verified);
    reg.register("admin", "sendVerificationLink", send_verification_link);
}

/// The exports of the upstream module this file ports so far, in source order.
pub const EXPORTS: &[&str] = &[
    "loadUserDetail",
    "updateUserAccount",
    "setUserRole",
    "setUserBanned",
    "createAccount",
    "resendInvite",
    "setUserEmailVerified",
    "sendVerificationLink",
];

/// Longest display name `updateUserAccount` stores.
const NAME_MAX: usize = 120;

/// Longest address accepted (RFC 5321's path limit).
const EMAIL_MAX: usize = 254;

// ---------------------------------------------------------------------------
// JavaScript readings of the arguments
// ---------------------------------------------------------------------------

/// A JavaScript `TypeError` (its message, as `next dev` shows it; production
/// masks it).
fn type_error(message: impl Into<String>) -> ActionError {
    ActionError::thrown(message)
}

/// `value.trim()` on something that must be a string; `expr` is how the
/// source spells it, for the TypeError.
fn trim_arg<'a>(v: Js<'a>, expr: &str) -> Result<&'a str, ActionError> {
    match v {
        Js::Str(s) => Ok(js_trim(s)),
        Js::Undefined => Err(type_error("Cannot read properties of undefined (reading 'trim')")),
        Js::Null => Err(type_error("Cannot read properties of null (reading 'trim')")),
        _ => Err(type_error(format!("{expr}.trim is not a function"))),
    }
}

/// `s.length` (UTF-16 code units).
fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `value.length` on a value that is not nullish: a string or an array has
/// one, an object has whatever its `length` property holds, a number, a
/// boolean, a bigint or a Date has none (`undefined`).
fn js_length(v: Js<'_>) -> Js<'_> {
    match v {
        Js::Str(s) => Js::Num(utf16_len(s) as f64),
        Js::Array(items) => Js::Num(items.len() as f64),
        Js::Object(_) => v.prop("length"),
        _ => Js::Undefined,
    }
}

/// `a === "literal"`
fn strict_eq_str(v: Js<'_>, s: &str) -> bool {
    matches!(v, Js::Str(x) if x == s)
}

/// The raw (tagged) value of `object.key`, for `JSON.stringify` and for
/// bodies handed on to Better Auth.
fn raw_prop<'a>(v: Js<'a>, key: &str) -> Option<&'a Value> {
    match v {
        Js::Object(map) => map.get(key).filter(|x| !crate::rpc::tagged::is_undefined(x)),
        _ => None,
    }
}

/// `{ by, byEmail, ...fields }` as the `jsonb` column receives it:
/// `JSON.stringify` of each value (`undefined` drops the key), or `None` when
/// it throws (a bigint), in which case `recordAudit` logs instead of writing.
fn detail(session: &AuthSession, fields: Vec<(&str, Option<Value>)>) -> Option<Value> {
    let mut map = Map::new();
    map.insert("by".into(), Value::String(session.user.id.clone()));
    map.insert("byEmail".into(), Value::String(session.user.email.clone()));
    for (key, value) in fields {
        let Some(value) = value else { continue };
        match json_of(&value) {
            Ok(Some(v)) => {
                map.insert(key.into(), v);
            }
            Ok(None) => {}
            Err(NotSerializable) => return None,
        }
    }
    Some(Value::Object(map))
}

/// `recordAudit(actor, action, detail)` with the request's client IP.
async fn audit(ctx: &ActionCtx, actor: &str, action: &str, detail: Option<Value>) {
    match detail {
        Some(detail) => crate::audit::record_audit(&ctx.state.db, actor, action, detail, &ctx.client_ip()).await,
        None => tracing::error!("[audit] could not record {action}: Do not know how to serialize a BigInt"),
    }
}

/// A Better Auth call's thrown error as the action would rethrow it.
fn rethrow(call: &ServerCall) -> Result<Value, ActionError> {
    call.json().map_err(|err| ActionError::thrown(err.message()))
}

// ---------------------------------------------------------------------------
// Account editing
// ---------------------------------------------------------------------------

/// Send a courtesy notification, never letting it fail the operation it follows.
///
/// "Your password was changed" is the only signal a user gets that somebody else
/// took over their account, so it is worth sending. It is not worth failing an
/// administrative action over: by the time it goes out, the password is already
/// changed, and an exception here would report a completed change as a failure.
async fn notify(ctx: &ActionCtx, to: &str, message: &RenderedMail) {
    let sent = async {
        if !mail_ready(&ctx.state).await? {
            return Ok(());
        }
        send_mail(&ctx.state, &MailMessage::from_rendered(to, message)).await
    }
    .await;
    if let Err(err) = sent {
        tracing::error!("[mail] notification not delivered: {err}");
    }
}

/// `(await headers()).get("accept-language")`: every value, joined like the
/// Fetch `Headers` object joins a repeated header.
fn accept_language(ctx: &ActionCtx) -> Option<String> {
    let values: Vec<&str> = ctx.headers.get_all("accept-language").iter().filter_map(|v| v.to_str().ok()).collect();
    if values.is_empty() { None } else { Some(values.join(", ")) }
}

/// Language for a message to `user`. Falls back to the request's own header.
fn mail_locale_for(ctx: &ActionCtx, locale: Option<&str>) -> MailLocale {
    pick_mail_locale(locale, accept_language(ctx).as_deref())
}

fn mail_texts(ctx: &ActionCtx) -> MailTexts {
    MailTexts::from_messages(&ctx.state.messages, &ctx.state.config.app_name)
}

/// `SELECT ... FROM "user" WHERE id = userId LIMIT 1`, with `userId` sent as
/// node-postgres sends whatever value it is (`user.id` is text, so a value of
/// another type never errors, it just matches nothing, or the id spelled the
/// same).
async fn find_user(db: &PgPool, user_id: Js<'_>) -> sqlx::Result<Option<User>> {
    let sql = format!("SELECT {} FROM {} WHERE \"id\" = $1 LIMIT 1", User::COLUMNS, User::TABLE);
    sqlx::query_as::<_, User>(&sql).bind(js::pg_text(user_id)).fetch_optional(db).await
}

/// Everything the edit form needs about one account, read fresh.
///
/// Not folded into the table query: the dialog opens minutes or hours after the
/// list was fetched, and editing a stale snapshot is how two admins overwrite
/// each other without either of them noticing.
async fn load_user_detail(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    let user_id = js(ctx.args.raw(0));
    if !user_id.truthy() {
        return Ok(Value::Null);
    }
    let db = &ctx.state.db;
    let Some(row) = find_user(db, user_id).await? else {
        return Ok(Value::Null);
    };

    let files: i32 =
        sqlx::query_scalar(r#"SELECT count(*)::int FROM "uploads" WHERE "owner_id" = $1 AND "deleted_at" IS NULL"#)
            .bind(&row.id)
            .fetch_one(db)
            .await?;
    let cred: Option<Option<String>> = sqlx::query_scalar(
        r#"SELECT "password" FROM "account" WHERE "user_id" = $1 AND "provider_id" = 'credential' LIMIT 1"#,
    )
    .bind(&row.id)
    .fetch_optional(db)
    .await?;
    let invite_pending = has_pending_invite(db, &row.id).await?;

    let detail = AdminUserDetail {
        row: AdminUserRow {
            avatar: avatar_url(row.user_code.as_deref(), row.image.as_deref()),
            id: row.id,
            name: row.name,
            email: row.email,
            role: row.role,
            banned: row.banned.unwrap_or(false),
            created_at: Some(row.created_at.to_iso_string()),
            tag: row.user_code,
            used_quota: row.used_quota.unwrap_or(0),
            max_quota: row.max_quota.unwrap_or(UNLIMITED_QUOTA),
            files: i64::from(files),
            allow_root_short_links: row.allow_root_short_links.unwrap_or(false),
            email_verified: row.email_verified,
            invite_pending,
        },
        username: row.username,
        two_factor_enabled: row.two_factor_enabled.unwrap_or(false),
        public_profile: row.public_profile.unwrap_or(false),
        skip_tag_in_url: row.skip_tag_in_url.unwrap_or(false),
        // A row with a NULL password is an invitation that has not been accepted:
        // `createAccount` strips the throwaway credential it had to create.
        has_password: cred.flatten().is_some_and(|p| !p.is_empty()),
    };
    crate::rpc::ok(&detail_json(&detail))
}

/// The object `loadUserDetail` returns, keys in the order of its literal.
fn detail_json(d: &AdminUserDetail) -> Value {
    let r = &d.row;
    json!({
        "id": r.id,
        "name": r.name,
        "email": r.email,
        "emailVerified": r.email_verified,
        "username": d.username,
        "role": r.role,
        "banned": r.banned,
        "createdAt": r.created_at,
        "tag": r.tag,
        "avatar": r.avatar,
        "usedQuota": r.used_quota,
        "maxQuota": r.max_quota,
        "files": r.files,
        "allowRootShortLinks": r.allow_root_short_links,
        "publicProfile": d.public_profile,
        "skipTagInUrl": d.skip_tag_in_url,
        "twoFactorEnabled": d.two_factor_enabled,
        "hasPassword": d.has_password,
        "invitePending": r.invite_pending,
    })
}

/// What the form changed, so the confirmation can name it rather than say "saved".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UserChange {
    Name,
    Username,
    Email,
    Tag,
    Password,
    Quota,
    Role,
    Banned,
    Permissions,
}

/// Why a field was refused. Travels as data, never as a thrown message: Next
/// masks the message of an error raised inside a Server Action in production, so
/// anything the operator has to read would arrive as "An error occurred".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserFieldIssue {
    pub field: &'static str,
    pub reason: String,
}

impl UserFieldIssue {
    fn new(field: &'static str, reason: &str) -> Self {
        UserFieldIssue { field, reason: reason.to_string() }
    }
}

fn update_not_found() -> ActionResult {
    Ok(json!({ "ok": false, "reason": "not-found" }))
}

fn update_invalid(issues: &[UserFieldIssue]) -> ActionResult {
    Ok(json!({ "ok": false, "reason": "invalid", "issues": issues }))
}

/// `UpdateUserPatch` as `updateUserAccount` reads it: each field `undefined`
/// unless the caller sent it. Fields the form may write; anything omitted is
/// left exactly as it is.
#[derive(Debug, Clone, Copy)]
struct Patch<'a> {
    /// The object itself, for the raw values handed on unchanged.
    raw: Js<'a>,
    name: Js<'a>,
    /// Empty string clears the login name.
    username: Js<'a>,
    email: Js<'a>,
    /// `user.userCode`: the first segment of every public link.
    tag: Js<'a>,
    /// A new password. Never a current one: there is no current one to read.
    new_password: Js<'a>,
    /// Byte limit; anything negative means unlimited.
    max_quota: Js<'a>,
    role: Js<'a>,
    banned: Js<'a>,
    allow_root_short_links: Js<'a>,
    public_profile: Js<'a>,
    skip_tag_in_url: Js<'a>,
}

impl<'a> Patch<'a> {
    /// The first property read (`patch.name`) throws on null and undefined.
    /// Any other value is read key by key; a primitive, an array or a Date
    /// has none of these properties.
    fn read(patch: Js<'a>) -> Result<Self, ActionError> {
        match patch {
            Js::Undefined => return Err(type_error("Cannot read properties of undefined (reading 'name')")),
            Js::Null => return Err(type_error("Cannot read properties of null (reading 'name')")),
            _ => {}
        }
        Ok(Patch {
            raw: patch,
            name: patch.prop("name"),
            username: patch.prop("username"),
            email: patch.prop("email"),
            tag: patch.prop("tag"),
            new_password: patch.prop("newPassword"),
            max_quota: patch.prop("maxQuota"),
            role: patch.prop("role"),
            banned: patch.prop("banned"),
            allow_root_short_links: patch.prop("allowRootShortLinks"),
            public_profile: patch.prop("publicProfile"),
            skip_tag_in_url: patch.prop("skipTagInUrl"),
        })
    }
}

/// `users.email` or `users.username`, the two columns `nameTaken` checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NameColumn {
    Email,
    Username,
}

impl NameColumn {
    fn sql(self) -> &'static str {
        match self {
            NameColumn::Email => "email",
            NameColumn::Username => "username",
        }
    }
}

/// `nameTaken(column, value, userId)`, behind a trait so phase 1 can be
/// tested without a database.
trait NameLookup: Sync {
    fn taken<'b>(&'b self, column: NameColumn, value: &'b str) -> BoxFuture<'b, sqlx::Result<bool>>;
}

/// The real `nameTaken`, for one target account.
struct NameTaken<'b> {
    db: &'b PgPool,
    /// `exceptUserId` as JavaScript compares it with `!==`: `None` when the
    /// caller passed something that is not a string, which equals no row.
    except_user_id: Option<&'b str>,
}

impl NameLookup for NameTaken<'_> {
    fn taken<'b>(&'b self, column: NameColumn, value: &'b str) -> BoxFuture<'b, sqlx::Result<bool>> {
        Box::pin(name_taken(self.db, column, value, self.except_user_id))
    }
}

/// Is `value` already used by a different account, ignoring case?
async fn name_taken(db: &PgPool, column: NameColumn, value: &str, except_user_id: Option<&str>) -> sqlx::Result<bool> {
    let sql = format!(r#"SELECT "id" FROM "user" WHERE lower("{}") = $1 LIMIT 1"#, column.sql());
    let hit: Option<String> = sqlx::query_scalar(&sql).bind(value.to_lowercase()).fetch_optional(db).await?;
    Ok(hit.is_some_and(|id| Some(id.as_str()) != except_user_id))
}

/// The columns phase 1 decided to write (`Partial<typeof users.$inferInsert>`).
#[derive(Debug, Default)]
struct UserColumns<'a> {
    name: Option<String>,
    username: Option<Option<String>>,
    /// Written together with `email_verified = true`.
    email: Option<String>,
    max_quota: Option<i64>,
    /// Permission flags, as sent: node-postgres hands them to a boolean
    /// column as text, and Postgres decides at the write.
    allow_root_short_links: Option<Js<'a>>,
    public_profile: Option<Js<'a>>,
    skip_tag_in_url: Option<Js<'a>>,
}

impl UserColumns<'_> {
    fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.username.is_none()
            && self.email.is_none()
            && self.max_quota.is_none()
            && self.allow_root_short_links.is_none()
            && self.public_profile.is_none()
            && self.skip_tag_in_url.is_none()
    }
}

/// The outcome of phase 1 (validation, no writes).
#[derive(Debug)]
struct Plan<'a> {
    columns: UserColumns<'a>,
    changed: Vec<UserChange>,
    issues: Vec<UserFieldIssue>,
    /// The stored address, when the address changes.
    previous_email: Option<String>,
    /// `patch.newPassword?.length ? patch.newPassword : null`.
    new_password: Option<Js<'a>>,
    role_changed: bool,
    ban_changed: bool,
}

/// Phase 1 of `updateUserAccount`: validate everything, write nothing.
async fn plan_update<'a>(
    patch: &Patch<'a>,
    target: &User,
    is_self: bool,
    names: &dyn NameLookup,
) -> Result<Plan<'a>, ActionError> {
    let mut issues = Vec::new();
    let mut changed = Vec::new();
    let mut columns = UserColumns::default();

    if !matches!(patch.name, Js::Undefined) {
        let next = js_slice(trim_arg(patch.name, "patch.name")?, NAME_MAX);
        if next.is_empty() {
            issues.push(UserFieldIssue::new("name", "empty"));
        } else if next != target.name {
            columns.name = Some(next);
            changed.push(UserChange::Name);
        }
    }

    if !matches!(patch.username, Js::Undefined) {
        let raw = match patch.username {
            Js::Null => "",
            Js::Str(s) => s,
            _ => return Err(type_error("(patch.username ?? \"\").trim is not a function")),
        };
        let next = Some(js_trim(raw)).filter(|s| !s.is_empty());
        if next != target.username.as_deref() {
            match next {
                Some(n) if !is_valid_username(n) => issues.push(UserFieldIssue::new("username", "charset")),
                // Case-insensitive: sign-in resolves a username with `lower(...)`, so
                // `Ada` and `ada` would be the same login for two different accounts.
                Some(n) if names.taken(NameColumn::Username, n).await? => {
                    issues.push(UserFieldIssue::new("username", "taken"));
                }
                _ => {
                    columns.username = Some(next.map(str::to_string));
                    changed.push(UserChange::Username);
                }
            }
        }
    }

    let mut previous_email = None;
    if !matches!(patch.email, Js::Undefined) {
        let next = trim_arg(patch.email, "patch.email")?.to_lowercase();
        if !is_valid_email(&next) || utf16_len(&next) > EMAIL_MAX {
            issues.push(UserFieldIssue::new("email", "invalid"));
        } else if next != target.email.to_lowercase() {
            if names.taken(NameColumn::Email, &next).await? {
                issues.push(UserFieldIssue::new("email", "taken"));
            } else {
                previous_email = Some(target.email.clone());
                // The admin is asserting this address. Leaving it unverified would
                // lock the account out on installs that require verification to
                // sign in, which turns "fix my email" into "lose my account".
                columns.email = Some(next);
                changed.push(UserChange::Email);
            }
        }
    }

    // `patch.newPassword?.length ? patch.newPassword : null`, then the length
    // compared with `<` / `>` (a non-number length is coerced; NaN is neither).
    let mut new_password = None;
    if !matches!(patch.new_password, Js::Undefined | Js::Null) {
        let length = js_length(patch.new_password);
        if length.truthy() {
            new_password = Some(patch.new_password);
            let n = length.to_number();
            if n < PASSWORD_MIN as f64 {
                issues.push(UserFieldIssue::new("password", "too-short"));
            } else if n > PASSWORD_MAX as f64 {
                issues.push(UserFieldIssue::new("password", "too-long"));
            }
        }
    }

    if !matches!(patch.max_quota, Js::Undefined) {
        // `Number.isFinite` does not coerce: only a real, finite number passes.
        match patch.max_quota {
            Js::Num(raw) if raw.is_finite() && is_safe_integer(js_round(raw)) => {
                let value = if raw < 0.0 { UNLIMITED_QUOTA } else { js_round(raw) as i64 };
                if value != target.max_quota.unwrap_or(UNLIMITED_QUOTA) {
                    columns.max_quota = Some(value);
                    changed.push(UserChange::Quota);
                }
            }
            _ => issues.push(UserFieldIssue::new("quota", "invalid")),
        }
    }

    // `nextRole !== (target.role ?? "user")`: the value is not validated.
    let role_changed =
        !matches!(patch.role, Js::Undefined) && !strict_eq_str(patch.role, target.role.as_deref().unwrap_or("user"));
    if role_changed && is_self {
        issues.push(UserFieldIssue::new("role", "self"));
    }

    let ban_changed =
        !matches!(patch.banned, Js::Undefined) && patch.banned != Js::Bool(target.banned.unwrap_or(false));
    if ban_changed && is_self {
        issues.push(UserFieldIssue::new("banned", "self"));
    }

    // Permission flags: booleans, nothing to reject.
    let mut permissions_changed = false;
    let flags = [
        (patch.allow_root_short_links, target.allow_root_short_links, &mut columns.allow_root_short_links),
        (patch.public_profile, target.public_profile, &mut columns.public_profile),
        (patch.skip_tag_in_url, target.skip_tag_in_url, &mut columns.skip_tag_in_url),
    ];
    for (value, stored, slot) in flags {
        if !matches!(value, Js::Undefined) && value != Js::Bool(stored.unwrap_or(false)) {
            *slot = Some(value);
            permissions_changed = true;
        }
    }
    if permissions_changed {
        changed.push(UserChange::Permissions);
    }

    Ok(Plan { columns, changed, issues, previous_email, new_password, role_changed, ban_changed })
}

/// `db.update(users).set(columns).where(eq(users.id, userId))`, drizzle's
/// `$onUpdate` stamping `updated_at`. A permission flag Postgres cannot read
/// as a boolean fails the whole statement, as it did upstream.
async fn write_columns(db: &PgPool, user_id: &str, columns: &UserColumns<'_>) -> Result<(), ActionError> {
    let flag = |v: Option<Js<'_>>| v.map(js::pg_boolean).transpose();
    let allow_root = flag(columns.allow_root_short_links)?;
    let public_profile = flag(columns.public_profile)?;
    let skip_tag = flag(columns.skip_tag_in_url)?;

    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(r#"UPDATE "user" SET "#);
    let mut set = qb.separated(", ");
    if let Some(name) = &columns.name {
        set.push(r#""name" = "#).push_bind_unseparated(name.clone());
    }
    if let Some(username) = &columns.username {
        set.push(r#""username" = "#).push_bind_unseparated(username.clone());
    }
    if let Some(email) = &columns.email {
        set.push(r#""email" = "#).push_bind_unseparated(email.clone());
        set.push(r#""email_verified" = true"#);
    }
    if let Some(quota) = columns.max_quota {
        set.push(r#""max_quota" = "#).push_bind_unseparated(quota);
    }
    if let Some(v) = allow_root {
        set.push(r#""allow_root_short_links" = "#).push_bind_unseparated(v);
    }
    if let Some(v) = public_profile {
        set.push(r#""public_profile" = "#).push_bind_unseparated(v);
    }
    if let Some(v) = skip_tag {
        set.push(r#""skip_tag_in_url" = "#).push_bind_unseparated(v);
    }
    set.push(r#""updated_at" = "#).push_bind_unseparated(JsDate::now());
    qb.push(r#" WHERE "id" = "#).push_bind(user_id.to_string());
    qb.build().execute(db).await?;
    Ok(())
}

/// Edit an account from the admin panel.
///
/// Two rules shape the whole function:
///
///  1. **Everything is validated before anything is written.** A form with eight
///     fields fails on one of them often enough that "half the account was
///     updated, work out which half" is not an acceptable outcome. The single
///     exception is the tag, whose uniqueness can only be settled by the write
///     itself, so it goes first, and a lost race returns before any other
///     column is touched.
///
///  2. **An admin may not disarm themselves.** Demoting or banning the account
///     you are signed in with is how an install ends up with nobody who can
///     administer it, so both are refused here (Better Auth refuses the self-ban
///     too; this is the near guard, not the only one).
///
/// Changing an email address or a password is logged to `audit_log`: those two
/// are the ones that hand over control of an account, and "who did this and
/// when" has to survive the session that did it.
async fn update_user_account(ctx: ActionCtx) -> ActionResult {
    let patch = ctx.args.raw(1);
    update_user_account_with(&ctx, ctx.args.raw(0), js(patch)).await
}

async fn update_user_account_with(ctx: &ActionCtx, user_id_raw: Option<&Value>, patch: Js<'_>) -> ActionResult {
    let session = ctx.require_admin().await?;
    let user_id = js(user_id_raw);
    if !user_id.truthy() {
        return update_not_found();
    }
    let db = &ctx.state.db;
    let Some(target) = find_user(db, user_id).await? else {
        return update_not_found();
    };

    let is_self = target.id == session.user.id;
    let patch = Patch::read(patch)?;
    let except = match user_id {
        Js::Str(s) => Some(s),
        _ => None,
    };

    // ── Validate ────────────────────────────────────────────────────────────
    let names = NameTaken { db, except_user_id: except };
    let plan = plan_update(&patch, &target, is_self, &names).await?;
    let Plan { columns, mut changed, issues, previous_email, new_password, role_changed, ban_changed } = plan;
    if !issues.is_empty() {
        return update_invalid(&issues);
    }

    // ── Apply ───────────────────────────────────────────────────────────────

    // Tag first. It is the one field whose validity depends on a race (two admins,
    // one free tag), and `setUserTag` settles that inside a transaction. Doing it
    // before any other write means a lost race leaves the account untouched.
    let next_tag = match patch.tag {
        Js::Undefined | Js::Null => None,
        Js::Str(s) => Some(js_trim(s)),
        _ => return Err(type_error("patch.tag?.trim is not a function")),
    };
    if let Some(tag) = next_tag
        && tag != target.user_code.as_deref().unwrap_or("")
    {
        let res = set_user_tag_as_admin(ctx, user_id_raw, Some(&Value::String(tag.to_string()))).await?;
        if res.get("ok") != Some(&Value::Bool(true)) {
            let reason = res.get("reason").and_then(Value::as_str).unwrap_or_default();
            return update_invalid(&[UserFieldIssue::new("tag", reason)]);
        }
        changed.push(UserChange::Tag);
    }

    if !columns.is_empty() {
        write_columns(db, &target.id, &columns).await?;
    }

    if role_changed && patch.role.truthy() {
        let role = raw_prop(patch.raw, "role").cloned().unwrap_or(Value::Null);
        let call = set_role(&ctx.state, &ctx.headers, ctx.peer, &target.id, &role).await;
        call.apply_cookies(ctx);
        rethrow(&call)?;
        changed.push(UserChange::Role);
    }

    if ban_changed {
        let call = if patch.banned.truthy() {
            ban_user(&ctx.state, &ctx.headers, ctx.peer, &target.id).await
        } else {
            unban_user(&ctx.state, &ctx.headers, ctx.peer, &target.id).await
        };
        call.apply_cookies(ctx);
        rethrow(&call)?;
        changed.push(UserChange::Banned);
    }

    let mut sessions_revoked = false;
    if let Some(password) = new_password {
        // `hashPassword` starts with `password.normalize("NFKC")`.
        let Js::Str(password) = password else {
            return Err(type_error("password.normalize is not a function"));
        };
        write_credential_password(db, &target.id, password).await?;
        // A password nobody knows any more is not a lock if the old sessions stay
        // open. Whoever the admin is locking out has a cookie, not a password.
        let call = revoke_user_sessions(&ctx.state, &ctx.headers, ctx.peer, &target.id).await;
        call.apply_cookies(ctx);
        rethrow(&call)?;
        sessions_revoked = true;
        changed.push(UserChange::Password);
    }

    // `if (previousEmail)`: the stored address, which is falsy when empty.
    let previous_email = previous_email.filter(|e| !e.is_empty());

    // An invitation is a bearer token that sets this account's password and marks
    // it confirmed. It was mailed to the OLD address, so once the address changes
    // that mailbox must stop being a way in; otherwise "move this account to a
    // new email" leaves the previous holder able to reclaim it at any point in
    // the next seven days. The administrator can always issue a fresh one.
    if previous_email.is_some() {
        revoke_invites(db, &target.id).await?;
    }

    let name = columns.name.clone().unwrap_or_else(|| target.name.clone());
    let email = columns.email.clone().unwrap_or_else(|| target.email.clone());

    // ── Audit the two that hand over control of the account ─────────────────
    let target_user_id = user_id_raw.cloned();
    if let Some(from) = &previous_email {
        let detail = detail(
            &session,
            vec![
                ("targetUserId", target_user_id.clone()),
                ("targetName", Some(Value::String(name.clone()))),
                ("from", Some(Value::String(from.clone()))),
                ("to", columns.email.clone().map(Value::String)),
            ],
        );
        audit(ctx, &session.user.id, "admin.user.email_changed", detail).await;
    }
    if new_password.is_some() {
        let detail = detail(
            &session,
            vec![
                ("targetUserId", target_user_id.clone()),
                ("targetName", Some(Value::String(name.clone()))),
                ("targetEmail", Some(Value::String(email.clone()))),
                ("sessionsRevoked", Some(Value::Bool(true))),
            ],
        );
        audit(ctx, &session.user.id, "admin.user.password_reset", detail).await;
    }

    // ── Tell the account holder, on the two changes that take it away ───────
    let locale = mail_locale_for(ctx, target.locale.as_deref());
    let texts = mail_texts(ctx);
    if let (Some(from), Some(to)) = (&previous_email, columns.email.as_deref().filter(|e| !e.is_empty())) {
        // Sent to the OLD address: the new one is by definition already in the
        // hands of whoever asked for the change, and the person who needs to know
        // is the one who just lost the account.
        let mail = render_email_changed(&texts, locale, from, to);
        notify(ctx, from, &mail).await;
    }
    if new_password.is_some() {
        let mail = render_password_changed(&texts, locale, &email);
        notify(ctx, &email, &mail).await;
    }

    ctx.revalidate_path("/dashboard/admin");
    // Tags, permissions and the display name all show up outside the admin page.
    ctx.revalidate_path("/");

    crate::rpc::ok(&json!({
        "ok": true,
        "changed": changed,
        "sessionsRevoked": sessions_revoked,
        "name": name,
        "email": email,
    }))
}

/// `{ [key]: value }` built server-side by the two row-menu wrappers.
fn one_key_patch(key: &str, value: Option<&Value>) -> Value {
    let mut map = Map::new();
    if let Some(v) = value {
        map.insert(key.to_string(), v.clone());
    }
    Value::Object(map)
}

/// Promote or demote, from the row menu.
///
/// A thin wrapper on purpose: the self-demotion guard lives in one place, and the
/// quick action in the table must not be a way around it. Better Auth's own
/// `setRole` has no such check: an admin can absolutely strip their own role
/// through it and lock the install out of its own administration.
async fn set_user_role(ctx: ActionCtx) -> ActionResult {
    let patch = one_key_patch("role", ctx.args.raw(1));
    update_user_account_with(&ctx, ctx.args.raw(0), js(Some(&patch))).await
}

/// Suspend or reinstate an account, from the row menu. Banning ends the
/// account's sessions (Better Auth does that as part of the ban).
async fn set_user_banned(ctx: ActionCtx) -> ActionResult {
    let patch = one_key_patch("banned", ctx.args.raw(1));
    update_user_account_with(&ctx, ctx.args.raw(0), js(Some(&patch))).await
}

// ---------------------------------------------------------------------------
// Creating accounts
//
// An administrator may create an account whatever `register_enabled` says. The
// two are unrelated: that flag governs strangers signing THEMSELVES up, and an
// administrator is not a stranger. The gate that used to conflate them lived in
// a database hook every creation path ran through (see lib/auth.ts).
// ---------------------------------------------------------------------------

/// A password nobody will ever use, long enough that guessing is pointless
/// (`randomBytes(24).toString("base64url")`).
fn random_password() -> String {
    crate::ids::secure_token(24)
}

fn create_refused(reason: &str) -> ActionResult {
    Ok(json!({ "ok": false, "reason": reason }))
}

/// `{ ok: false, reason: "failed", detail: err.message }`. Returned as data,
/// so the message is NOT masked.
fn create_failed(detail: &str) -> ActionResult {
    Ok(json!({ "ok": false, "reason": "failed", "detail": detail }))
}

/// A body field for Better Auth's validator, and the type name zod would put
/// in "received ..." when the field fails `z.string()` but the validator
/// would misname it: JSON cannot carry a Date or a bigint, so their tagged
/// forms reach it as objects.
fn zod_body_value(raw: &Value) -> (Value, Option<&'static str>) {
    match js(Some(raw)) {
        Js::Date(_) => (Value::Object(Map::new()), Some("Date")),
        Js::BigInt(_) => (Value::Object(Map::new()), Some("bigint")),
        // Infinity: a number to zod.
        Js::Num(n) if !n.is_finite() => (json!(0), None),
        _ => (raw.clone(), None),
    }
}

/// Create an account: "password" mode (the administrator sets one now and
/// passes it on out of band) or "invite" mode (the account starts with NO
/// password and the new user sets their own through a mailed link; preferred
/// where mail works: nothing guessable exists during the window between
/// creating the account and the person using it, and the administrator never
/// learns a password that is not theirs).
async fn create_account(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_admin().await?;
    let input = js(ctx.args.raw(0));

    let email_value = match input {
        Js::Undefined => return Err(type_error("Cannot read properties of undefined (reading 'email')")),
        Js::Null => return Err(type_error("Cannot read properties of null (reading 'email')")),
        other => other.prop("email"),
    };
    let email = trim_arg(email_value, "input.email")?.to_lowercase();
    if !is_valid_email(&email) || utf16_len(&email) > EMAIL_MAX {
        return create_refused("invalid-email");
    }

    let invite = strict_eq_str(input.prop("mode"), "invite");
    let password = input.prop("password");
    // `!input.password || input.password.length < PASSWORD_MIN`; there is no max.
    if !invite && (!password.truthy() || js_length(password).to_number() < PASSWORD_MIN as f64) {
        return create_refused("weak-password");
    }
    if invite && !mail_ready(&ctx.state).await? {
        return create_refused("mail-unavailable");
    }

    let db = &ctx.state.db;
    let existing: Option<String> = sqlx::query_scalar(r#"SELECT "id" FROM "user" WHERE lower("email") = $1 LIMIT 1"#)
        .bind(&email)
        .fetch_optional(db)
        .await?;
    if existing.is_some() {
        return create_refused("taken");
    }

    // Better Auth owns account creation (id generation, the credential row, the
    // database hooks that assign a tag and a quota), so an invitation is created
    // WITH a throwaway password and then stripped of it. Writing the row by hand
    // would mean re-implementing all of that and drifting from it later.
    //
    // The body literal is evaluated inside upstream's `try`, so a name that is
    // not a string fails the creation (as data) rather than the action.
    let name = match input.prop("name") {
        Js::Undefined | Js::Null => email.clone(),
        Js::Str(s) => Some(js_trim(s)).filter(|t| !t.is_empty()).unwrap_or(&email).to_string(),
        // V8 names the expression as the production bundle spells it, where
        // the minifier renamed `input` to `a` (verified on the reference).
        _ => return create_failed("a.name?.trim is not a function"),
    };
    let (password_value, received) = if invite {
        (Value::String(random_password()), None)
    } else {
        zod_body_value(raw_prop(input, "password").unwrap_or(&Value::Null))
    };
    // `input.role ?? "user"`
    let role_value = match input.prop("role") {
        Js::Undefined | Js::Null => Value::String("user".into()),
        _ => raw_prop(input, "role").cloned().unwrap_or(Value::Null),
    };
    let body = json!({ "email": email, "name": name, "password": password_value, "role": role_value.clone() });
    let call = create_user(&ctx.state, &ctx.headers, ctx.peer, body).await;
    call.apply_cookies(&ctx);
    let user_id = match call.json() {
        Ok(created) => match created.get("user").and_then(|u| u.get("id")).and_then(Value::as_str) {
            Some(id) => id.to_string(),
            None => return create_failed("Cannot read properties of undefined (reading 'id')"),
        },
        Err(err) => {
            let mut message = err.message();
            if let Some(name) = received {
                message = message.replace(
                    "[body.password] Invalid input: expected string, received object",
                    &format!("[body.password] Invalid input: expected string, received {name}"),
                );
            }
            return create_failed(&message);
        }
    };

    let mut mail_sent = false;
    if invite {
        // Remove the throwaway credential: until the invitation is accepted the
        // account must have no password at all, not one nobody has been told.
        sqlx::query(
            r#"UPDATE "account" SET "password" = NULL, "updated_at" = $2 WHERE "user_id" = $1 AND "provider_id" = 'credential'"#,
        )
        .bind(&user_id)
        .bind(JsDate::now())
        .execute(db)
        .await?;

        let token = create_invite(db, &user_id).await?;
        let url = format!("{}/invite/{token}", ctx.state.config.app_url);
        let mail = render_account_invited(&mail_texts(&ctx), mail_locale_for(&ctx, None), &email, &url);
        match send_mail(&ctx.state, &MailMessage::from_rendered(&email, &mail)).await {
            Ok(()) => mail_sent = true,
            Err(err) => tracing::error!("[mail] invitation not delivered: {err}"),
        }
    } else {
        // The administrator is asserting this address. Leaving it unconfirmed would
        // lock the account out the moment confirmation is switched on, turning
        // "create an account" into "create an account nobody can use".
        sqlx::query(r#"UPDATE "user" SET "email_verified" = true, "updated_at" = $2 WHERE "id" = $1"#)
            .bind(&user_id)
            .bind(JsDate::now())
            .execute(db)
            .await?;
    }

    let detail = detail(
        &session,
        vec![
            ("targetUserId", Some(Value::String(user_id.clone()))),
            ("targetEmail", Some(Value::String(email.clone()))),
            ("mode", raw_prop(input, "mode").cloned()),
            ("role", Some(role_value)),
            ("mailSent", Some(Value::Bool(mail_sent))),
        ],
    );
    audit(&ctx, &session.user.id, "admin.user.created", detail).await;

    ctx.revalidate_path("/dashboard/admin");
    crate::rpc::ok(&json!({ "ok": true, "userId": user_id, "email": email, "invited": invite, "mailSent": mail_sent }))
}

fn invite_refused(reason: &str) -> ActionResult {
    Ok(json!({ "ok": false, "reason": reason }))
}

/// Issue a fresh invitation, invalidating any earlier one.
async fn resend_invite(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_admin().await?;
    let user_id = js(ctx.args.raw(0));
    if !user_id.truthy() {
        return invite_refused("not-found");
    }
    if !mail_ready(&ctx.state).await? {
        return invite_refused("mail-unavailable");
    }
    let db = &ctx.state.db;
    let Some(target) = find_user(db, user_id).await? else {
        return invite_refused("not-found");
    };

    let token = create_invite(db, &target.id).await?;
    let url = format!("{}/invite/{token}", ctx.state.config.app_url);
    let locale = mail_locale_for(&ctx, target.locale.as_deref());
    let mail = render_account_invited(&mail_texts(&ctx), locale, &target.email, &url);

    if let Err(err) = send_mail(&ctx.state, &MailMessage::from_rendered(&target.email, &mail)).await {
        tracing::error!("[mail] invitation not delivered: {err}");
        return invite_refused("send-failed");
    }

    let detail = detail(
        &session,
        vec![("targetUserId", ctx.args.raw(0).cloned()), ("targetEmail", Some(Value::String(target.email.clone())))],
    );
    audit(&ctx, &session.user.id, "admin.user.invited", detail).await;
    ctx.revalidate_path("/dashboard/admin");
    Ok(json!({ "ok": true }))
}

// ---------------------------------------------------------------------------
// Email confirmation
// ---------------------------------------------------------------------------

/// Confirm an address by hand.
///
/// This is an administrator overriding a security gate, so it is audited. It
/// exists because the alternative, on an install whose mail is broken or whose
/// user's mailbox is gone, is an account that can never sign in again.
async fn set_user_email_verified(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_admin().await?;
    let user_id = js(ctx.args.raw(0));
    if !user_id.truthy() {
        return Ok(json!({ "ok": false }));
    }
    let db = &ctx.state.db;
    let Some(target) = find_user(db, user_id).await? else {
        return Ok(json!({ "ok": false }));
    };

    // `.set({ emailVerified: verified })`: undefined is drizzle's "No values to
    // set", anything else goes to Postgres as text (NULL is refused there, the
    // column is NOT NULL).
    let verified = js::pg_boolean(js(ctx.args.raw(1)))?;
    sqlx::query(r#"UPDATE "user" SET "email_verified" = $1, "updated_at" = $2 WHERE "id" = $3"#)
        .bind(verified)
        .bind(JsDate::now())
        .bind(&target.id)
        .execute(db)
        .await?;

    let detail = detail(
        &session,
        vec![
            ("targetUserId", ctx.args.raw(0).cloned()),
            ("targetEmail", Some(Value::String(target.email.clone()))),
            ("verified", ctx.args.raw(1).cloned()),
        ],
    );
    audit(&ctx, &session.user.id, "admin.user.email_verified", detail).await;

    ctx.revalidate_path("/dashboard/admin");
    Ok(json!({ "ok": true }))
}

/// Send the confirmation link again, on the user's behalf.
async fn send_verification_link(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    let user_id = js(ctx.args.raw(0));
    if !user_id.truthy() {
        return invite_refused("not-found");
    }
    if !mail_ready(&ctx.state).await? {
        return invite_refused("mail-unavailable");
    }
    let Some(target) = find_user(&ctx.state.db, user_id).await? else {
        return invite_refused("not-found");
    };

    // WITHOUT the administrator's session. Better Auth's
    // /send-verification-email is two endpoints in one: with a session it
    // means "resend MY link" and rejects any other address outright
    // (EMAIL_MISMATCH), so forwarding the caller's cookie made this button
    // fail 100% of the time, for every user, with nothing but a console line.
    // Session-less it looks the account up by address, which is what an
    // administrator sending somebody else's link needs.
    let headers = headers_without_session(&ctx.headers);
    let call = send_verification_email(&ctx.state, &headers, ctx.peer, &target.email, "/dashboard").await;
    call.apply_cookies(&ctx);
    if let Err(err) = call.json() {
        tracing::error!("[mail] verification link not delivered: {}", err.message());
        return invite_refused("send-failed");
    }
    Ok(json!({ "ok": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: &str) -> User {
        User {
            id: id.into(),
            name: "Ada".into(),
            email: "Ada@Example.test".into(),
            email_verified: true,
            image: None,
            created_at: JsDate::now(),
            updated_at: JsDate::now(),
            role: None,
            banned: None,
            ban_reason: None,
            ban_expires: None,
            two_factor_enabled: None,
            user_code: Some("ada".into()),
            naming_scheme: None,
            naming_pattern: None,
            public_profile: None,
            used_quota: None,
            max_quota: None,
            hide_uploads: None,
            copy_raw: None,
            theme: None,
            locale: None,
            username: Some("ada".into()),
            skip_tag_in_url: None,
            allow_root_short_links: Some(false),
            default_folder_id: None,
            naming_word_count: None,
            naming_random_length: None,
            preferred_domain: None,
            default_password_hash: None,
            default_password_version: 0,
            background_key: None,
            background_dim: 0,
        }
    }

    /// A fake `nameTaken`: the lower-cased values another account holds.
    struct Taken(Vec<String>);

    impl NameLookup for Taken {
        fn taken<'b>(&'b self, _column: NameColumn, value: &'b str) -> BoxFuture<'b, sqlx::Result<bool>> {
            let hit = self.0.contains(&value.to_lowercase());
            Box::pin(async move { Ok(hit) })
        }
    }

    /// Phase 1 with the fake.
    fn plan(patch: Value, target: &User, is_self: bool, taken: &[&str]) -> Result<Plan<'static>, String> {
        let patch: &'static Value = Box::leak(Box::new(patch));
        let p = Patch::read(js(Some(patch))).map_err(|e| format!("{e:?}"))?;
        let names = Taken(taken.iter().map(|s| s.to_string()).collect());
        let fut = plan_update(&p, target, is_self, &names);
        futures_util::FutureExt::now_or_never(fut).expect("no real IO").map_err(|e| format!("{e:?}"))
    }

    fn issues(p: &Plan<'_>) -> Vec<(String, String)> {
        p.issues.iter().map(|i| (i.field.to_string(), i.reason.clone())).collect()
    }

    #[test]
    fn random_passwords_are_32_base64url_characters() {
        let a = random_password();
        let b = random_password();
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
        assert_ne!(a, b);
    }

    #[test]
    fn the_patch_must_not_be_nullish() {
        assert!(Patch::read(Js::Undefined).is_err());
        assert!(Patch::read(Js::Null).is_err());
        let text = json!("name");
        let p = Patch::read(js(Some(&text))).unwrap();
        assert!(matches!(p.name, Js::Undefined));
        let raw = json!({ "name": { "$undefined": 1 }, "role": "admin" });
        let p = Patch::read(js(Some(&raw))).unwrap();
        assert!(matches!(p.name, Js::Undefined));
        assert_eq!(raw_prop(p.raw, "role"), Some(&json!("admin")));
        assert_eq!(raw_prop(p.raw, "name"), None);
    }

    #[test]
    fn an_empty_patch_changes_nothing() {
        let t = user("u1");
        let p = plan(json!({}), &t, false, &[]).unwrap();
        assert!(p.columns.is_empty() && p.changed.is_empty() && p.issues.is_empty());
        assert!(!p.role_changed && !p.ban_changed && p.new_password.is_none());
    }

    #[test]
    fn names_are_trimmed_cut_and_required() {
        let t = user("u1");
        let p = plan(json!({ "name": "  Ada  " }), &t, false, &[]).unwrap();
        assert!(p.changed.is_empty());
        let p = plan(json!({ "name": format!(" {} ", "x".repeat(130)) }), &t, false, &[]).unwrap();
        assert_eq!(p.columns.name.as_deref().map(str::len), Some(120));
        assert_eq!(p.changed, vec![UserChange::Name]);
        let p = plan(json!({ "name": " \t " }), &t, false, &[]).unwrap();
        assert_eq!(issues(&p), vec![("name".into(), "empty".into())]);
        assert!(plan(json!({ "name": 5 }), &t, false, &[]).is_err());
        assert!(plan(json!({ "name": null }), &t, false, &[]).is_err());
    }

    #[test]
    fn usernames_follow_the_pattern_and_ignore_case_for_ownership() {
        let t = user("u1");
        let p = plan(json!({ "username": " ada " }), &t, false, &["ada"]).unwrap();
        assert!(p.changed.is_empty(), "unchanged: not even checked");
        let p = plan(json!({ "username": "Bob" }), &t, false, &["bob"]).unwrap();
        assert_eq!(issues(&p), vec![("username".into(), "taken".into())]);
        let p = plan(json!({ "username": "b" }), &t, false, &[]).unwrap();
        assert_eq!(issues(&p), vec![("username".into(), "charset".into())]);
        let p = plan(json!({ "username": "" }), &t, false, &[]).unwrap();
        assert_eq!(p.columns.username, Some(None));
        let p = plan(json!({ "username": null }), &t, false, &[]).unwrap();
        assert_eq!(p.columns.username, Some(None));
        assert!(plan(json!({ "username": 7 }), &t, false, &[]).is_err());
    }

    #[test]
    fn emails_are_lower_cased_and_unique() {
        let t = user("u1");
        let p = plan(json!({ "email": " ADA@example.TEST " }), &t, false, &[]).unwrap();
        assert!(p.changed.is_empty(), "same address in another case");
        let p = plan(json!({ "email": "New@Example.test" }), &t, false, &[]).unwrap();
        assert_eq!(p.columns.email.as_deref(), Some("new@example.test"));
        assert_eq!(p.previous_email.as_deref(), Some("Ada@Example.test"));
        let p = plan(json!({ "email": "b@x.test" }), &t, false, &["b@x.test"]).unwrap();
        assert_eq!(issues(&p), vec![("email".into(), "taken".into())]);
        for bad in ["nope", "a@b.c", &format!("{}@x.test", "a".repeat(250))] {
            let p = plan(json!({ "email": bad }), &t, false, &[]).unwrap();
            assert_eq!(issues(&p), vec![("email".into(), "invalid".into())], "{bad}");
        }
    }

    #[test]
    fn new_passwords_are_measured_like_javascript() {
        let t = user("u1");
        let p = plan(json!({ "newPassword": "" }), &t, false, &[]).unwrap();
        assert!(p.new_password.is_none());
        let p = plan(json!({ "newPassword": "short" }), &t, false, &[]).unwrap();
        assert_eq!(issues(&p), vec![("password".into(), "too-short".into())]);
        let p = plan(json!({ "newPassword": "x".repeat(129) }), &t, false, &[]).unwrap();
        assert_eq!(issues(&p), vec![("password".into(), "too-long".into())]);
        let p = plan(json!({ "newPassword": "long enough" }), &t, false, &[]).unwrap();
        assert!(p.issues.is_empty() && p.new_password.is_some());
        // A number has no length: no password at all.
        let p = plan(json!({ "newPassword": 12345678 }), &t, false, &[]).unwrap();
        assert!(p.new_password.is_none());
        // An array or an object with a length is a "password" (that later fails to hash).
        let p = plan(json!({ "newPassword": ["a"] }), &t, false, &[]).unwrap();
        assert_eq!(issues(&p), vec![("password".into(), "too-short".into())]);
        let p = plan(json!({ "newPassword": { "length": "10" } }), &t, false, &[]).unwrap();
        assert!(p.issues.is_empty() && p.new_password.is_some());
        let p = plan(json!({ "newPassword": { "length": "x" } }), &t, false, &[]).unwrap();
        assert!(p.issues.is_empty() && p.new_password.is_some());
    }

    #[test]
    fn quotas_must_be_real_numbers() {
        let t = user("u1");
        let p = plan(json!({ "maxQuota": -5 }), &t, false, &[]).unwrap();
        assert!(p.changed.is_empty(), "negative = unlimited = the stored NULL");
        let p = plan(json!({ "maxQuota": 1024.5 }), &t, false, &[]).unwrap();
        assert_eq!(p.columns.max_quota, Some(1025));
        for bad in [json!("1024"), json!(null), json!({ "$num": "Infinity" }), json!({ "$bigint": "5" }), json!(1e300)]
        {
            let p = plan(json!({ "maxQuota": bad }), &t, false, &[]).unwrap();
            assert_eq!(issues(&p), vec![("quota".into(), "invalid".into())], "{bad}");
        }
    }

    #[test]
    fn role_and_ban_changes_are_strict_comparisons() {
        let t = user("u1");
        let p = plan(json!({ "role": "user", "banned": false }), &t, false, &[]).unwrap();
        assert!(!p.role_changed && !p.ban_changed);
        let p = plan(json!({ "role": "admin", "banned": "yes" }), &t, false, &[]).unwrap();
        assert!(p.role_changed && p.ban_changed);
        let p = plan(json!({ "role": null, "banned": 0 }), &t, false, &[]).unwrap();
        assert!(p.role_changed && p.ban_changed);
        let p = plan(json!({ "role": "admin", "banned": true }), &t, true, &[]).unwrap();
        assert_eq!(issues(&p), vec![("role".into(), "self".into()), ("banned".into(), "self".into())]);
    }

    #[test]
    fn permission_flags_are_kept_raw_and_counted_once() {
        let t = user("u1");
        let p = plan(
            json!({ "allowRootShortLinks": false, "publicProfile": "maybe", "skipTagInUrl": null }),
            &t,
            false,
            &[],
        )
        .unwrap();
        assert!(p.columns.allow_root_short_links.is_none());
        assert_eq!(p.columns.public_profile, Some(Js::Str("maybe")));
        assert_eq!(p.columns.skip_tag_in_url, Some(Js::Null));
        assert_eq!(p.changed, vec![UserChange::Permissions]);
    }

    #[test]
    fn issues_and_changes_keep_the_upstream_order() {
        let t = user("u1");
        let p = plan(
            json!({ "role": "admin", "maxQuota": "x", "newPassword": "a", "email": "x", "username": "!", "name": "" }),
            &t,
            true,
            &[],
        )
        .unwrap();
        let fields: Vec<&str> = p.issues.iter().map(|i| i.field).collect();
        assert_eq!(fields, ["name", "username", "email", "password", "quota", "role"]);
        let p = plan(
            json!({ "publicProfile": true, "maxQuota": 1, "email": "z@x.test", "username": "zed", "name": "Zed" }),
            &t,
            false,
            &[],
        )
        .unwrap();
        assert_eq!(
            p.changed,
            vec![UserChange::Name, UserChange::Username, UserChange::Email, UserChange::Quota, UserChange::Permissions]
        );
        assert_eq!(
            serde_json::to_value(&p.changed).unwrap(),
            json!(["name", "username", "email", "quota", "permissions"])
        );
    }

    #[test]
    fn zod_names_the_values_json_cannot_carry() {
        assert_eq!(zod_body_value(&json!({ "$date": "2026-10-18T12:00:00.000Z" })).1, Some("Date"));
        assert_eq!(zod_body_value(&json!({ "$bigint": "5" })).1, Some("bigint"));
        assert_eq!(zod_body_value(&json!({ "$num": "Infinity" })), (json!(0), None));
        assert_eq!(zod_body_value(&json!(["x"])), (json!(["x"]), None));
    }

    #[test]
    fn lengths_follow_javascript() {
        assert_eq!(js_length(js(Some(&json!("\u{1F600}")))), Js::Num(2.0));
        assert_eq!(js_length(js(Some(&json!([1, 2])))), Js::Num(2.0));
        assert_eq!(js_length(js(Some(&json!(5)))), Js::Undefined);
        assert_eq!(js_length(js(Some(&json!({ "length": true })))), Js::Bool(true));
    }

    #[test]
    fn exports_are_registered() {
        let mut reg = RpcRegistry::default();
        register(&mut reg);
        for export in EXPORTS {
            assert!(reg.get("admin", export).is_some(), "{export}");
        }
        assert_eq!(reg.names().len(), EXPORTS.len());
    }
}
