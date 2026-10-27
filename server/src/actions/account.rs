//! Port of `app/actions/account.ts`: the settings of /dashboard/profile and
//! the per-account controls of Admin -> Users (CONTRACT 7.4, module
//! `account`).
//!
//! Every export is a public POST endpoint, so arguments are whatever a client
//! sent. Upstream did no runtime type checks beyond the ones written here; a
//! value of the wrong type ended in a TypeError, in drizzle's "No values to
//! set", or in Postgres refusing the text node-postgres sent for it. The [`js`]
//! helpers reproduce exactly those outcomes (see each action).
//!
//! Arguments arrive as tagged JSON (CONTRACT 3). React's `decodeReply` DROPS an
//! object key whose value is `undefined` (verified on the reference:
//! `updatePreferences({ locale: undefined })` reaches upstream as `{}`), so a
//! key holding the `$undefined` tag is treated as absent everywhere here.

use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use crate::admin_users::AdminUserRow;
use crate::auth::server::{revoke_other_sessions, revoke_session};
use crate::avatar::avatar_url;
use crate::background::{DimInput, background_url, clamp_dim, clamp_dim_i};
use crate::config::{Config, js_round, js_trim};
use crate::data::sessions::{js_slice, list_my_sessions, my_session_token};
use crate::data::user_prefs::{count_files_on_default_password, get_default_password_prefs};
use crate::db::types::JsDate;
use crate::http::set_cookie::{SameSite, SetCookie};
use crate::media::urls::{is_known_domain, user_base_url};
use crate::naming::{NAMING_SCHEMES, check_tag, clamp_random_length, clamp_word_count, is_naming_scheme};
use crate::quota::{UNLIMITED_QUOTA, is_safe_integer};
use crate::rpc::tagged::Num;
use crate::rpc::{ActionCtx, ActionError, ActionResult, RpcRegistry};
use crate::theme_cookies::{SKIN_COOKIE, THEME_COOKIE_MAX_AGE};
use crate::themes::{THEMES, resolve_theme};
use js::{Js, js};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("account", "updatePreferences", update_preferences);
    reg.register("account", "checkTagAvailability", check_tag_availability);
    reg.register("account", "updateTag", update_tag);
    reg.register("account", "updateSkipTagInUrl", update_skip_tag_in_url);
    reg.register("account", "setAllowRootShortLinks", set_allow_root_short_links);
    reg.register("account", "updateDefaultFolder", update_default_folder);
    reg.register("account", "myUrlPrefs", my_url_prefs);
    reg.register("account", "updateNaming", update_naming);
    reg.register("account", "updatePreferredDomain", update_preferred_domain);
    reg.register("account", "myDefaultFilePassword", my_default_file_password);
    reg.register("account", "updateDefaultFilePassword", update_default_file_password);
    reg.register("account", "myBackground", my_background);
    reg.register("account", "updateBackgroundDim", update_background_dim);
    reg.register("account", "updatePublicProfile", update_public_profile);
    reg.register("account", "listAdminUsers", list_admin_users);
    reg.register("account", "setUserQuota", set_user_quota);
    reg.register("account", "setUserTag", set_user_tag);
    reg.register("account", "recalculateUserQuota", recalculate_user_quota);
    reg.register("account", "revokeMySession", revoke_my_session);
    reg.register("account", "revokeMyOtherSessions", revoke_my_other_sessions);
}

/// Every export of the upstream module, in source order.
pub const EXPORTS: &[&str] = &[
    "updatePreferences",
    "checkTagAvailability",
    "updateTag",
    "updateSkipTagInUrl",
    "setAllowRootShortLinks",
    "updateDefaultFolder",
    "myUrlPrefs",
    "updateNaming",
    "updatePreferredDomain",
    "myDefaultFilePassword",
    "updateDefaultFilePassword",
    "myBackground",
    "updateBackgroundDim",
    "updatePublicProfile",
    "listAdminUsers",
    "setUserQuota",
    "setUserTag",
    "recalculateUserQuota",
    "revokeMySession",
    "revokeMyOtherSessions",
];

/// `ml_locale` lives as long as the skin cookie: a preference, not a session.
const LOCALE_COOKIE: &str = "ml_locale";
const LOCALE_COOKIE_MAX_AGE: i64 = 60 * 60 * 24 * 365;

/// Longest display name `updatePreferences` stores.
const NAME_MAX: usize = 120;

/// Longest custom naming pattern `updateNaming` stores.
const PATTERN_MAX: usize = 120;

/// Shortest default file password we accept.
const MIN_DEFAULT_PASSWORD: usize = 4;

/// A JavaScript `TypeError` (its message, as `next dev` shows it).
fn type_error(message: impl Into<String>) -> ActionError {
    ActionError::thrown(message)
}

/// `value.trim()` on an argument that must be a string.
fn trimmed_string<'a>(value: Js<'a>, name: &str) -> Result<&'a str, ActionError> {
    match value {
        Js::Str(s) => Ok(js_trim(s)),
        Js::Undefined => Err(type_error("Cannot read properties of undefined (reading 'trim')")),
        Js::Null => Err(type_error("Cannot read properties of null (reading 'trim')")),
        _ => Err(type_error(format!("{name}.trim is not a function"))),
    }
}

fn now() -> JsDate {
    JsDate::now()
}

// ---------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PreferencesResult {
    /// The skin the database holds AFTER the write, never the requested one.
    pub theme: &'static str,
    pub locale: Option<String>,
}

/// What `updatePreferences` writes: only the keys that passed its checks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PreferencesPatch {
    pub name: Option<String>,
    pub hide_uploads: Option<bool>,
    pub copy_raw: Option<bool>,
    pub theme: Option<&'static str>,
    /// `"locale" in input`: the value to store, `pl`/`en` or NULL.
    pub locale: Option<Option<&'static str>>,
}

impl PreferencesPatch {
    pub fn is_empty(&self) -> bool {
        *self == PreferencesPatch::default()
    }
}

/// The patch `updatePreferences` builds from its argument, with the
/// TypeErrors upstream hits on a non-object (`input.name` on null/undefined,
/// `"locale" in input` on a primitive).
pub fn preferences_patch(input: Js<'_>) -> Result<PreferencesPatch, ActionError> {
    match input {
        Js::Undefined => return Err(type_error("Cannot read properties of undefined (reading 'name')")),
        Js::Null => return Err(type_error("Cannot read properties of null (reading 'name')")),
        _ => {}
    }
    let mut patch = PreferencesPatch::default();
    if let Js::Str(name) = input.prop("name") {
        let t = js_trim(name);
        if !t.is_empty() {
            patch.name = Some(js_slice(t, NAME_MAX));
        }
    }
    if let Js::Bool(b) = input.prop("hideUploads") {
        patch.hide_uploads = Some(b);
    }
    if let Js::Bool(b) = input.prop("copyRaw") {
        patch.copy_raw = Some(b);
    }
    if let Js::Str(theme) = input.prop("theme") {
        patch.theme = THEMES.iter().find(|t| t.id == theme).map(|t| t.id);
    }
    let Some(has_locale) = input.has("locale") else {
        return Err(type_error("Cannot use 'in' operator to search for 'locale' in input"));
    };
    if has_locale {
        patch.locale = Some(match input.prop("locale") {
            Js::Str("pl") => Some("pl"),
            Js::Str("en") => Some("en"),
            _ => None,
        });
    }
    Ok(patch)
}

