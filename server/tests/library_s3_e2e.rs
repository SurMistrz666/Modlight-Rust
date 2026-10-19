//! The upload paths end to end on S3: tus (create, PATCH, finish into the
//! adopted object), ShareX, the link-info and listing answers, permanent
//! deletion. Runs only when both a test database
//! (`MODLIGHT_TEST_DATABASE_URL`) and an S3 server (`MODLIGHT_TEST_S3_*`, see
//! tests/s3_common) are configured; passes by skipping otherwise.
//!
//! One test in this binary on purpose: the S3 store and the tus server are
//! process-wide (`STORAGE_DRIVER` and `S3_*` are read from the environment,
//! as upstream reads `process.env`).

mod common;
mod s3_common;

use std::collections::HashMap;
use std::str::FromStr;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use modlight_server::http::App;
use modlight_server::paths::Paths;
use modlight_server::state::AppState;
use sqlx::postgres::PgConnectOptions;
use sqlx::{ConnectOptions, PgPool};

struct TempDb {
    admin: PgConnectOptions,
    name: String,
    pool: PgPool,
}

impl TempDb {
    async fn create() -> Option<TempDb> {
        let url = std::env::var("MODLIGHT_TEST_DATABASE_URL").ok()?;
        let admin = PgConnectOptions::from_str(&url).expect("MODLIGHT_TEST_DATABASE_URL");
        let name = format!("modlight_s3e2e_{}_{}", std::process::id(), rand::random::<u32>());
        let mut conn = admin.connect().await.expect("connect to the test server");
        sqlx::query(&format!("CREATE DATABASE \"{name}\"")).execute(&mut conn).await.expect("create database");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect_with(admin.clone().database(&name).options([("TimeZone", "UTC")]))
            .await
            .expect("connect to the temp database");
        modlight_server::db::migrate::run(&pool, &Paths::crate_dir().migrations, false).await.expect("migrate");
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

fn payload(n: usize) -> Vec<u8> {
    (0..n).map(|i| ((i * 31 + 7) & 0xff) as u8).collect()
}

#[tokio::test]
async fn uploads_land_in_s3_and_leave_it_again() {
    let Some(bucket) = s3_common::TestBucket::create(None).await else {
        return;
    };
    let Some(db) = TempDb::create().await else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let tus_dir = tmp.path().join("tus");
    let upload_dir = tmp.path().join("tmp");
    let cfg = &bucket.config;
    let mut vars: Vec<(&str, String)> = vec![
        ("STORAGE_DRIVER", "s3".into()),
        ("S3_BUCKET", cfg.bucket.clone()),
        ("S3_REGION", cfg.region.clone()),
        ("S3_ACCESS_KEY_ID", cfg.access_key_id.clone()),
        ("S3_SECRET_ACCESS_KEY", cfg.secret_access_key.clone()),
        ("S3_FORCE_PATH_STYLE", cfg.force_path_style.to_string()),
        ("TUS_TMP_PATH", tus_dir.to_string_lossy().into_owned()),
        ("UPLOAD_TMP_PATH", upload_dir.to_string_lossy().into_owned()),
        ("APP_URL", "http://127.0.0.1:8421".into()),
        ("NODE_ENV", "production".into()),
        ("SKIP_ENV_VALIDATION", "1".into()),
    ];
    if let Some(endpoint) = &cfg.endpoint {
        vars.push(("S3_ENDPOINT", endpoint.clone()));
    }
    for (k, v) in &vars {
        // SAFETY: the only test in this binary; nothing else reads the
        // environment concurrently.
        unsafe { std::env::set_var(k, v) };
    }
    let map: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    let config = modlight_server::config::Config::from_env_map(&map).expect("config");
    let state = AppState::new(config, Paths::crate_dir(), db.pool.clone()).expect("state");
    let app = App::new(state.clone());

    sqlx::query(
        r#"insert into "user" ("id", "name", "email", "email_verified", "created_at", "updated_at", "user_code", "role", "naming_scheme")
           values ('s3owner', 'S3', 's3@example.test', true, now(), now(), 's3tag', 'user', 'filename')"#,
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let token = modlight_server::api_token::create_api_token(
        &db.pool,
        "s3owner",
        "e2e",
        modlight_server::api_scopes::TokenScope::Full,
    )
    .await
    .unwrap();

    // ── tus: create, two PATCHes, finish (the object is adopted in place) ──
    let data = payload(7 * 1024 * 1024 + 123);
    let meta = format!("filename {},filetype {}", b64("tus-e2e.bin"), b64("application/octet-stream"));
    let created = common::call(
        &app,
        Request::post("/api/files")
            .header("host", "127.0.0.1:8421")
            .header("tus-resumable", "1.0.0")
            .header("upload-length", data.len().to_string())
            .header("upload-metadata", meta)
            .header("x-api-key", &token)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let location = created.header("location").expect("location");
    let path = url::Url::parse(&location).unwrap().path().to_string();
    let half = 5 * 1024 * 1024 + 7;
    let first = patch(&app, &path, 0, &data[..half]).await;
    assert_eq!(first.status, StatusCode::NO_CONTENT, "{}", first.text());
    let last = patch(&app, &path, half, &data[half..]).await;
    assert!(last.status.is_success(), "{} {}", last.status, last.text());
    assert!(last.header("x-media-url").is_some_and(|u| u.contains("/s3tag/tus-e2e")));

    let (key, size): (String, i64) =
        sqlx::query_as(r#"select "storage_key", "size" from "uploads" where "filename" = 'tus-e2e.bin'"#)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(size as usize, data.len());
    let storage = state.storage.get().unwrap();
    assert_eq!(storage.stat(&key).await.unwrap().map(|s| s.size as usize), Some(data.len()));
    assert_eq!(storage.read_buffer(&key).await.unwrap().as_ref(), data.as_slice());

    // ── ShareX into the same bucket ──
    let boundary = "----e2e";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"note.txt\"\r\nContent-Type: text/plain\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(b"hello from sharex\n");
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let sharex = common::call(
        &app,
        Request::post("/upload")
            .header("content-type", format!("multipart/form-data; boundary={boundary}"))
            .header("x-api-key", &token)
            .body(Body::from(body))
            .unwrap(),
    )
    .await;
    assert_eq!(sharex.status, StatusCode::CREATED, "{}", sharex.text());
    let note_key: String = sqlx::query_scalar(r#"select "storage_key" from "uploads" where "filename" = 'note.txt'"#)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(storage.read_buffer(&note_key).await.unwrap().as_ref(), b"hello from sharex\n");

    // ── The listing, then a permanent delete takes the object with the row ──
    let listing = common::call(
        &app,
        Request::get("/api/uploads?sort=name").header("x-api-key", &token).body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(listing.json()["total"], 2);
    let id: uuid::Uuid = sqlx::query_scalar(r#"select "id" from "uploads" where "filename" = 'tus-e2e.bin'"#)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let deleted = common::call(
        &app,
        Request::delete("/api/uploads")
            .header("content-type", "application/json")
            .header("x-api-key", &token)
            .body(Body::from(serde_json::json!({ "ids": [id], "mode": "permanent" }).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(deleted.json()["affected"], 1, "{}", deleted.text());
    assert_eq!(storage.stat(&key).await.unwrap(), None);
    let used: i64 = sqlx::query_scalar(r#"select "used_quota" from "user" where "id" = 's3owner'"#)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(used, "hello from sharex\n".len() as i64);

    db.drop().await;
}

async fn patch(app: &App, path: &str, offset: usize, chunk: &[u8]) -> common::Answer {
    common::call(
        app,
        Request::patch(path)
            .header("tus-resumable", "1.0.0")
            .header("upload-offset", offset.to_string())
            .header("content-type", "application/offset+octet-stream")
            .body(Body::from(chunk.to_vec()))
            .unwrap(),
    )
    .await
}

fn b64(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(s)
}
