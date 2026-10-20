//! The release channel end to end: `/api/releases/app`, `/download/app`, the
//! `releases` actions and the `/download` and `/dashboard/tokens` page loaders.
//!
//! The first two tests need no database (they prove what happens BEFORE one is
//! touched). The last runs only when `MODLIGHT_TEST_DATABASE_URL` points at a
//! server where the user may create databases (see tests/db_compat.rs); it
//! creates, migrates and drops its own database. The `(app)` layout loader is
//! another module's, so that test registers a stand-in for it in its own page
//! registry (the shared `pages::register_all` is untouched).

mod common;

use std::str::FromStr;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{Answer, call};
use modlight_server::api_scopes::TokenScope;
use modlight_server::api_token::create_api_token;
use modlight_server::auth::cookies::sign_cookie_value;
use modlight_server::config::Config;
use modlight_server::http::App;
use modlight_server::pages::{Layout, LayoutResult, PageCtx, PageRegistry, dashboard_tokens, download, root_layout};
use modlight_server::paths::Paths;
use modlight_server::state::AppState;
use serde_json::{Value, json};
use sqlx::postgres::PgConnectOptions;
use sqlx::{ConnectOptions, PgPool};

const SECRET: &str = "releases-test-secret-0123456789abcdef";
const ORIGIN: &str = "http://127.0.0.1:8421";

fn rpc(action: &str, args: Value, cookie: Option<&str>) -> Request<Body> {
    let mut req = Request::post(format!("/_rpc/releases/{action}"))
        .header("host", "127.0.0.1:8421")
        .header("origin", ORIGIN)
        .header("x-modlight-rpc", "1")
        .header("content-type", "application/json");
    if let Some(cookie) = cookie {
        req = req.header("cookie", cookie);
    }
    req.body(Body::from(args.to_string())).unwrap()
}

#[tokio::test]
async fn release_actions_check_admin_before_doing_anything() {
    // The database of `common::state()` cannot be reached: an action that did
    // any work before its guard would answer an error envelope, not the redirect.
    let app = App::new(common::state());
    for (action, args) in [
        ("loadRelease", json!([])),
        ("publishAppRelease", json!([{ "version": "1.0", "code": "abc" }])),
        ("unpublishAppRelease", json!([])),
    ] {
        let a = call(&app, rpc(action, args, None)).await;
        assert_eq!(a.status, StatusCode::OK, "{action}");
        assert_eq!(a.json(), json!({ "redirect": "/login" }), "{action}");
    }
}

#[tokio::test]
async fn publishing_authenticates_before_it_reads_the_body() {
    let app = App::new(common::state());
    let req = Request::post("/api/releases/app")
        .header("host", "127.0.0.1:8421")
        .header("content-type", "text/plain")
        .body(Body::from("x".repeat(100_000)))
        .unwrap();
    let a = call(&app, req).await;
    assert_eq!(a.status, StatusCode::UNAUTHORIZED);
    assert_eq!(a.header("cache-control").as_deref(), Some("no-store"));
    assert_eq!(a.header("content-type").as_deref(), Some("application/json"));
    assert_eq!(a.json(), json!({ "message": "Administrator credentials required." }));
}

// ── against a database ──────────────────────────────────────────────────────

struct TempDb {
    admin: PgConnectOptions,
    name: String,
    pool: PgPool,
}

