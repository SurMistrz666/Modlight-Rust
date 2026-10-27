//! Not upstream: Downloads end to end (`src/downloads.rs`,
//! `src/routes/api/downloads.rs`, `src/pages/dashboard_downloads.rs`).
//!
//! The first tests need no database: every refusal they check happens before
//! one is touched (and before a byte of a body is read). The last runs only
//! when `MODLIGHT_TEST_DATABASE_URL` points at a server where the user may
//! create databases (see tests/db_compat.rs); it creates, migrates and drops
//! its own database, and keeps its files in a temporary directory.

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
use modlight_server::downloads::CHUNK_SIZE;
use modlight_server::http::App;
use modlight_server::paths::Paths;
use modlight_server::state::AppState;
use modlight_server::storage::{LocalDriver, Storage};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgConnectOptions;
use sqlx::{ConnectOptions, PgPool};

const SECRET: &str = "downloads-test-secret-0123456789abcdef";
const ORIGIN: &str = "http://127.0.0.1:8421";
const PUBLISH: &str = "publish-token-for-tests-0123456789abcdefgh";

fn request(method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder().method(method).uri(uri).header("host", "127.0.0.1:8421").header("x-real-ip", "203.0.113.9")
}

/// `{"error", "code"}`, never cached.
fn assert_refusal(a: &Answer, status: StatusCode, code: &str) {
    assert_eq!(a.status, status, "{}", a.text());
    assert_eq!(a.header("content-type").as_deref(), Some("application/json"));
    assert_eq!(a.header("cache-control").as_deref(), Some("private, no-store"));
    let body = a.json();
    assert_eq!(body["code"], code, "{body}");
    assert!(body["error"].as_str().is_some_and(|e| !e.is_empty()), "{body}");
    assert_eq!(body.as_object().unwrap().len(), 2, "{body}");
}

/// A state whose database answers nothing, quickly.
fn no_database() -> App {
    let mut state = common::state();
    state.db = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(200))
        .connect_lazy("postgresql://nobody@127.0.0.1:1/none")
        .unwrap();
    App::new(state)
}

#[tokio::test]
async fn without_credentials_everything_is_refused_before_the_body() {
    let app = no_database();
    let id = "0123456789abcdef0123456789abcdef";
    let cases = [
        ("GET", "/api/downloads".to_string()),
        ("GET", format!("/downloads/{id}/Modlight-1.0.0-1-macos-arm64.zip")),
        ("POST", "/api/downloads/upload".to_string()),
        ("PUT", format!("/api/downloads/upload/{id}/0")),
        ("POST", format!("/api/downloads/upload/{id}/finish")),
        ("DELETE", format!("/api/downloads/{id}")),
    ];
    for (method, uri) in cases {
        // A body nobody should read: wrong type, far over every limit.
        let req = request(method, &uri).header("content-type", "text/plain").body(Body::from("x".repeat(100_000)));
        let a = call(&app, req.unwrap()).await;
        if uri.starts_with("/downloads/") {
            // A file route looks its build up first (by id); the database of
            // `common::state()` cannot be reached, which is an internal error,
            // never the file.
            assert_refusal(&a, StatusCode::INTERNAL_SERVER_ERROR, "internal");
            continue;
        }
        assert_refusal(&a, StatusCode::UNAUTHORIZED, "unauthorized");
    }
}

