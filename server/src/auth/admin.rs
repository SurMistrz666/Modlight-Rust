//! The admin plugin, `admin()` with its defaults (better-auth 1.6.13
//! `plugins/admin/`: `admin.mjs`, `routes.mjs`, `has-permission.mjs`,
//! `access/statement.mjs`): roles "admin"/"user", bans, other users'
//! sessions, impersonation.
//!
//! - Permissions: the default roles only. "admin" may do everything except
//!   impersonate other admins; "user" nothing. A role string may list several
//!   roles separated by commas; any of them granting the request is enough.
//!   No role allow-list is configured, so `set-role` stores any string.
//! - Database hooks: a new user gets `role = "user"` unless one was given
//!   ([`super::adapter::create_user`]), and a banned account gets no new
//!   session unless its ban has expired ([`super::session::check_ban_before_session_create`]).
//! - After hook: `/list-sessions` hides sessions an admin is impersonating.
//!
//! Server-side calls Modlight makes (app/actions/admin.ts) go through the same
//! endpoints ([`super::server`]).

use axum::http::Method;
use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};

use super::adapter::{self, CreateSessionError, NewUser, SessionOverride, SqlValue, pg_text, user_field_column};
use super::endpoint::{Endpoint, EndpointCtx, HandlerResult, Middleware, Out};
use super::error::{ApiError, EndpointError, admin as codes, base};
use super::routes::sign_in::banned_error;
use super::routes::sign_up::{MAX_PASSWORD_LENGTH, MIN_PASSWORD_LENGTH};
use super::routes::{auth_cookie, delete_session_cookie, set_session_cookie};
use super::session::{AuthSession, SessionOut, UserOut};
use super::validate::{Schema, is_zod_email};
use crate::db::models::User;
use crate::db::types::JsDate;

// ── Permissions ─────────────────────────────────────────────────────────────

const ADMIN_USER_ACTIONS: &[&str] =
    &["create", "list", "set-role", "ban", "impersonate", "delete", "set-password", "get", "update"];
const ADMIN_SESSION_ACTIONS: &[&str] = &["list", "revoke", "delete"];

/// `role(statements).authorize(request)` for one default role (AND connector).
fn role_allows(role: &str, request: &[(&str, Vec<String>)]) -> bool {
    let allowed = |resource: &str| -> Option<&'static [&'static str]> {
        match (role, resource) {
            ("admin", "user") => Some(ADMIN_USER_ACTIONS),
            ("admin", "session") => Some(ADMIN_SESSION_ACTIONS),
            ("user", "user") | ("user", "session") => Some(&[]),
            _ => None,
        }
    };
    if !matches!(role, "admin" | "user") {
        return false;
    }
    let mut success = false;
    for (resource, actions) in request {
        let Some(allowed_actions) = allowed(resource) else {
            return false;
        };
        success = !actions.is_empty() && actions.iter().all(|a| allowed_actions.contains(&a.as_str()));
        if !success {
            return false;
        }
    }
    success
}

/// `hasPermission({ role, permissions })` with the default roles.
pub fn has_permission(role: Option<&str>, request: &[(&str, Vec<String>)]) -> bool {
    let roles = role.filter(|r| !r.is_empty()).unwrap_or("user");
    roles.split(',').any(|r| role_allows(r, request))
}

fn can(ctx: &EndpointCtx, resource: &'static str, action: &str) -> bool {
    let role = ctx.session.as_ref().and_then(|s| s.user.role.clone());
    has_permission(role.as_deref(), &[(resource, vec![action.to_string()])])
}

