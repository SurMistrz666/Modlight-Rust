//! The account actions, the session list and the /dashboard/profile loader
//! against a real PostgreSQL (the differential checks against the reference
//! are in tests/parity/account.py; this pins the same behaviour without one).
//!
//! Runs only when `MODLIGHT_TEST_DATABASE_URL` points at a server where the
//! user may create databases (see tests/db_compat.rs); each run creates and
//! drops its own database. The `(app)` layout is another port's, so the page
//! is loaded through a registry with a stand-in for it.

mod common;

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use common::{Answer, call};
use modlight_server::auth::cookies::sign_cookie_value;
use modlight_server::auth::session::find_session;
use modlight_server::config::Config;
use modlight_server::data::sessions::{list_my_sessions, my_session_token};
use modlight_server::http::App;
use modlight_server::pages::{Layout, PageCtx, PageRegistry, dashboard_profile, root_layout};
use modlight_server::paths::Paths;
use modlight_server::state::AppState;
use serde_json::{Value, json};
use sqlx::postgres::PgConnectOptions;
use sqlx::{ConnectOptions, PgPool};

const SECRET: &str = "account-db-test-secret-0123456789abcdef";
const ORIGIN: &str = "http://127.0.0.1:8421";

struct TempDb {
    admin: PgConnectOptions,
    name: String,
    pool: PgPool,
}

