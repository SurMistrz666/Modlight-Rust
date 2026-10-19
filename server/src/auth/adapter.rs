//! Better Auth's internal adapter (`db/internal-adapter.mjs`) over the drizzle
//! adapter (`@better-auth/drizzle-adapter` 1.6.13, pg provider, no
//! transactions: lib/auth.ts passes none, so `runWithTransaction` is a plain
//! call) with the database hooks lib/auth.ts and the plugins install.
//!
//! The writes are byte-for-byte what the TypeScript server writes:
//! - ids are `generateId()` (32 x `[A-Za-z0-9]`), timestamps the app clock;
//! - every UPDATE of `user`, `session`, `account`, `verification` also sets
//!   `updated_at` (Better Auth's `onUpdate` and drizzle's `$onUpdate`);
//! - a new user gets the schema defaults Better Auth writes explicitly
//!   (`banned` false, `two_factor_enabled` false, `used_quota` 0,
//!   `hide_uploads`/`copy_raw` false, `theme` "modern") plus lib/auth.ts's
//!   `role`, `user_code` and `max_quota`;
//! - values that come straight from a request body for columns Better Auth
//!   lets clients set (`hideUploads`, `copyRaw`, `theme`, `locale`, and the
//!   admin plugin's `data`) reach Postgres as text the way node-postgres sends
//!   them and are cast by the column type, so `"1"` is a boolean true and
//!   `"zz"` is a database error, exactly as upstream.
//!
//! SECURITY FIX 3 (PORT.md 12): lib/auth.ts decided "first account becomes
//! admin" from a `count(*)` taken outside any transaction, so two sign-ups
//! racing on an empty install could both become administrators. Here the count
//! and the INSERT run in one transaction holding a transaction-scoped advisory
//! lock, so user creations are serialised and exactly one sees an empty table.

use serde_json::{Map, Value};
use sqlx::{PgPool, Postgres, Transaction};

use super::crypto::generate_id;
use super::session::AuthSession;
use crate::db::models::{Account, Columns, Session, TwoFactor, User, Verification};
use crate::db::types::JsDate;

/// `pg_advisory_xact_lock` key for user creation (FIX 3): the ASCII of
/// "mlusers", any constant shared by every process works.
pub const USER_CREATE_LOCK_KEY: i64 = 0x006d_6c75_7365_7273;

/// A value bound for one column of a dynamic INSERT/UPDATE.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    Null,
    Text(String),
    Bool(bool),
    BigInt(i64),
    Int(i32),
    Date(JsDate),
    /// A NULL for a column of this SQL type (a bare text NULL would not fit a
    /// timestamp or boolean column).
    NullOf(&'static str),
    /// Text from a request body, cast by Postgres to the column's type
    /// (`$n::text::<type>`), the way node-postgres sends untyped parameters.
    Cast(String, &'static str),
}

/// node-postgres `prepareValue` for a JSON value (`None` = NULL).
pub fn pg_text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(crate::i18n::format::js_number_to_string(n.as_f64().unwrap_or(f64::NAN))),
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => Some(pg_array_literal(items)),
        Value::Object(_) => Some(serde_json::to_string(v).unwrap_or_default()),
    }
}

/// node-postgres `arrayString`.
fn pg_array_literal(items: &[Value]) -> String {
    let parts: Vec<String> = items
        .iter()
        .map(|v| match v {
            Value::Null => "NULL".to_string(),
            Value::Array(inner) => pg_array_literal(inner),
            other => {
                let text = match other {
                    Value::Object(_) => serde_json::to_string(other).unwrap_or_default(),
                    _ => pg_text(other).unwrap_or_default(),
                };
                format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
            }
        })
        .collect();
    format!("{{{}}}", parts.join(","))
}