/// `parseRoles(role)`: an array is joined with commas.
fn parse_roles(v: &Value) -> Option<String> {
    match v {
        Value::Array(items) => Some(items.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(",")),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

// ── Schemas ─────────────────────────────────────────────────────────────────

fn user_id_body() -> Schema {
    Schema::object(vec![("userId", Schema::CoerceString { min: None })])
}

fn role_schema() -> Schema {
    Schema::Union(vec![Schema::string(), Schema::Array(Box::new(Schema::string()))])
}

fn set_role_body() -> Schema {
    Schema::object(vec![("userId", Schema::CoerceString { min: None }), ("role", role_schema())])
}

fn get_user_query() -> Schema {
    Schema::object(vec![("id", Schema::string())])
}

fn create_user_body() -> Schema {
    Schema::object(vec![
        ("email", Schema::string()),
        ("password", Schema::string().optional()),
        ("name", Schema::string()),
        ("role", role_schema().optional()),
        ("data", Schema::Record(Box::new(Schema::Any)).optional()),
    ])
}

fn update_user_body() -> Schema {
    Schema::object(vec![
        ("userId", Schema::CoerceString { min: None }),
        ("data", Schema::Record(Box::new(Schema::Any))),
    ])
}

const SEARCH_FIELDS: &[&str] = &["email", "name"];
const SEARCH_OPERATORS: &[&str] = &["contains", "starts_with", "ends_with"];
const SORT_DIRECTIONS: &[&str] = &["asc", "desc"];
const WHERE_OPERATORS: &[&str] =
    &["eq", "ne", "lt", "lte", "gt", "gte", "in", "not_in", "contains", "starts_with", "ends_with"];

fn string_or_number() -> Schema {
    Schema::Union(vec![Schema::string(), Schema::Number])
}

fn list_users_query() -> Schema {
    Schema::object(vec![
        ("searchValue", Schema::string().optional()),
        ("searchField", Schema::Enum(SEARCH_FIELDS).optional()),
        ("searchOperator", Schema::Enum(SEARCH_OPERATORS).optional()),
        ("limit", string_or_number().optional()),
        ("offset", string_or_number().optional()),
        ("sortBy", Schema::string().optional()),
        ("sortDirection", Schema::Enum(SORT_DIRECTIONS).optional()),
        ("filterField", Schema::string().optional()),
        (
            "filterValue",
            Schema::Union(vec![
                Schema::string(),
                Schema::Number,
                Schema::Boolean,
                Schema::Array(Box::new(Schema::string())),
                Schema::Array(Box::new(Schema::Number)),
            ])
            .optional(),
        ),
        ("filterOperator", Schema::Enum(WHERE_OPERATORS).optional()),
    ])
}

fn ban_body() -> Schema {
    Schema::object(vec![
        ("userId", Schema::CoerceString { min: None }),
        ("banReason", Schema::string().optional()),
        ("banExpiresIn", Schema::Number.optional()),
    ])
}

fn revoke_user_session_body() -> Schema {
    Schema::object(vec![("sessionToken", Schema::string())])
}

fn set_user_password_body() -> Schema {
    Schema::object(vec![
        ("newPassword", Schema::String { min: Some((1, Some("newPassword cannot be empty"))) }),
        ("userId", Schema::CoerceString { min: Some((1, Some("userId cannot be empty"))) }),
    ])
}

fn has_permission_body() -> Schema {
    let record = || Schema::Record(Box::new(Schema::Array(Box::new(Schema::string()))));
    Schema::Intersection(
        Box::new(Schema::object(vec![
            ("userId", Schema::CoerceString { min: None }.optional()),
            ("role", Schema::string().optional()),
        ])),
        Box::new(Schema::Xor(vec![
            Schema::object(vec![("permission", record())]),
            Schema::object(vec![("permissions", record())]),
        ])),
    )
}

macro_rules! endpoint {
    ($path:expr, $methods:expr, $body:expr, $query:expr, $mw:expr, $handler:expr) => {
        Endpoint {
            path: $path,
            methods: $methods,
            body: $body,
            query: $query,
            media_types: &[],
            middleware: $mw,
            handler: $handler,
        }
    };
}

/// The plugin's endpoints in its `endpoints` order.
pub fn endpoints() -> Vec<Endpoint> {
    const ADMIN: &[Middleware] = &[Middleware::Admin];
    vec![
        endpoint!("/admin/set-role", &[Method::POST], Some(set_role_body), None, ADMIN, set_role),
        endpoint!("/admin/get-user", &[Method::GET], None, Some(get_user_query), ADMIN, get_user),
        endpoint!("/admin/create-user", &[Method::POST], Some(create_user_body), None, &[], create_user),
        endpoint!("/admin/update-user", &[Method::POST], Some(update_user_body), None, ADMIN, admin_update_user),
        endpoint!("/admin/list-users", &[Method::GET], None, Some(list_users_query), ADMIN, list_users),
        endpoint!("/admin/list-user-sessions", &[Method::POST], Some(user_id_body), None, ADMIN, list_user_sessions),
        endpoint!("/admin/unban-user", &[Method::POST], Some(user_id_body), None, ADMIN, unban_user),
        endpoint!("/admin/ban-user", &[Method::POST], Some(ban_body), None, ADMIN, ban_user),
        endpoint!("/admin/impersonate-user", &[Method::POST], Some(user_id_body), None, ADMIN, impersonate_user),
        endpoint!("/admin/stop-impersonating", &[Method::POST], None, None, &[], stop_impersonating),
        endpoint!(
            "/admin/revoke-user-session",
            &[Method::POST],
            Some(revoke_user_session_body),
            None,
            ADMIN,
            revoke_user_session
        ),
        endpoint!(
            "/admin/revoke-user-sessions",
            &[Method::POST],
            Some(user_id_body),
            None,
            ADMIN,
            revoke_user_sessions
        ),
        endpoint!("/admin/remove-user", &[Method::POST], Some(user_id_body), None, ADMIN, remove_user),
        endpoint!(
            "/admin/set-user-password",
            &[Method::POST],
            Some(set_user_password_body),
            None,
            ADMIN,
            set_user_password
        ),
        endpoint!("/admin/has-permission", &[Method::POST], Some(has_permission_body), None, &[], user_has_permission),
    ]
}

fn forbidden(code: super::error::ErrorCode) -> EndpointError {
    ApiError::from("FORBIDDEN", code).into()
}

fn user_json(user: Option<&User>) -> Value {
    user.map(|u| serde_json::to_value(UserOut::from(u)).unwrap_or(Value::Null)).unwrap_or(Value::Null)
}

fn body_user_id(ctx: &EndpointCtx) -> String {
    ctx.body_str("userId").unwrap_or_default().to_string()
}

fn set_role(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        if !can(ctx, "user", "set-role") {
            return Err(forbidden(codes::YOU_ARE_NOT_ALLOWED_TO_CHANGE_USERS_ROLE));
        }
        let role = ctx.body_field("role").and_then(parse_roles).unwrap_or_default();
        let user = adapter::update_user(ctx.db(), &body_user_id(ctx), &[("role", SqlValue::Text(role))]).await?;
        ctx.json(json!({ "user": user_json(user.as_ref()) }))
    })
}