impl TempDb {
    async fn create(url: &str) -> TempDb {
        let admin = PgConnectOptions::from_str(url).expect("MODLIGHT_TEST_DATABASE_URL");
        let name = format!("modlight_test_account_{}_{}", std::process::id(), rand::random::<u32>());
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

#[tokio::test]
async fn account_actions_and_profile_page() {
    let Ok(url) = std::env::var("MODLIGHT_TEST_DATABASE_URL") else {
        eprintln!("MODLIGHT_TEST_DATABASE_URL is not set: skipping the account database tests");
        return;
    };
    let db = TempDb::create(&url).await;
    let outcome = futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(run(&db.pool))).await;
    db.drop().await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

fn app(pool: &PgPool) -> App {
    let vars: HashMap<String, String> =
        [("APP_URL", ORIGIN), ("APP_DOMAINS", "https://H.example.com"), ("AUTH_SECRET", SECRET)]
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
    let config = Config::from_env_map(&vars).expect("config");
    let mut state = AppState::new(config, Paths::crate_dir(), pool.clone()).expect("state");
    let mut pages = PageRegistry::default();
    pages.layout(Layout::Root, root_layout::load);
    // Stand-in for the (app) layout: its gate only.
    pages.layout(Layout::App, |ctx: PageCtx| async move {
        ctx.require_user().await?;
        Ok(json!({}))
    });
    pages.page("/dashboard/profile", dashboard_profile::load);
    state.pages = Arc::new(pages);
    App::new(state)
}

fn cookie(token: &str) -> String {
    format!("better-auth.session_token={}", sign_cookie_value(token, SECRET))
}

async fn rpc(app: &App, token: &str, export: &str, args: Value) -> Answer {
    let req = Request::post(format!("/_rpc/account/{export}"))
        .header("host", "127.0.0.1:8421")
        .header("origin", ORIGIN)
        .header("x-modlight-rpc", "1")
        .header("content-type", "application/json")
        .header("cookie", cookie(token))
        .body(Body::from(args.to_string()))
        .unwrap();
    call(app, req).await
}

async fn ok(app: &App, token: &str, export: &str, args: Value) -> Value {
    let a = rpc(app, token, export, args).await;
    let body = a.json();
    assert!(body.get("ok").is_some(), "{export}: {body}");
    body
}

async fn exec(pool: &PgPool, sql: &str) {
    sqlx::raw_sql(sql).execute(pool).await.unwrap_or_else(|e| panic!("{sql}: {e}"));
}

async fn scalar<T>(pool: &PgPool, sql: &str) -> T
where
    T: for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres> + Send + Unpin,
{
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap_or_else(|e| panic!("{sql}: {e}"))
}

const F1: &str = "11111111-1111-4111-8111-111111111111";
const F2: &str = "22222222-2222-4222-8222-222222222222";
const F3: &str = "33333333-3333-4333-8333-333333333333";
const F4: &str = "44444444-4444-4444-8444-444444444444";

async fn seed(pool: &PgPool) {
    exec(
        pool,
        r#"
        INSERT INTO "user" ("id", "name", "email", "email_verified", "role", "user_code", "theme", "created_at", "updated_at",
                            "image", "background_key", "background_dim", "default_password_hash",
                            "default_password_version", "naming_scheme", "naming_pattern", "max_quota", "used_quota")
        VALUES ('adm', 'Ada', 'ada@example.test', true, 'admin', 'admtag', 'modern',
                now() - interval '3 hours', now() - interval '3 hours', NULL, NULL, 70, NULL, 0, 'random', NULL, -1, 0),
               ('u1', 'Alicja', 'alicja@example.test', true, 'user', 'alicja', 'modern',
                now() - interval '2 hours', now() - interval '2 hours', 'avatars/u1-tok.webp',
                'backgrounds/u1-bg.webp', 40, 'scrypt$a$b', 1, 'custom', '{name}', 1073741824, 7),
               ('u2', 'Bob', 'bob@example.test', false, 'user', 'bob', NULL,
                now() - interval '1 hour', now() - interval '1 hour', NULL, NULL, 70, NULL, 0, NULL, NULL, NULL, NULL);
        INSERT INTO "user_code_aliases" ("code", "user_id") VALUES ('old-bob', 'u2');
        INSERT INTO "folders" ("id", "owner_id", "parent_id", "name", "slug", "sort_order") VALUES
            ('11111111-1111-4111-8111-111111111111', 'u1', NULL, 'A', 'a', 0),
            ('22222222-2222-4222-8222-222222222222', 'u1', '11111111-1111-4111-8111-111111111111', 'B', 'b', 0),
            ('33333333-3333-4333-8333-333333333333', 'u1', NULL, 'C', 'c', 1),
            ('44444444-4444-4444-8444-444444444444', 'u2', NULL, 'D', 'd', 0);
        INSERT INTO "uploads" ("owner_id", "code", "filename", "mime_type", "size", "storage_key", "delete_token", "views",
                               "folder_id", "password_hash", "password_version", "deleted_at") VALUES
            ('u1', 'c1', 'a.png', 'image/png', 100, 'k1', 'd1', 3, '11111111-1111-4111-8111-111111111111', 'scrypt$a$b', 1, NULL),
            ('u1', 'c2', 'b.png', 'image/png', 200, 'k2', 'd2', 4, NULL, 'scrypt$own$pw', NULL, NULL),
            ('u1', 'c3', 'c.png', 'image/png', 300, 'k3', 'd3', 5, NULL, 'scrypt$a$b', 1, now()),
            ('u2', 'c4', 'd.png', 'image/png', 400, 'k4', 'd4', 0, NULL, NULL, NULL, NULL);
        INSERT INTO "session" ("id", "token", "user_id", "created_at", "updated_at", "expires_at", "ip_address",
                               "user_agent", "impersonated_by") VALUES
            ('s-cur', 'tok-cur', 'u1', (now() at time zone 'utc') - interval '1 hour',
             (now() at time zone 'utc') - interval '1 hour', (now() at time zone 'utc') + interval '7 days' - interval '1 hour',
             ' 203.0.113.7 ', 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/141.0.0.0 Safari/537.36', NULL),
            ('s-old', 'tok-old', 'u1', (now() at time zone 'utc') - interval '2 days',
             (now() at time zone 'utc') - interval '2 days', (now() at time zone 'utc') + interval '5 days', '',
             'Mozilla/5.0 (iPhone; CPU iPhone OS 18_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.2 Mobile/15E148 Safari/604.1', NULL),
            ('s-imp', 'tok-imp', 'u1', (now() at time zone 'utc') - interval '3 hours',
             (now() at time zone 'utc') - interval '3 hours', (now() at time zone 'utc') + interval '5 days', NULL, 'curl/8.9.1', 'adm'),
            ('s-exp', 'tok-exp', 'u1', (now() at time zone 'utc') - interval '9 days',
             (now() at time zone 'utc') - interval '9 days', (now() at time zone 'utc') - interval '1 day', NULL, NULL, NULL),
            ('s-bob', 'tok-bob', 'u2', (now() at time zone 'utc') - interval '1 hour',
             (now() at time zone 'utc') - interval '1 hour', (now() at time zone 'utc') + interval '7 days' - interval '1 hour', NULL, NULL, NULL),
            ('s-adm', 'tok-adm', 'adm', (now() at time zone 'utc') - interval '1 hour',
             (now() at time zone 'utc') - interval '1 hour', (now() at time zone 'utc') + interval '7 days' - interval '1 hour', NULL, NULL, NULL);
        "#,
    )
    .await;
}

async fn run(pool: &PgPool) {
    let dir = Paths::crate_dir().migrations;
    modlight_server::db::migrate::run(pool, &dir, true).await.unwrap();
    seed(pool).await;
    let app = app(pool);

    profile_page(&app, pool).await;
    session_list(pool).await;
    preferences(&app, pool).await;
    tags(&app, pool).await;
    default_password(&app, pool).await;
    folders_and_domains(&app, pool).await;
    admin(&app, pool).await;
    sessions(&app, pool).await;
}

async fn profile_page(app: &App, pool: &PgPool) {
    let req = Request::get("/_page?path=%2Fdashboard%2Fprofile")
        .header("host", "127.0.0.1:8421")
        .header("cookie", cookie("tok-cur"))
        .body(Body::empty())
        .unwrap();
    let page = call(app, req).await.json();
    assert_eq!(page["route"], "/dashboard/profile", "{page}");
    assert_eq!(page["meta"]["title"], "Profile - Modlight");
    let p = &page["props"];
    assert_eq!(
        p["u"],
        json!({
            "name": "Alicja", "email": "alicja@example.test", "userCode": "alicja", "hideUploads": false,
            "copyRaw": false, "usedQuota": 7, "maxQuota": 1073741824, "role": "user", "theme": "modern",
            "locale": null, "twoFactorEnabled": false, "emailVerified": true
        })
    );
    // Trashed files are not in the stats.
    assert_eq!(p["stats"], json!({ "count": 2, "totalSize": 300, "totalViews": 7 }));
    assert_eq!(
        p["prefs"],
        json!({ "userCode": "alicja", "skipTagInUrl": false, "defaultFolderId": null, "backgroundDim": 40 })
    );
    assert_eq!(
        p["urlPrefs"],
        json!({
            "scheme": "custom", "pattern": "{name}", "wordCount": 2, "randomLength": 6,
            "preferredDomain": null, "baseUrl": ORIGIN
        })
    );
    assert_eq!(
        p["folders"],
        json!([
            { "id": F1, "name": "A", "depth": 0 },
            { "id": F2, "name": "B", "depth": 1 },
            { "id": F3, "name": "C", "depth": 0 }
        ])
    );
    assert_eq!(p["avatarUrl"], "/avatar/alicja?v=tok");
    assert_eq!(p["backgroundUrl"], "/bg/alicja?v=bg");
    // This device first, then the newest sign-in; expired and impersonated
    // sessions are not listed.
    let sessions = p["sessions"].as_array().unwrap();
    let ids: Vec<&str> = sessions.iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["s-cur", "s-old"]);
    assert_eq!(sessions[0]["current"], true);
    assert_eq!(sessions[0]["ip"], "203.0.113.7");
    assert_eq!(sessions[0]["device"], json!({ "browser": "Chrome 141", "os": "Windows", "kind": "desktop" }));
    assert_eq!(sessions[1]["ip"], Value::Null);
    assert_eq!(sessions[1]["device"], json!({ "browser": "Safari 18", "os": "iOS 18", "kind": "mobile" }));
    let text = p.to_string();
    for secret in ["tok-cur", "tok-old", "tok-imp", "backgrounds/u1-bg.webp", "avatars/u1-tok.webp", "scrypt$"] {
        assert!(!text.contains(secret), "{secret} leaked into the page data");
    }

    // Signed out: the gate redirects before the page loads.
    let req = Request::get("/_page?path=%2Fdashboard%2Fprofile")
        .header("host", "127.0.0.1:8421")
        .header("cookie", "better-auth.session_token=garbage")
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(app, req).await.json(), json!({ "redirect": "/login", "status": 307 }));
    let _ = pool;
}

async fn session_list(pool: &PgPool) {
    // tests/unit/sessions.test.ts describe("mySessionToken"), on real rows.
    let me = find_session(pool, "tok-cur").await.unwrap().unwrap();
    assert_eq!(my_session_token(pool, &me, "s-old").await.unwrap().as_deref(), Some("tok-old"));
    // Somebody else's session is not in the caller's list: the ownership check.
    assert_eq!(my_session_token(pool, &me, "s-bob").await.unwrap(), None);
    assert_eq!(my_session_token(pool, &me, "").await.unwrap(), None);
    // Impersonated (admin plugin filter) and expired sessions are not either.
    assert_eq!(my_session_token(pool, &me, "s-imp").await.unwrap(), None);
    assert_eq!(my_session_token(pool, &me, "s-exp").await.unwrap(), None);
    let views = list_my_sessions(pool, &me).await.unwrap();
    assert_eq!(views.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), vec!["s-cur", "s-old"]);
}