impl TempDb {
    async fn create(url: &str) -> TempDb {
        let admin = PgConnectOptions::from_str(url).expect("MODLIGHT_TEST_DATABASE_URL");
        let name = format!("modlight_test_rel_{}_{}", std::process::id(), rand::random::<u32>());
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

/// A stand-in for the `(app)` layout loader: its gate only.
async fn app_layout_gate(ctx: PageCtx) -> LayoutResult {
    ctx.require_user().await?;
    Ok(json!({}))
}

fn app_with(pool: PgPool) -> App {
    let vars = [
        ("APP_URL", ORIGIN),
        ("NODE_ENV", "production"),
        ("SKIP_ENV_VALIDATION", "1"),
        ("AUTH_SECRET", SECRET),
        ("TRUST_PROXY", "true"),
    ];
    let map = vars.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
    let mut state = AppState::new(Config::from_env_map(&map).expect("config"), Paths::crate_dir(), pool).unwrap();
    let mut reg = PageRegistry::default();
    reg.layout(Layout::Root, root_layout::load);
    reg.layout(Layout::App, app_layout_gate);
    reg.page("/download", download::load);
    reg.page("/dashboard/tokens", dashboard_tokens::load);
    state.pages = Arc::new(reg);
    App::new(state)
}

async fn sql(pool: &PgPool, statement: &str) {
    sqlx::query(statement).execute(pool).await.unwrap_or_else(|e| panic!("{statement}: {e}"));
}

/// A session row for `user_id`, returned as its `Cookie` header.
async fn session_for(pool: &PgPool, user_id: &str, token: &str) -> String {
    sqlx::query(
        "insert into \"session\" (id, expires_at, token, updated_at, user_id) values ($1, (now() at time zone 'utc') + interval '7 days', $2, now() at time zone 'utc', $3)",
    )
    .bind(format!("s-{token}"))
    .bind(token)
    .bind(user_id)
    .execute(pool)
    .await
    .unwrap();
    format!("better-auth.session_token={}", sign_cookie_value(token, SECRET))
}

async fn get(app: &App, uri: &str, cookie: Option<&str>) -> Answer {
    let mut req = Request::get(uri).header("host", "127.0.0.1:8421");
    if let Some(cookie) = cookie {
        req = req.header("cookie", cookie);
    }
    call(app, req.body(Body::empty()).unwrap()).await
}

async fn post_release(app: &App, body: &str, content_type: &str, token: &str) -> Answer {
    let req = Request::post("/api/releases/app")
        .header("host", "127.0.0.1:8421")
        .header("content-type", content_type)
        .header("authorization", format!("Bearer {token}"))
        .header("x-real-ip", "203.0.113.7")
        .body(Body::from(body.to_string()))
        .unwrap();
    call(app, req).await
}

#[tokio::test]
async fn release_channel_against_a_database() {
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
    modlight_server::db::migrate::run(pool, &Paths::crate_dir().migrations, true).await.unwrap();
    sql(pool, "insert into \"user\" (id, name, email, role, user_code, theme) values ('adm', 'Alicja Admin', 'alicja@example.test', 'admin', 'alicja', 'modern')").await;
    sql(pool, "insert into \"user\" (id, name, email, role, user_code, theme, preferred_domain) values ('usr', 'Bob Builder', 'bob@example.test', 'user', 'bob', 'modern', 'https://nope.example')").await;
    let digest = "ab".repeat(32);
    sql(pool, &format!("insert into uploads (owner_id, code, filename, ext, mime_type, size, storage_key, checksum, delete_token) values ('adm', 'saluy6', 'Modlight-0.7.0-setup-x64.exe', 'exe', 'application/octet-stream', 82317446, 'alicja/saluy6.exe', '{digest}', 'dt1')")).await;
    sql(pool, "insert into uploads (owner_id, code, filename, ext, mime_type, size, storage_key, published, delete_token) values ('adm', 'unpub1', 'x.exe', 'exe', 'application/octet-stream', 1, 'alicja/unpub1.exe', false, 'dt2')").await;
    let token = create_api_token(pool, "adm", "release script", TokenScope::Full).await.unwrap();
    create_api_token(pool, "usr", "ShareX", TokenScope::Upload).await.unwrap();
    let admin = session_for(pool, "adm", "admintoken0123456789abcdefghijkl").await;
    let user = session_for(pool, "usr", "usertoken0123456789abcdefghijklm").await;
    let app = app_with(pool.clone());

    // Nothing published.
    let a = get(&app, "/api/releases/app", None).await;
    assert_eq!(a.status, StatusCode::NOT_FOUND);
    assert_eq!(a.header("cache-control").as_deref(), Some("no-store"));
    assert_eq!(a.json(), json!({ "name": "Modlight App", "message": "No release published." }));
    let a = get(&app, "/download/app", None).await;
    assert_eq!(a.status, StatusCode::NOT_FOUND);
    assert_eq!(a.header("content-type").as_deref(), Some("text/plain; charset=utf-8"));
    assert_eq!(a.text(), "No release published yet.");
    let page = get(&app, "/_page?path=%2Fdownload", None).await.json();
    assert_eq!(page["props"], json!({ "release": null }));
    assert_eq!(page["meta"]["title"], "Modlight App - Modlight");
    assert_eq!(page["meta"]["description"], "Modlight App for Modlight.");

    // Publish by token; security fix 4 first.
    let body = json!({ "version": "v0.7.0", "code": " saluy6 ", "notes": "  Hi.  " }).to_string();
    let a = post_release(&app, &body, "text/plain", &token).await;
    assert_eq!(a.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(a.json(), json!({ "message": "Content-Type must be application/json." }));
    // ... and the size ceiling of every other JSON write endpoint (16 KiB).
    let huge = json!({ "version": "v0.7.0", "code": "saluy6", "notes": "x".repeat(20 * 1024) }).to_string();
    let a = post_release(&app, &huge, "application/json", &token).await;
    assert_eq!(a.status, StatusCode::PAYLOAD_TOO_LARGE, "{}", a.text());
    let a = post_release(&app, &body, "application/json", &token).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    assert_eq!(a.header("cache-control").as_deref(), Some("no-store"));
    let manifest = a.json();
    assert_eq!(manifest["url"], "http://127.0.0.1:8421/alicja/saluy6/raw.exe");
    assert_eq!(manifest["version"], "0.7.0");
    assert_eq!(manifest["notes"], "Hi.");
    assert_eq!(manifest["sha256"], digest);
    let audit: (String, Value, String) =
        sqlx::query_as("select user_id, detail, ip from audit_log where action = 'release.published'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(
        audit,
        (
            "adm".to_string(),
            json!({ "by": "adm", "byEmail": "alicja@example.test", "version": "0.7.0", "code": " saluy6 " }),
            "203.0.113.7".to_string()
        )
    );
    let a = post_release(&app, &json!({ "version": "1.0", "code": "unpub1" }).to_string(), "application/json", &token)
        .await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST);
    assert_eq!(a.json()["reason"], "not-public");

    // Published.
    let a = get(&app, "/api/releases/app", None).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(a.header("cache-control").as_deref(), Some("public, max-age=300"));
    assert_eq!(a.json(), manifest);
    let a = get(&app, "/download/app", None).await;
    assert_eq!(a.status, StatusCode::FOUND);
    assert_eq!(a.header("location").as_deref(), Some("http://127.0.0.1:8421/alicja/saluy6/raw.exe"));
    assert_eq!(a.header("cache-control").as_deref(), Some("public, max-age=60"));
    assert_eq!(a.header("x-release-version").as_deref(), Some("0.7.0"));
    assert_eq!(a.header("x-release-sha256").as_deref(), Some(digest.as_str()));
    let page = get(&app, "/_page?path=%2Fdownload", None).await.json();
    assert_eq!(page["props"]["release"], manifest);
    assert_eq!(page["meta"]["description"], "Download Modlight App 0.7.0 for Modlight.");

    // The Integrations page of a user whose preferred domain is not configured.
    let page = get(&app, "/_page?path=%2Fdashboard%2Ftokens", Some(&user)).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.text());
    let page = page.json();
    assert_eq!(page["meta"]["title"], "Integrations - Modlight");
    let props = &page["props"];
    assert_eq!(props["appUrl"], ORIGIN);
    assert_eq!(props["username"], "Bob");
    assert_eq!(props["user"], json!({ "userCode": "bob", "name": "Bob Builder", "email": "bob@example.test" }));
    assert_eq!(props["release"], manifest);
    let tokens = props["tokens"].as_array().unwrap();
    assert_eq!(tokens.len(), 1);
    let keys: Vec<&String> = tokens[0].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["id", "name", "scope", "preview", "createdAt", "lastUsedAt", "revokedAt"]);
    assert_eq!(tokens[0]["scope"], "upload");
    assert!(tokens[0]["createdAt"].as_str().unwrap().ends_with('Z'));
    assert_eq!(
        get(&app, "/_page?path=%2Fdashboard%2Ftokens", None).await.json()["redirect"],
        "/login?next=%2Fdashboard%2Ftokens"
    );