fn get_user(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let id = ctx.query_str("id").unwrap_or_default().to_string();
        if !can(ctx, "user", "get") {
            return Err(forbidden(codes::YOU_ARE_NOT_ALLOWED_TO_GET_USER));
        }
        let Some(user) = adapter::find_user_by_id(ctx.db(), &id).await? else {
            return Err(ApiError::from("NOT_FOUND", base::USER_NOT_FOUND).into());
        };
        ctx.json(user_json(Some(&user)))
    })
}

fn create_user(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let session = ctx.session_from_ctx().await;
        if session.is_none() && (ctx.is_request || ctx.has_headers) {
            return Err(ApiError::named("UNAUTHORIZED", None).into());
        }
        if let Some(s) = &session
            && !has_permission(s.user.role.as_deref(), &[("user", vec!["create".into()])])
        {
            return Err(forbidden(codes::YOU_ARE_NOT_ALLOWED_TO_CREATE_USERS));
        }
        let email = ctx.body_str("email").unwrap_or_default().to_lowercase();
        if !is_zod_email(&email) {
            return Err(ApiError::from("BAD_REQUEST", base::INVALID_EMAIL).into());
        }
        if adapter::find_user_by_email(ctx.db(), &email).await?.is_some() {
            return Err(ApiError::from("BAD_REQUEST", codes::USER_ALREADY_EXISTS_USE_ANOTHER_EMAIL).into());
        }
        let mut data = NewUser::default();
        data.set("email", SqlValue::Text(email.clone()));
        data.set("name", SqlValue::Text(ctx.body_str("name").unwrap_or_default().to_string()));
        // `(role && parseRoles(role)) ?? defaultRole ?? "user"`
        let role = match ctx.body_field("role") {
            Some(Value::String(s)) if s.is_empty() => String::new(),
            Some(v) => parse_roles(v).unwrap_or_else(|| "user".into()),
            None => "user".into(),
        };
        data.set("role", SqlValue::Text(role));
        if let Some(Value::Object(extra)) = ctx.body_field("data").cloned() {
            for (k, v) in extra {
                if user_field_column(&k).is_none() {
                    continue;
                }
                let value = match pg_text(&v) {
                    None => SqlValue::Null,
                    Some(text) if k == "email" => SqlValue::Text(text),
                    Some(text) => SqlValue::Cast(text, "text"),
                };
                data.set(&k, value);
            }
        }
        let user = adapter::create_user(ctx.db(), ctx.state.config.default_user_quota, data).await?;
        if let Some(password) = ctx.body_str("password").filter(|p| !p.is_empty()).map(str::to_string) {
            let hash = crate::auth::password::hash_password(&password).await?;
            adapter::create_account(ctx.db(), &user.id, "credential", &user.id, Some(&hash)).await?;
        }
        ctx.json(json!({ "user": user_json(Some(&user)) }))
    })
}

