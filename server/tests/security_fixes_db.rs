//! PORT.md section 12, security fixes 2 and 3, end to end against a real
//! PostgreSQL (the other fixes have their own tests: 1, 5 and 6 in
//! tests/media_serving_db.rs, 4 in tests/releases.rs, 7 in `password` and
//! `auth::ip`; the integration fixes in `upload::tus_server::server` and
//! tests/library_routes_db.rs, see SKELETON.md 9).
//!
//! Runs only when `MODLIGHT_TEST_DATABASE_URL` names a server where the user
//! may create databases (see tests/db_compat.rs).

mod common;

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use bytes::Bytes;
use common::call;
use modlight_server::api_token::{TokenScope, create_api_token};
use modlight_server::config::Config;
use modlight_server::http::App;
use modlight_server::paths::Paths;
use modlight_server::state::AppState;
use serde_json::json;
use sqlx::postgres::PgConnectOptions;
use sqlx::{ConnectOptions, PgPool};

const SECRET: &str = "security-fixes-test-secret-0123456789abcdef";

struct TempDb {
    admin: PgConnectOptions,
    name: String,
    pool: PgPool,
}

impl TempDb {
    async fn create(registration: bool) -> Option<TempDb> {
        let url = std::env::var("MODLIGHT_TEST_DATABASE_URL").ok()?;
        let admin = PgConnectOptions::from_str(&url).expect("MODLIGHT_TEST_DATABASE_URL");
        let name = format!("modlight_secfix_{}_{}", std::process::id(), rand::random::<u32>());
        let mut conn = admin.connect().await.expect("connect to the test server");
        sqlx::query(&format!("CREATE DATABASE \"{name}\"")).execute(&mut conn).await.expect("create database");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(16)
            .connect_with(admin.clone().database(&name).options([("TimeZone", "UTC")]))
            .await
            .expect("connect to the temp database");
        modlight_server::db::migrate::run(&pool, &Paths::crate_dir().migrations, registration).await.expect("migrate");
        Some(TempDb { admin, name, pool })
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

macro_rules! db_or_skip {
    ($registration:expr) => {
        match TempDb::create($registration).await {
            Some(db) => db,
            None => {
                eprintln!("MODLIGHT_TEST_DATABASE_URL not set: skipped");
                return;
            }
        }
    };
}

/// The application on the temp database, its files under `root`.
fn app_on(db: &TempDb, root: &std::path::Path) -> App {
    let storage = root.join("storage");
    let tmp = root.join("tmp");
    let tus = root.join("tus");
    for dir in [&storage, &tmp, &tus] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let vars: std::collections::HashMap<String, String> = [
        ("APP_URL", "http://127.0.0.1:8421".to_string()),
        ("NODE_ENV", "production".to_string()),
        ("SKIP_ENV_VALIDATION", "1".to_string()),
        ("AUTH_SECRET", SECRET.to_string()),
        ("STORAGE_LOCAL_PATH", storage.to_string_lossy().into_owned()),
        ("UPLOAD_TMP_PATH", tmp.to_string_lossy().into_owned()),
        ("TUS_TMP_PATH", tus.to_string_lossy().into_owned()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let mut state = AppState::new(Config::from_env_map(&vars).unwrap(), Paths::crate_dir(), db.pool.clone()).unwrap();
    state.storage = modlight_server::storage::Storage::with_driver(Arc::new(
        modlight_server::storage::local::LocalDriver::new(storage.to_str().unwrap()),
    ));
    App::new(state)
}

async fn user(db: &PgPool, id: &str, max_quota: i64) {
    sqlx::query(
        r#"insert into "user" ("id", "name", "email", "email_verified", "created_at", "updated_at", "user_code", "role", "theme", "max_quota")
           values ($1, $1, $1 || '@example.test', true, now(), now(), $1, 'user', 'modern', $2)"#,
    )
    .bind(id)
    .bind(max_quota)
    .execute(db)
    .await
    .unwrap();
}

/// A multipart body of `total` bytes that counts what the server pulled.
fn counted_body(boundary: &str, total: usize, pulled: Arc<AtomicU64>) -> Body {
    let head = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"big.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    );
    let tail = format!("\r\n--{boundary}--\r\n");
    let chunk = Bytes::from(vec![0x5au8; 256 * 1024]);
    let mut parts: Vec<Bytes> = vec![Bytes::from(head)];
    let mut left = total;
    while left > 0 {
        let n = left.min(chunk.len());
        parts.push(chunk.slice(..n));
        left -= n;
    }
    parts.push(Bytes::from(tail));
    // Pending between chunks, as a socket is: a parser that reads whatever is
    // ready (multer does) would otherwise take the whole body in one poll.
    let stream = futures_util::stream::unfold(parts.into_iter(), move |mut parts| {
        let pulled = pulled.clone();
        async move {
            tokio::task::yield_now().await;
            let b = parts.next()?;
            pulled.fetch_add(b.len() as u64, Ordering::SeqCst);
            Some((Ok::<Bytes, std::io::Error>(b), parts))
        }
    });
    Body::from_stream(stream)
}

fn files_in(dir: &std::path::Path) -> usize {
    walk(dir)
}

fn walk(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .map(|d| d.filter_map(|e| e.ok()).map(|e| if e.path().is_dir() { walk(&e.path()) } else { 1 }).sum())
        .unwrap_or(0)
}

/// Fix 2: ShareX authenticates the header token before reading a byte of the
/// body; a request without one may carry a legacy `token` field but writes at
/// most 8 MiB before it has authenticated; a chunked body is held to the
/// account's quota while it streams.
#[tokio::test]
async fn fix2_sharex_authenticates_before_the_body_and_caps_what_streams() {
    let db = db_or_skip!(false);
    let root = tempfile::tempdir().unwrap();
    let app = app_on(&db, root.path());
    user(&db.pool, "sxowner", 1024 * 1024).await;
    let token = create_api_token(&db.pool, "sxowner", "test", TokenScope::Full).await.unwrap();
    let boundary = "----fix2";
    let content_type = format!("multipart/form-data; boundary={boundary}");

    // (a) An unknown header token: refused, the body never pulled.
    let pulled = Arc::new(AtomicU64::new(0));
    let refused = call(
        &app,
        Request::post("/upload")
            .header("content-type", &content_type)
            .header("x-api-key", "ml_definitely-not-a-token")
            .body(counted_body(boundary, 64 * 1024 * 1024, pulled.clone()))
            .unwrap(),
    )
    .await;
    // ShareX's wording for an unknown token (404), as upstream.
    assert_eq!(refused.status, StatusCode::NOT_FOUND, "{}", refused.text());
    assert_eq!(refused.json()["message"], "Token specified not found.");
    assert_eq!(pulled.load(Ordering::SeqCst), 0, "the body was read before the token was checked");

    // (b) No header token and no legacy field: refused after at most 8 MiB.
    let pulled = Arc::new(AtomicU64::new(0));
    let anon = call(
        &app,
        Request::post("/upload")
            .header("content-type", &content_type)
            .body(counted_body(boundary, 64 * 1024 * 1024, pulled.clone()))
            .unwrap(),
    )
    .await;
    assert_eq!(anon.status, StatusCode::PAYLOAD_TOO_LARGE, "{}", anon.text());
    assert!(pulled.load(Ordering::SeqCst) < 16 * 1024 * 1024, "pulled {}", pulled.load(Ordering::SeqCst));

    // (c) A valid token, a chunked body over the account's 1 MiB quota:
    // 507 while streaming, long before the 64 MiB arrive; nothing kept.
    let pulled = Arc::new(AtomicU64::new(0));
    let over = call(
        &app,
        Request::post("/upload")
            .header("content-type", &content_type)
            .header("x-api-key", &token)
            .body(counted_body(boundary, 64 * 1024 * 1024, pulled.clone()))
            .unwrap(),
    )
    .await;
    assert_eq!(over.status.as_u16(), 507, "{}", over.text());
    assert_eq!(over.json(), json!({ "version": "0.1.0", "message": "User disk quota exceeded." }));
    assert!(pulled.load(Ordering::SeqCst) < 32 * 1024 * 1024, "pulled {}", pulled.load(Ordering::SeqCst));
    assert_eq!(files_in(&root.path().join("tmp")), 0);
    assert_eq!(files_in(&root.path().join("storage")), 0);

    // The same account within its quota still uploads.
    let pulled = Arc::new(AtomicU64::new(0));
    let ok = call(
        &app,
        Request::post("/upload")
            .header("content-type", &content_type)
            .header("x-api-key", &token)
            .body(counted_body(boundary, 1000, pulled))
            .unwrap(),
    )
    .await;
    assert_eq!(ok.status, StatusCode::CREATED, "{}", ok.text());
    db.drop().await;
}

/// Fix 3: the first account becomes the administrator, decided inside a
/// transaction holding an advisory lock: concurrent sign-ups on an empty
/// install produce exactly one administrator.
#[tokio::test]
async fn fix3_concurrent_first_sign_ups_make_exactly_one_admin() {
    let db = db_or_skip!(true);
    let root = tempfile::tempdir().unwrap();
    let app = app_on(&db, root.path());
    let mut tasks = Vec::new();
    for i in 0..12 {
        let app = app.clone();
        tasks.push(tokio::spawn(async move {
            let body = json!({ "email": format!("first{i}@example.test"), "password": "correct horse battery", "name": format!("First {i}") });
            call(
                &app,
                Request::post("/api/auth/sign-up/email")
                    .header("host", "127.0.0.1:8421")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .status
        }));
    }
    let mut created = 0;
    for t in tasks {
        if t.await.unwrap() == StatusCode::OK {
            created += 1;
        }
    }
    let accounts: i64 = sqlx::query_scalar(r#"select count(*) from "user""#).fetch_one(&db.pool).await.unwrap();
    let admins: i64 =
        sqlx::query_scalar(r#"select count(*) from "user" where "role" = 'admin'"#).fetch_one(&db.pool).await.unwrap();
    assert_eq!(accounts, created);
    assert!(created >= 2, "only {created} sign-ups went through");
    assert_eq!(admins, 1, "{admins} administrators out of {accounts} accounts");
    db.drop().await;
}