/// The SQL type of a Better Auth `user` column, for [`SqlValue::Cast`].
pub fn user_column_type(column: &str) -> &'static str {
    match column {
        "email_verified" | "banned" | "two_factor_enabled" | "hide_uploads" | "copy_raw" => "boolean",
        "used_quota" | "max_quota" => "bigint",
        "created_at" | "updated_at" | "ban_expires" => "timestamp",
        _ => "text",
    }
}

/// Better Auth `user` field name -> column, for the fields its schema knows
/// (core, admin plugin, twoFactor plugin, lib/auth.ts additionalFields).
pub fn user_field_column(field: &str) -> Option<&'static str> {
    Some(match field {
        "id" => "id",
        "name" => "name",
        "email" => "email",
        "emailVerified" => "email_verified",
        "image" => "image",
        "createdAt" => "created_at",
        "updatedAt" => "updated_at",
        "role" => "role",
        "banned" => "banned",
        "banReason" => "ban_reason",
        "banExpires" => "ban_expires",
        "twoFactorEnabled" => "two_factor_enabled",
        "userCode" => "user_code",
        "usedQuota" => "used_quota",
        "maxQuota" => "max_quota",
        "hideUploads" => "hide_uploads",
        "copyRaw" => "copy_raw",
        "theme" => "theme",
        "locale" => "locale",
        _ => return None,
    })
}

/// The `user` schema fields in Better Auth's order (core, admin, twoFactor,
/// additionalFields... as `getFields` merges them for INPUT: additional
/// fields first, then the plugins').
pub const USER_INPUT_FIELDS: &[&str] = &[
    "userCode",
    "usedQuota",
    "maxQuota",
    "hideUploads",
    "copyRaw",
    "theme",
    "locale",
    "role",
    "banned",
    "banReason",
    "banExpires",
    "twoFactorEnabled",
];

fn push_bind<'q>(
    q: sqlx::query::QueryAs<'q, Postgres, User, sqlx::postgres::PgArguments>,
    v: &'q SqlValue,
) -> sqlx::query::QueryAs<'q, Postgres, User, sqlx::postgres::PgArguments> {
    match v {
        SqlValue::Null | SqlValue::NullOf(_) => q.bind(None::<String>),
        SqlValue::Text(s) | SqlValue::Cast(s, _) => q.bind(s),
        SqlValue::Bool(b) => q.bind(*b),
        SqlValue::BigInt(n) => q.bind(*n),
        SqlValue::Int(n) => q.bind(*n),
        SqlValue::Date(d) => q.bind(*d),
    }
}

fn placeholder(i: usize, v: &SqlValue) -> String {
    match v {
        SqlValue::Cast(_, ty) | SqlValue::NullOf(ty) => format!("${i}::text::{ty}"),
        _ => format!("${i}"),
    }
}

// ── Users ───────────────────────────────────────────────────────────────────

/// `findUserById`.
pub async fn find_user_by_id(db: &PgPool, id: &str) -> sqlx::Result<Option<User>> {
    if id.is_empty() {
        return Ok(None);
    }
    let sql = format!("SELECT {} FROM {} WHERE \"id\" = $1 LIMIT 1", User::COLUMNS, User::TABLE);
    sqlx::query_as::<_, User>(&sql).bind(id).fetch_optional(db).await
}

/// `findUserByEmail(email, { includeAccounts })`: the address is lowercased.
pub async fn find_user_by_email(db: &PgPool, email: &str) -> sqlx::Result<Option<User>> {
    let sql = format!("SELECT {} FROM {} WHERE \"email\" = $1 LIMIT 1", User::COLUMNS, User::TABLE);
    sqlx::query_as::<_, User>(&sql).bind(email.to_lowercase()).fetch_optional(db).await
}

/// `findAccounts(userId)` (no ORDER BY upstream).
pub async fn find_accounts(db: &PgPool, user_id: &str) -> sqlx::Result<Vec<Account>> {
    let sql = format!("SELECT {} FROM {} WHERE \"user_id\" = $1", Account::COLUMNS, Account::TABLE);
    sqlx::query_as::<_, Account>(&sql).bind(user_id).fetch_all(db).await
}