fn admin_update_user(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        if !can(ctx, "user", "update") {
            return Err(forbidden(codes::YOU_ARE_NOT_ALLOWED_TO_UPDATE_USERS));
        }
        let mut data = ctx.body_field("data").and_then(Value::as_object).cloned().unwrap_or_default();
        if data.is_empty() {
            return Err(ApiError::from("BAD_REQUEST", codes::NO_DATA_TO_UPDATE).into());
        }
        if let Some(role_value) = data.get("role").cloned() {
            if !can(ctx, "user", "set-role") {
                return Err(forbidden(codes::YOU_ARE_NOT_ALLOWED_TO_CHANGE_USERS_ROLE));
            }
            let roles: Vec<Value> = match role_value {
                Value::Array(items) => items,
                other => vec![other],
            };
            if roles.iter().any(|r| !r.is_string()) {
                return Err(ApiError::from("BAD_REQUEST", codes::INVALID_ROLE_TYPE).into());
            }
            data.insert(
                "role".into(),
                Value::String(roles.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(",")),
            );
        }
        let patch: Vec<(String, SqlValue)> = data
            .iter()
            .filter(|(k, _)| user_field_column(k).is_some())
            .map(|(k, v)| {
                let value = match pg_text(v) {
                    None => SqlValue::Null,
                    Some(text) if k == "email" => SqlValue::Text(text),
                    Some(text) => SqlValue::Cast(text, "text"),
                };
                (k.clone(), value)
            })
            .collect();
        let patch_refs: Vec<(&str, SqlValue)> = patch.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        let user = adapter::update_user(ctx.db(), &body_user_id(ctx), &patch_refs).await?;
        ctx.json(user_json(user.as_ref()))
    })
}

/// A `where` clause of `/admin/list-users` over a Better Auth user field.
struct Filter {
    column: &'static str,
    operator: String,
    value: Value,
}

fn filter_sql(f: &Filter, binds: &mut Vec<String>) -> Option<String> {
    let ty = adapter::user_column_type(f.column);
    let col = format!("\"{}\"", f.column);
    let mut bind = |v: &Value| -> String {
        binds.push(pg_text(v).unwrap_or_default());
        format!("${}::text::{ty}", binds.len())
    };
    Some(match f.operator.as_str() {
        "in" | "not_in" => {
            let items = f.value.as_array()?;
            let placeholders: Vec<String> = items.iter().map(&mut bind).collect();
            let list = if placeholders.is_empty() { "NULL".to_string() } else { placeholders.join(", ") };
            if f.operator == "in" { format!("{col} IN ({list})") } else { format!("{col} NOT IN ({list})") }
        }
        "contains" | "starts_with" | "ends_with" => {
            let text = pg_text(&f.value).unwrap_or_default();
            let pattern = match f.operator.as_str() {
                "contains" => format!("%{text}%"),
                "starts_with" => format!("{text}%"),
                _ => format!("%{text}"),
            };
            binds.push(pattern);
            format!("{col} LIKE ${}::text", binds.len())
        }
        "lt" => format!("{col} < {}", bind(&f.value)),
        "lte" => format!("{col} <= {}", bind(&f.value)),
        "gt" => format!("{col} > {}", bind(&f.value)),
        "gte" => format!("{col} >= {}", bind(&f.value)),
        "ne" if f.value.is_null() => format!("{col} IS NOT NULL"),
        "ne" => format!("{col} <> {}", bind(&f.value)),
        _ if f.value.is_null() => format!("{col} IS NULL"),
        _ => format!("{col} = {}", bind(&f.value)),
    })
}

