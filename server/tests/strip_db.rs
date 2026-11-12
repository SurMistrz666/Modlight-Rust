//! Removing metadata on upload (SKELETON.md 9.47) end to end, against a real
//! PostgreSQL and a real storage directory: the switch (API and RPC, token
//! scopes), ShareX, tus from the panel (session) and from the app (token), the
//! stored bytes, size and checksum describing the cleaned file, duplicates of
//! the cleaned file, a broken file stored as it came and listed for its owner,
//! an encrypted account that cleans first and encrypts after, documents, and an
//! account with the switch off whose uploads are stored byte for byte.
//!
//! Runs only when `MODLIGHT_TEST_DATABASE_URL` names a server where the user
//! may create databases (see tests/db_compat.rs).

mod common;

use std::io::Write as _;
use std::str::FromStr;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{Answer, call};
use modlight_server::api_token::{TokenScope, create_api_token};
use modlight_server::config::Config;
use modlight_server::http::App;
use modlight_server::paths::Paths;
use modlight_server::state::AppState;
use serde_json::{Value, json};
use sha2::Digest;
use sqlx::postgres::PgConnectOptions;
use sqlx::{ConnectOptions, PgPool};
use uuid::Uuid;

const SECRET: &str = "strip-test-secret-0123456789abcdef0123456789";
const HOST: &str = "127.0.0.1:8421";
const FILE_PASSWORD: &str = "correct horse battery";
const SIGN_IN_PASSWORD: &str = "the sign-in password";

struct World {
    admin: PgConnectOptions,
    name: String,
    pool: PgPool,
    state: AppState,
    app: App,
    root: tempfile::TempDir,
}

impl World {
    async fn new() -> Option<World> {
        let url = std::env::var("MODLIGHT_TEST_DATABASE_URL").ok()?;
        let admin = PgConnectOptions::from_str(&url).expect("MODLIGHT_TEST_DATABASE_URL");
        let name = format!("modlight_strip_{}_{}", std::process::id(), rand::random::<u32>());
        let mut conn = admin.connect().await.expect("connect to the test server");
        sqlx::query(&format!("CREATE DATABASE \"{name}\"")).execute(&mut conn).await.expect("create database");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(8)
            .connect_with(admin.clone().database(&name).options([("TimeZone", "UTC")]))
            .await
            .expect("connect to the temp database");
        modlight_server::db::migrate::run(&pool, &Paths::crate_dir().migrations, false).await.expect("migrate");
        let root = tempfile::tempdir().unwrap();
        let state = state_on(&pool, root.path());
        let app = App::new(state.clone());
        Some(World { admin, name, pool, state, app, root })
    }