/// The credential account with a password (`providerId === "credential"`).
pub fn credential_account(accounts: &[Account]) -> Option<&Account> {
    accounts.iter().find(|a| a.provider_id == "credential")
}

/// What a caller asks `createUser` to write (Better Auth field names).
#[derive(Debug, Clone, Default)]
pub struct NewUser {
    /// Field -> value, in the order the caller spread them.
    pub fields: Vec<(String, SqlValue)>,
}

impl NewUser {
    pub fn set(&mut self, field: &str, value: SqlValue) {
        match self.fields.iter_mut().find(|(f, _)| f == field) {
            Some(slot) => slot.1 = value,
            None => self.fields.push((field.to_string(), value)),
        }
    }
    pub fn get(&self, field: &str) -> Option<&SqlValue> {
        self.fields.iter().find(|(f, _)| f == field).map(|(_, v)| v)
    }
}

/// `uniqueUserCode()` (lib/auth.ts): a 6-char pronounceable code, retried on
/// the rare collision.
async fn unique_user_code(tx: &mut Transaction<'_, Postgres>) -> anyhow::Result<String> {
    for _ in 0..50 {
        let code = crate::ids::user_code(6);
        let hit: Option<(String,)> = sqlx::query_as("SELECT \"id\" FROM \"user\" WHERE \"user_code\" = $1 LIMIT 1")
            .bind(&code)
            .fetch_optional(&mut **tx)
            .await?;
        if hit.is_none() {
            return Ok(code);
        }
    }
    anyhow::bail!("Could not allocate a unique user code")
}

/// `internalAdapter.createUser(data)` with the database hooks, in upstream
/// order: the admin plugin's (`role` defaults to "user"), then lib/auth.ts's
/// (first account is admin, else an explicit "admin"/"user" is kept; a
/// unique `user_code`; `max_quota = DEFAULT_USER_QUOTA`), then the adapter's
/// schema defaults. `Err` is any failure (hooks or database), which callers
/// map the way their endpoint does.
pub async fn create_user(db: &PgPool, default_user_quota: i64, data: NewUser) -> anyhow::Result<User> {
    let mut data = data;
    let now = JsDate::now();
    // internalAdapter.createUser: { createdAt, updatedAt, ...user, email: lower }
    let mut ordered = NewUser::default();
    ordered.set("createdAt", SqlValue::Date(now));
    ordered.set("updatedAt", SqlValue::Date(now));
    for (f, v) in std::mem::take(&mut data.fields) {
        ordered.set(&f, v);
    }
    if let Some(SqlValue::Text(email)) = ordered.get("email").cloned() {
        ordered.set("email", SqlValue::Text(email.to_lowercase()));
    }
    // admin(): { role: defaultRole ?? "user", ...user }
    if ordered.get("role").is_none() {
        ordered.set("role", SqlValue::Text("user".into()));
    }

    let mut tx = db.begin().await?;
    // FIX 3: serialise user creation so exactly one creator sees an empty table.
    sqlx::query("SELECT pg_advisory_xact_lock($1)").bind(USER_CREATE_LOCK_KEY).execute(&mut *tx).await?;
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM \"user\"").fetch_one(&mut *tx).await?;
    let is_first = n == 0;
    let supplied = ordered.get("role").cloned();
    let role = if is_first {
        "admin".to_string()
    } else {
        match supplied {
            Some(SqlValue::Text(r)) if r == "admin" || r == "user" => r,
            _ => "user".to_string(),
        }
    };
    ordered.set("role", SqlValue::Text(role));
    ordered.set("userCode", SqlValue::Text(unique_user_code(&mut tx).await?));
    ordered.set("maxQuota", SqlValue::BigInt(default_user_quota));

    // transformInput: schema defaults for fields the data does not carry.
    let defaults: [(&str, SqlValue); 7] = [
        ("emailVerified", SqlValue::Bool(false)),
        ("banned", SqlValue::Bool(false)),
        ("twoFactorEnabled", SqlValue::Bool(false)),
        ("usedQuota", SqlValue::BigInt(0)),
        ("hideUploads", SqlValue::Bool(false)),
        ("copyRaw", SqlValue::Bool(false)),
        ("theme", SqlValue::Text("modern".into())),
    ];
    for (field, value) in defaults {
        if ordered.get(field).is_none() {
            ordered.set(field, value);
        }
    }
    if ordered.get("id").is_none() {
        ordered.set("id", SqlValue::Text(generate_id(32)));
    }

    let mut columns = Vec::new();
    let mut values: Vec<SqlValue> = Vec::new();
    for (field, value) in &ordered.fields {
        let Some(column) = user_field_column(field) else {
            continue;
        };
        let value = match value {
            SqlValue::Cast(text, _) => SqlValue::Cast(text.clone(), user_column_type(column)),
            SqlValue::Null => SqlValue::NullOf(user_column_type(column)),
            other => other.clone(),
        };
        columns.push(format!("\"{column}\""));
        values.push(value);
    }
    let placeholders: Vec<String> = values.iter().enumerate().map(|(i, v)| placeholder(i + 1, v)).collect();
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({}) RETURNING {}",
        User::TABLE,
        columns.join(", "),
        placeholders.join(", "),
        User::COLUMNS
    );
    let mut q = sqlx::query_as::<_, User>(&sql);
    for v in &values {
        q = push_bind(q, v);
    }
    let user = q.fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(user)
}