/// `Number(x) || undefined` for limit/offset.
fn js_positive_number(v: Option<&Value>) -> Option<f64> {
    let n = match v? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => crate::config::js_number(s),
        _ => f64::NAN,
    };
    if n.is_nan() || n == 0.0 { None } else { Some(n) }
}

fn list_users(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        if !can(ctx, "user", "list") {
            return Err(forbidden(codes::YOU_ARE_NOT_ALLOWED_TO_LIST_USERS));
        }
        let q = ctx.query.clone();
        let limit = js_positive_number(q.get("limit"));
        let offset = js_positive_number(q.get("offset"));
        let result: anyhow::Result<(Vec<User>, i64)> = async {
            let mut filters = Vec::new();
            if let Some(search) = q.get("searchValue").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                let field = q.get("searchField").and_then(Value::as_str).unwrap_or("email");
                filters.push(Filter {
                    column: user_field_column(field).ok_or_else(|| anyhow::anyhow!("unknown field"))?,
                    operator: q.get("searchOperator").and_then(Value::as_str).unwrap_or("contains").to_string(),
                    value: Value::String(search.to_string()),
                });
            }
            if let Some(value) = q.get("filterValue") {
                let field = q.get("filterField").and_then(Value::as_str).filter(|f| !f.is_empty()).unwrap_or("email");
                filters.push(Filter {
                    column: user_field_column(field).ok_or_else(|| anyhow::anyhow!("unknown field"))?,
                    operator: q.get("filterOperator").and_then(Value::as_str).unwrap_or("eq").to_string(),
                    value: value.clone(),
                });
            }
            let mut binds = Vec::new();
            let mut clauses = Vec::new();
            for f in &filters {
                clauses.push(filter_sql(f, &mut binds).ok_or_else(|| anyhow::anyhow!("value must be an array"))?);
            }
            let where_sql =
                if clauses.is_empty() { String::new() } else { format!(" WHERE {}", clauses.join(" AND ")) };
            let order = match q.get("sortBy").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                Some(field) => {
                    let column = user_field_column(field).ok_or_else(|| anyhow::anyhow!("unknown sort field"))?;
                    let dir =
                        if q.get("sortDirection").and_then(Value::as_str) == Some("desc") { "DESC" } else { "ASC" };
                    format!(" ORDER BY \"{column}\" {dir}")
                }
                None => String::new(),
            };
            let mut paging = String::new();
            if let Some(l) = limit {
                paging.push_str(&format!(" LIMIT {}", l as i64));
            }
            if let Some(o) = offset {
                paging.push_str(&format!(" OFFSET {}", o as i64));
            }
            let sql = format!(
                "SELECT {} FROM \"user\"{where_sql}{order}{paging}",
                <User as crate::db::models::Columns>::COLUMNS
            );
            let mut query = sqlx::query_as::<_, User>(&sql);
            for b in &binds {
                query = query.bind(b);
            }
            let users = query.fetch_all(ctx.db()).await?;
            let count_sql = format!("SELECT count(*) FROM \"user\"{where_sql}");
            let mut count = sqlx::query_as::<_, (i64,)>(&count_sql);
            for b in &binds {
                count = count.bind(b);
            }
            let (total,) = count.fetch_one(ctx.db()).await?;
            Ok((users, total))
        }
        .await;
        match result {
            Ok((users, total)) => {
                let mut out = Map::new();
                out.insert("users".into(), Value::Array(users.iter().map(|u| user_json(Some(u))).collect()));
                out.insert("total".into(), json!(total));
                if let Some(l) = limit {
                    out.insert("limit".into(), json!(l));
                }
                if let Some(o) = offset {
                    out.insert("offset".into(), json!(o));
                }
                ctx.json(Value::Object(out))
            }
            Err(_) => ctx.json(json!({ "users": [], "total": 0 })),
        }
    })
}

