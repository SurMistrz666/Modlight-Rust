//! Database compatibility against a real PostgreSQL (the dev database).
//!
//! Runs only when `MODLIGHT_TEST_DATABASE_URL` points at a server where the
//! user may create databases, e.g. the dev database of scripts/dev.sh:
//!
//! ```text
//! MODLIGHT_TEST_DATABASE_URL=postgresql://modlight:<pw>@127.0.0.1:55421/modlight cargo test --test db_compat
//! ```
//!
//! Each run creates a throwaway database, migrates it with the drizzle-compatible
//! migrator, checks every model against `information_schema`, exercises the
//! session reader and the API-token store, and drops the database again.

use std::str::FromStr;

use axum::http::{HeaderMap, HeaderValue, header};
use modlight_server::api_scopes::{TokenCapability, TokenScope};
use modlight_server::api_token::{
    NarrowResult, TokenAuth, authenticate_api_token, create_api_token, narrow_api_token_scope, revoke_api_token,
};
use modlight_server::auth::cookies::{AuthCookies, SignedCookie, sign_cookie_value, verify_signed_cookie};
use modlight_server::auth::session::{RefreshPolicy, get_session};
use modlight_server::db::models::{self, Columns, JsDate};
use modlight_server::paths::Paths;
use sqlx::postgres::PgConnectOptions;
use sqlx::{ConnectOptions, PgPool};

const SECRET: &str = "test-secret-test-secret-test-secret-0123";

struct TempDb {
    admin: PgConnectOptions,
    name: String,
    pool: PgPool,
}

impl TempDb {
    async fn create(url: &str) -> TempDb {
        let admin = PgConnectOptions::from_str(url).expect("MODLIGHT_TEST_DATABASE_URL");
        let name = format!("modlight_test_{}_{}", std::process::id(), rand::random::<u32>());
        let mut conn = admin.connect().await.expect("connect to the test server");
        sqlx::query(&format!("CREATE DATABASE \"{name}\"")).execute(&mut conn).await.expect("create database");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect_with(admin.clone().database(&name).options([("TimeZone", "UTC")]))
            .await
            .expect("connect to the temp database");
        TempDb { admin, name, pool }
    }

    async fn drop(self) {
        self.pool.close().await;
        if let Ok(mut conn) = self.admin.connect().await {
            let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)", self.name))
                .execute(&mut conn)
                .await;
        }
    }
}

fn cookie_headers(cookies: &[(&str, String)]) -> HeaderMap {
    let mut h = HeaderMap::new();
    let joined: Vec<String> = cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
    h.insert(header::COOKIE, HeaderValue::from_str(&joined.join("; ")).unwrap());
    h
}