async fn preferences(app: &App, pool: &PgPool) {
    let a =
        rpc(app, "tok-cur", "updatePreferences", json!([{ "name": " Ala ", "theme": "glass", "locale": null }])).await;
    assert_eq!(a.json(), json!({ "ok": { "theme": "glass", "locale": null }, "revalidate": true }));
    let cookies: Vec<String> =
        a.headers.get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect();
    assert_eq!(cookies.len(), 2, "{cookies:?}");
    assert!(cookies[0].starts_with("ml_skin=glass; Path=/; Expires="), "{}", cookies[0]);
    assert!(cookies[0].ends_with("; Max-Age=31536000; SameSite=lax"), "{}", cookies[0]);
    // Next's cookies().delete(): an epoch Expires and no Max-Age.
    assert_eq!(cookies[1], "ml_locale=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT");
    let (name, theme): (String, Option<String>) =
        sqlx::query_as(r#"SELECT "name", "theme" FROM "user" WHERE "id" = 'u1'"#).fetch_one(pool).await.unwrap();
    assert_eq!((name.as_str(), theme.as_deref()), ("Ala", Some("glass")));

    // decodeReply drops an undefined key: nothing written, no cookie, no revalidate.
    exec(pool, r#"UPDATE "user" SET "updated_at" = '2000-01-01' WHERE "id" = 'u1'"#).await;
    let a = rpc(app, "tok-cur", "updatePreferences", json!([{ "locale": { "$undefined": 1 } }])).await;
    assert_eq!(a.json(), json!({ "ok": { "theme": "glass", "locale": null } }));
    assert!(a.headers.get("set-cookie").is_none());
    assert!(!scalar::<bool>(pool, r#"SELECT "updated_at" > '2001-01-01' FROM "user" WHERE "id" = 'u1'"#).await);

    let a = rpc(app, "tok-cur", "updatePreferences", json!([{ "locale": "pl" }])).await;
    let set: Vec<String> = a.headers.get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect();
    assert!(set.len() == 1 && set[0].starts_with("ml_locale=pl; Path=/; Expires="), "{set:?}");
    assert!(scalar::<bool>(pool, r#"SELECT "updated_at" > '2001-01-01' FROM "user" WHERE "id" = 'u1'"#).await);

    // Development mode sends the real message of a TypeError.
    let a = rpc(app, "tok-cur", "updatePreferences", json!([null])).await;
    assert_eq!(a.json()["error"]["message"], "Cannot read properties of null (reading 'name')");

    // Background dim and public profile write their columns.
    assert_eq!(ok(app, "tok-cur", "updateBackgroundDim", json!(["12.5"])).await["ok"], json!({ "dim": 13 }));
    assert_eq!(
        ok(app, "tok-cur", "myBackground", json!([])).await["ok"],
        json!({ "url": "/bg/alicja?v=bg", "dim": 13 })
    );
    ok(app, "tok-cur", "updatePublicProfile", json!(["yes"])).await;
    assert!(scalar::<bool>(pool, r#"SELECT "public_profile" FROM "user" WHERE "id" = 'u1'"#).await);
    let a = rpc(app, "tok-cur", "updatePublicProfile", json!([2])).await;
    assert_eq!(a.json()["error"]["message"], "invalid input syntax for type boolean: \"2\"");
}

async fn tags(app: &App, pool: &PgPool) {
    let check =
        |tag: &'static str| async move { ok(app, "tok-cur", "checkTagAvailability", json!([tag])).await["ok"].clone() };
    assert_eq!(check("bob").await, json!({ "status": "unavailable", "reason": "taken" }));
    assert_eq!(check("old-bob").await, json!({ "status": "unavailable", "reason": "retired" }));
    assert_eq!(check(" alicja ").await, json!({ "status": "current" }));
    assert_eq!(check("login").await, json!({ "status": "unavailable", "reason": "reserved" }));
    assert_eq!(check("fresh").await, json!({ "status": "ok" }));

    assert_eq!(
        ok(app, "tok-cur", "updateTag", json!(["old-bob"])).await["ok"],
        json!({ "ok": false, "reason": "retired" })
    );
    let a = ok(app, "tok-cur", "updateTag", json!(["fresh"])).await;
    assert_eq!(a, json!({ "ok": { "ok": true, "tag": "fresh" }, "revalidate": true }));
    let aliases: Vec<(String, String)> =
        sqlx::query_as(r#"SELECT "code", "user_id" FROM "user_code_aliases" ORDER BY "code""#)
            .fetch_all(pool)
            .await
            .unwrap();
    assert_eq!(aliases, vec![("alicja".into(), "u1".into()), ("old-bob".into(), "u2".into())]);
    // Taking the old tag back is allowed: the alias is the caller's own.
    assert_eq!(ok(app, "tok-cur", "updateTag", json!(["alicja"])).await["ok"], json!({ "ok": true, "tag": "alicja" }));
    assert_eq!(scalar::<String>(pool, r#"SELECT "user_code" FROM "user" WHERE "id" = 'u1'"#).await, "alicja");
}

async fn default_password(app: &App, pool: &PgPool) {
    assert_eq!(
        ok(app, "tok-cur", "myDefaultFilePassword", json!([])).await["ok"],
        json!({ "enabled": true, "affected": 2 })
    );
    assert_eq!(
        ok(app, "tok-cur", "updateDefaultFilePassword", json!(["abc"])).await["ok"],
        json!({ "ok": false, "reason": "too-short" })
    );
    assert_eq!(
        ok(app, "tok-cur", "updateDefaultFilePassword", json!(["secret1", true])).await["ok"],
        json!({ "ok": true, "enabled": true, "updated": 2 })
    );
    let (hash, version): (String, i32) =
        sqlx::query_as(r#"SELECT "default_password_hash", "default_password_version" FROM "user" WHERE "id" = 'u1'"#)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(version, 2);
    assert!(modlight_server::password::verify_password("secret1", Some(&hash)).await);
    let rows: Vec<(String, Option<String>, Option<i32>)> = sqlx::query_as(
        r#"SELECT "code", "password_hash", "password_version" FROM "uploads" WHERE "owner_id" = 'u1' ORDER BY "code""#,
    )
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            ("c1".into(), Some(hash.clone()), Some(2)),
            // A hand-picked password is never trampled.
            ("c2".into(), Some("scrypt$own$pw".into()), None),
            // Trashed files are rewritten too.
            ("c3".into(), Some(hash.clone()), Some(2)),
        ]
    );
    // Clearing with "apply" un-protects the files that were on the default.
    assert_eq!(
        ok(app, "tok-cur", "updateDefaultFilePassword", json!([null, true])).await["ok"],
        json!({ "ok": true, "enabled": false, "updated": 2 })
    );
    assert_eq!(scalar::<i64>(pool, r#"SELECT count(*) FROM "uploads" WHERE "password_version" IS NOT NULL"#).await, 0);
    assert_eq!(scalar::<i32>(pool, r#"SELECT "default_password_version" FROM "user" WHERE "id" = 'u1'"#).await, 3);
}

async fn folders_and_domains(app: &App, pool: &PgPool) {
    assert_eq!(ok(app, "tok-cur", "updateDefaultFolder", json!([F4])).await["ok"], json!({ "ok": false }));
    assert_eq!(
        ok(app, "tok-cur", "updateDefaultFolder", json!([format!("{{{F2}}}")])).await["ok"],
        json!({ "ok": true })
    );
    assert_eq!(
        scalar::<uuid::Uuid>(pool, r#"SELECT "default_folder_id" FROM "user" WHERE "id" = 'u1'"#).await.to_string(),
        F2
    );
    let a = rpc(app, "tok-cur", "updateDefaultFolder", json!([""])).await;
    assert_eq!(a.json()["error"]["message"], "invalid input syntax for type uuid: \"\"");
    let a = rpc(app, "tok-cur", "updateDefaultFolder", json!([])).await;
    assert_eq!(a.json()["error"]["message"], "No values to set");

    assert_eq!(
        ok(app, "tok-cur", "updatePreferredDomain", json!(["https://h.example.com/"])).await["ok"],
        json!({ "ok": true, "baseUrl": "https://H.example.com" })
    );
    assert_eq!(
        ok(app, "tok-cur", "updatePreferredDomain", json!(["https://evil.example"])).await["ok"],
        json!({ "ok": false, "baseUrl": ORIGIN })
    );
    assert_eq!(
        scalar::<String>(pool, r#"SELECT "preferred_domain" FROM "user" WHERE "id" = 'u1'"#).await,
        "https://h.example.com"
    );
    ok(app, "tok-cur", "updateNaming", json!([{ "scheme": "words", "wordCount": 3.5, "randomLength": "9" }])).await;
    assert_eq!(
        ok(app, "tok-cur", "myUrlPrefs", json!([])).await["ok"],
        json!({
            "scheme": "words", "pattern": null, "wordCount": 4, "randomLength": 9,
            "preferredDomain": "https://h.example.com", "baseUrl": "https://H.example.com"
        })
    );
    let a = rpc(app, "tok-cur", "updateNaming", json!([{ "scheme": "custom", "pattern": "  " }])).await;
    assert_eq!(a.json()["error"]["message"], "A custom pattern is required for the custom scheme");
}

async fn admin(app: &App, pool: &PgPool) {
    // Not an administrator: redirected, nothing written.
    assert_eq!(rpc(app, "tok-cur", "listAdminUsers", json!([])).await.json(), json!({ "redirect": "/dashboard" }));
    assert_eq!(rpc(app, "tok-cur", "setUserQuota", json!(["u2", 5])).await.json(), json!({ "redirect": "/dashboard" }));

    let rows = ok(app, "tok-adm", "listAdminUsers", json!([])).await["ok"].clone();
    let ids: Vec<&str> = rows.as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["adm", "u1", "u2"]);
    let bob = &rows[2];
    assert_eq!(bob["maxQuota"], -1);
    assert_eq!(bob["usedQuota"], 0);
    assert_eq!(bob["files"], 1);
    assert_eq!(bob["emailVerified"], false);
    assert_eq!(bob["invitePending"], false);
    assert_eq!(rows[1]["avatar"], "/avatar/alicja?v=tok");
    assert_eq!(rows[1]["files"], 2);

    assert_eq!(ok(app, "tok-adm", "setUserQuota", json!(["u2", -5])).await["ok"], json!({ "ok": true }));
    assert_eq!(ok(app, "tok-adm", "setUserQuota", json!(["u2", "100"])).await["ok"], json!({ "ok": false }));
    assert_eq!(scalar::<i64>(pool, r#"SELECT "max_quota" FROM "user" WHERE "id" = 'u2'"#).await, -1);
    assert_eq!(ok(app, "tok-adm", "recalculateUserQuota", json!(["u1"])).await["ok"], json!({ "used": 600 }));
    assert_eq!(
        ok(app, "tok-adm", "setUserTag", json!(["u2", "old-bob"])).await["ok"],
        json!({ "ok": true, "tag": "old-bob" })
    );
    assert_eq!(
        ok(app, "tok-adm", "setUserTag", json!(["nobody", "zz-top"])).await["ok"],
        json!({ "ok": false, "reason": "error" })
    );
    ok(app, "tok-adm", "setAllowRootShortLinks", json!(["u2", true])).await;
    assert!(scalar::<bool>(pool, r#"SELECT "allow_root_short_links" FROM "user" WHERE "id" = 'u2'"#).await);
    let a = rpc(app, "tok-adm", "updateSkipTagInUrl", json!([{ "$undefined": 1 }])).await;
    assert_eq!(a.json()["error"]["message"], "No values to set");
}

async fn sessions(app: &App, pool: &PgPool) {
    let outcome =
        |id: Value| async move { ok(app, "tok-cur", "revokeMySession", json!([id])).await["ok"]["outcome"].clone() };
    assert_eq!(outcome(json!("s-cur")).await, "current");
    assert_eq!(outcome(json!("s-bob")).await, "gone");
    assert_eq!(outcome(json!("  ")).await, "gone");
    assert_eq!(outcome(json!(5)).await, "gone");
    assert_eq!(outcome(json!("s-old")).await, "ok");
    assert_eq!(scalar::<i64>(pool, r#"SELECT count(*) FROM "session" WHERE "id" = 's-old'"#).await, 0);
    assert_eq!(scalar::<i64>(pool, r#"SELECT count(*) FROM "session" WHERE "id" = 's-bob'"#).await, 1);
    // Nothing countable is left (the impersonated session is not listed), so
    // Better Auth is not called and the impersonated row survives.
    assert_eq!(
        ok(app, "tok-cur", "revokeMyOtherSessions", json!([])).await,
        json!({ "ok": { "ok": true, "revoked": 0 } })
    );
    assert_eq!(scalar::<i64>(pool, r#"SELECT count(*) FROM "session" WHERE "id" = 's-imp'"#).await, 1);
    // With a listed session it is, and every unexpired other session goes,
    // the impersonated one included; the expired row stays.
    exec(
        pool,
        r#"INSERT INTO "session" ("id", "token", "user_id", "created_at", "updated_at", "expires_at")
           VALUES ('s-new', 'tok-new', 'u1', now() at time zone 'utc', now() at time zone 'utc', (now() at time zone 'utc') + interval '7 days')"#,
    )
    .await;
    assert_eq!(
        ok(app, "tok-cur", "revokeMyOtherSessions", json!([])).await,
        json!({ "ok": { "ok": true, "revoked": 1 }, "revalidate": true })
    );
    let left: Vec<String> = sqlx::query_scalar(r#"SELECT "id" FROM "session" WHERE "user_id" = 'u1' ORDER BY "id""#)
        .fetch_all(pool)
        .await
        .unwrap();
    assert_eq!(left, vec!["s-cur".to_string(), "s-exp".to_string()]);
}