fn list_user_sessions(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        if !can(ctx, "session", "list") {
            return Err(forbidden(codes::YOU_ARE_NOT_ALLOWED_TO_LIST_USERS_SESSIONS));
        }
        let sessions = adapter::list_sessions(ctx.db(), &body_user_id(ctx), false).await?;
        let out: Vec<Value> =
            sessions.iter().map(|s| serde_json::to_value(SessionOut::from(s)).unwrap_or(Value::Null)).collect();
        ctx.json(json!({ "sessions": out }))
    })
}

fn unban_user(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        if !can(ctx, "user", "ban") {
            return Err(forbidden(codes::YOU_ARE_NOT_ALLOWED_TO_BAN_USERS));
        }
        let user = unban(ctx.db(), &body_user_id(ctx)).await?;
        ctx.json(json!({ "user": user_json(user.as_ref()) }))
    })
}

/// The unban write: `banned = false`, reason and expiry cleared.
pub async fn unban(db: &sqlx::PgPool, user_id: &str) -> sqlx::Result<Option<User>> {
    adapter::update_user(
        db,
        user_id,
        &[
            ("banned", SqlValue::Bool(false)),
            ("banExpires", SqlValue::Null),
            ("banReason", SqlValue::Null),
            ("updatedAt", SqlValue::Date(JsDate::now())),
        ],
    )
    .await
}

fn ban_user(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        if !can(ctx, "user", "ban") {
            return Err(forbidden(codes::YOU_ARE_NOT_ALLOWED_TO_BAN_USERS));
        }
        let user_id = body_user_id(ctx);
        if adapter::find_user_by_id(ctx.db(), &user_id).await?.is_none() {
            return Err(ApiError::from("NOT_FOUND", base::USER_NOT_FOUND).into());
        }
        if ctx.session.as_ref().is_some_and(|s| s.user.id == user_id) {
            return Err(ApiError::from("BAD_REQUEST", codes::YOU_CANNOT_BAN_YOURSELF).into());
        }
        let reason = ctx.body_str("banReason").filter(|r| !r.is_empty()).unwrap_or("No reason").to_string();
        let mut patch = vec![("banned", SqlValue::Bool(true)), ("banReason", SqlValue::Text(reason))];
        if let Some(secs) = ctx.body_field("banExpiresIn").and_then(Value::as_f64).filter(|s| *s != 0.0 && !s.is_nan())
        {
            let at = JsDate::from_millis(JsDate::now().millis() + (secs * 1000.0) as i64).unwrap_or_else(JsDate::now);
            patch.push(("banExpires", SqlValue::Date(at)));
        }
        patch.push(("updatedAt", SqlValue::Date(JsDate::now())));
        let user = adapter::update_user(ctx.db(), &user_id, &patch).await?;
        adapter::delete_user_sessions(ctx.db(), &user_id).await?;
        ctx.json(json!({ "user": user_json(user.as_ref()) }))
    })
}