#[tokio::test]
async fn database_compatibility() {
    let Ok(url) = std::env::var("MODLIGHT_TEST_DATABASE_URL") else {
        eprintln!("MODLIGHT_TEST_DATABASE_URL is not set: skipping the database tests");
        return;
    };
    let db = TempDb::create(&url).await;
    let result = std::panic::AssertUnwindSafe(run(&db.pool));
    let outcome = futures_util::FutureExt::catch_unwind(result).await;
    db.drop().await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

async fn run(pool: &PgPool) {
    // ── migrations ──
    let dir = Paths::crate_dir().migrations;
    let report = modlight_server::db::migrate::run(pool, &dir, true).await.unwrap();
    assert_eq!(report.applied.len(), 16);
    let again = modlight_server::db::migrate::run(pool, &dir, false).await.unwrap();
    assert!(again.applied.is_empty());
    let seeded: String =
        sqlx::query_scalar("select value from settings where key = 'register_enabled'").fetch_one(pool).await.unwrap();
    assert_eq!(seeded, "on", "the seed is a starting value: the second run must not overwrite it");

    // ── every model matches the database, column for column, in order ──
    for (table, columns) in models::ALL_TABLES {
        let (schema, name) = match table.split_once('.') {
            Some((s, t)) => (s.trim_matches('"'), t.trim_matches('"')),
            None => ("public", table.trim_matches('"')),
        };
        let actual: Vec<String> = sqlx::query_scalar(
            "select column_name::text from information_schema.columns where table_schema = $1 and table_name = $2 order by ordinal_position",
        )
        .bind(schema)
        .bind(name)
        .fetch_all(pool)
        .await
        .unwrap();
        let expected: Vec<String> = columns.split(", ").map(|c| c.trim_matches('"').to_string()).collect();
        assert_eq!(actual, expected, "columns of {table}");
    }
    let tables: Vec<String> = sqlx::query_scalar(
        "select table_name::text from information_schema.tables where table_schema = 'public' order by 1",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let modelled: Vec<String> = models::ALL_TABLES
        .iter()
        .filter(|(t, _)| !t.contains('.'))
        .map(|(t, _)| t.trim_matches('"').to_string())
        .collect();
    for t in &tables {
        assert!(modelled.contains(t), "table {t} has no model");
    }

    // ── rows decode into the models ──
    sqlx::query("insert into \"user\" (id, name, email, user_code, theme) values ('u1', 'Alice', 'alice@example.test', 'alice', 'modern')")
        .execute(pool)
        .await
        .unwrap();
    let user: models::User =
        sqlx::query_as(&format!("select {} from {} where id = 'u1'", models::User::COLUMNS, models::User::TABLE))
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(user.background_dim, 70);
    assert_eq!(user.max_quota, Some(-1));
    for (table, columns) in models::ALL_TABLES {
        sqlx::query(&format!("select {columns} from {table} limit 1")).fetch_all(pool).await.unwrap();
    }

    // ── session reader ──
    let cookies = AuthCookies::new(false, SECRET);
    let token = "sessiontoken0123456789abcdefghij";
    sqlx::query(
        "insert into \"session\" (id, expires_at, token, updated_at, user_id) values ('s1', (now() at time zone 'utc') + interval '5 days', $1, now() at time zone 'utc', 'u1')",
    )
    .bind(token)
    .execute(pool)
    .await
    .unwrap();
    let signed = sign_cookie_value(token, SECRET);
    let headers = cookie_headers(&[("better-auth.session_token", signed.clone())]);
    let days_left = || async {
        let v: f64 = sqlx::query_scalar(
            "select (extract(epoch from (expires_at - (now() at time zone 'utc'))) / 86400.0)::float8 from session where token = $1",
        )
        .bind(token)
        .fetch_one(pool)
        .await
        .unwrap();
        v
    };

    // Client-side navigation: no refresh.
    let s = get_session(pool, &cookies, &headers, RefreshPolicy::Skip).await.unwrap();
    assert_eq!(s.session.as_ref().unwrap().user.id, "u1");
    assert!(s.cookies.is_empty());
    assert!((days_left().await - 5.0).abs() < 0.01);

    // Page render: refresh the row, no cookie.
    let s = get_session(pool, &cookies, &headers, RefreshPolicy::DbOnly).await.unwrap();
    assert!(s.session.is_some());
    assert!(s.cookies.is_empty());
    assert!((days_left().await - 7.0).abs() < 0.01);

    // Fresh session: nothing to refresh.
    let s = get_session(pool, &cookies, &headers, RefreshPolicy::DbAndCookie).await.unwrap();
    assert!(s.cookies.is_empty());

    // Action / route handler: refresh and re-issue the same signed cookie.
    sqlx::query("update session set expires_at = (now() at time zone 'utc') + interval '5 days' where token = $1")
        .bind(token)
        .execute(pool)
        .await
        .unwrap();
    let s = get_session(pool, &cookies, &headers, RefreshPolicy::DbAndCookie).await.unwrap();
    assert_eq!(s.cookies.len(), 1);
    let c = &s.cookies[0];
    assert_eq!(c.name, "better-auth.session_token");
    assert!(matches!(c.max_age, Some(m) if (604_790..=604_800).contains(&m)));
    assert_eq!(verify_signed_cookie(Some(&c.value), SECRET), SignedCookie::Valid(token.into()));
    assert!((days_left().await - 7.0).abs() < 0.01);

    // A forged signature is no session and touches nothing.
    let forged = cookie_headers(&[("better-auth.session_token", sign_cookie_value(token, "wrong-secret"))]);
    let s = get_session(pool, &cookies, &forged, RefreshPolicy::DbAndCookie).await.unwrap();
    assert!(s.session.is_none() && s.cookies.is_empty());

    // An unknown token expires the cookies.
    let unknown = cookie_headers(&[("better-auth.session_token", sign_cookie_value("nope", SECRET))]);
    let s = get_session(pool, &cookies, &unknown, RefreshPolicy::DbAndCookie).await.unwrap();
    assert!(s.session.is_none());
    let names: Vec<&str> = s.cookies.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["better-auth.session_token", "better-auth.session_data", "better-auth.dont_remember"]);

    // An expired session is deleted.
    sqlx::query("update session set expires_at = (now() at time zone 'utc') - interval '1 minute' where token = $1")
        .bind(token)
        .execute(pool)
        .await
        .unwrap();
    let s = get_session(pool, &cookies, &headers, RefreshPolicy::DbOnly).await.unwrap();
    assert!(s.session.is_none());
    let left: i64 =
        sqlx::query_scalar("select count(*) from session where token = $1").bind(token).fetch_one(pool).await.unwrap();
    assert_eq!(left, 0);

    // ── API tokens ──
    let plain = create_api_token(pool, "u1", "laptop", TokenScope::Write).await.unwrap();
    let row: models::ApiToken = sqlx::query_as(&format!(
        "select {} from {} where user_id = 'u1'",
        models::ApiToken::COLUMNS,
        models::ApiToken::TABLE
    ))
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(row.last_used_at.is_none());
    assert_eq!(row.preview, plain[plain.len() - 6..]);
    let auth = authenticate_api_token(pool, Some(&plain), TokenCapability::Read).await.unwrap();
    assert!(matches!(auth, TokenAuth::Ok { scope: TokenScope::Write, .. }));
    let stamped: Option<JsDate> = sqlx::query_scalar("select last_used_at from api_tokens where id = $1")
        .bind(row.id)
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(stamped.is_some(), "last_used_at is stamped by the same statement");
    let auth = authenticate_api_token(pool, Some(&plain), TokenCapability::Admin).await.unwrap();
    assert!(matches!(auth, TokenAuth::Scope { .. }));
    let id = row.id.to_string();
    assert_eq!(narrow_api_token_scope(pool, &id, "u1", TokenScope::Full).await.unwrap(), NarrowResult::NotNarrower);
    assert_eq!(
        narrow_api_token_scope(pool, &id, "u1", TokenScope::Read).await.unwrap(),
        NarrowResult::Ok(TokenScope::Read)
    );
    assert_eq!(
        narrow_api_token_scope(pool, &id, "someone-else", TokenScope::Upload).await.unwrap(),
        NarrowResult::NotFound
    );
    revoke_api_token(pool, &id, "u1").await.unwrap();
    assert_eq!(authenticate_api_token(pool, Some(&plain), TokenCapability::Read).await.unwrap(), TokenAuth::Unknown);
    // The CHECK constraint refuses scopes this build does not know.
    let bad = sqlx::query("insert into api_tokens (user_id, token_hash, scope) values ('u1', 'h', 'superuser')")
        .execute(pool)
        .await;
    assert!(bad.is_err());
}