/// The members of `Object.prototype`.
const OBJECT_PROTOTYPE_KEYS: &[&str] = &[
    "constructor",
    "__defineGetter__",
    "__defineSetter__",
    "hasOwnProperty",
    "__lookupGetter__",
    "__lookupSetter__",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toString",
    "valueOf",
    "__proto__",
    "toLocaleString",
];

/// `resolveTheme(v)` as a value this action RETURNS. Upstream tests
/// `v in THEME_ALIASES`, which is also true for the members of
/// `Object.prototype`; `THEME_ALIASES[v]` is then a function (or
/// `Object.prototype` itself), which React refuses to serialise, so the action
/// fails after its side effects ran. Only a value written outside this module
/// can get there (Better Auth's `update-user` accepts any `theme` string).
pub fn resolve_theme_result(v: Option<&str>) -> Result<&'static str, ActionError> {
    match v {
        Some(v) if OBJECT_PROTOTYPE_KEYS.contains(&v) && !crate::themes::is_theme_id(v) => Err(ActionError::thrown(
            "Functions cannot be passed directly to Client Components unless you explicitly expose it by marking it with \"use server\".",
        )),
        _ => Ok(resolve_theme(v)),
    }
}

/// Persist the account preferences edited on /dashboard/profile.
///
/// Returns what the row actually contains afterwards. The skin in particular
/// has to be read back rather than assumed: the caller repaints the entire
/// document from it, and an UPDATE that matched no row is indistinguishable
/// from a successful save until the next reload puts the old skin back, which
/// is exactly how "the theme picker does nothing" presented.
async fn update_preferences(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let patch = preferences_patch(js(ctx.args.raw(0)))?;

    if patch.is_empty() {
        let theme = resolve_theme_result(session.user.theme.as_deref())?;
        return crate::rpc::ok(&PreferencesResult { theme, locale: session.user.locale.clone() });
    }

    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(r#"UPDATE "user" SET "#);
    {
        let mut set = qb.separated(", ");
        if let Some(name) = &patch.name {
            set.push(r#""name" = "#).push_bind_unseparated(name.clone());
        }
        if let Some(v) = patch.hide_uploads {
            set.push(r#""hide_uploads" = "#).push_bind_unseparated(v);
        }
        if let Some(v) = patch.copy_raw {
            set.push(r#""copy_raw" = "#).push_bind_unseparated(v);
        }
        if let Some(theme) = patch.theme {
            set.push(r#""theme" = "#).push_bind_unseparated(theme);
        }
        if let Some(locale) = patch.locale {
            set.push(r#""locale" = "#).push_bind_unseparated(locale);
        }
        // drizzle's `$onUpdate` stamps every UPDATE of the user row.
        set.push(r#""updated_at" = "#).push_bind_unseparated(now());
    }
    qb.push(r#" WHERE "id" = "#).push_bind(session.user.id.clone()).push(r#" RETURNING "theme", "locale""#);
    let row: Option<(Option<String>, Option<String>)> = qb.build_query_as().fetch_optional(&ctx.state.db).await?;

    let Some((row_theme, row_locale)) = row else {
        return Err(ActionError::thrown("Preferences could not be saved"));
    };

    let theme = resolve_theme_result(row_theme.as_deref());

    // The root layout paints <html data-theme> from `ml_skin`, so that cookie
    // is what decides the FIRST frame of every later request. Writing it here,
    // on the server, in the same action as the database write, is what makes a
    // skin change survive a reload.
    if patch.theme.is_some()
        && let Ok(theme) = theme
    {
        ctx.set_cookie(
            SetCookie::new(SKIN_COOKIE, theme).path("/").max_age(THEME_COOKIE_MAX_AGE).same_site(SameSite::Lax),
        );
    }

    // Reflect the language choice into the `ml_locale` cookie so the next
    // render (and SSR locale detection) uses it immediately. null = auto ->
    // clear cookie.
    if let Some(locale) = patch.locale {
        match locale {
            Some(l) => ctx.set_cookie(
                SetCookie::new(LOCALE_COOKIE, l).path("/").max_age(LOCALE_COOKIE_MAX_AGE).same_site(SameSite::Lax),
            ),
            None => ctx.delete_cookie(LOCALE_COOKIE),
        }
    }

    // <html> belongs to the ROOT layout: revalidating only the profile page
    // would leave the document element rendering the skin it had before.
    ctx.revalidate_path("/");

    crate::rpc::ok(&PreferencesResult { theme: theme?, locale: row_locale })
}

// ---------------------------------------------------------------------------
// Tags
// ---------------------------------------------------------------------------

/// Why a tag cannot be used. The client maps these to translated copy (Next
/// masks the message of an error thrown from a Server Action in production,
/// so anything the user must read travels as data).
///
/// `checkTag`'s reasons, plus `taken`, `retired` and `error`.
fn tag_refused(reason: &str) -> ActionResult {
    Ok(json!({ "ok": false, "reason": reason }))
}

async fn tag_owner(conn: &mut PgConnection, tag: &str) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar(r#"SELECT "id" FROM "user" WHERE "user_code" = $1 LIMIT 1"#).bind(tag).fetch_optional(conn).await
}

async fn alias_owner(conn: &mut PgConnection, tag: &str) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar(r#"SELECT "user_id" FROM "user_code_aliases" WHERE "code" = $1 LIMIT 1"#)
        .bind(tag)
        .fetch_optional(conn)
        .await
}

/// Is `tag` claimed by somebody other than `user_id`, either as their current
/// tag or as one they retired (aliases keep resolving forever, so handing one
/// over would redirect their old links into this account's namespace)?
///
/// `user_id` is the id as JavaScript compares it with `!==`: `None` when the
/// caller passed something that is not a string, which equals no row.
async fn tag_conflict(conn: &mut PgConnection, tag: &str, user_id: Option<&str>) -> sqlx::Result<Option<&'static str>> {
    if let Some(owner) = tag_owner(conn, tag).await?
        && Some(owner.as_str()) != user_id
    {
        return Ok(Some("taken"));
    }
    if let Some(alias) = alias_owner(conn, tag).await?
        && Some(alias.as_str()) != user_id
    {
        return Ok(Some("retired"));
    }
    Ok(None)
}

/// Live availability check for the tag field. Read-only and cheap: two
/// indexed lookups. Runs on every debounced keystroke, so it never mutates
/// anything.
async fn check_tag_availability(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let clean = trimmed_string(js(ctx.args.raw(0)), "tag")?;

    if let Err(problem) = check_tag(clean) {
        return Ok(json!({ "status": "unavailable", "reason": problem.as_str() }));
    }

    let current = session.user.user_code.as_deref().filter(|c| !c.is_empty());
    if current == Some(clean) {
        return Ok(json!({ "status": "current" }));
    }

    let mut conn = ctx.state.db.acquire().await?;
    Ok(match tag_conflict(&mut conn, clean, Some(&session.user.id)).await? {
        Some(reason) => json!({ "status": "unavailable", "reason": reason }),
        None => json!({ "status": "ok" }),
    })
}

/// How the tag transaction ended.
enum Claim {
    Done,
    /// Taken by another account, or retired by one: both are hard stops.
    Conflict(&'static str),
    /// Anything else, e.g. a unique-index race: someone claimed the tag
    /// between the check and the write.
    Failed,
}

/// The transaction of `updateTag` / `setUserTag`: refuse a tag somebody else
/// holds or retired, park the outgoing tag in `user_code_aliases` (so links
/// already in circulation keep resolving), take the new one.
///
/// `target` is the id as sent to Postgres, `target_js` as JavaScript compares
/// it (see [`tag_conflict`]).
async fn claim_tag(
    db: &PgPool,
    target: Option<&str>,
    target_js: Option<&str>,
    current: Option<&str>,
    clean: &str,
) -> Claim {
    let attempt = async {
        let mut tx = db.begin().await?;
        if let Some(reason) = tag_conflict(&mut tx, clean, target_js).await? {
            // Dropping the transaction rolls it back.
            return Ok::<_, sqlx::Error>(Claim::Conflict(reason));
        }
        if let Some(current) = current {
            sqlx::query(
                r#"INSERT INTO "user_code_aliases" ("code", "user_id") VALUES ($1, $2) ON CONFLICT DO NOTHING"#,
            )
            .bind(current)
            .bind(target)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(r#"UPDATE "user" SET "user_code" = $1, "updated_at" = $2 WHERE "id" = $3"#)
            .bind(clean)
            .bind(now())
            .bind(target)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Claim::Done)
    };
    match attempt.await {
        Ok(claim) => claim,
        Err(err) => {
            tracing::warn!(error = ?err, "tag change failed");
            Claim::Failed
        }
    }
}

/// Change the user's tag, the first segment of every public URL.
///
/// The previous tag is parked in `user_code_aliases` and keeps resolving, so
/// links already shared never break. A tag can therefore never be reused by
/// anyone else, even after being abandoned.
async fn update_tag(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let clean = trimmed_string(js(ctx.args.raw(0)), "tag")?;

    if let Err(problem) = check_tag(clean) {
        return tag_refused(problem.as_str());
    }

    let current = session.user.user_code.as_deref().filter(|c| !c.is_empty());
    if current == Some(clean) {
        return Ok(json!({ "ok": true, "tag": clean }));
    }

    let me = session.user.id.as_str();
    match claim_tag(&ctx.state.db, Some(me), Some(me), current, clean).await {
        Claim::Done => {}
        Claim::Conflict(reason) => return tag_refused(reason),
        Claim::Failed => return tag_refused("error"),
    }

    ctx.revalidate_path("/dashboard/profile");
    ctx.revalidate_path("/dashboard");
    Ok(json!({ "ok": true, "tag": clean }))
}

/// `setUserTag(userId, tag)` once `requireAdmin()` passed, as
/// `admin.updateUserAccount` calls it too.
///
/// Same rules as the self-service version in the profile: validated charset,
/// reserved words refused, and the outgoing tag parked in `user_code_aliases`
/// so the links already in circulation keep resolving. An admin gets no
/// shortcut here; handing over a retired tag would silently redirect someone
/// else's old links into this account's namespace.
pub async fn set_user_tag_as_admin(ctx: &ActionCtx, user_id: Option<&Value>, tag: Option<&Value>) -> ActionResult {
    let user_id = js(user_id);
    let clean = trimmed_string(js(tag), "tag")?;

    if let Err(problem) = check_tag(clean) {
        return tag_refused(problem.as_str());
    }

    let target_sql = js::pg_text(user_id);
    let target: Option<(String, Option<String>)> =
        sqlx::query_as(r#"SELECT "id", "user_code" FROM "user" WHERE "id" = $1 LIMIT 1"#)
            .bind(target_sql.as_deref())
            .fetch_optional(&ctx.state.db)
            .await?;
    let Some((_, current)) = target else {
        return tag_refused("error");
    };

    let current = current.as_deref().filter(|c| !c.is_empty());
    if current == Some(clean) {
        return Ok(json!({ "ok": true, "tag": clean }));
    }

    let target_js = match user_id {
        Js::Str(s) => Some(s),
        _ => None,
    };
    match claim_tag(&ctx.state.db, target_sql.as_deref(), target_js, current, clean).await {
        Claim::Done => {}
        Claim::Conflict(reason) => return tag_refused(reason),
        Claim::Failed => return tag_refused("error"),
    }

    ctx.revalidate_path("/dashboard/admin");
    ctx.revalidate_path("/dashboard");
    Ok(json!({ "ok": true, "tag": clean }))
}

/// Change another account's tag (Admin -> Users).
async fn set_user_tag(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    set_user_tag_as_admin(&ctx, ctx.args.raw(0), ctx.args.raw(1)).await
}

// ---------------------------------------------------------------------------
// Flags written straight into boolean columns
// ---------------------------------------------------------------------------

/// Admin-only: generate links for NEW uploads without the tag segment
/// (https://host/name.png instead of https://host/tag/name.png).
///
/// The role is checked HERE, on the server, not just hidden in the UI: a
/// tagless code occupies the root namespace shared with every top-level route,
/// so it is not something an arbitrary account may claim. Files already
/// uploaded keep the tagged URL they were shared with; both forms resolve.
async fn update_skip_tag_in_url(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_admin().await?;
    let enabled = js::pg_boolean(js(ctx.args.raw(0)))?;
    sqlx::query(r#"UPDATE "user" SET "skip_tag_in_url" = $1, "updated_at" = $2 WHERE "id" = $3"#)
        .bind(enabled)
        .bind(now())
        .bind(&session.user.id)
        .execute(&ctx.state.db)
        .await?;
    ctx.revalidate_path("/dashboard/profile");
    ctx.revalidate_path("/dashboard");
    crate::rpc::void()
}

/// Admin-only: let a specific user create short links at the root of the
/// shortener domain (no /{userCode}/ prefix). No existence check: an unknown
/// id is a silent no-op.
async fn set_allow_root_short_links(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    let user_id = js::pg_text(js(ctx.args.raw(0)));
    let allowed = js::pg_boolean(js(ctx.args.raw(1)))?;
    sqlx::query(r#"UPDATE "user" SET "allow_root_short_links" = $1, "updated_at" = $2 WHERE "id" = $3"#)
        .bind(allowed)
        .bind(now())
        .bind(user_id)
        .execute(&ctx.state.db)
        .await?;
    ctx.revalidate_path("/dashboard/admin");
    crate::rpc::void()
}

/// Opt in/out of the public profile gallery at /u/{tag}.
async fn update_public_profile(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let enabled = js::pg_boolean(js(ctx.args.raw(0)))?;
    sqlx::query(r#"UPDATE "user" SET "public_profile" = $1, "updated_at" = $2 WHERE "id" = $3"#)
        .bind(enabled)
        .bind(now())
        .bind(&session.user.id)
        .execute(&ctx.state.db)
        .await?;
    ctx.revalidate_path("/dashboard/profile");
    crate::rpc::void()
}

/// Folder that new uploads land in. null clears it.
///
/// Ownership is verified server-side: the select is populated from the user's
/// own folders, but a hand-crafted call must not be able to point their
/// uploads at somebody else's tree.
async fn update_default_folder(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let folder = js(ctx.args.raw(0));

    if folder.truthy() {
        let id = js::pg_uuid(folder)?;
        let owned: Option<Uuid> =
            sqlx::query_scalar(r#"SELECT "id" FROM "folders" WHERE "id" = $1 AND "owner_id" = $2 LIMIT 1"#)
                .bind(id)
                .bind(&session.user.id)
                .fetch_optional(&ctx.state.db)
                .await?;
        if owned.is_none() {
            return Ok(json!({ "ok": false }));
        }
    }

    // `.set({ defaultFolderId: folderId })`: undefined is "No values to set",
    // null clears, anything else must parse as a uuid.
    let value = match folder {
        Js::Undefined => return Err(js::no_values_to_set()),
        Js::Null => None,
        other => Some(js::pg_uuid(other)?),
    };
    sqlx::query(r#"UPDATE "user" SET "default_folder_id" = $1, "updated_at" = $2 WHERE "id" = $3"#)
        .bind(value)
        .bind(now())
        .bind(&session.user.id)
        .execute(&ctx.state.db)
        .await?;
    ctx.revalidate_path("/dashboard/profile");
    Ok(json!({ "ok": true }))
}

// ---------------------------------------------------------------------------
// Naming and domain
// ---------------------------------------------------------------------------

/// `NamingPrefs` + `UrlPrefs`, the URL/naming preferences of the profile page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlPrefs {
    pub scheme: &'static str,
    pub pattern: Option<String>,
    pub word_count: i32,
    pub random_length: i32,
    /// Stored preference, or None when the account follows the default.
    pub preferred_domain: Option<String>,
    /// The origin actually used to build this user's links right now.
    pub base_url: String,
}

type NamingRow = (Option<String>, Option<String>, Option<i32>, Option<i32>, Option<String>);

/// URL/naming preferences of `user_id` (what `myUrlPrefs()` returns for the
/// caller; the profile and tokens pages call it).
///
/// These columns are not part of Better Auth's session payload, so they are
/// read explicitly.
pub async fn url_prefs(db: &PgPool, config: &Config, user_id: &str) -> sqlx::Result<UrlPrefs> {
    let row: Option<NamingRow> = sqlx::query_as(
        r#"SELECT "naming_scheme", "naming_pattern", "naming_word_count", "naming_random_length", "preferred_domain"
           FROM "user" WHERE "id" = $1 LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    let (scheme, pattern, word_count, random_length, preferred_domain) = row.unwrap_or_default();
    let scheme = NAMING_SCHEMES.iter().find(|s| scheme.as_deref() == Some(**s)).copied().unwrap_or("random");
    Ok(UrlPrefs {
        scheme,
        pattern,
        word_count: clamp_word_count(word_count.map(f64::from)),
        random_length: clamp_random_length(random_length.map(f64::from)),
        base_url: user_base_url(config, preferred_domain.as_deref()),
        preferred_domain,
    })
}

/// URL/naming preferences for the profile page.
async fn my_url_prefs(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    crate::rpc::ok(&url_prefs(&ctx.state.db, &ctx.state.config, &session.user.id).await?)
}

/// `clampWordCount(v)` / `clampRandomLength(v)` take any value: `null` and
/// `undefined` fall back, everything else goes through `Number()`.
fn clamp_input(v: Js<'_>) -> Option<f64> {
    match v {
        Js::Undefined | Js::Null => None,
        other => Some(other.to_number()),
    }
}

/// Choose how NEW uploads are named. Existing files keep the code they were
/// created with, so changing this is always safe for links already shared.
///
/// The two numeric parameters are stored regardless of the active scheme, so
/// flipping between "words" and "random characters" doesn't lose the length
/// the user configured for the other one.
async fn update_naming(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let input = js(ctx.args.raw(0));
    // `const { scheme } = input`
    match input {
        Js::Undefined => {
            return Err(type_error("Cannot destructure property 'scheme' of 'input' as it is undefined."));
        }
        Js::Null => return Err(type_error("Cannot destructure property 'scheme' of 'input' as it is null.")),
        _ => {}
    }
    let scheme = match input.prop("scheme") {
        Js::Str(s) if is_naming_scheme(Some(s)) => s,
        _ => return Err(ActionError::thrown("Unknown naming scheme")),
    };

    let pattern = match input.prop("pattern") {
        Js::Undefined | Js::Null => "",
        Js::Str(s) => s,
        _ => return Err(type_error("(input.pattern ?? \"\").trim is not a function")),
    };
    let clean = Some(js_slice(js_trim(pattern), PATTERN_MAX)).filter(|p| !p.is_empty());
    if scheme == "custom" && clean.is_none() {
        return Err(ActionError::thrown("A custom pattern is required for the custom scheme"));
    }
    // Upstream also rejects a custom pattern that "expands to nothing usable"
    // by probing `buildCodeBase({ scheme: "custom", pattern, filename })`. That
    // check cannot fire: `buildCodeBase` answers `expanded || humanCode(10)`,
    // and `humanCode(10)` is always ten characters (the expansion itself never
    // throws: every sized token is clamped), so the probe is not ported.

    sqlx::query(
        r#"UPDATE "user" SET "naming_scheme" = $1, "naming_pattern" = $2, "naming_word_count" = $3,
                             "naming_random_length" = $4, "updated_at" = $5
           WHERE "id" = $6"#,
    )
    .bind(scheme)
    .bind(if scheme == "custom" { clean } else { None })
    .bind(clamp_word_count(clamp_input(input.prop("wordCount"))))
    .bind(clamp_random_length(clamp_input(input.prop("randomLength"))))
    .bind(now())
    .bind(&session.user.id)
    .execute(&ctx.state.db)
    .await?;

    ctx.revalidate_path("/dashboard/profile");
    ctx.revalidate_path("/dashboard");
    crate::rpc::void()
}

/// Pick which of the deployment's hostnames this account's links are built
/// from.
///
/// `null` restores the default. The value is validated against
/// `config.appDomains` here as well as when it is read back: it is echoed into
/// ShareX configs, Open Graph tags and copied links, so an origin the operator
/// never configured must never make it out of this function.
///
/// Nothing already shared is affected: every configured name is served by the
/// same application, indefinitely.
async fn update_preferred_domain(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    // `domain?.trim().replace(/\/+$/, "") || null`
    let clean = match js(ctx.args.raw(0)) {
        Js::Undefined | Js::Null => None,
        Js::Str(s) => Some(js_trim(s).trim_end_matches('/')).filter(|c| !c.is_empty()),
        _ => return Err(type_error("domain?.trim is not a function")),
    };
    let config = &ctx.state.config;

    if let Some(c) = clean
        && !is_known_domain(config, c)
    {
        return Ok(json!({ "ok": false, "baseUrl": user_base_url(config, None) }));
    }

    sqlx::query(r#"UPDATE "user" SET "preferred_domain" = $1, "updated_at" = $2 WHERE "id" = $3"#)
        .bind(clean)
        .bind(now())
        .bind(&session.user.id)
        .execute(&ctx.state.db)
        .await?;

    // The dashboard renders copy-links from this, and the root layout is where
    // the tokens page picks it up: revalidate broadly rather than one route.
    ctx.revalidate_path("/");

    Ok(json!({ "ok": true, "baseUrl": user_base_url(config, clean) }))
}

// ---------------------------------------------------------------------------
// Default file password
// ---------------------------------------------------------------------------

/// Current state of the setting, for the profile form. Never returns the hash.
async fn my_default_file_password(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let db = &ctx.state.db;
    let (prefs, affected) = tokio::try_join!(
        get_default_password_prefs(db, &session.user.id),
        count_files_on_default_password(db, &session.user.id)
    )?;
    Ok(json!({ "enabled": prefs.enabled, "affected": affected }))
}

/// Set, replace or clear the password stamped on this account's NEW uploads.
/// `null` (or an empty string) turns the feature off.
///
/// Every one of those is a change, so every one of them bumps
/// `default_password_version`. That counter is signed into the "remember for
/// 30 days" cookies handed to recipients, so bumping it is what withdraws
/// their remembered consent: nothing uploaded from here on will match a cookie
/// minted before the change.
///
/// Files already uploaded are left alone unless `applyToExisting` says
/// otherwise. With it, every file stamped with a PREVIOUS generation of the
/// default is rewritten to the new one: same links, new password. A
/// hand-picked password stores `password_version = null` and a file without a
/// password has none, so neither is in the set; the rewritten rows take the NEW
/// generation number, so every remembered consent stops matching them.
///
/// Both writes share one transaction: a bumped counter with the uploads left
/// on the old hash would leave files claiming a generation whose password
/// nobody can produce.
async fn update_default_file_password(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let clean = match js(ctx.args.raw(0)) {
        Js::Str(s) => s,
        _ => "",
    };
    // `applyToExisting = false` (an absent or undefined argument).
    let apply_to_existing = js(ctx.args.raw(1)).truthy();

    let length = clean.encode_utf16().count();
    if length > 0 && length < MIN_DEFAULT_PASSWORD {
        return Ok(json!({ "ok": false, "reason": "too-short" }));
    }

    let hash = if length > 0 { Some(crate::password::hash_password(clean).await?) } else { None };

    let mut tx = ctx.state.db.begin().await?;
    let version: Option<i32> = sqlx::query_scalar(
        r#"UPDATE "user" SET "default_password_hash" = $1,
                             "default_password_version" = coalesce("default_password_version", 0) + 1,
                             "updated_at" = $2
           WHERE "id" = $3 RETURNING "default_password_version""#,
    )
    .bind(hash.as_deref())
    .bind(now())
    .bind(&session.user.id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(version) = version else {
        return Err(ActionError::thrown("Account not found"));
    };

    let mut updated = 0usize;
    if apply_to_existing {
        // Clearing the default un-protects the files that were on it: "stop
        // password-protecting my uploads, including the ones already out
        // there" is the only coherent reading of the switch plus the checkbox.
        let rewritten: Vec<Uuid> = sqlx::query_scalar(
            r#"UPDATE "uploads" SET "password_hash" = $1, "password_version" = $2
               WHERE "owner_id" = $3 AND "password_version" IS NOT NULL RETURNING "id""#,
        )
        .bind(hash.as_deref())
        .bind(hash.as_ref().map(|_| version))
        .bind(&session.user.id)
        .fetch_all(&mut *tx)
        .await?;
        updated = rewritten.len();
    }
    tx.commit().await?;

    ctx.revalidate_path("/dashboard/profile");
    ctx.revalidate_path("/dashboard");
    Ok(json!({ "ok": true, "enabled": hash.is_some(), "updated": updated }))
}

// ---------------------------------------------------------------------------
// Wallpaper
// ---------------------------------------------------------------------------

/// Current wallpaper state, for the profile form.
async fn my_background(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let row: Option<(Option<String>, Option<String>, Option<i32>)> =
        sqlx::query_as(r#"SELECT "user_code", "background_key", "background_dim" FROM "user" WHERE "id" = $1 LIMIT 1"#)
            .bind(&session.user.id)
            .fetch_optional(&ctx.state.db)
            .await?;
    let (user_code, key, dim) = row.unwrap_or_default();
    Ok(json!({
        "url": background_url(user_code.as_deref(), key.as_deref()),
        "dim": clamp_dim_i(dim),
    }))
}

/// `clampDim(value)` for whatever the client sent.
pub fn dim_input(v: Js<'_>) -> DimInput<'_> {
    match v {
        Js::Undefined | Js::Null => DimInput::Absent,
        Js::Str(s) => DimInput::Text(s),
        Js::Num(n) => DimInput::Number(n),
        other => DimInput::Other(other.to_number()),
    }
}

/// How strongly the skin's own background colour veils the wallpaper.
///
/// The image itself is written by app/api/background/route.ts (a route
/// handler, because Server Actions cap the request body at 1 MB); this is just
/// a number, so it belongs here with the rest of the preferences.
async fn update_background_dim(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let value = clamp_dim(dim_input(js(ctx.args.raw(0))));
    sqlx::query(r#"UPDATE "user" SET "background_dim" = $1, "updated_at" = $2 WHERE "id" = $3"#)
        .bind(value)
        .bind(now())
        .bind(&session.user.id)
        .execute(&ctx.state.db)
        .await?;
    // <html> carries the wallpaper variables, and those are emitted by the
    // ROOT layout: revalidating only the profile page would leave the old veil.
    ctx.revalidate_path("/");
    Ok(json!({ "dim": value }))
}

// ---------------------------------------------------------------------------
// Admin -> Users
//
// Every action below re-checks the admin role on the server. The panel hides
// these controls from non-admins, but a Server Action is a public POST
// endpoint: the UI is not the authorisation boundary.
// ---------------------------------------------------------------------------

type AdminUserSql = (
    String,
    String,
    String,
    bool,
    Option<String>,
    Option<bool>,
    JsDate,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<bool>,
);

/// The user table for Admin -> Users, joined with the two numbers an operator
/// actually needs: how much disk the account occupies and how many live files
/// it holds. The file counts come from one grouped scan rather than a
/// correlated subquery per row.
pub async fn admin_user_rows(db: &PgPool) -> anyhow::Result<Vec<AdminUserRow>> {
    let users = async {
        sqlx::query_as::<_, AdminUserSql>(
            r#"SELECT "id", "name", "email", "email_verified", "role", "banned", "created_at", "user_code", "image",
                      "used_quota", "max_quota", "allow_root_short_links"
               FROM "user" ORDER BY "created_at" ASC"#,
        )
        .fetch_all(db)
        .await
    };
    let counts = async {
        sqlx::query_as::<_, (String, i32)>(
            r#"SELECT "owner_id", count(*)::int FROM "uploads" WHERE "deleted_at" IS NULL GROUP BY "owner_id""#,
        )
        .fetch_all(db)
        .await
    };
    // One query for the whole table rather than one per row: the badge is
    // cosmetic and must not cost a round trip per account.
    let invited = crate::invites::pending_invite_user_ids(db);
    // Not upstream: which accounts encrypt their files (docs/ENCRYPTION.md).
    let vaults = async { crate::vault::store::all_vaults(db).await };
    let (users, counts, invited, vaults) = tokio::try_join!(users, counts, invited, vaults)?;
    let vaults: std::collections::HashMap<String, String> = vaults.into_iter().collect();

    let by_owner: std::collections::HashMap<String, i64> =
        counts.into_iter().map(|(owner, n)| (owner, i64::from(n))).collect();
    Ok(users
        .into_iter()
        .map(|(id, name, email, email_verified, role, banned, created_at, tag, image, used, max, allow_root)| {
            AdminUserRow {
                avatar: avatar_url(tag.as_deref(), image.as_deref()),
                files: by_owner.get(&id).copied().unwrap_or(0),
                invite_pending: invited.contains(&id),
                encryption: vaults.get(&id).cloned(),
                id,
                name,
                email,
                role,
                banned: banned.unwrap_or(false),
                created_at: Some(created_at.to_iso_string()),
                tag,
                used_quota: used.unwrap_or(0),
                max_quota: max.unwrap_or(UNLIMITED_QUOTA),
                allow_root_short_links: allow_root.unwrap_or(false),
                email_verified,
            }
        })
        .collect())
}

async fn list_admin_users(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    crate::rpc::ok(&admin_user_rows(&ctx.state.db).await?)
}

/// Set an account's disk limit, in bytes. `-1` means unlimited.
///
/// The value lands in a bigint column: real installs run past int4's 2.14 GB
/// ceiling immediately, so nothing on this path may narrow to a 32-bit int.
async fn set_user_quota(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    // `!Number.isFinite(bytes) || !Number.isSafeInteger(Math.round(bytes))`:
    // no coercion, so a numeric string is refused too.
    let bytes = match js(ctx.args.raw(1)) {
        Js::Num(n) if n.is_finite() && is_safe_integer(js_round(n)) => n,
        _ => return Ok(json!({ "ok": false })),
    };
    let value = if bytes < 0.0 { UNLIMITED_QUOTA } else { js_round(bytes) as i64 };
    sqlx::query(r#"UPDATE "user" SET "max_quota" = $1, "updated_at" = $2 WHERE "id" = $3"#)
        .bind(value)
        .bind(now())
        .bind(js::pg_text(js(ctx.args.raw(0))))
        .execute(&ctx.state.db)
        .await?;
    ctx.revalidate_path("/dashboard/admin");
    Ok(json!({ "ok": true }))
}

/// Recompute `used_quota` from the files that actually exist.
///
/// The counter is maintained incrementally on upload and on permanent delete,
/// so it drifts if a row is ever removed out-of-band. Trashed files are
/// counted deliberately: they still occupy disk until they are purged.
async fn recalculate_user_quota(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    let user_id = js::pg_text(js(ctx.args.raw(0)));
    // sum(bigint) is numeric; upstream read it as a string and took `Number()`.
    let total: String =
        sqlx::query_scalar(r#"SELECT coalesce(sum("size"), 0)::text FROM "uploads" WHERE "owner_id" = $1"#)
            .bind(user_id.as_deref())
            .fetch_one(&ctx.state.db)
            .await?;
    let used = crate::config::js_number(&total);
    sqlx::query(r#"UPDATE "user" SET "used_quota" = $1, "updated_at" = $2 WHERE "id" = $3"#)
        .bind(used as i64)
        .bind(now())
        .bind(user_id.as_deref())
        .execute(&ctx.state.db)
        .await?;
    ctx.revalidate_path("/dashboard/admin");
    crate::rpc::ok(&json!({ "used": Num(used) }))
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// Sign out ONE of the caller's own sessions.
///
/// Two things are load-bearing here. The id is resolved to a token through
/// `my_session_token`, which only ever looks inside the caller's own list, so
/// this cannot reach another account's session (an administrator included).
/// And the revocation itself goes through Better Auth's operation rather than
/// a DELETE of our own.
///
/// The current session is refused rather than revoked. Signing yourself out
/// from a list where every row looks alike is the one mistake this screen can
/// cause, and "Sign out" in the account menu is right there for when it is
/// meant.
async fn revoke_my_session(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let id = match js(ctx.args.raw(0)) {
        Js::Str(s) if !js_trim(s).is_empty() => s,
        _ => return Ok(json!({ "outcome": "gone" })),
    };
    if id == session.session.id {
        return Ok(json!({ "outcome": "current" }));
    }

    let Some(token) = my_session_token(&ctx.state.db, &session, id).await? else {
        return Ok(json!({ "outcome": "gone" }));
    };

    if let Err(err) = revoke_session(&ctx.state.db, &session, &token).await {
        tracing::warn!(error = ?err, "revokeSession failed");
        return Ok(json!({ "outcome": "error" }));
    }

    ctx.revalidate_path("/dashboard/profile");
    Ok(json!({ "outcome": "ok" }))
}

/// Sign out every session except this one.
///
/// The count is taken BEFORE the call because Better Auth answers with a bare
/// `{ status: true }`: without this the panel could only say "done", and "3
/// other devices signed out" is the part that tells the owner whether the
/// number matched what they expected.
async fn revoke_my_other_sessions(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;

    let others = list_my_sessions(&ctx.state.db, &session).await?.iter().filter(|s| !s.current).count();
    if others == 0 {
        return Ok(json!({ "ok": true, "revoked": 0 }));
    }

    if let Err(err) = revoke_other_sessions(&ctx.state.db, &session).await {
        tracing::warn!(error = ?err, "revokeOtherSessions failed");
        return Ok(json!({ "ok": false, "revoked": 0 }));
    }

    ctx.revalidate_path("/dashboard/profile");
    Ok(json!({ "ok": true, "revoked": others }))
}

/// JavaScript values as the actions receive them, and what node-postgres and
/// Postgres made of them.
pub mod js {
    use serde_json::{Map, Value};
    use uuid::Uuid;

    use crate::config::js_number;
    use crate::i18n::format::js_number_to_string;
    use crate::rpc::ActionError;
    use crate::rpc::tagged;

    /// One argument (or property) after React's `decodeReply`.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum Js<'a> {
        Undefined,
        Null,
        Bool(bool),
        Num(f64),
        /// The decimal digits.
        BigInt(&'a str),
        Str(&'a str),
        /// The ISO string.
        Date(&'a str),
        Array(&'a [Value]),
        Object(&'a Map<String, Value>),
    }

    /// Read a tagged value (`None` = an argument that was not passed).
    pub fn js(v: Option<&Value>) -> Js<'_> {
        let Some(v) = v else {
            return Js::Undefined;
        };
        match v {
            Value::Null => Js::Null,
            Value::Bool(b) => Js::Bool(*b),
            Value::Number(n) => Js::Num(n.as_f64().unwrap_or(f64::NAN)),
            Value::String(s) => Js::Str(s),
            Value::Array(items) => Js::Array(items),
            Value::Object(map) => {
                if map.len() == 1 {
                    if tagged::is_undefined(v) {
                        return Js::Undefined;
                    }
                    if let Some(Value::String(iso)) = map.get("$date") {
                        return Js::Date(iso);
                    }
                    if let Some(Value::String(digits)) = map.get("$bigint") {
                        return Js::BigInt(digits);
                    }
                    if let Some(n) = tagged::raw_num(v) {
                        return Js::Num(n);
                    }
                }
                Js::Object(map)
            }
        }
    }

    impl<'a> Js<'a> {
        /// `value.key` for the plain objects JSON can carry (no prototype
        /// member is read by these actions). A key holding `$undefined` is
        /// absent, as `decodeReply` leaves it.
        pub fn prop(self, key: &str) -> Js<'a> {
            match self {
                Js::Object(map) => js(map.get(key)),
                _ => Js::Undefined,
            }
        }

        /// `key in value`: `None` for a primitive, where `in` throws.
        pub fn has(self, key: &str) -> Option<bool> {
            match self {
                Js::Object(map) => Some(map.get(key).is_some_and(|v| !tagged::is_undefined(v))),
                Js::Array(_) | Js::Date(_) => Some(false),
                _ => None,
            }
        }

        /// JavaScript truthiness.
        pub fn truthy(self) -> bool {
            match self {
                Js::Undefined | Js::Null => false,
                Js::Bool(b) => b,
                Js::Num(n) => n != 0.0 && !n.is_nan(),
                Js::BigInt(digits) => digits.trim_start_matches('-').bytes().any(|b| b != b'0'),
                Js::Str(s) => !s.is_empty(),
                Js::Date(_) | Js::Array(_) | Js::Object(_) => true,
            }
        }

        /// `Number(value)`.
        pub fn to_number(self) -> f64 {
            match self {
                Js::Undefined => f64::NAN,
                Js::Null => 0.0,
                Js::Bool(b) => f64::from(u8::from(b)),
                Js::Num(n) => n,
                Js::BigInt(digits) => digits.parse().unwrap_or(f64::NAN),
                Js::Str(s) => js_number(s),
                Js::Date(iso) => crate::db::types::JsDate::parse_iso(iso).map_or(f64::NAN, |d| d.millis() as f64),
                // ToPrimitive: an array becomes `array.join(",")`.
                Js::Array(items) => js_number(&join(items)),
                Js::Object(_) => f64::NAN,
            }
        }
    }

    /// `String(value)` for an array element (`join` writes null and
    /// undefined as "").
    fn element_string(v: &Value) -> String {
        match js(Some(v)) {
            Js::Undefined | Js::Null => String::new(),
            Js::Bool(b) => b.to_string(),
            Js::Num(n) => js_number_to_string(n),
            Js::BigInt(digits) => digits.to_string(),
            Js::Str(s) => s.to_string(),
            // `Date.prototype.toString()`: never a number.
            Js::Date(iso) => crate::db::types::JsDate::parse_iso(iso).map_or("Invalid Date".to_string(), |d| {
                d.0.format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)").to_string()
            }),
            Js::Array(items) => join(items),
            Js::Object(_) => "[object Object]".to_string(),
        }
    }

    fn join(items: &[Value]) -> String {
        items.iter().map(element_string).collect::<Vec<_>>().join(",")
    }

    /// node-postgres `dateToString` in a UTC process.
    fn date_to_string(iso: &str) -> String {
        match crate::db::types::JsDate::parse_iso(iso) {
            Some(d) => d.0.format("%Y-%m-%dT%H:%M:%S%.3f+00:00").to_string(),
            None => "Invalid Date".to_string(),
        }
    }

    /// node-postgres `arrayString`.
    fn array_string(items: &[Value]) -> String {
        let parts: Vec<String> = items
            .iter()
            .map(|item| match js(Some(item)) {
                Js::Undefined | Js::Null => "NULL".to_string(),
                Js::Array(inner) => array_string(inner),
                other => {
                    let text = pg_text(other).unwrap_or_default();
                    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
                }
            })
            .collect();
        format!("{{{}}}", parts.join(","))
    }

    /// The text node-postgres sends for a parameter (`prepareValue`), or
    /// `None` for NULL. Objects go as `JSON.stringify` (approximated with the
    /// tags removed; only ever compared with text ids, where nothing matches).
    pub fn pg_text(v: Js<'_>) -> Option<String> {
        Some(match v {
            Js::Undefined | Js::Null => return None,
            Js::Bool(b) => b.to_string(),
            Js::Num(n) => js_number_to_string(n),
            Js::BigInt(digits) => digits.to_string(),
            Js::Str(s) => s.to_string(),
            Js::Date(iso) => date_to_string(iso),
            Js::Array(items) => array_string(items),
            Js::Object(map) => {
                serde_json::to_string(&tagged::normalize(Value::Object(map.clone()))).unwrap_or_default()
            }
        })
    }

    /// drizzle's `.set()` drops `undefined` values and refuses an empty set.
    pub fn no_values_to_set() -> ActionError {
        ActionError::thrown("No values to set")
    }

    /// PostgreSQL's `boolin`: surrounding white space ignored, then any
    /// case-insensitive prefix of true/false/yes/no, at least `on`/`of`(`f`),
    /// or exactly `1`/`0`.
    pub fn pg_parse_bool(text: &str) -> Option<bool> {
        let t = text.trim_matches(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0B' | '\x0C'));
        let lower = t.to_ascii_lowercase();
        let prefix_of = |word: &str, min: usize| lower.len() >= min && word.starts_with(lower.as_str());
        match lower.as_bytes().first()? {
            b't' if prefix_of("true", 1) => Some(true),
            b'f' if prefix_of("false", 1) => Some(false),
            b'y' if prefix_of("yes", 1) => Some(true),
            b'n' if prefix_of("no", 1) => Some(false),
            b'o' if prefix_of("on", 2) => Some(true),
            b'o' if prefix_of("off", 2) => Some(false),
            b'1' if lower.len() == 1 => Some(true),
            b'0' if lower.len() == 1 => Some(false),
            _ => None,
        }
    }

    /// A value written into a boolean column with `.set({ column: value })`:
    /// `undefined` is drizzle's "No values to set", `null` is NULL, anything
    /// else is the text node-postgres sends, parsed by Postgres.
    pub fn pg_boolean(v: Js<'_>) -> Result<Option<bool>, ActionError> {
        match v {
            Js::Undefined => Err(no_values_to_set()),
            Js::Null => Ok(None),
            other => {
                let text = pg_text(other).unwrap_or_default();
                match pg_parse_bool(&text) {
                    Some(b) => Ok(Some(b)),
                    None => Err(ActionError::thrown(format!("invalid input syntax for type boolean: \"{text}\""))),
                }
            }
        }
    }

    /// PostgreSQL's `uuid_in`: optional braces, 32 hex digits, a hyphen
    /// allowed after any group of four.
    pub fn pg_parse_uuid(text: &str) -> Option<Uuid> {
        let mut src = text.as_bytes();
        let braces = src.first() == Some(&b'{');
        if braces {
            src = &src[1..];
        }
        let mut bytes = [0u8; 16];
        for (i, byte) in bytes.iter_mut().enumerate() {
            if src.len() < 2 {
                return None;
            }
            let hi = (src[0] as char).to_digit(16)?;
            let lo = (src[1] as char).to_digit(16)?;
            *byte = (hi * 16 + lo) as u8;
            src = &src[2..];
            if src.first() == Some(&b'-') && i % 2 == 1 && i < 15 {
                src = &src[1..];
            }
        }
        if braces {
            if src.first() != Some(&b'}') {
                return None;
            }
            src = &src[1..];
        }
        if !src.is_empty() {
            return None;
        }
        Some(Uuid::from_bytes(bytes))
    }

    /// A value compared with or written into a uuid column (never NULL here).
    pub fn pg_uuid(v: Js<'_>) -> Result<Uuid, ActionError> {
        let text = pg_text(v).unwrap_or_default();
        pg_parse_uuid(&text)
            .ok_or_else(|| ActionError::thrown(format!("invalid input syntax for type uuid: \"{text}\"")))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::js::{Js, js, pg_boolean, pg_parse_bool, pg_parse_uuid, pg_text};
    use super::*;

    fn patch(v: Value) -> Result<PreferencesPatch, ActionError> {
        preferences_patch(js(Some(&v)))
    }

    #[test]
    fn preferences_patch_keeps_only_what_passes_its_checks() {
        let p = patch(json!({
            "name": "  Alicja  ",
            "hideUploads": true,
            "copyRaw": "yes",
            "theme": "glass",
            "locale": "pl"
        }))
        .unwrap();
        assert_eq!(p.name.as_deref(), Some("Alicja"));
        assert_eq!(p.hide_uploads, Some(true));
        assert_eq!(p.copy_raw, None);
        assert_eq!(p.theme, Some("glass"));
        assert_eq!(p.locale, Some(Some("pl")));

        assert_eq!(patch(json!({ "name": "   ", "theme": "modern" })).unwrap(), PreferencesPatch::default());
        assert_eq!(patch(json!({ "name": "x".repeat(200) })).unwrap().name.unwrap().len(), 120);
        // `"locale" in input` with any value but pl/en stores NULL.
        assert_eq!(patch(json!({ "locale": null })).unwrap().locale, Some(None));
        assert_eq!(patch(json!({ "locale": "de" })).unwrap().locale, Some(None));
        // decodeReply drops an undefined key: nothing to write.
        assert!(patch(json!({ "locale": { "$undefined": 1 } })).unwrap().is_empty());
        // Arrays and dates are objects without these keys; primitives throw.
        assert!(patch(json!([])).unwrap().is_empty());
        assert!(patch(json!({ "$date": "2026-10-18T12:00:00.000Z" })).unwrap().is_empty());
        for bad in [json!(null), json!("abc"), json!(5), json!(true), json!({ "$undefined": 1 })] {
            assert!(patch(bad.clone()).is_err(), "{bad}");
        }
        assert!(preferences_patch(js(None)).is_err());
    }

    #[test]
    fn the_prototype_quirk_of_resolve_theme() {
        assert_eq!(resolve_theme_result(Some("modern")).unwrap(), "graphite");
        assert_eq!(resolve_theme_result(Some("nope")).unwrap(), "graphite");
        assert_eq!(resolve_theme_result(None).unwrap(), "graphite");
        assert!(resolve_theme_result(Some("toString")).is_err());
        assert!(resolve_theme_result(Some("__proto__")).is_err());
    }

    #[test]
    fn javascript_values() {
        assert_eq!(js(None), Js::Undefined);
        assert!(js(Some(&json!({ "$num": "NaN" }))).to_number().is_nan());
        assert_eq!(js(Some(&json!({ "$bigint": "12" }))).to_number(), 12.0);
        assert_eq!(js(Some(&json!(true))).to_number(), 1.0);
        assert_eq!(js(Some(&json!("  33 "))).to_number(), 33.0);
        assert_eq!(js(Some(&json!([]))).to_number(), 0.0);
        assert_eq!(js(Some(&json!([50]))).to_number(), 50.0);
        assert_eq!(js(Some(&json!([[7]]))).to_number(), 7.0);
        assert!(js(Some(&json!([1, 2]))).to_number().is_nan());
        assert!(js(Some(&json!([true]))).to_number().is_nan());
        assert!(js(Some(&json!({}))).to_number().is_nan());
        assert_eq!(js(Some(&json!({ "$date": "1970-01-01T00:00:01.000Z" }))).to_number(), 1000.0);
        assert!(!js(Some(&json!(""))).truthy());
        assert!(!js(Some(&json!(0))).truthy());
        assert!(!js(Some(&json!({ "$bigint": "0" }))).truthy());
        assert!(js(Some(&json!([]))).truthy());
        assert!(js(Some(&json!({}))).truthy());
        assert_eq!(js(Some(&json!({ "a": { "$undefined": 1 } }))).prop("a"), Js::Undefined);
        assert_eq!(js(Some(&json!({ "a": { "$undefined": 1 } }))).has("a"), Some(false));
        assert_eq!(js(Some(&json!("x"))).has("a"), None);
    }

    #[test]
    fn node_postgres_parameter_text() {
        assert_eq!(pg_text(js(Some(&json!(1)))).as_deref(), Some("1"));
        assert_eq!(pg_text(js(Some(&json!(-0.0)))).as_deref(), Some("0"));
        assert_eq!(pg_text(js(Some(&json!(true)))).as_deref(), Some("true"));
        assert_eq!(pg_text(js(Some(&json!(["a", null, [1]])))).as_deref(), Some("{\"a\",NULL,{\"1\"}}"));
        assert_eq!(pg_text(js(Some(&json!({ "a": 1 })))).as_deref(), Some("{\"a\":1}"));
        assert_eq!(
            pg_text(js(Some(&json!({ "$date": "2026-10-18T12:00:00.000Z" })))).as_deref(),
            Some("2026-10-18T12:00:00.000+00:00")
        );
        assert_eq!(pg_text(js(Some(&json!(null)))), None);
    }

    #[test]
    fn postgres_boolean_input() {
        for (text, want) in [
            ("true", Some(true)),
            (" TRUE ", Some(true)),
            ("t", Some(true)),
            ("tr", Some(true)),
            ("yes", Some(true)),
            ("y", Some(true)),
            ("on", Some(true)),
            ("1", Some(true)),
            ("false", Some(false)),
            ("f", Some(false)),
            ("no", Some(false)),
            ("off", Some(false)),
            ("of", Some(false)),
            ("0", Some(false)),
            ("o", None),
            ("truex", None),
            ("2", None),
            ("10", None),
            ("", None),
            ("{}", None),
        ] {
            assert_eq!(pg_parse_bool(text), want, "{text:?}");
        }
        assert!(matches!(pg_boolean(Js::Undefined), Err(ActionError::Thrown(m)) if m == "No values to set"));
        assert_eq!(pg_boolean(Js::Null).unwrap(), None);
        assert_eq!(pg_boolean(Js::Num(1.0)).unwrap(), Some(true));
        assert!(pg_boolean(Js::Num(1.5)).is_err());
    }

    #[test]
    fn postgres_uuid_input() {
        let id = Uuid::parse_str("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11").unwrap();
        for text in [
            "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11",
            "A0EEBC99-9C0B-4EF8-BB6D-6BB9BD380A11",
            "{a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11}",
            "a0eebc999c0b4ef8bb6d6bb9bd380a11",
            "a0ee-bc99-9c0b-4ef8-bb6d-6bb9-bd38-0a11",
            "{a0eebc99-9c0b4ef8-bb6d6bb9-bd380a11}",
        ] {
            assert_eq!(pg_parse_uuid(text), Some(id), "{text}");
        }
        for text in [
            "",
            "0",
            "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a1",
            "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a111",
            " a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11",
            "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11}",
            "{a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11",
            "a0eebc9-99c0b-4ef8-bb6d-6bb9bd380a11",
            "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a1-",
        ] {
            assert_eq!(pg_parse_uuid(text), None, "{text}");
        }
    }

    #[test]
    fn dim_input_follows_clamp_dim() {
        let dim = |v: Value| clamp_dim(dim_input(js(Some(&v))));
        assert_eq!(dim(json!(null)), 70);
        assert_eq!(dim(json!("")), 70);
        assert_eq!(dim(json!("33")), 33);
        assert_eq!(dim(json!(true)), 1);
        assert_eq!(dim(json!([50])), 50);
        assert_eq!(dim(json!({})), 70);
        assert_eq!(dim(json!(42.5)), 43);
        assert_eq!(dim(json!({ "$num": "Infinity" })), 70);
        assert_eq!(clamp_dim(dim_input(js(None))), 70);
    }

    #[test]
    fn every_upstream_export_is_registered() {
        let mut reg = RpcRegistry::default();
        register(&mut reg);
        let names = reg.names();
        assert_eq!(names.len(), EXPORTS.len());
        for export in EXPORTS {
            assert!(reg.get("account", export).is_some(), "{export}");
        }
    }
}