fn impersonate_user(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        if !can(ctx, "user", "impersonate") {
            return Err(forbidden(codes::YOU_ARE_NOT_ALLOWED_TO_IMPERSONATE_USERS));
        }
        let Some(target) = adapter::find_user_by_id(ctx.db(), &body_user_id(ctx)).await? else {
            return Err(ApiError::from("NOT_FOUND", base::USER_NOT_FOUND).into());
        };
        let target_roles = target.role.clone().filter(|r| !r.is_empty()).unwrap_or_else(|| "user".into());
        if target_roles.split(',').any(|r| r == "admin") && !can(ctx, "user", "impersonate-admins") {
            return Err(forbidden(codes::YOU_CANNOT_IMPERSONATE_ADMINS));
        }
        let admin = ctx.session.clone().expect("admin middleware ran");
        let ip = ctx.ip().unwrap_or_default();
        let ua = ctx.user_agent();
        let over = SessionOverride {
            impersonated_by: Some(Some(admin.user.id.clone())),
            expires_at: JsDate::from_millis(JsDate::now().millis() + 3600 * 1000),
            ..Default::default()
        };
        let session = match adapter::create_session(ctx.db(), &target.id, true, &ip, &ua, Some(&over), true).await {
            Ok(s) => s,
            Err(CreateSessionError::Banned) => return Err(banned_error()),
            Err(CreateSessionError::Db(e)) => return Err(e.into()),
        };
        delete_session_cookie(ctx, false);
        let dont_remember = ctx.signed_cookie_value(&ctx.state.auth_cookies.dont_remember.name).unwrap_or_default();
        let admin_cookie = auth_cookie(ctx, "admin_session", Some(super::cookies::SESSION_EXPIRES_IN));
        let value = super::cookies::signed_value(&format!("{}:{dont_remember}", admin.session.token), ctx.secret());
        ctx.set_cookie(admin_cookie.set(value));
        let session_json = serde_json::to_value(SessionOut::from(&session)).map_err(anyhow::Error::from)?;
        set_session_cookie(ctx, AuthSession { session, user: target.clone() }, Some(true), None);
        ctx.json(json!({ "session": session_json, "user": user_json(Some(&target)) }))
    })
}

fn stop_impersonating(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let Some(session) = ctx.session_from_ctx().await else {
            return Err(ApiError::named("UNAUTHORIZED", None).into());
        };
        let Some(by) = session.session.impersonated_by.clone().filter(|b| !b.is_empty()) else {
            return Err(ApiError::message_only("BAD_REQUEST", "You are not impersonating anyone").into());
        };
        let Some(admin_user) = adapter::find_user_by_id(ctx.db(), &by).await? else {
            return Err(ApiError::message_only("INTERNAL_SERVER_ERROR", "Failed to find user").into());
        };
        let admin_cookie = auth_cookie(ctx, "admin_session", None);
        let Some(value) = ctx.signed_cookie_value(&admin_cookie.name) else {
            return Err(ApiError::message_only("INTERNAL_SERVER_ERROR", "Failed to find admin session").into());
        };
        let mut parts = value.splitn(2, ':');
        let admin_token = parts.next().unwrap_or_default().to_string();
        let dont_remember = parts.next().is_some_and(|v| !v.is_empty());
        let admin_session = adapter::find_session(ctx.db(), &admin_token).await?;
        let Some(admin_session) = admin_session.filter(|s| s.session.user_id == admin_user.id) else {
            return Err(ApiError::message_only("INTERNAL_SERVER_ERROR", "Failed to find admin session").into());
        };
        adapter::delete_session(ctx.db(), &session.session.token).await?;
        set_session_cookie(ctx, admin_session.clone(), Some(dont_remember), None);
        ctx.expire_cookie(&admin_cookie);
        let s = serde_json::to_value(SessionOut::from(&admin_session.session)).map_err(anyhow::Error::from)?;
        ctx.json(json!({ "session": s, "user": user_json(Some(&admin_session.user)) }))
    })
}

fn revoke_user_session(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        if !can(ctx, "session", "revoke") {
            return Err(forbidden(codes::YOU_ARE_NOT_ALLOWED_TO_REVOKE_USERS_SESSIONS));
        }
        let token = ctx.body_str("sessionToken").unwrap_or_default().to_string();
        adapter::delete_session(ctx.db(), &token).await?;
        ctx.json(json!({ "success": true }))
    })
}

fn revoke_user_sessions(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        if !can(ctx, "session", "revoke") {
            return Err(forbidden(codes::YOU_ARE_NOT_ALLOWED_TO_REVOKE_USERS_SESSIONS));
        }
        adapter::delete_user_sessions(ctx.db(), &body_user_id(ctx)).await?;
        ctx.json(json!({ "success": true }))
    })
}

fn remove_user(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        if !can(ctx, "user", "delete") {
            return Err(forbidden(codes::YOU_ARE_NOT_ALLOWED_TO_DELETE_USERS));
        }
        let user_id = body_user_id(ctx);
        if ctx.session.as_ref().is_some_and(|s| s.user.id == user_id) {
            return Err(ApiError::from("BAD_REQUEST", codes::YOU_CANNOT_REMOVE_YOURSELF).into());
        }
        if adapter::find_user_by_id(ctx.db(), &user_id).await?.is_none() {
            return Err(ApiError::from("NOT_FOUND", base::USER_NOT_FOUND).into());
        }
        adapter::delete_user_sessions(ctx.db(), &user_id).await?;
        adapter::delete_user(ctx.db(), &user_id).await?;
        ctx.json(json!({ "success": true }))
    })
}