/// `internalAdapter.updateUser(userId, data)`: the changed columns plus
/// `updated_at`; the updated row, or `None` when no row matched.
pub async fn update_user(db: &PgPool, user_id: &str, patch: &[(&str, SqlValue)]) -> sqlx::Result<Option<User>> {
    update_user_where(db, "id", user_id, patch).await
}

/// `internalAdapter.updateUserByEmail(email, data)`.
pub async fn update_user_by_email(db: &PgPool, email: &str, patch: &[(&str, SqlValue)]) -> sqlx::Result<Option<User>> {
    update_user_where(db, "email", &email.to_lowercase(), patch).await
}

async fn update_user_where(
    db: &PgPool,
    key: &str,
    key_value: &str,
    patch: &[(&str, SqlValue)],
) -> sqlx::Result<Option<User>> {
    let mut sets = Vec::new();
    let mut values: Vec<SqlValue> = Vec::new();
    let mut has_updated_at = false;
    for (field, value) in patch {
        let Some(column) = user_field_column(field) else {
            continue;
        };
        if column == "updated_at" {
            has_updated_at = true;
        }
        let value = match value {
            SqlValue::Cast(text, _) => SqlValue::Cast(text.clone(), user_column_type(column)),
            SqlValue::Null => SqlValue::NullOf(user_column_type(column)),
            SqlValue::Text(email) if column == "email" => SqlValue::Text(email.to_lowercase()),
            other => other.clone(),
        };
        values.push(value);
        sets.push(format!("\"{column}\" = {}", placeholder(values.len(), values.last().expect("pushed"))));
    }
    if !has_updated_at {
        values.push(SqlValue::Date(JsDate::now()));
        sets.push(format!("\"updated_at\" = ${}", values.len()));
    }
    values.push(SqlValue::Text(key_value.to_string()));
    let sql = format!(
        "UPDATE {} SET {} WHERE \"{key}\" = ${} RETURNING {}",
        User::TABLE,
        sets.join(", "),
        values.len(),
        User::COLUMNS
    );
    let mut q = sqlx::query_as::<_, User>(&sql);
    for v in &values {
        q = push_bind(q, v);
    }
    q.fetch_optional(db).await
}

/// `internalAdapter.deleteUser(userId)`: sessions, accounts, then the user.
pub async fn delete_user(db: &PgPool, user_id: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM \"session\" WHERE \"user_id\" = $1").bind(user_id).execute(db).await?;
    sqlx::query("DELETE FROM \"account\" WHERE \"user_id\" = $1").bind(user_id).execute(db).await?;
    sqlx::query("DELETE FROM \"user\" WHERE \"id\" = $1").bind(user_id).execute(db).await?;
    Ok(())
}