    async fn drop(self) {
        self.pool.close().await;
        if let Ok(mut conn) = self.admin.connect().await {
            let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)", self.name))
                .execute(&mut conn)
                .await;
        }
    }

    fn dir(&self, name: &str) -> std::path::PathBuf {
        self.root.path().join(name)
    }

    fn stored(&self, key: &str) -> Vec<u8> {
        std::fs::read(self.dir("storage").join(key)).unwrap_or_default()
    }

    fn left_in(&self, name: &str) -> Vec<String> {
        std::fs::read_dir(self.dir(name))
            .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default()
    }

    /// An account with a sign-in password; returns its id (also its tag).
    async fn user(&self) -> String {
        let id = format!("u{}", rand::random::<u32>());
        sqlx::query(
            r#"insert into "user" ("id", "name", "email", "email_verified", "created_at", "updated_at", "user_code", "role", "theme")
               values ($1, $1, $1 || '@example.test', true, now(), now(), $1, 'user', 'graphite')"#,
        )
        .bind(&id)
        .execute(&self.pool)
        .await
        .unwrap();
        let hash = modlight_server::auth::password::hash_password_blocking(SIGN_IN_PASSWORD).unwrap();
        sqlx::query(
            r#"insert into "account" ("id", "account_id", "provider_id", "user_id", "password", "created_at", "updated_at")
               values ($1, $2, 'credential', $2, $3, now(), now())"#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&id)
        .bind(hash)
        .execute(&self.pool)
        .await
        .unwrap();
        id
    }

    async fn session(&self, user_id: &str) -> String {
        let token = format!("tok{}", Uuid::new_v4().simple());
        sqlx::query(
            r#"insert into "session" ("id", "token", "user_id", "expires_at", "created_at", "updated_at")
               values ($1, $2, $3, now() + interval '7 days', now(), now())"#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&token)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .unwrap();
        format!("better-auth.session_token={}", modlight_server::auth::cookies::sign_cookie_value(&token, SECRET))
    }

    async fn token(&self, user_id: &str, scope: TokenScope) -> String {
        create_api_token(&self.pool, user_id, "test", scope).await.unwrap()
    }

    async fn request(&self, req: axum::http::request::Builder, body: Body) -> Answer {
        call(&self.app, req.header("host", HOST).header("x-real-ip", "198.51.100.1").body(body).unwrap()).await
    }

    async fn get(&self, uri: &str, headers: &[(&str, &str)]) -> Answer {
        let mut b = Request::get(uri);
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        self.request(b, Body::empty()).await
    }

    async fn switch(&self, token: &str, enabled: bool) -> Answer {
        self.request(
            Request::put("/api/strip-metadata").header("x-api-key", token).header("content-type", "application/json"),
            Body::from(json!({ "enabled": enabled }).to_string()),
        )
        .await
    }

    async fn rpc(&self, cookie: &str, module: &str, export: &str, args: Value) -> (Value, Vec<String>) {
        let req = Request::post(format!("/_rpc/{module}/{export}"))
            .header("origin", format!("http://{HOST}"))
            .header("x-modlight-rpc", "1")
            .header("content-type", "application/json")
            .header("cookie", cookie);
        let a = self.request(req, Body::from(args.to_string())).await;
        let cookies = a.headers.get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect();
        (a.json(), cookies)
    }

    async fn sharex(&self, token: &str, filename: &str, mime: &str, bytes: &[u8]) -> Answer {
        let boundary = "----strip";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"{filename}\"\r\nContent-Type: {mime}\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        self.request(
            Request::post("/upload")
                .header("content-type", format!("multipart/form-data; boundary={boundary}"))
                .header("x-api-key", token),
            Body::from(body),
        )
        .await
    }

    /// A whole tus upload in one PATCH, through this world's own tus server
    /// (the process-wide one belongs to whichever test made it first).
    async fn tus(&self, auth: (&str, &str), filename: &str, mime: &str, bytes: &[u8]) -> Answer {
        let server = modlight_server::upload::tus::create_tus_server(&self.state);
        let path = self.tus_create(&server, auth, filename, mime, bytes.len()).await;
        self.tus_send(&server, Request::patch(&path).header("upload-offset", "0"), bytes).await
    }

    /// POST a tus upload of `length` bytes; returns its path.
    async fn tus_create(
        &self,
        server: &modlight_server::upload::tus_server::server::TusServer,
        auth: (&str, &str),
        filename: &str,
        mime: &str,
        length: usize,
    ) -> String {
        let b64 = |s: &str| base64::Engine::encode(&base64::engine::general_purpose::STANDARD, s);
        let created = server
            .handle(
                Request::post("/api/files")
                    .header("host", HOST)
                    .header("tus-resumable", "1.0.0")
                    .header("upload-length", length.to_string())
                    .header("upload-metadata", format!("filename {},filetype {}", b64(filename), b64(mime)))
                    .header(auth.0, auth.1)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        let created = answer(created).await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
        let location = created.header("location").unwrap();
        location.split_once(HOST).map(|(_, p)| p.to_string()).unwrap_or(location)
    }

    /// One more request to a tus upload (PATCH bytes, a HEAD).
    async fn tus_send(
        &self,
        server: &modlight_server::upload::tus_server::server::TusServer,
        req: axum::http::request::Builder,
        bytes: &[u8],
    ) -> Answer {
        let req = req.header("host", HOST).header("tus-resumable", "1.0.0");
        let req = if bytes.is_empty() { req } else { req.header("content-type", "application/offset+octet-stream") };
        answer(server.handle(req.body(Body::from(bytes.to_vec())).unwrap()).await).await
    }

    async fn newest(&self, owner: &str) -> Row {
        sqlx::query_as(
            r#"select "id", "code", "ext", "size", "storage_key", "checksum", "metadata_kept", "encryption", "width", "height"
               from "uploads" where "owner_id" = $1 order by "created_at" desc limit 1"#,
        )
        .bind(owner)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn used_quota(&self, owner: &str) -> i64 {
        sqlx::query_scalar(r#"select coalesce("used_quota", 0) from "user" where "id" = $1"#)
            .bind(owner)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

#[derive(Debug, sqlx::FromRow)]
struct Row {
    id: Uuid,
    code: String,
    ext: String,
    size: i64,
    storage_key: String,
    checksum: Option<String>,
    metadata_kept: Option<String>,
    encryption: Option<String>,
    width: Option<i32>,
    height: Option<i32>,
}

async fn answer(res: axum::response::Response) -> Answer {
    use http_body_util::BodyExt;
    let (parts, body) = res.into_parts();
    Answer { status: parts.status, headers: parts.headers, body: body.collect().await.unwrap().to_bytes() }
}

fn state_on(pool: &PgPool, root: &std::path::Path) -> AppState {
    let storage = root.join("storage");
    for dir in ["storage", "tmp", "tus"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    let vars: std::collections::HashMap<String, String> = [
        ("APP_URL", "http://127.0.0.1:8421".to_string()),
        ("NODE_ENV", "production".to_string()),
        ("SKIP_ENV_VALIDATION", "1".to_string()),
        ("AUTH_SECRET", SECRET.to_string()),
        ("TRUST_PROXY", "true".to_string()),
        ("STORAGE_LOCAL_PATH", storage.to_string_lossy().into_owned()),
        ("UPLOAD_TMP_PATH", root.join("tmp").to_string_lossy().into_owned()),
        ("TUS_TMP_PATH", root.join("tus").to_string_lossy().into_owned()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let mut state = AppState::new(Config::from_env_map(&vars).unwrap(), Paths::crate_dir(), pool.clone()).unwrap();
    state.storage = modlight_server::storage::Storage::with_driver(Arc::new(
        modlight_server::storage::LocalDriver::new(storage.to_str().unwrap()),
    ));
    state
}

macro_rules! world {
    () => {
        match World::new().await {
            Some(w) => w,
            None => {
                eprintln!("MODLIGHT_TEST_DATABASE_URL not set: skipped");
                return;
            }
        }
    };
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn segment(marker: u8, body: &[u8]) -> Vec<u8> {
    let mut v = vec![0xFF, marker];
    v.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    v.extend_from_slice(body);
    v
}

/// A real JPEG (libvips) with EXIF (rotated 6, a camera, an author), XMP and a
/// comment planted.
fn phone_jpeg() -> Vec<u8> {
    use modlight_server::media::vips::{JpegOptions, Sharp};
    let raw: Vec<u8> = (0..40 * 30).flat_map(|i| [(i % 251) as u8, (i % 13 * 19) as u8, 120]).collect();
    let plain =
        Sharp::raw(bytes::Bytes::from(raw), 40, 30, 3).jpeg(JpegOptions::default()).to_buffer_blocking().unwrap();
    let mut exif = b"Exif\0\0II*\0\x08\0\0\0\x02\0".to_vec();
    exif.extend_from_slice(&[0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0]);
    exif.extend_from_slice(&[0x3B, 0x01, 2, 0, 13, 0, 0, 0, 38, 0, 0, 0]);
    exif.extend_from_slice(&[0, 0, 0, 0]);
    exif.extend_from_slice(b"Jan Kowalski\0");
    let mut out = plain[..2].to_vec();
    out.extend(segment(0xE1, &exif));
    out.extend(segment(0xE1, b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>PLANTED GPS 52.2297</x:xmpmeta>"));
    out.extend(segment(0xFE, b"PLANTED comment"));
    out.extend_from_slice(&plain[2..]);
    out
}

fn assert_clean(name: &str, bytes: &[u8]) {
    for planted in ["PLANTED", "Kowalski", "52.2297"] {
        assert!(!contains(bytes, planted.as_bytes()), "{name}: {planted} survived");
    }
}

/// The keys of a ShareX answer, sorted.
fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

// ── The switch ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_switch_is_off_until_turned_on_and_follows_token_scopes() {
    let w = world!();
    let owner = w.user().await;
    let full = w.token(&owner, TokenScope::Full).await;
    let read = w.token(&owner, TokenScope::Read).await;
    let upload = w.token(&owner, TokenScope::Upload).await;

    let off = w.get("/api/strip-metadata", &[("x-api-key", &read)]).await;
    assert_eq!((off.status, off.json()), (StatusCode::OK, json!({ "enabled": false })));
    assert_eq!(off.header("cache-control").as_deref(), Some("private, no-store"));
    // An upload-only token may not even read it; a read token may not write it.
    assert_eq!(w.get("/api/strip-metadata", &[("x-api-key", &upload)]).await.status, StatusCode::FORBIDDEN);
    assert_eq!(w.switch(&read, true).await.status, StatusCode::FORBIDDEN);
    assert_eq!(w.get("/api/strip-metadata", &[]).await.status, StatusCode::UNAUTHORIZED);

    let on = w.switch(&full, true).await;
    assert_eq!((on.status, on.json()), (StatusCode::OK, json!({ "enabled": true })));
    let stored: bool = sqlx::query_scalar(r#"select "strip_metadata" from "user" where "id" = $1"#)
        .bind(&owner)
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert!(stored);

    // The panel's way: the RPC module, with the session.
    let session = w.session(&owner).await;
    let (res, _) = w.rpc(&session, "strip-metadata", "setStripMetadata", json!([false])).await;
    assert_eq!(res["ok"]["enabled"], false, "{res}");
    assert!(res["ok"]["formats"].as_array().unwrap().iter().any(|f| f == "HEIC"));
    let (res, _) = w.rpc(&session, "strip-metadata", "setStripMetadata", json!(["yes"])).await;
    assert!(res.get("error").is_some(), "{res}");
    let (res, _) = w.rpc(&session, "strip-metadata", "loadStripMetadata", json!([])).await;
    assert_eq!(res["ok"]["enabled"], false);
    w.drop().await;
}

// ── Uploads ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn with_the_switch_off_an_upload_is_stored_byte_for_byte() {
    let w = world!();
    let owner = w.user().await;
    let token = w.token(&owner, TokenScope::Full).await;
    let photo = phone_jpeg();
    let up = w.sharex(&token, "IMG_0001.jpg", "image/jpeg", &photo).await;
    assert_eq!(up.status, StatusCode::CREATED, "{}", up.text());
    let row = w.newest(&owner).await;
    assert_eq!(w.stored(&row.storage_key), photo);
    assert_eq!(row.size, photo.len() as i64);
    assert_eq!(row.checksum.as_deref(), Some(sha256(&photo).as_str()));
    assert_eq!(row.metadata_kept, None);
    // The same through tus: adopted as it is, never hashed (as upstream).
    let tus = w.tus(("x-api-key", &token), "IMG_0002.jpg", "image/jpeg", &photo).await;
    assert_eq!(tus.status, StatusCode::OK, "{}", tus.text());
    let row = w.newest(&owner).await;
    assert_eq!(w.stored(&row.storage_key), photo);
    assert_eq!((row.checksum, row.metadata_kept), (None, None));
    w.drop().await;
}

#[tokio::test]
async fn sharex_uploads_are_cleaned_before_they_are_described() {
    let w = world!();
    let owner = w.user().await;
    let token = w.token(&owner, TokenScope::Full).await;
    let photo = phone_jpeg();
    let before = w.sharex(&token, "before.jpg", "image/jpeg", &photo).await;
    assert_eq!(before.status, StatusCode::CREATED);
    let shape = keys(&before.json());
    assert_eq!(w.switch(&token, true).await.status, StatusCode::OK);
    let quota_before = w.used_quota(&owner).await;

    let up = w.sharex(&token, "IMG_0001.jpg", "image/jpeg", &photo).await;
    assert_eq!(up.status, StatusCode::CREATED, "{}", up.text());
    assert_eq!(keys(&up.json()), shape, "the answer keeps its shape");
    let first = w.newest(&owner).await;
    let stored = w.stored(&first.storage_key);
    assert_clean("sharex", &stored);
    assert!(stored.len() < photo.len());
    assert_eq!(first.size, stored.len() as i64);
    assert_eq!(first.checksum.as_deref(), Some(sha256(&stored).as_str()));
    assert_eq!((first.width, first.height), (Some(40), Some(30)));
    assert_eq!(first.metadata_kept, None);
    assert_eq!(w.used_quota(&owner).await - quota_before, stored.len() as i64, "the quota counts the stored file");
    // The same photo again: a duplicate of the CLEANED first copy.
    let again = w.sharex(&token, "IMG_0001 (1).jpg", "image/jpeg", &photo).await;
    let dup = &again.json()["duplicate_of"];
    assert_eq!(dup["code"], first.code.as_str(), "{}", again.text());
    // Nothing left in the temp folders.
    assert!(w.left_in("tmp").is_empty(), "{:?}", w.left_in("tmp"));
    w.drop().await;
}

#[tokio::test]
async fn tus_uploads_from_the_panel_and_from_the_app_are_cleaned() {
    let w = world!();
    let owner = w.user().await;
    let token = w.token(&owner, TokenScope::Upload).await;
    let full = w.token(&owner, TokenScope::Full).await;
    assert_eq!(w.switch(&full, true).await.status, StatusCode::OK);
    let session = w.session(&owner).await;
    let photo = phone_jpeg();
    for (label, auth) in [("panel", ("cookie", session.as_str())), ("app", ("x-api-key", token.as_str()))] {
        let done = w.tus(auth, &format!("{label}.jpg"), "image/jpeg", &photo).await;
        assert_eq!(done.status, StatusCode::OK, "{label}: {}", done.text());
        assert!(done.header("x-media-url").is_some(), "{label}");
        let row = w.newest(&owner).await;
        let stored = w.stored(&row.storage_key);
        assert_clean(label, &stored);
        assert_eq!(row.size, stored.len() as i64, "{label}");
        assert_eq!(row.checksum.as_deref(), Some(sha256(&stored).as_str()), "{label}: hashed after cleaning");
        assert_eq!(row.metadata_kept, None);
    }
    assert!(w.left_in("tus").is_empty(), "tus leftovers: {:?}", w.left_in("tus"));
    assert!(w.left_in("tmp").is_empty(), "tmp leftovers: {:?}", w.left_in("tmp"));
    w.drop().await;
}

#[tokio::test]
async fn a_file_that_cannot_be_cleaned_is_stored_as_it_came_and_listed() {
    let w = world!();
    let owner = w.user().await;
    let token = w.token(&owner, TokenScope::Full).await;
    assert_eq!(w.switch(&token, true).await.status, StatusCode::OK);
    // A JPEG whose header is cut short: stored, never lost.
    let photo = phone_jpeg();
    let broken = &photo[..60];
    let up = w.sharex(&token, "broken.jpg", "image/jpeg", broken).await;
    assert_eq!(up.status, StatusCode::CREATED, "{}", up.text());
    let row = w.newest(&owner).await;
    assert_eq!(w.stored(&row.storage_key), broken);
    assert_eq!(row.metadata_kept.as_deref(), Some("damaged"));
    assert_eq!(row.checksum.as_deref(), Some(sha256(broken).as_str()));
    // A text file is not something with metadata: nothing to say about it.
    w.sharex(&token, "notes.txt", "text/plain", b"plain words").await;
    assert_eq!(w.newest(&owner).await.metadata_kept, None);

    let session = w.session(&owner).await;
    let (res, _) = w.rpc(&session, "strip-metadata", "loadStripMetadata", json!([])).await;
    let kept = &res["ok"];
    assert_eq!(kept["enabled"], true);
    assert_eq!(kept["keptTotal"], 1);
    assert_eq!(kept["kept"][0]["filename"], "broken.jpg");
    assert_eq!(kept["kept"][0]["reason"], "damaged");
    assert_eq!(kept["kept"][0]["code"], row.code.as_str());
    // The Settings page carries the same.
    let page = w.get("/_page?path=/dashboard/profile", &[("cookie", &session)]).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.text());
    let props = &page.json()["props"]["stripMetadata"];
    assert_eq!(props["enabled"], true, "{props}");
    assert_eq!(props["kept"][0]["reason"], "damaged");
    // A trashed file is not listed.
    sqlx::query(r#"update "uploads" set "deleted_at" = now() where "id" = $1"#)
        .bind(row.id)
        .execute(&w.pool)
        .await
        .unwrap();
    let (res, _) = w.rpc(&session, "strip-metadata", "loadStripMetadata", json!([])).await;
    assert_eq!(res["ok"]["keptTotal"], 0);
    w.drop().await;
}

#[tokio::test]
async fn an_encrypted_account_cleans_first_and_encrypts_after() {
    let w = world!();
    let owner = w.user().await;
    let token = w.token(&owner, TokenScope::Full).await;
    let session = w.session(&owner).await;
    let (res, cookies) =
        w.rpc(&session, "encryption", "enableEncryption", json!([FILE_PASSWORD, FILE_PASSWORD, true])).await;
    assert_eq!(res["ok"], json!({ "ok": true }), "{res}");
    let grant = cookies
        .iter()
        .find(|c| c.starts_with(&format!("ml_vault_{owner}=")))
        .map(|c| c.split(';').next().unwrap().to_string())
        .expect("a grant for this browser");
    assert_eq!(w.switch(&token, true).await.status, StatusCode::OK);

    let photo = phone_jpeg();
    let up = w.sharex(&token, "secret.jpg", "image/jpeg", &photo).await;
    assert_eq!(up.status, StatusCode::CREATED, "{}", up.text());
    let row = w.newest(&owner).await;
    assert!(row.encryption.as_deref().is_some_and(|h| h.starts_with("v1.")), "{:?}", row.encryption);
    let on_disk = w.stored(&row.storage_key);
    assert!(!contains(&on_disk, b"\xFF\xD8\xFF"), "the stored object is ciphertext");
    assert_clean("ciphertext", &on_disk);
    // Opened with the grant: the cleaned picture, described by the row.
    let raw = w.get(&format!("/{owner}/{}/raw", row.code), &[("cookie", &grant)]).await;
    assert_eq!(raw.status, StatusCode::OK, "{}", raw.text());
    assert_clean("decrypted", &raw.body);
    assert!(raw.body.starts_with(b"\xFF\xD8\xFF"));
    assert_eq!(row.size, raw.body.len() as i64);
    assert_eq!(row.checksum.as_deref(), Some(sha256(&raw.body).as_str()));
    assert_eq!(row.metadata_kept, None);
    assert!(w.left_in("tmp").is_empty());
    w.drop().await;
}

/// Every entry of a ZIP, read through its central directory, as `(name, bytes)`.
fn zip_entries(zip: &[u8]) -> Vec<(String, Vec<u8>)> {
    use std::io::Read as _;
    let u16_at = |at: usize| u16::from_le_bytes([zip[at], zip[at + 1]]) as usize;
    let u32_at = |at: usize| u32::from_le_bytes([zip[at], zip[at + 1], zip[at + 2], zip[at + 3]]) as usize;
    let eocd = (0..zip.len() - 21).rev().find(|&i| zip[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]).expect("EOCD");
    let (count, mut at) = (u16_at(eocd + 10), u32_at(eocd + 16));
    let mut out = Vec::new();
    for _ in 0..count {
        let method = u16_at(at + 10);
        let compressed = u32_at(at + 20);
        let (name_len, extra_len, comment_len) = (u16_at(at + 28), u16_at(at + 30), u16_at(at + 32));
        let local = u32_at(at + 42);
        let name = String::from_utf8_lossy(&zip[at + 46..at + 46 + name_len]).into_owned();
        let data_at = local + 30 + u16_at(local + 26) + u16_at(local + 28);
        let raw = &zip[data_at..data_at + compressed];
        let data = if method == 8 {
            let mut v = Vec::new();
            flate2::read::DeflateDecoder::new(raw).read_to_end(&mut v).unwrap();
            v
        } else {
            raw.to_vec()
        };
        out.push((name, data));
        at += 46 + name_len + extra_len + comment_len;
    }
    out
}

/// A minimal .docx: content types, relationships, the document and its properties.
fn docx() -> Vec<u8> {
    let parts: [(&str, &[u8]); 6] = [
        (
            "[Content_Types].xml",
            br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/><Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/><Override PartName="/docProps/custom.xml" ContentType="application/vnd.openxmlformats-officedocument.custom-properties+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties" Target="docProps/custom.xml"/></Relationships>"#,
        ),
        (
            "word/document.xml",
            br#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Umowa najmu</w:t></w:r></w:p></w:body></w:document>"#,
        ),
        ("docProps/core.xml", b"<cp:coreProperties><dc:creator>Jan Kowalski</dc:creator></cp:coreProperties>"),
        ("docProps/app.xml", b"<Properties><Company>PLANTED Firma</Company></Properties>"),
        ("docProps/custom.xml", b"<Properties><property name=\"PLANTED\"/></Properties>"),
    ];
    let mut out = Vec::new();
    let mut directory = Vec::new();
    for (name, data) in parts {
        let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        let body = e.finish().unwrap();
        let start = out.len() as u32;
        let mut common = Vec::new();
        common.extend_from_slice(&20u16.to_le_bytes());
        common.extend_from_slice(&0u16.to_le_bytes());
        common.extend_from_slice(&8u16.to_le_bytes());
        common.extend_from_slice(&0x6000u16.to_le_bytes());
        common.extend_from_slice(&0x5AA5u16.to_le_bytes());
        common.extend_from_slice(&crc32fast::hash(data).to_le_bytes());
        common.extend_from_slice(&(body.len() as u32).to_le_bytes());
        common.extend_from_slice(&(data.len() as u32).to_le_bytes());
        common.extend_from_slice(&(name.len() as u16).to_le_bytes());
        common.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&common);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&body);
        directory.extend_from_slice(b"PK\x01\x02");
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&common);
        directory.extend_from_slice(&[0; 10]);
        directory.extend_from_slice(&start.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let at = out.len() as u32;
    out.extend_from_slice(&directory);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&6u16.to_le_bytes());
    out.extend_from_slice(&6u16.to_le_bytes());
    out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    out.extend_from_slice(&at.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

#[tokio::test]
async fn documents_lose_their_properties_and_keep_their_words() {
    let w = world!();
    let owner = w.user().await;
    let token = w.token(&owner, TokenScope::Full).await;
    assert_eq!(w.switch(&token, true).await.status, StatusCode::OK);
    let doc = docx();
    let up = w.sharex(&token, "umowa.docx", "application/octet-stream", &doc).await;
    assert_eq!(up.status, StatusCode::CREATED, "{}", up.text());
    let row = w.newest(&owner).await;
    let stored = w.stored(&row.storage_key);
    assert_eq!(row.ext, "docx");
    assert_eq!(row.checksum.as_deref(), Some(sha256(&stored).as_str()));
    let entries = zip_entries(&stored);
    let part = |name: &str| entries.iter().find(|(n, _)| n == name).map(|(_, d)| d.clone());
    // The words are still there, the author is not.
    assert!(contains(&part("word/document.xml").unwrap(), b"Umowa najmu"));
    assert_eq!(part("docProps/custom.xml"), None);
    for (name, data) in &entries {
        assert_clean(name, data);
        assert!(!contains(data, b"custom.xml"), "{name} still points at the custom properties");
    }
    assert!(contains(&part("docProps/core.xml").unwrap(), b"coreProperties"));
    w.drop().await;
}

/// Hunt round 2 (SRVF-N4): a tus upload whose finish fails is dropped, so its
/// URL answers 404 and a client that retries sends it again. Before, it stayed
/// complete: a retry's HEAD found nothing left to send and the client reported
/// a success with no file, and on S3 the object never left the bucket.
#[tokio::test]
async fn a_tus_upload_whose_finish_fails_is_dropped() {
    let w = world!();
    let owner = w.user().await;
    let token = w.token(&owner, TokenScope::Upload).await;
    let server = modlight_server::upload::tus::create_tus_server(&w.state);
    let bytes = b"a few plain words, too many for the quota".to_vec();
    let path = w.tus_create(&server, ("x-api-key", &token), "note.txt", "text/plain", bytes.len()).await;
    // The quota shrinks while the bytes are on their way.
    let set_max = |max: Option<i64>| {
        sqlx::query(r#"update "user" set "max_quota" = $1 where "id" = $2"#).bind(max).bind(&owner).execute(&w.pool)
    };
    set_max(Some(4)).await.unwrap();
    let done = w.tus_send(&server, Request::patch(&path).header("upload-offset", "0"), &bytes).await;
    assert_eq!(done.status, StatusCode::INSUFFICIENT_STORAGE, "{}", done.text());
    assert!(w.left_in("tus").is_empty(), "tus leftovers: {:?}", w.left_in("tus"));
    let head = w.tus_send(&server, Request::head(&path), &[]).await;
    assert_eq!(head.status, StatusCode::NOT_FOUND, "{:?}", head.headers);
    assert_eq!(w.used_quota(&owner).await, 0);
    // With room again, the upload starts over and lands.
    set_max(None).await.unwrap();
    let again = w.tus(("x-api-key", &token), "note.txt", "text/plain", &bytes).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text());
    assert_eq!(w.used_quota(&owner).await, bytes.len() as i64);
    assert!(w.left_in("tus").is_empty(), "tus leftovers: {:?}", w.left_in("tus"));
    w.drop().await;
}