fn set_user_password(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        if !can(ctx, "user", "set-password") {
            return Err(forbidden(codes::YOU_ARE_NOT_ALLOWED_TO_SET_USERS_PASSWORD));
        }
        let new_password = ctx.body_str("newPassword").unwrap_or_default().to_string();
        let length = new_password.encode_utf16().count();
        if length < MIN_PASSWORD_LENGTH {
            return Err(ApiError::from("BAD_REQUEST", base::PASSWORD_TOO_SHORT).into());
        }
        if length > MAX_PASSWORD_LENGTH {
            return Err(ApiError::from("BAD_REQUEST", base::PASSWORD_TOO_LONG).into());
        }
        let hash = crate::auth::password::hash_password(&new_password).await?;
        adapter::update_password(ctx.db(), &body_user_id(ctx), &hash).await?;
        ctx.json(json!({ "status": true }))
    })
}

fn user_has_permission(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let Some(Value::Object(perms)) = ctx.body_field("permissions").cloned() else {
            return Err(ApiError::message_only(
                "BAD_REQUEST",
                "invalid permission check. no permission(s) were passed.",
            )
            .into());
        };
        let session = ctx.session_from_ctx().await;
        if session.is_none() && (ctx.is_request || ctx.has_headers) {
            return Err(ApiError::named("UNAUTHORIZED", None).into());
        }
        let user_id = ctx.body_str("userId").map(str::to_string);
        let role = ctx.body_str("role").map(str::to_string);
        if session.is_none()
            && user_id.as_deref().is_none_or(str::is_empty)
            && role.as_deref().is_none_or(str::is_empty)
        {
            return Err(ApiError::message_only("BAD_REQUEST", "user id or role is required").into());
        }
        let effective_role: Option<Option<String>> = match &session {
            Some(s) => Some(s.user.role.clone()),
            None => match role.filter(|r| !r.is_empty()) {
                Some(r) => Some(Some(r)),
                None => match user_id.filter(|u| !u.is_empty()) {
                    Some(id) => adapter::find_user_by_id(ctx.db(), &id).await?.map(|u| u.role),
                    None => None,
                },
            },
        };
        let Some(role) = effective_role else {
            return Err(ApiError::message_only("BAD_REQUEST", "user not found").into());
        };
        let request: Vec<(String, Vec<String>)> = perms
            .iter()
            .map(|(k, v)| {
                let actions = v
                    .as_array()
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                (k.clone(), actions)
            })
            .collect();
        let request_refs: Vec<(&str, Vec<String>)> = request.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        let success = !request_refs.is_empty() && has_permission(role.as_deref(), &request_refs);
        Ok(Out::Json(json!({ "error": null, "success": success })))
    })
}

/// The plugin's `/list-sessions` after hook: drop impersonation sessions from
/// a successful answer.
pub fn list_sessions_after_hook(result: HandlerResult) -> HandlerResult {
    match result {
        Ok(Out::Json(Value::Array(items))) => Ok(Out::Json(Value::Array(
            items
                .into_iter()
                .filter(|s| {
                    let by = s.get("impersonatedBy");
                    !crate::auth::validate::js_truthy(by)
                })
                .collect(),
        ))),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_roles() {
        let user = |a: &str| vec![("user", vec![a.to_string()])];
        assert!(has_permission(Some("admin"), &user("ban")));
        assert!(!has_permission(Some("admin"), &user("impersonate-admins")));
        assert!(!has_permission(Some("user"), &user("list")));
        assert!(has_permission(Some("user,admin"), &user("list")));
        assert!(!has_permission(None, &user("list")));
        assert!(!has_permission(Some("toString"), &user("list")));
        assert!(!has_permission(Some("admin"), &[("files", vec!["x".into()])]));
        assert!(!has_permission(Some("admin"), &[("user", vec![])]));
    }
}