// ── Accounts ────────────────────────────────────────────────────────────────

/// `linkAccount` / `createAccount`: a new `account` row.
pub async fn create_account(
    db: &PgPool,
    user_id: &str,
    provider_id: &str,
    account_id: &str,
    password: Option<&str>,
) -> sqlx::Result<Account> {
    let now = JsDate::now();
    let sql = format!(
        "INSERT INTO {} (\"id\", \"account_id\", \"provider_id\", \"user_id\", \"password\", \"created_at\", \"updated_at\") VALUES ($1, $2, $3, $4, $5, $6, $6) RETURNING {}",
        Account::TABLE,
        Account::COLUMNS
    );
    sqlx::query_as::<_, Account>(&sql)
        .bind(generate_id(32))
        .bind(account_id)
        .bind(provider_id)
        .bind(user_id)
        .bind(password)
        .bind(now)
        .fetch_one(db)
        .await
}

/// `updatePassword(userId, hash)`: every credential account of the user.
pub async fn update_password(db: &PgPool, user_id: &str, hash: &str) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE \"account\" SET \"password\" = $1, \"updated_at\" = $2 WHERE \"user_id\" = $3 AND \"provider_id\" = 'credential'",
    )
    .bind(hash)
    .bind(JsDate::now())
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(())
}

/// `updateAccount(id, { password })`.
pub async fn update_account_password(db: &PgPool, id: &str, hash: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE \"account\" SET \"password\" = $1, \"updated_at\" = $2 WHERE \"id\" = $3")
        .bind(hash)
        .bind(JsDate::now())
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn delete_account(db: &PgPool, id: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM \"account\" WHERE \"id\" = $1").bind(id).execute(db).await?;
    Ok(())
}

// ── Sessions ────────────────────────────────────────────────────────────────

/// What `createSession` copies from an existing session (`override`).
#[derive(Debug, Clone, Default)]
pub struct SessionOverride {
    pub ip_address: Option<Option<String>>,
    pub user_agent: Option<Option<String>>,
    pub impersonated_by: Option<Option<String>>,
    pub expires_at: Option<JsDate>,
}

impl SessionOverride {
    /// `{ id: _, ...rest } = session`: everything of an existing row but its
    /// id (the token, dates and user id are always replaced).
    pub fn from_session(s: &Session) -> SessionOverride {
        SessionOverride {
            ip_address: Some(s.ip_address.clone()),
            user_agent: Some(s.user_agent.clone()),
            impersonated_by: Some(s.impersonated_by.clone()),
            expires_at: None,
        }
    }
}

/// Why `createSession` refused.
#[derive(Debug)]
pub enum CreateSessionError {
    /// The admin plugin's hook: `FORBIDDEN` `{message: bannedUserMessage, code: "BANNED_USER"}`.
    Banned,
    Db(anyhow::Error),
}

impl From<sqlx::Error> for CreateSessionError {
    fn from(e: sqlx::Error) -> Self {
        CreateSessionError::Db(e.into())
    }
}

impl From<anyhow::Error> for CreateSessionError {
    fn from(e: anyhow::Error) -> Self {
        CreateSessionError::Db(e)
    }
}