#[tokio::test]
async fn the_routes_answer_only_their_methods() {
    let app = no_database();
    for (method, uri) in [
        ("POST", "/api/downloads"),
        ("GET", "/api/downloads/upload"),
        ("DELETE", "/api/downloads/upload"),
        ("PUT", "/api/downloads/upload/0123456789abcdef0123456789abcdef/finish"),
    ] {
        let a = call(&app, request(method, uri).body(Body::empty()).unwrap()).await;
        assert_eq!(a.status, StatusCode::METHOD_NOT_ALLOWED, "{method} {uri}");
    }
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
        let name = format!("modlight_test_dl_{}_{}", std::process::id(), rand::random::<u32>());
        let mut conn = admin.connect().await.expect("connect to the test server");
        sqlx::query(&format!("CREATE DATABASE \"{name}\"")).execute(&mut conn).await.expect("create database");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(8)
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
async fn downloads_against_a_database() {
    let Ok(url) = std::env::var("MODLIGHT_TEST_DATABASE_URL") else {
        eprintln!("MODLIGHT_TEST_DATABASE_URL is not set: skipping the database tests");
        return;
    };
    let db = TempDb::create(&url).await;
    let dir = tempfile::tempdir().unwrap();
    let result = std::panic::AssertUnwindSafe(run(&db.pool, dir.path()));
    let outcome = futures_util::FutureExt::catch_unwind(result).await;
    db.drop().await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

struct Env {
    app: App,
    pool: PgPool,
    storage: std::path::PathBuf,
    staging: std::path::PathBuf,
}

fn app_with(pool: PgPool, dir: &std::path::Path) -> Env {
    let storage = dir.join("storage");
    let tmp = dir.join("tmp");
    let vars = [
        ("APP_URL", ORIGIN.to_string()),
        ("NODE_ENV", "production".to_string()),
        ("SKIP_ENV_VALIDATION", "1".to_string()),
        ("AUTH_SECRET", SECRET.to_string()),
        ("TRUST_PROXY", "true".to_string()),
        ("RELEASE_PUBLISH_TOKEN", format!("  {PUBLISH}  ")),
        ("UPLOAD_TMP_PATH", tmp.to_string_lossy().into_owned()),
        ("STORAGE_LOCAL_PATH", storage.to_string_lossy().into_owned()),
        ("MIN_FREE_DISK_BYTES", "0".to_string()),
    ];
    let map = vars.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect();
    let mut state =
        AppState::new(Config::from_env_map(&map).expect("config"), Paths::crate_dir(), pool.clone()).unwrap();
    state.storage = Storage::with_driver(Arc::new(LocalDriver::new(&storage.to_string_lossy())));
    Env { app: App::new(state), pool, storage, staging: tmp.join("releases") }
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

/// Who a request is from.
#[derive(Clone, Copy)]
enum As<'a> {
    Nobody,
    Cookie(&'a str),
    Bearer(&'a str),
    ApiKey(&'a str),
}

fn with(builder: axum::http::request::Builder, who: As<'_>) -> axum::http::request::Builder {
    match who {
        As::Nobody => builder,
        As::Cookie(c) => builder.header("cookie", c),
        As::Bearer(t) => builder.header("authorization", format!("Bearer {t}")),
        As::ApiKey(t) => builder.header("x-api-key", t),
    }
}

impl Env {
    async fn send(&self, method: &str, uri: &str, who: As<'_>, body: Body, content_type: Option<&str>) -> Answer {
        let mut b = with(request(method, uri), who);
        if let Some(ct) = content_type {
            b = b.header("content-type", ct);
        }
        call(&self.app, b.body(body).unwrap()).await
    }

    async fn get(&self, uri: &str, who: As<'_>) -> Answer {
        self.send("GET", uri, who, Body::empty(), None).await
    }

    async fn start(&self, who: As<'_>, body: &Value) -> Answer {
        self.send("POST", "/api/downloads/upload", who, Body::from(body.to_string()), Some("application/json")).await
    }

    async fn chunk(&self, who: As<'_>, upload: &str, n: u32, bytes: Vec<u8>) -> Answer {
        let uri = format!("/api/downloads/upload/{upload}/{n}");
        self.send("PUT", &uri, who, Body::from(bytes), Some("application/octet-stream")).await
    }

    async fn finish(&self, who: As<'_>, upload: &str) -> Answer {
        self.send("POST", &format!("/api/downloads/upload/{upload}/finish"), who, Body::empty(), None).await
    }

    /// Start, send `chunks` (each its own PUT), finish: the finish's answer.
    async fn publish(&self, who: As<'_>, meta: Value, chunks: &[&[u8]]) -> Answer {
        let a = self.start(who, &meta).await;
        assert_eq!(a.status, StatusCode::OK, "start: {}", a.text());
        let upload = a.json()["upload"].as_str().unwrap().to_string();
        for (n, piece) in chunks.iter().enumerate() {
            let a = self.chunk(who, &upload, n as u32, piece.to_vec()).await;
            assert_eq!(a.status, StatusCode::OK, "chunk {n}: {}", a.text());
        }
        self.finish(who, &upload).await
    }

    fn stored(&self, build: &Value) -> Option<Vec<u8>> {
        let path =
            self.storage.join("releases").join(build["id"].as_str().unwrap()).join(build["filename"].as_str().unwrap());
        std::fs::read(path).ok()
    }

    async fn list(&self, who: As<'_>) -> Value {
        let a = self.get("/api/downloads", who).await;
        assert_eq!(a.status, StatusCode::OK, "{}", a.text());
        assert_eq!(a.header("cache-control").as_deref(), Some("private, no-store"));
        a.json()
    }

    async fn audit(&self, action: &str) -> Vec<(Option<String>, Value, String)> {
        sqlx::query_as("select user_id, detail, ip from audit_log where action = $1 order by created_at, id")
            .bind(action)
            .fetch_all(&self.pool)
            .await
            .unwrap()
    }
}

fn sha_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A start body for `bytes`.
fn meta(platform: &str, arch: &str, version: &str, build: &str, bytes: &[u8]) -> Value {
    json!({
        "platform": platform,
        "arch": arch,
        "version": version,
        "build": build,
        "built": "2026-10-27T10:00:00Z",
        "size": bytes.len(),
        "sha256": sha_hex(bytes),
        "filename": format!("Modlight-{version}-{build}-{platform}-{arch}.zip"),
        "note": format!("Build {version}"),
    })
}

/// Deterministic bytes that differ from build to build.
fn payload(seed: u8, len: usize) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
}

fn platform_builds<'a>(list: &'a Value, platform: &str) -> &'a Vec<Value> {
    let p = list["platforms"].as_array().unwrap().iter().find(|p| p["platform"] == platform).unwrap();
    p["builds"].as_array().unwrap()
}

async fn run(pool: &PgPool, dir: &std::path::Path) {
    modlight_server::db::migrate::run(pool, &Paths::crate_dir().migrations, true).await.unwrap();
    sql(pool, "insert into \"user\" (id, name, email, role, user_code, theme) values ('adm', 'Alicja Admin', 'alicja@example.test', 'admin', 'alicja', 'modern')").await;
    sql(pool, "insert into \"user\" (id, name, email, role, user_code, theme) values ('usr', 'Bob Builder', 'bob@example.test', 'user', 'bob', 'modern')").await;
    sql(pool, "insert into \"user\" (id, name, email, role, user_code, theme, banned) values ('ban', 'Eve', 'eve@example.test', 'user', 'eve', 'modern', true)").await;
    let admin = session_for(pool, "adm", "admintoken0123456789abcdefghijkl").await;
    let user = session_for(pool, "usr", "usertoken0123456789abcdefghijklm").await;
    let banned = session_for(pool, "ban", "bantoken0123456789abcdefghijklmn").await;
    let admin_full = create_api_token(pool, "adm", "release script", TokenScope::Full).await.unwrap();
    let admin_write = create_api_token(pool, "adm", "laptop", TokenScope::Write).await.unwrap();
    let user_write = create_api_token(pool, "usr", "Desktop: Modlight", TokenScope::Write).await.unwrap();
    let user_upload = create_api_token(pool, "usr", "ShareX", TokenScope::Upload).await.unwrap();
    let user_full = create_api_token(pool, "usr", "everything", TokenScope::Full).await.unwrap();
    let banned_token = create_api_token(pool, "ban", "app", TokenScope::Write).await.unwrap();
    let env = app_with(pool.clone(), dir);

    // ── Reading: any signed-in account, by session or by an API token of any scope ──
    let empty = json!({
        "platforms": [{ "platform": "macos", "builds": [] }, { "platform": "windows", "builds": [] }],
        "keep": 5,
        "admin": false,
        "publishToken": null,
    });
    assert_eq!(env.list(As::Cookie(&user)).await, empty);
    assert_eq!(env.list(As::Bearer(&user_write)).await, empty);
    assert_eq!(env.list(As::ApiKey(&user_upload)).await, empty);
    let mut admin_view = empty.clone();
    admin_view["admin"] = json!(true);
    admin_view["publishToken"] = json!(true);
    assert_eq!(env.list(As::Cookie(&admin)).await, admin_view);
    assert_eq!(env.list(As::Bearer(&admin_write)).await, admin_view);
    assert_refusal(&env.get("/api/downloads", As::Nobody).await, StatusCode::UNAUTHORIZED, "unauthorized");
    assert_refusal(
        &env.get("/api/downloads", As::Bearer("no-such-token")).await,
        StatusCode::UNAUTHORIZED,
        "unauthorized",
    );
    // The publish token opens the publish routes only.
    assert_refusal(&env.get("/api/downloads", As::Bearer(PUBLISH)).await, StatusCode::UNAUTHORIZED, "unauthorized");
    assert_refusal(&env.get("/api/downloads", As::Cookie(&banned)).await, StatusCode::UNAUTHORIZED, "account_disabled");
    assert_refusal(
        &env.get("/api/downloads", As::Bearer(&banned_token)).await,
        StatusCode::UNAUTHORIZED,
        "account_disabled",
    );
    // A token decides alone: a wrong one is not rescued by a good cookie.
    let a = call(
        &env.app,
        request("GET", "/api/downloads")
            .header("cookie", &user)
            .header("authorization", "Bearer no-such-token")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_refusal(&a, StatusCode::UNAUTHORIZED, "unauthorized");

    // ── Who may publish ──
    let small = payload(1, 3000);
    let m = meta("macos", "arm64", "1.0.0", "1", &small);
    assert_refusal(&env.start(As::Nobody, &m).await, StatusCode::UNAUTHORIZED, "unauthorized");
    assert_refusal(&env.start(As::Bearer("no-such-token"), &m).await, StatusCode::UNAUTHORIZED, "unauthorized");
    assert_refusal(&env.start(As::Cookie(&user), &m).await, StatusCode::FORBIDDEN, "forbidden");
    assert_refusal(&env.start(As::Bearer(&user_write), &m).await, StatusCode::FORBIDDEN, "forbidden");
    assert_refusal(&env.start(As::Bearer(&user_full), &m).await, StatusCode::FORBIDDEN, "forbidden");
    // An administrator's everyday token is not a publishing credential.
    assert_refusal(&env.start(As::Bearer(&admin_write), &m).await, StatusCode::FORBIDDEN, "insufficient_scope");
    assert_refusal(&env.start(As::Cookie(&banned), &m).await, StatusCode::UNAUTHORIZED, "account_disabled");
    for who in [As::Bearer(PUBLISH), As::ApiKey(PUBLISH), As::Cookie(&admin), As::Bearer(&admin_full)] {
        let a = env.start(who, &m).await;
        assert_eq!(a.status, StatusCode::OK, "{}", a.text());
        assert_eq!(a.header("cache-control").as_deref(), Some("private, no-store"));
        let body = a.json();
        assert!(modlight_server::downloads::is_id(body["upload"].as_str().unwrap()), "{body}");
        assert_eq!(body["chunkSize"], json!(CHUNK_SIZE));
        assert_eq!(body.as_object().unwrap().len(), 2);
    }

    // ── The start body ──
    let a = env
        .send("POST", "/api/downloads/upload", As::Bearer(PUBLISH), Body::from(m.to_string()), Some("text/plain"))
        .await;
    assert_refusal(&a, StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_media_type");
    let a = env
        .send("POST", "/api/downloads/upload", As::Bearer(PUBLISH), Body::from("[1]"), Some("application/json"))
        .await;
    assert_refusal(&a, StatusCode::BAD_REQUEST, "invalid_body");
    let mut bad = m.clone();
    bad["platform"] = json!("linux");
    assert_refusal(&env.start(As::Bearer(PUBLISH), &bad).await, StatusCode::BAD_REQUEST, "invalid_platform");
    let mut bad = m.clone();
    bad["filename"] = json!("../../etc/passwd");
    assert_refusal(&env.start(As::Bearer(PUBLISH), &bad).await, StatusCode::BAD_REQUEST, "invalid_filename");
    let mut bad = m.clone();
    bad["size"] = json!(5u64 << 30);
    assert_refusal(&env.start(As::Bearer(PUBLISH), &bad).await, StatusCode::PAYLOAD_TOO_LARGE, "too_large");

    // ── A chunked publish: one chunk of exactly chunkSize, then the rest ──
    let big = payload(7, CHUNK_SIZE as usize + 1234);
    let mut m = meta("macos", "arm64", "1.2.0", "45", &big);
    m["note"] = json!("Uploads resume after sleep.\r\nFaster thumbnails.");
    let a = env.start(As::Bearer(PUBLISH), &m).await;
    let upload = a.json()["upload"].as_str().unwrap().to_string();
    // The other credentials of the publish routes: refused as at the start.
    assert_refusal(&env.chunk(As::Cookie(&user), &upload, 0, vec![1]).await, StatusCode::FORBIDDEN, "forbidden");
    assert_refusal(&env.chunk(As::Nobody, &upload, 0, vec![1]).await, StatusCode::UNAUTHORIZED, "unauthorized");
    assert_refusal(&env.finish(As::Bearer(&user_write), &upload).await, StatusCode::FORBIDDEN, "forbidden");
    // Malformed chunk numbers, unknown uploads, a chunk over chunkSize (by its declared length).
    for n in ["-1", "01", "x", "100000"] {
        let uri = format!("/api/downloads/upload/{upload}/{n}");
        let a = env.send("PUT", &uri, As::Bearer(PUBLISH), Body::from("x"), None).await;
        assert_refusal(&a, StatusCode::BAD_REQUEST, "invalid_chunk");
    }
    let unknown = "ffffffffffffffffffffffffffffffff";
    assert_refusal(&env.chunk(As::Bearer(PUBLISH), unknown, 0, vec![1]).await, StatusCode::NOT_FOUND, "not_found");
    assert_refusal(&env.chunk(As::Bearer(PUBLISH), "nope", 0, vec![1]).await, StatusCode::NOT_FOUND, "not_found");
    assert_refusal(&env.finish(As::Bearer(PUBLISH), unknown).await, StatusCode::NOT_FOUND, "not_found");
    let over = call(
        &env.app,
        request("PUT", &format!("/api/downloads/upload/{upload}/0"))
            .header("authorization", format!("Bearer {PUBLISH}"))
            .header("content-length", (CHUNK_SIZE + 1).to_string())
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_refusal(&over, StatusCode::PAYLOAD_TOO_LARGE, "too_large");
    // An empty chunk carries nothing.
    assert_refusal(
        &env.chunk(As::Bearer(PUBLISH), &upload, 0, Vec::new()).await,
        StatusCode::BAD_REQUEST,
        "empty_chunk",
    );
    let first = env.chunk(As::Bearer(PUBLISH), &upload, 0, big[..CHUNK_SIZE as usize].to_vec()).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert_eq!(first.json(), json!({ "upload": upload, "chunk": 0, "size": CHUNK_SIZE, "received": CHUNK_SIZE }));
    // More than the upload declared is refused while it streams.
    let a = env.chunk(As::Bearer(PUBLISH), &upload, 1, vec![0; 1235]).await;
    assert_refusal(&a, StatusCode::PAYLOAD_TOO_LARGE, "too_large");
    // A chunk sent again replaces the first copy.
    let a = env.chunk(As::Bearer(PUBLISH), &upload, 1, vec![0; 1234]).await;
    assert_eq!(a.json()["received"], json!(CHUNK_SIZE + 1234));
    let a = env.chunk(As::Bearer(PUBLISH), &upload, 1, big[CHUNK_SIZE as usize..].to_vec()).await;
    assert_eq!(a.status, StatusCode::OK);
    let a = env.finish(As::Bearer(PUBLISH), &upload).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    assert_eq!(a.header("cache-control").as_deref(), Some("private, no-store"));
    let mac = a.json();
    let id = mac["id"].as_str().unwrap().to_string();
    assert!(modlight_server::downloads::is_id(&id));
    let keys: Vec<&String> = mac.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "id",
            "platform",
            "arch",
            "version",
            "build",
            "built",
            "published",
            "size",
            "sha256",
            "filename",
            "uploader",
            "note",
            "url"
        ]
    );
    assert_eq!(mac["platform"], "macos");
    assert_eq!(mac["arch"], "arm64");
    assert_eq!(mac["version"], "1.2.0");
    assert_eq!(mac["build"], "45");
    assert_eq!(mac["built"], "2026-10-27T10:00:00.000Z");
    assert!(mac["published"].as_str().unwrap().ends_with('Z'));
    assert_eq!(mac["size"], json!(big.len()));
    assert_eq!(mac["sha256"], sha_hex(&big));
    assert_eq!(mac["filename"], "Modlight-1.2.0-45-macos-arm64.zip");
    assert_eq!(mac["uploader"], "publish-token");
    assert_eq!(mac["note"], "Uploads resume after sleep.\nFaster thumbnails.");
    assert_eq!(mac["url"], format!("/downloads/{id}/Modlight-1.2.0-45-macos-arm64.zip"));
    assert_eq!(env.stored(&mac).as_deref(), Some(big.as_slice()));
    // Only the marker that answers a repeated finish is left of the upload.
    let left: Vec<String> = std::fs::read_dir(env.staging.join(&upload))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["done"]);
    let again = env.finish(As::Bearer(PUBLISH), &upload).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.json(), mac);
    assert_refusal(&env.chunk(As::Bearer(PUBLISH), &upload, 2, vec![1]).await, StatusCode::NOT_FOUND, "not_found");
    let published = env.audit("downloads.published").await;
    assert_eq!(published.len(), 1);
    let (actor, detail, ip) = &published[0];
    assert_eq!(actor, &None);
    assert_eq!(ip, "203.0.113.9");
    assert_eq!(detail["by"], "publish-token");
    assert_eq!(detail["id"], id.as_str());
    assert_eq!(detail["platform"], "macos");
    assert_eq!(detail["version"], "1.2.0");
    assert_eq!(detail["sha256"], sha_hex(&big));

    // ── Listing and downloading it ──
    let list = env.list(As::Cookie(&user)).await;
    assert_eq!(platform_builds(&list, "macos"), &vec![mac.clone()]);
    assert!(platform_builds(&list, "windows").is_empty());
    let path = mac["url"].as_str().unwrap().to_string();
    for who in [As::Cookie(&user), As::Bearer(&user_write), As::ApiKey(&user_upload)] {
        let a = env.get(&path, who).await;
        assert_eq!(a.status, StatusCode::OK);
        assert_eq!(a.body.as_ref(), big.as_slice());
        assert_eq!(a.header("content-length"), Some(big.len().to_string()));
        assert_eq!(a.header("content-type").as_deref(), Some("application/zip"));
        assert_eq!(
            a.header("content-disposition").as_deref(),
            Some("attachment; filename=\"Modlight-1.2.0-45-macos-arm64.zip\"")
        );
        assert_eq!(a.header("etag"), Some(format!("\"{}\"", sha_hex(&big))));
        assert_eq!(a.header("cache-control").as_deref(), Some("private, no-store"));
        assert_eq!(a.header("accept-ranges").as_deref(), Some("bytes"));
    }
    assert_refusal(&env.get(&path, As::Nobody).await, StatusCode::UNAUTHORIZED, "unauthorized");
    assert_refusal(&env.get(&path, As::Bearer(PUBLISH)).await, StatusCode::UNAUTHORIZED, "unauthorized");
    assert_refusal(&env.get(&path, As::Cookie(&banned)).await, StatusCode::UNAUTHORIZED, "account_disabled");
    let ranged = call(
        &env.app,
        request("GET", &path).header("cookie", &user).header("range", "bytes=10-19").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(ranged.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(ranged.body.as_ref(), &big[10..20]);
    assert_eq!(ranged.header("content-range"), Some(format!("bytes 10-19/{}", big.len())));
    let etag = format!("\"{}\"", sha_hex(&big));
    let cached = call(
        &env.app,
        request("GET", &path).header("cookie", &user).header("if-none-match", &etag).body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(cached.status, StatusCode::NOT_MODIFIED);
    assert!(cached.body.is_empty());
    assert_eq!(cached.header("etag"), Some(etag.clone()));
    // A range for another version of the file gets the whole file.
    let stale = call(
        &env.app,
        request("GET", &path)
            .header("cookie", &user)
            .header("range", "bytes=10-19")
            .header("if-range", "\"other\"")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(stale.status, StatusCode::OK);
    assert_eq!(stale.body.len(), big.len());
    // The same id under another name is no build.
    let wrong = env.get(&format!("/downloads/{id}/other.zip"), As::Cookie(&user)).await;
    assert_eq!(wrong.status, StatusCode::NOT_FOUND);

    // ── An administrator publishes with a session; the name goes on record ──
    let win1 = payload(11, 2048);
    let a = env
        .publish(As::Cookie(&admin), meta("windows", "x64", "1.0.0", "1", &win1), &[&win1[..1000], &win1[1000..]])
        .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    let win1_build = a.json();
    assert_eq!(win1_build["uploader"], "Alicja Admin");
    let (actor, detail, _) = &env.audit("downloads.published").await[1];
    assert_eq!(actor.as_deref(), Some("adm"));
    assert_eq!(detail["byEmail"], "alicja@example.test");
    assert_eq!(detail["via"], "session");

    // ── A wrong SHA-256, a wrong size and a missing chunk drop the upload ──
    let x = payload(21, 500);
    let mut wrong_sha = meta("windows", "x64", "9.9.9", "9", &x);
    wrong_sha["sha256"] = json!(sha_hex(b"something else"));
    let a = env.publish(As::Bearer(PUBLISH), wrong_sha, &[&x]).await;
    assert_refusal(&a, StatusCode::BAD_REQUEST, "checksum_mismatch");
    let mut wrong_size = meta("windows", "x64", "9.9.8", "9", &x);
    wrong_size["size"] = json!(600);
    let a = env.publish(As::Bearer(PUBLISH), wrong_size, &[&x]).await;
    assert_refusal(&a, StatusCode::BAD_REQUEST, "size_mismatch");
    let a = env.start(As::Bearer(PUBLISH), &meta("windows", "x64", "9.9.7", "9", &x)).await;
    let gap = a.json()["upload"].as_str().unwrap().to_string();
    assert_eq!(env.chunk(As::Bearer(PUBLISH), &gap, 0, x[..200].to_vec()).await.status, StatusCode::OK);
    assert_eq!(env.chunk(As::Bearer(PUBLISH), &gap, 2, x[400..].to_vec()).await.status, StatusCode::OK);
    let a = env.finish(As::Bearer(PUBLISH), &gap).await;
    assert_refusal(&a, StatusCode::BAD_REQUEST, "size_mismatch");
    assert!(a.json()["error"].as_str().unwrap().contains("chunk 1 is missing"));
    // Dropped: nothing left on disk, the upload answers 404, nothing published.
    assert!(!env.staging.join(&gap).exists());
    assert_refusal(&env.chunk(As::Bearer(PUBLISH), &gap, 1, vec![1]).await, StatusCode::NOT_FOUND, "not_found");
    assert_refusal(&env.finish(As::Bearer(PUBLISH), &gap).await, StatusCode::NOT_FOUND, "not_found");
    let rejected = env.audit("downloads.rejected").await;
    let codes: Vec<&str> = rejected.iter().map(|(_, d, _)| d["code"].as_str().unwrap()).collect();
    assert_eq!(codes, ["checksum_mismatch", "size_mismatch", "size_mismatch"]);
    assert_eq!(platform_builds(&env.list(As::Cookie(&user)).await, "windows").len(), 1);

    // ── The newest five per platform and architecture stay ──
    let mut windows = vec![win1_build.clone()];
    for i in 2..=6u8 {
        let bytes = payload(30 + i, 700 + i as usize);
        let a = env
            .publish(
                As::Bearer(PUBLISH),
                meta("windows", "x64", &format!("1.0.{i}"), &i.to_string(), &bytes),
                &[&bytes],
            )
            .await;
        assert_eq!(a.status, StatusCode::OK, "{}", a.text());
        windows.push(a.json());
    }
    let arm = payload(50, 900);
    let a = env.publish(As::Bearer(PUBLISH), meta("windows", "arm64", "1.0.6", "6", &arm), &[&arm]).await;
    let arm_build = a.json();
    let list = env.list(As::Cookie(&user)).await;
    let builds = platform_builds(&list, "windows");
    // Newest first: the arm64 build, then x64 1.0.6 .. 1.0.2; 1.0.0 is gone.
    let versions: Vec<(&str, &str)> =
        builds.iter().map(|b| (b["arch"].as_str().unwrap(), b["version"].as_str().unwrap())).collect();
    assert_eq!(
        versions,
        [("arm64", "1.0.6"), ("x64", "1.0.6"), ("x64", "1.0.5"), ("x64", "1.0.4"), ("x64", "1.0.3"), ("x64", "1.0.2")]
    );
    assert_eq!(builds[0], arm_build);
    assert!(env.stored(&win1_build).is_none(), "the file of a pruned build is deleted");
    assert!(!env.storage.join("releases").join(win1_build["id"].as_str().unwrap()).exists());
    for b in &windows[1..] {
        assert!(env.stored(b).is_some());
    }
    assert_eq!(platform_builds(&list, "macos"), &vec![mac.clone()], "another platform is untouched");
    let pruned = env.audit("downloads.pruned").await;
    assert_eq!(pruned.len(), 1);
    assert_eq!(pruned[0].1["id"], win1_build["id"]);
    assert_eq!(pruned[0].1["keep"], 5);
    let count: i64 = sqlx::query_scalar("select count(*) from releases").fetch_one(pool).await.unwrap();
    assert_eq!(count, 7);

    // ── Deleting: administrators only; the publish token is no administrator ──
    let doomed = format!("/api/downloads/{}", windows[1]["id"].as_str().unwrap());
    let send = |who| env.send("DELETE", &doomed, who, Body::empty(), None);
    assert_refusal(&send(As::Nobody).await, StatusCode::UNAUTHORIZED, "unauthorized");
    assert_refusal(&send(As::Bearer(PUBLISH)).await, StatusCode::UNAUTHORIZED, "unauthorized");
    assert_refusal(&send(As::Cookie(&user)).await, StatusCode::FORBIDDEN, "forbidden");
    assert_refusal(&send(As::Bearer(&user_full)).await, StatusCode::FORBIDDEN, "forbidden");
    assert_refusal(&send(As::Bearer(&admin_write)).await, StatusCode::FORBIDDEN, "insufficient_scope");
    let a = send(As::Cookie(&admin)).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    assert_eq!(a.json(), json!({ "id": windows[1]["id"], "deleted": true }));
    assert!(env.stored(&windows[1]).is_none());
    assert_refusal(&send(As::Cookie(&admin)).await, StatusCode::NOT_FOUND, "not_found");
    let gone = env.get(windows[1]["url"].as_str().unwrap(), As::Cookie(&user)).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    let deleted = env.audit("downloads.deleted").await;
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].0.as_deref(), Some("adm"));
    assert_eq!(deleted[0].1["version"], "1.0.2");
    // An administrator's full token deletes too.
    let a = env
        .send(
            "DELETE",
            &format!("/api/downloads/{}", arm_build["id"].as_str().unwrap()),
            As::Bearer(&admin_full),
            Body::empty(),
            None,
        )
        .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text());

    // ── The page: the same list, behind the panel's sign-in ──
    let page = env.get("/_page?path=%2Fdashboard%2Fdownloads", As::Cookie(&user)).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.text());
    let page = page.json();
    assert_eq!(page["route"], "/dashboard/downloads");
    assert_eq!(page["props"], env.list(As::Cookie(&user)).await);
    assert_eq!(page["meta"]["title"], "Downloads - Modlight");
    let page = env.get("/_page?path=%2Fdashboard%2Fdownloads", As::Cookie(&admin)).await.json();
    assert_eq!(page["props"]["admin"], true);
    assert_eq!(page["props"]["publishToken"], true);
    let polish = call(
        &env.app,
        request("GET", "/_page?path=%2Fdashboard%2Fdownloads")
            .header("cookie", format!("{user}; ml_locale=pl"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .json();
    assert_eq!(polish["meta"]["title"], "Do pobrania - Modlight");
    assert_eq!(
        env.get("/_page?path=%2Fdashboard%2Fdownloads", As::Nobody).await.json()["redirect"],
        "/login?next=%2Fdashboard%2Fdownloads"
    );
    let doc = env.get("/dashboard/downloads", As::Cookie(&user)).await;
    assert_eq!(doc.status, StatusCode::OK);
    assert!(doc.header("content-type").unwrap().starts_with("text/html"));

    // ── `/downloads/...` that names no build is still upstream's media action ──
    sql(pool, "insert into \"user\" (id, name, email, role, user_code, theme) values ('dl', 'Old account', 'dl@example.test', 'user', 'downloads', 'modern')").await;
    sql(pool, "insert into uploads (owner_id, code, filename, ext, mime_type, size, storage_key, delete_token) values ('dl', 'old123', 'hello.txt', 'txt', 'text/plain', 5, 'downloads/old123.txt', 'dt-dl')").await;
    std::fs::create_dir_all(env.storage.join("downloads")).unwrap();
    std::fs::write(env.storage.join("downloads/old123.txt"), b"hello").unwrap();
    let a = env.get("/downloads/old123/raw", As::Nobody).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    assert_eq!(a.text(), "hello");

    // ── The admin page's storage scan leaves the builds alone ──
    let later = modlight_server::db::models::JsDate::now().millis() + 2 * 3600 * 1000;
    let scan = modlight_server::data::admin::scan_storage_orphans(pool, &env.app.state.config, later).await.unwrap();
    assert_eq!((scan.scanned, scan.orphans), (1, 0), "{:?}", scan.sample);
}