    // The actions.
    let a = call(&app, rpc("publishAppRelease", json!([{ "version": "1.0", "code": "saluy6" }]), Some(&user))).await;
    assert_eq!(a.json(), json!({ "redirect": "/dashboard" }));
    let link = "https://h.example/alicja/saluy6.exe?x=1";
    let a = call(&app, rpc("publishAppRelease", json!([{ "version": "0.7.1", "code": link }]), Some(&admin))).await;
    assert_eq!(
        a.json(),
        json!({ "ok": { "ok": false, "reason": "version-mismatch", "fileVersion": "0.7.0", "filename": "Modlight-0.7.0-setup-x64.exe" } })
    );
    let args = json!([{ "version": "0.7.1", "code": link, "allowVersionMismatch": true }]);
    let a = call(&app, rpc("publishAppRelease", args, Some(&admin))).await.json();
    assert_eq!(a["ok"]["ok"], true);
    assert_eq!(a["ok"]["manifest"]["version"], "0.7.1");
    assert_eq!(a["revalidate"], true);
    let a = call(&app, rpc("loadRelease", json!([]), Some(&admin))).await.json();
    assert_eq!(a["ok"]["appName"], "Modlight App");
    assert_eq!(a["ok"]["manifest"]["version"], "0.7.1");
    let a = call(&app, rpc("unpublishAppRelease", json!([]), Some(&admin))).await.json();
    assert_eq!(a, json!({ "ok": { "ok": true }, "revalidate": true }));
    assert_eq!(get(&app, "/api/releases/app", None).await.status, StatusCode::NOT_FOUND);
    let stored: Option<Option<String>> =
        sqlx::query_scalar("select value from settings where key = 'release_app'").fetch_optional(pool).await.unwrap();
    assert_eq!(stored, Some(None), "unpublishing keeps the row with a NULL value");
}