/// `internalAdapter.createSession(userId, dontRememberMe, override, overrideAll)`
/// with the admin plugin's `session.create.before` hook. `ip` / `user_agent`
/// come from the request (`getIp(headers) || ""`, `user-agent || ""`); an
/// `override` wins over them, and `override_all` lets it win over the
/// computed expiry too (impersonation).
pub async fn create_session(
    db: &PgPool,
    user_id: &str,
    dont_remember: bool,
    ip: &str,
    user_agent: &str,
    over: Option<&SessionOverride>,
    override_all: bool,
) -> Result<Session, CreateSessionError> {
    // admin(): a banned account gets no session unless the ban has expired.
    if let Err(_message) = super::session::check_ban_before_session_create(db, user_id).await? {
        return Err(CreateSessionError::Banned);
    }
    let now = JsDate::now();
    let lifetime = if dont_remember { 3600 * 24 } else { super::cookies::SESSION_EXPIRES_IN };
    let mut expires_at = JsDate::from_millis(now.millis() + lifetime * 1000).unwrap_or(now);
    let mut ip_address = Some(ip.to_string());
    let mut agent = Some(user_agent.to_string());
    let mut impersonated_by: Option<String> = None;
    if let Some(o) = over {
        if let Some(v) = &o.ip_address {
            ip_address = v.clone();
        }
        if let Some(v) = &o.user_agent {
            agent = v.clone();
        }
        if let Some(v) = &o.impersonated_by {
            impersonated_by = v.clone();
        }
        if override_all && let Some(e) = o.expires_at {
            expires_at = e;
        }
    }
    let sql = format!(
        "INSERT INTO {} (\"id\", \"expires_at\", \"token\", \"created_at\", \"updated_at\", \"ip_address\", \"user_agent\", \"user_id\", \"impersonated_by\") VALUES ($1, $2, $3, $4, $4, $5, $6, $7, $8) RETURNING {}",
        Session::TABLE,
        Session::COLUMNS
    );
    Ok(sqlx::query_as::<_, Session>(&sql)
        .bind(generate_id(32))
        .bind(expires_at)
        .bind(generate_id(32))
        .bind(now)
        .bind(ip_address)
        .bind(agent)
        .bind(user_id)
        .bind(impersonated_by)
        .fetch_one(db)
        .await?)
}

/// `findSession(token)` (session row + user).
pub async fn find_session(db: &PgPool, token: &str) -> anyhow::Result<Option<AuthSession>> {
    super::session::find_session(db, token).await
}

pub async fn delete_session(db: &PgPool, token: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM \"session\" WHERE \"token\" = $1").bind(token).execute(db).await?;
    Ok(())
}

/// `deleteUserSessions(userId)`.
pub async fn delete_user_sessions(db: &PgPool, user_id: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM \"session\" WHERE \"user_id\" = $1").bind(user_id).execute(db).await?;
    Ok(())
}

/// `listSessions(userId, { onlyActiveSessions })`.
pub async fn list_sessions(db: &PgPool, user_id: &str, only_active: bool) -> sqlx::Result<Vec<Session>> {
    if only_active {
        let sql = format!(
            "SELECT {} FROM {} WHERE \"user_id\" = $1 AND \"expires_at\" > $2",
            Session::COLUMNS,
            Session::TABLE
        );
        return sqlx::query_as::<_, Session>(&sql).bind(user_id).bind(JsDate::now()).fetch_all(db).await;
    }
    let sql = format!("SELECT {} FROM {} WHERE \"user_id\" = $1", Session::COLUMNS, Session::TABLE);
    sqlx::query_as::<_, Session>(&sql).bind(user_id).fetch_all(db).await
}

// ── Verification values ─────────────────────────────────────────────────────

/// `createVerificationValue({ identifier, value, expiresAt })`.
pub async fn create_verification_value(
    db: &PgPool,
    identifier: &str,
    value: &str,
    expires_at: JsDate,
) -> sqlx::Result<Verification> {
    let now = JsDate::now();
    let sql = format!(
        "INSERT INTO {} (\"id\", \"identifier\", \"value\", \"expires_at\", \"created_at\", \"updated_at\") VALUES ($1, $2, $3, $4, $5, $5) RETURNING {}",
        Verification::TABLE,
        Verification::COLUMNS
    );
    sqlx::query_as::<_, Verification>(&sql)
        .bind(generate_id(32))
        .bind(identifier)
        .bind(value)
        .bind(expires_at)
        .bind(now)
        .fetch_one(db)
        .await
}

/// `findVerificationValue(identifier)`: the newest row for the identifier,
/// after which EVERY expired verification row is deleted (Better Auth's
/// cleanup; invitations included).
pub async fn find_verification_value(db: &PgPool, identifier: &str) -> sqlx::Result<Option<Verification>> {
    let sql = format!(
        "SELECT {} FROM {} WHERE \"identifier\" = $1 ORDER BY \"created_at\" DESC LIMIT 1",
        Verification::COLUMNS,
        Verification::TABLE
    );
    let row = sqlx::query_as::<_, Verification>(&sql).bind(identifier).fetch_optional(db).await?;
    sqlx::query("DELETE FROM \"verification\" WHERE \"expires_at\" < $1").bind(JsDate::now()).execute(db).await?;
    Ok(row)
}

/// `deleteVerificationByIdentifier(identifier)`.
pub async fn delete_verification_by_identifier(db: &PgPool, identifier: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM \"verification\" WHERE \"identifier\" = $1").bind(identifier).execute(db).await?;
    Ok(())
}

/// `updateVerificationByIdentifier(identifier, { value })`.
pub async fn update_verification_value(db: &PgPool, identifier: &str, value: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE \"verification\" SET \"value\" = $1, \"updated_at\" = $2 WHERE \"identifier\" = $3")
        .bind(value)
        .bind(JsDate::now())
        .bind(identifier)
        .execute(db)
        .await?;
    Ok(())
}

// ── two_factor ──────────────────────────────────────────────────────────────

pub async fn find_two_factor(db: &PgPool, user_id: &str) -> sqlx::Result<Option<TwoFactor>> {
    let sql = format!("SELECT {} FROM {} WHERE \"user_id\" = $1 LIMIT 1", TwoFactor::COLUMNS, TwoFactor::TABLE);
    sqlx::query_as::<_, TwoFactor>(&sql).bind(user_id).fetch_optional(db).await
}

pub async fn delete_two_factor(db: &PgPool, user_id: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM \"two_factor\" WHERE \"user_id\" = $1").bind(user_id).execute(db).await?;
    Ok(())
}

pub async fn create_two_factor(
    db: &PgPool,
    user_id: &str,
    secret: &str,
    backup_codes: &str,
    verified: bool,
) -> sqlx::Result<TwoFactor> {
    let sql = format!(
        "INSERT INTO {} (\"id\", \"secret\", \"backup_codes\", \"user_id\", \"verified\") VALUES ($1, $2, $3, $4, $5) RETURNING {}",
        TwoFactor::TABLE,
        TwoFactor::COLUMNS
    );
    sqlx::query_as::<_, TwoFactor>(&sql)
        .bind(generate_id(32))
        .bind(secret)
        .bind(backup_codes)
        .bind(user_id)
        .bind(verified)
        .fetch_one(db)
        .await
}

/// Users as the admin plugin lists them (`listUsers`), filtered and sorted by
/// Better Auth field names; unknown fields are an error (upstream's adapter
/// throws, which the endpoint answers with an empty list).
pub fn object_to_user_patch(data: &Map<String, Value>) -> Vec<(String, SqlValue)> {
    data.iter()
        .filter(|(k, _)| user_field_column(k).is_some())
        .map(|(k, v)| {
            let value = match pg_text(v) {
                None => SqlValue::Null,
                Some(text) => SqlValue::Cast(text, "text"),
            };
            (k.clone(), value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn node_postgres_text_forms() {
        assert_eq!(pg_text(&json!(true)).as_deref(), Some("true"));
        assert_eq!(pg_text(&json!(1.5)).as_deref(), Some("1.5"));
        assert_eq!(pg_text(&json!(["a", "b\"c", null])).as_deref(), Some(r#"{"a","b\"c",NULL}"#));
        assert_eq!(pg_text(&json!({"a": 1})).as_deref(), Some(r#"{"a":1}"#));
        assert_eq!(pg_text(&json!(null)), None);
    }
}
