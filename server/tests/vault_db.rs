//! Per-account file encryption end to end (docs/ENCRYPTION.md) against a real
//! PostgreSQL and a real storage directory: switching on and off, uploads from
//! a bearer client, every public surface asking for the file password, grants
//! (expiry, revocation, password change), the background pass (with an
//! interruption and a restart), the stored bytes being ciphertext, and the
//! rate limits.
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
use modlight_server::vault::pass::{Stop, run_with};
use serde_json::{Value, json};
use sqlx::postgres::PgConnectOptions;
use sqlx::{ConnectOptions, PgPool};
use uuid::Uuid;

const SECRET: &str = "vault-test-secret-0123456789abcdef0123456789";
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
        let name = format!("modlight_vault_{}_{}", std::process::id(), rand::random::<u32>());
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

    /// The same database and storage seen by a new process (a restart).
    fn restarted(&self) -> AppState {
        state_on(&self.pool, self.root.path())
    }

    async fn drop(self) {
        self.pool.close().await;
        if let Ok(mut conn) = self.admin.connect().await {
            let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)", self.name))
                .execute(&mut conn)
                .await;
        }
    }

    fn storage_dir(&self) -> std::path::PathBuf {
        self.root.path().join("storage")
    }

    /// An account with a sign-in password; returns its id (also its tag).
    async fn user(&self, role: &str) -> String {
        let id = format!("u{}", rand::random::<u32>());
        sqlx::query(
            r#"insert into "user" ("id", "name", "email", "email_verified", "created_at", "updated_at", "user_code", "role", "theme")
               values ($1, $1, $1 || '@example.test', true, now(), now(), $1, $2, 'graphite')"#,
        )
        .bind(&id)
        .bind(role)
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

    /// A plaintext file with its bytes on disk (as an older upload would be).
    async fn file(&self, owner: &str, code: &str, filename: &str, mime: &str, bytes: &[u8]) -> Uuid {
        let ext = filename.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        let key = format!("{owner}/{code}.{ext}");
        let path = self.storage_dir().join(&key);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        sqlx::query_scalar(
            r#"insert into "uploads" ("owner_id", "code", "filename", "ext", "mime_type", "size", "storage_key", "delete_token", "blurhash")
               values ($1, $2, $3, $4, $5, $6, $7, 'dt-' || $2, 'LKO2?U%2Tw=w]~RBVZRi};RPxuwH') returning "id""#,
        )
        .bind(owner)
        .bind(code)
        .bind(filename)
        .bind(ext)
        .bind(mime)
        .bind(bytes.len() as i64)
        .bind(&key)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn row(&self, id: Uuid) -> (String, Option<String>, Option<String>) {
        sqlx::query_as(r#"select "storage_key", "encryption", "blurhash" from "uploads" where "id" = $1"#)
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    fn stored(&self, key: &str) -> Vec<u8> {
        std::fs::read(self.storage_dir().join(key)).unwrap_or_default()
    }

    async fn get(&self, uri: &str, headers: &[(&str, &str)]) -> Answer {
        let mut b = Request::get(uri).header("host", HOST).header("x-real-ip", "198.51.100.1");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        call(&self.app, b.body(Body::empty()).unwrap()).await
    }

    /// One RPC call: (envelope, set-cookie headers).
    async fn rpc(&self, cookie: &str, ip: &str, module: &str, export: &str, args: Value) -> (Value, Vec<String>) {
        let mut req = Request::post(format!("/_rpc/{module}/{export}"))
            .header("host", HOST)
            .header("origin", format!("http://{HOST}"))
            .header("x-modlight-rpc", "1")
            .header("x-real-ip", ip)
            .header("content-type", "application/json");
        if !cookie.is_empty() {
            req = req.header("cookie", cookie);
        }
        let a = call(&self.app, req.body(Body::from(args.to_string())).unwrap()).await;
        let cookies = a.headers.get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect();
        (a.json(), cookies)
    }

    async fn vault_state(&self, user_id: &str) -> Option<String> {
        sqlx::query_scalar(r#"select "state" from "file_vaults" where "user_id" = $1"#)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .unwrap()
    }

    /// Switch encryption on through the panel; returns the browser's grant
    /// cookie (`ml_vault_<id>=<grant>`).
    async fn enable(&self, user_id: &str, session: &str) -> String {
        let (res, cookies) = self
            .rpc(session, "192.0.2.1", "encryption", "enableEncryption", json!([FILE_PASSWORD, FILE_PASSWORD, true]))
            .await;
        assert_eq!(res["ok"], json!({ "ok": true }), "{res}");
        grant_cookie(&cookies, user_id).expect("the browser that switched it on is unlocked")
    }

    async fn pass(&self, limit: u64) -> u64 {
        run_with(&self.state, false, &Stop::default(), limit).await.unwrap()
    }
}

fn state_on(pool: &PgPool, root: &std::path::Path) -> AppState {
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
        ("TRUST_PROXY", "true".to_string()),
        ("STORAGE_LOCAL_PATH", storage.to_string_lossy().into_owned()),
        ("UPLOAD_TMP_PATH", tmp.to_string_lossy().into_owned()),
        ("TUS_TMP_PATH", tus.to_string_lossy().into_owned()),
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

/// `ml_vault_<id>=<value>` out of a response's Set-Cookie headers.
fn grant_cookie(set_cookies: &[String], user_id: &str) -> Option<String> {
    let prefix = format!("ml_vault_{user_id}=");
    set_cookies
        .iter()
        .find(|c| c.starts_with(&prefix))
        .map(|c| c.split(';').next().unwrap().to_string())
        .filter(|c| c.len() > prefix.len())
}

fn with(cookies: &[&str]) -> String {
    cookies.iter().filter(|c| !c.is_empty()).copied().collect::<Vec<_>>().join("; ")
}

/// A valid RGB PNG (stored blocks, no compression needed by the reader).
fn png(width: u32, height: u32) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut crc = crc32fast::Hasher::new();
        crc.update(kind);
        crc.update(data);
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&crc.finalize().to_be_bytes());
    }
    let mut raw = Vec::new();
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            raw.extend_from_slice(&[(x * 40) as u8, (y * 60) as u8, 200]);
        }
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&raw).unwrap();
    let idat = z.finish().unwrap();
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Every entry of a ZIP, read through its central directory (stored or
/// deflated), as `(name, bytes)`.
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

fn noise(n: usize) -> Vec<u8> {
    (0..n).map(|i| ((i * 7919) ^ (i >> 3)) as u8).collect()
}

async fn sharex(w: &World, token: &str, filename: &str, mime: &str, bytes: &[u8]) -> Answer {
    let boundary = "----vault";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"{filename}\"\r\nContent-Type: {mime}\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    call(
        &w.app,
        Request::post("/upload")
            .header("host", HOST)
            .header("content-type", format!("multipart/form-data; boundary={boundary}"))
            .header("x-api-key", token)
            .body(Body::from(body))
            .unwrap(),
    )
    .await
}

async fn newest(w: &World, owner: &str) -> (Uuid, String, String) {
    sqlx::query_as(
        r#"select "id", "code", "ext" from "uploads" where "owner_id" = $1 order by "created_at" desc limit 1"#,
    )
    .bind(owner)
    .fetch_one(&w.pool)
    .await
    .unwrap()
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

// ── Switching on, uploads from a bearer client, reading with a grant ─────────

#[tokio::test]
async fn bearer_uploads_are_stored_encrypted_and_read_only_with_a_grant() {
    let w = world!();
    let owner = w.user("user").await;
    let session = w.session(&owner).await;

    // The refusals of the switch, each with its reason and nothing created.
    for (args, reason) in [
        (json!(["short", "short", true]), "too-short"),
        (json!([FILE_PASSWORD, "something else", true]), "mismatch"),
        (json!([FILE_PASSWORD, FILE_PASSWORD, false]), "not-acknowledged"),
        (json!([SIGN_IN_PASSWORD, SIGN_IN_PASSWORD, true]), "same-as-sign-in"),
    ] {
        let (res, _) = w.rpc(&session, "192.0.2.1", "encryption", "enableEncryption", args).await;
        assert_eq!(res["ok"], json!({ "ok": false, "reason": reason }), "{res}");
    }
    assert_eq!(w.vault_state(&owner).await, None);

    let grant = w.enable(&owner, &session).await;
    assert_eq!(w.vault_state(&owner).await.as_deref(), Some("encrypting"));
    w.pass(u64::MAX).await;
    assert_eq!(w.vault_state(&owner).await.as_deref(), Some("on"));
    let (again, _) = w
        .rpc(&session, "192.0.2.1", "encryption", "enableEncryption", json!([FILE_PASSWORD, FILE_PASSWORD, true]))
        .await;
    assert_eq!(again["ok"]["reason"], "already-on");

    // ShareX (a bearer client) uploads exactly as before: no password involved.
    let token = create_api_token(&w.pool, &owner, "ShareX", TokenScope::Full).await.unwrap();
    let picture = png(5, 3);
    let up = sharex(&w, &token, "shot.png", "image/png", &picture).await;
    assert_eq!(up.status, StatusCode::CREATED, "{}", up.text());
    let (id, code, _) = newest(&w, &owner).await;
    let (key, header, blurhash) = w.row(id).await;
    assert!(header.as_deref().is_some_and(|h| h.starts_with("v1.65536.")), "{header:?}");
    assert_eq!(blurhash, None, "no picture of an encrypted file is kept");
    let on_disk = w.stored(&key);
    assert_eq!(on_disk.len(), picture.len() + 16);
    assert!(!on_disk.windows(8).any(|win| win == &picture[..8]), "the PNG signature shows through");
    // What was read from the plaintext before it was encrypted.
    let (width, height): (Option<i32>, Option<i32>) =
        sqlx::query_as(r#"select "width", "height" from "uploads" where "id" = $1"#)
            .bind(id)
            .fetch_one(&w.pool)
            .await
            .unwrap();
    assert_eq!((width, height), (Some(5), Some(3)));
    // No temp file left behind.
    assert_eq!(std::fs::read_dir(w.root.path().join("tmp")).unwrap().count(), 0);

    let raw = format!("/{owner}/{code}/raw");
    // The bearer client is told "locked", never "your token is bad".
    let locked = w.get(&raw, &[("x-api-key", &token)]).await;
    assert_eq!(locked.status.as_u16(), 423, "{}", locked.text());
    assert_eq!(locked.json()["error"], "files_locked");
    assert_eq!(locked.header("x-modlight-locked").as_deref(), Some("1"));
    // A browser opening the link gets the unlock form, with a policy that lets
    // it post (the raw URL's own policy would sandbox it).
    let page = w.get(&raw, &[("sec-fetch-dest", "document"), ("accept", "text/html")]).await;
    assert_eq!(page.status, StatusCode::UNAUTHORIZED);
    assert!(page.text().contains(r#"action="/api/encryption/unlock""#));
    assert!(!page.text().contains("shot.png"));
    let csp = page.header("content-security-policy").unwrap();
    assert!(csp.contains("form-action 'self'") && !csp.contains("sandbox") && !csp.contains("script-src"), "{csp}");
    // An <img>: a plain 401.
    let img = w.get(&raw, &[("sec-fetch-dest", "image")]).await;
    assert_eq!(img.status, StatusCode::UNAUTHORIZED);
    // The owner's session alone opens nothing.
    assert_eq!(w.get(&raw, &[("cookie", &session)]).await.status, StatusCode::UNAUTHORIZED);

    // With the grant: the plaintext, never cached publicly.
    let ok = w.get(&raw, &[("cookie", &with(&[&session, &grant]))]).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text());
    assert_eq!(ok.body.as_ref(), picture.as_slice());
    assert_eq!(ok.header("content-length").as_deref(), Some(picture.len().to_string().as_str()));
    assert_eq!(ok.header("cache-control").as_deref(), Some("private, no-store"));
    // A thumbnail is made of the decrypted bytes.
    let thumb = w.get(&format!("{raw}?w=64"), &[("cookie", &grant)]).await;
    assert_eq!(thumb.status, StatusCode::OK);
    assert_eq!(thumb.header("content-type").as_deref(), Some("image/webp"));

    // A larger file, read in ranges across chunk borders (video seeking).
    let big = noise(200_000);
    assert_eq!(sharex(&w, &token, "clip.bin", "application/octet-stream", &big).await.status, StatusCode::CREATED);
    let (_, big_code, _) = newest(&w, &owner).await;
    let big_raw = format!("/{owner}/{big_code}/raw");
    for (a, b) in [(0usize, 0usize), (65_530, 131_080), (131_072, 199_999), (65_536, 65_536)] {
        let r = w.get(&big_raw, &[("cookie", &grant), ("range", &format!("bytes={a}-{b}"))]).await;
        assert_eq!(r.status, StatusCode::PARTIAL_CONTENT, "{a}-{b}");
        assert_eq!(r.header("content-range"), Some(format!("bytes {a}-{b}/200000")));
        assert_eq!(r.body.as_ref(), &big[a..=b], "{a}-{b}");
    }
    let tail = w.get(&big_raw, &[("cookie", &grant), ("range", "bytes=-10")]).await;
    assert_eq!(tail.body.as_ref(), &big[199_990..]);
    let whole = w.get(&format!("/{owner}/{big_code}/download"), &[("cookie", &grant)]).await;
    assert_eq!(whole.body.as_ref(), big.as_slice());

    // The app's way in: the password for a grant, the grant as a header.
    let unlock = |password: &'static str| {
        Request::post("/api/encryption/unlock")
            .header("host", HOST)
            .header("x-real-ip", "192.0.2.50")
            .header("x-api-key", &token)
            .header("content-type", "application/json")
            .body(Body::from(json!({ "password": password, "label": "Modlight for Mac" }).to_string()))
            .unwrap()
    };
    let wrong = call(&w.app, unlock("not it at all")).await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong.json(), json!({ "error": "wrong_password" }));
    let granted = call(&w.app, unlock(FILE_PASSWORD)).await;
    assert_eq!(granted.status, StatusCode::OK, "{}", granted.text());
    let app_grant = granted.json()["grant"].as_str().unwrap().to_string();
    assert!(granted.json()["expiresAt"].as_str().is_some());
    let via_header = w.get(&raw, &[("x-api-key", &token), ("x-modlight-unlock", &app_grant)]).await;
    assert_eq!(via_header.status, StatusCode::OK);
    assert_eq!(via_header.body.as_ref(), picture.as_slice());
    let status = w.get("/api/encryption", &[("x-api-key", &token), ("x-modlight-unlock", &app_grant)]).await;
    assert_eq!(status.json()["state"], "on");
    assert_eq!(status.json()["unlocked"], true);
    assert_eq!(status.json()["progress"], json!({ "done": 2, "total": 2 }));
    // The app forgets it: the grant is revoked on the server.
    let lock = call(
        &w.app,
        Request::post("/api/encryption/lock")
            .header("host", HOST)
            .header("x-modlight-unlock", &app_grant)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(lock.status, StatusCode::NO_CONTENT);
    assert_eq!(w.get(&raw, &[("x-api-key", &token), ("x-modlight-unlock", &app_grant)]).await.status.as_u16(), 423);

    // A paste of the account is encrypted too.
    let (paste, _) = w
        .rpc(
            &session,
            "192.0.2.1",
            "pastes",
            "createPaste",
            json!([{ "content": "secret paste text", "language": "text" }]),
        )
        .await;
    let paste_code = paste["ok"]["code"].as_str().unwrap().to_string();
    let (paste_key, paste_header): (String, Option<String>) =
        sqlx::query_as(r#"select "storage_key", "encryption" from "uploads" where "code" = $1"#)
            .bind(&paste_code)
            .fetch_one(&w.pool)
            .await
            .unwrap();
    assert!(paste_header.is_some());
    assert!(!String::from_utf8_lossy(&w.stored(&paste_key)).contains("secret paste"));
    let read = w.get(&format!("/{owner}/{paste_code}/raw"), &[("cookie", &grant)]).await;
    assert_eq!(read.text(), "secret paste text");

    // Nothing derived was cached for these files.
    assert!(!w.storage_dir().join("derived").exists());
    w.drop().await;
}

// ── Every public surface ────────────────────────────────────────────────────

#[tokio::test]
async fn every_public_surface_asks_for_the_file_password() {
    let w = world!();
    let owner = w.user("user").await;
    let admin = w.user("admin").await;
    let session = w.session(&owner).await;
    let admin_session = w.session(&admin).await;
    let picture = png(4, 4);
    let image = w.file(&owner, "pic", "zq-private-name.png", "image/png", &picture).await;
    let text = w.file(&owner, "txt", "notes.txt", "text/plain", b"hello from a text file").await;
    let page = w.file(&owner, "doc", "page.html", "text/html", b"<p>rendered</p>").await;
    // A public folder holding the picture, a public profile, a short link.
    let folder: Uuid = sqlx::query_scalar(
        r#"insert into "folders" ("owner_id", "name", "slug", "is_public") values ($1, 'Trip', 'trip', true) returning "id""#,
    )
    .bind(&owner)
    .fetch_one(&w.pool)
    .await
    .unwrap();
    sqlx::query(r#"update "uploads" set "folder_id" = $1 where "id" = $2"#)
        .bind(folder)
        .bind(image)
        .execute(&w.pool)
        .await
        .unwrap();
    sqlx::query(r#"update "user" set "public_profile" = true, "skip_tag_in_url" = true where "id" = $1"#)
        .bind(&owner)
        .execute(&w.pool)
        .await
        .unwrap();
    sqlx::query(r#"insert into "short_links" ("owner_id", "code", "target") values ($1, 'go-pic', $2)"#)
        .bind(&owner)
        .bind(format!("http://{HOST}/{owner}/pic"))
        .execute(&w.pool)
        .await
        .unwrap();

    let grant = w.enable(&owner, &session).await;
    w.pass(u64::MAX).await;
    assert_eq!(w.vault_state(&owner).await.as_deref(), Some("on"));
    for id in [image, text, page] {
        assert!(w.row(id).await.1.is_some());
    }
    let anon: &[(&str, &str)] = &[];
    let pic = format!("/{owner}/pic");

    // The file page: the unlock view, nothing about the file, a product-name card.
    let doc = w.get(&format!("/_page?path={pic}"), anon).await;
    let payload = doc.json();
    assert_eq!(payload["props"]["view"], "vault", "{payload}");
    assert_eq!(payload["props"]["mediaId"], image.to_string());
    assert!(!doc.text().contains("zq-private"), "the name leaks: {}", doc.text());
    assert!(!doc.text().contains("og:image") && payload["meta"]["openGraph"].is_null(), "{payload}");
    // Tagless (/{code}) is the same page.
    assert_eq!(w.get("/_page?path=/pic", anon).await.json()["props"]["view"], "vault");
    // A document load of the page: the same view inlined, the brand as title.
    let html = w.get(&pic, &[("accept", "text/html")]).await;
    assert!(html.text().contains("\"view\":\"vault\""));
    assert!(!html.text().contains("zq-private"));

    // Every action of every file: locked for a stranger and for an admin.
    let ticket = modlight_server::media::render_ticket::mint_render_ticket(SECRET, &page.to_string(), None);
    let urls = [
        format!("/{owner}/pic/raw"),
        format!("/{owner}/pic/raw.png"),
        format!("/{owner}/pic/raw?w=256"),
        format!("/{owner}/pic/raw?motion=1"),
        format!("/{owner}/pic/download"),
        format!("/{owner}/pic/convert?to=webp"),
        format!("/{owner}/pic/timeline.vtt"),
        format!("/{owner}/pic/timeline.jpg"),
        format!("/{owner}/txt/raw"),
        format!("/{owner}/doc/render?t={}", modlight_server::http::cookies::encode_uri_component(&ticket)),
        format!("/a/{owner}/trip/zip"),
    ];
    for url in &urls {
        let dest = if url.contains("/render") { "iframe" } else { "image" };
        let stranger = w.get(url, &[("sec-fetch-dest", dest)]).await;
        assert_eq!(stranger.status, StatusCode::UNAUTHORIZED, "{url}: {}", stranger.text());
        let by_admin = w.get(url, &[("sec-fetch-dest", dest), ("cookie", &admin_session)]).await;
        assert_eq!(by_admin.status, StatusCode::UNAUTHORIZED, "{url} (admin)");
    }
    // Unfurlers: what a password-protected file gets.
    let oembed = w.get(&format!("/oembed?url=http://{HOST}/{owner}/pic"), anon).await;
    assert_eq!(oembed.status, StatusCode::NOT_FOUND);
    let info = w.get(&format!("/api/link-info?url=http://{HOST}/{owner}/pic"), anon).await;
    assert_eq!(info.json()["status"], "password-required", "{}", info.text());
    assert!(!info.text().contains("zq-private"));
    // The gallery and the album ask for the password and show no tiles.
    let gallery = w.get(&format!("/_page?path=/u/{owner}"), anon).await.json();
    assert_eq!(gallery["props"]["vault"], json!({ "locked": true, "tag": owner }), "{gallery}");
    assert_eq!(gallery["props"]["items"], json!([]));
    assert!(gallery["meta"]["openGraph"]["images"].as_array().is_none_or(|a| a.is_empty()), "{gallery}");
    let album = w.get(&format!("/_page?path=/a/{owner}/trip"), anon).await.json();
    assert_eq!(album["props"]["vault"]["locked"], true);
    assert_eq!(album["props"]["tiles"], json!([]));
    // A short link still only redirects; its target asks.
    let short = w.get("/s/go-pic", anon).await;
    assert!(short.status.is_redirection(), "{}", short.status);

    // A visitor who types the password on the file page gets in everywhere.
    let (wrong, _) = w
        .rpc(
            "",
            "203.0.113.20",
            "encryption",
            "unlockOwnerFiles",
            json!([{ "mediaId": image }, "nope nope nope", true]),
        )
        .await;
    assert_eq!(wrong["ok"], json!({ "ok": false, "reason": "wrong" }));
    let (res, cookies) = w
        .rpc("", "203.0.113.20", "encryption", "unlockOwnerFiles", json!([{ "mediaId": image }, FILE_PASSWORD, true]))
        .await;
    assert_eq!(res["ok"], json!({ "ok": true }));
    let visitor = grant_cookie(&cookies, &owner).unwrap();
    let remembered = cookies.iter().find(|c| c.starts_with(&format!("ml_vault_{owner}="))).unwrap();
    assert!(remembered.contains("Max-Age=2592000") && remembered.contains("HttpOnly"), "{remembered}");
    assert!(remembered.to_lowercase().contains("samesite=lax"), "{remembered}");
    let opened = w.get(&format!("/_page?path={pic}"), &[("cookie", &visitor)]).await.json();
    assert_eq!(opened["props"]["view"], "media");
    let bytes = w.get(&format!("/{owner}/pic/raw"), &[("cookie", &visitor)]).await;
    assert_eq!(bytes.body.as_ref(), picture.as_slice());
    let shown = w.get(&format!("/_page?path=/{owner}/txt"), &[("cookie", &visitor)]).await.json();
    assert_eq!(shown["props"]["viewer"]["textContent"], "hello from a text file");
    let unlocked_gallery = w.get(&format!("/_page?path=/u/{owner}"), &[("cookie", &visitor)]).await.json();
    assert_eq!(unlocked_gallery["props"]["vault"]["locked"], false);
    assert_eq!(unlocked_gallery["props"]["items"].as_array().unwrap().len(), 3);
    let zip = w.get(&format!("/a/{owner}/trip/zip"), &[("cookie", &visitor)]).await;
    assert_eq!(zip.status, StatusCode::OK);
    assert_eq!(
        zip_entries(&zip.body),
        vec![("zq-private-name.png".to_string(), picture.clone())],
        "the album zip holds the plaintext"
    );
    let converted = w.get(&format!("/{owner}/pic/convert?to=webp"), &[("cookie", &visitor)]).await;
    assert_eq!(converted.status, StatusCode::OK, "{}", converted.text());
    assert!(!w.storage_dir().join("derived").exists(), "a conversion of an encrypted file was cached");
    // The unlock form of a raw link (no script): 303 back, with the cookie.
    let form = call(
        &w.app,
        Request::post("/api/encryption/unlock")
            .header("host", HOST)
            .header("origin", format!("http://{HOST}"))
            .header("x-real-ip", "203.0.113.21")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!(
                "next=%2F{owner}%2Fpic%2Fraw&password={}&remember=1",
                FILE_PASSWORD.replace(' ', "+")
            )))
            .unwrap(),
    )
    .await;
    assert_eq!(form.status, StatusCode::SEE_OTHER, "{}", form.text());
    assert_eq!(form.header("location"), Some(format!("/{owner}/pic/raw")));
    assert!(form.header("set-cookie").unwrap().starts_with(&format!("ml_vault_{owner}=")));
    let bad_form = call(
        &w.app,
        Request::post("/api/encryption/unlock")
            .header("host", HOST)
            .header("x-real-ip", "203.0.113.22")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!("next=%2F{owner}%2Fpic%2Fraw&password=wrong")))
            .unwrap(),
    )
    .await;
    assert_eq!(bad_form.status, StatusCode::UNAUTHORIZED);
    assert!(bad_form.text().contains("Wrong password"));
    // A `next` that leaves the site is refused outright.
    let away = call(
        &w.app,
        Request::post("/api/encryption/unlock")
            .header("host", HOST)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("next=%2F%2Fevil.example.com%2F&password=x"))
            .unwrap(),
    )
    .await;
    assert_eq!(away.status, StatusCode::BAD_REQUEST);

    // The owner's grant works for the owner's library thumbnails too.
    assert_eq!(w.get(&format!("/{owner}/pic/raw?w=64"), &[("cookie", &grant)]).await.status, StatusCode::OK);
    // The admin's list marks the account; the admin still cannot read.
    let (rows, _) = w.rpc(&admin_session, "192.0.2.9", "account", "listAdminUsers", json!([])).await;
    let mine = rows["ok"].as_array().unwrap().iter().find(|r| r["id"] == owner).unwrap().clone();
    assert_eq!(mine["encryption"], "on");
    let other = rows["ok"].as_array().unwrap().iter().find(|r| r["id"] == admin).unwrap().clone();
    assert!(other.get("encryption").is_none(), "an account without a vault keeps the upstream row");
    let files =
        w.get(&format!("/_page?path=/dashboard/admin/users/{owner}/files"), &[("cookie", &admin_session)]).await;
    assert_eq!(files.json()["props"]["encryption"], "on");
    w.drop().await;
}

// ── Grants ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn grants_expire_can_be_revoked_and_die_with_a_password_change() {
    let w = world!();
    let owner = w.user("user").await;
    let session = w.session(&owner).await;
    let picture = png(2, 2);
    w.file(&owner, "pic", "a.png", "image/png", &picture).await;
    let first = w.enable(&owner, &session).await;
    w.pass(u64::MAX).await;
    let raw = format!("/{owner}/pic/raw");
    assert_eq!(w.get(&raw, &[("cookie", &first)]).await.status, StatusCode::OK);

    // Expiry is the server's, whatever the cookie says.
    sqlx::query(r#"update "vault_grants" set "expires_at" = now() - interval '1 second' where "user_id" = $1"#)
        .bind(&owner)
        .execute(&w.pool)
        .await
        .unwrap();
    assert_eq!(w.get(&raw, &[("cookie", &first)]).await.status, StatusCode::UNAUTHORIZED);
    let (status, _) =
        w.rpc(&with(&[&session, &first]), "192.0.2.1", "encryption", "loadEncryptionStatus", json!([])).await;
    assert_eq!(status["ok"]["unlocked"], false);

    // A session cookie (not remembered) lives a day on the server.
    let (res, cookies) =
        w.rpc(&session, "192.0.2.1", "encryption", "unlockMyFiles", json!([FILE_PASSWORD, false])).await;
    assert_eq!(res["ok"], json!({ "ok": true }));
    let session_grant = cookies.iter().find(|c| c.starts_with(&format!("ml_vault_{owner}="))).unwrap();
    assert!(!session_grant.contains("Max-Age"), "{session_grant}");
    let left: f64 = sqlx::query_scalar(
        r#"select extract(epoch from max("expires_at") - now())::float8 from "vault_grants" where "user_id" = $1"#,
    )
    .bind(&owner)
    .fetch_one(&w.pool)
    .await
    .unwrap();
    assert!((86_000.0..=86_400.0).contains(&left), "{left}");
    let second = grant_cookie(&cookies, &owner).unwrap();
    assert_eq!(w.get(&raw, &[("cookie", &second)]).await.status, StatusCode::OK);

    // Listed, then revoked one by one.
    let (status, _) =
        w.rpc(&with(&[&session, &second]), "192.0.2.1", "encryption", "loadEncryptionStatus", json!([])).await;
    let grants = status["ok"]["grants"].as_array().unwrap().clone();
    let current = grants.iter().find(|g| g["current"] == true).expect("this browser is listed").clone();
    assert_eq!(current["kind"], "owner");
    let (revoked, cookies) =
        w.rpc(&with(&[&session, &second]), "192.0.2.1", "encryption", "revokeVaultGrant", json!([current["id"]])).await;
    assert_eq!(revoked["ok"]["ok"], true);
    assert!(cookies.iter().any(|c| c.starts_with(&format!("ml_vault_{owner}=;"))), "{cookies:?}");
    assert_eq!(w.get(&raw, &[("cookie", &second)]).await.status, StatusCode::UNAUTHORIZED);

    // Two live grants (a browser and a visitor), then a new password.
    let (_, c) = w.rpc(&session, "192.0.2.1", "encryption", "unlockMyFiles", json!([FILE_PASSWORD, true])).await;
    let browser = grant_cookie(&c, &owner).unwrap();
    let (_, c) = w
        .rpc("", "203.0.113.5", "encryption", "unlockOwnerFiles", json!([{ "tag": owner }, FILE_PASSWORD, true]))
        .await;
    let visitor = grant_cookie(&c, &owner).unwrap();
    assert_eq!(w.get(&raw, &[("cookie", &visitor)]).await.status, StatusCode::OK);
    let generation = |w: &World| {
        let pool = w.pool.clone();
        let owner = owner.clone();
        async move {
            sqlx::query_scalar::<_, i32>(r#"select "generation" from "file_vaults" where "user_id" = $1"#)
                .bind(owner)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let before = generation(&w).await;
    let (wrong, _) = w
        .rpc(
            &with(&[&session, &browser]),
            "192.0.2.1",
            "encryption",
            "changeFilePassword",
            json!(["nope nope", "a new password", "a new password"]),
        )
        .await;
    assert_eq!(wrong["ok"]["reason"], "wrong");
    let (changed, c) = w
        .rpc(
            &with(&[&session, &browser]),
            "192.0.2.1",
            "encryption",
            "changeFilePassword",
            json!([FILE_PASSWORD, "a new password", "a new password"]),
        )
        .await;
    assert_eq!(changed["ok"], json!({ "ok": true }), "{changed}");
    assert_eq!(generation(&w).await, before + 1);
    let renewed = grant_cookie(&c, &owner).unwrap();
    // Every older grant is dead; the browser that changed it keeps working.
    for old in [&browser, &visitor] {
        assert_eq!(w.get(&raw, &[("cookie", old)]).await.status, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(w.get(&raw, &[("cookie", &renewed)]).await.status, StatusCode::OK);
    // The old password opens nothing, the new one does; the files were not touched.
    let (old_pw, _) = w
        .rpc("", "203.0.113.6", "encryption", "unlockOwnerFiles", json!([{ "tag": owner }, FILE_PASSWORD, true]))
        .await;
    assert_eq!(old_pw["ok"]["reason"], "wrong");
    let (new_pw, _) = w
        .rpc("", "203.0.113.6", "encryption", "unlockOwnerFiles", json!([{ "tag": owner }, "a new password", true]))
        .await;
    assert_eq!(new_pw["ok"], json!({ "ok": true }));

    // Revoke everything.
    let (all, _) = w.rpc(&session, "192.0.2.1", "encryption", "revokeAllVaultGrants", json!([])).await;
    assert!(all["ok"]["count"].as_i64().unwrap() >= 2);
    assert_eq!(w.get(&raw, &[("cookie", &renewed)]).await.status, StatusCode::UNAUTHORIZED);
    // Audit trail.
    let actions: Vec<String> =
        sqlx::query_scalar(r#"select "action" from "audit_log" where "user_id" = $1 order by "created_at""#)
            .bind(&owner)
            .fetch_all(&w.pool)
            .await
            .unwrap();
    for expected in
        ["vault.enabled", "vault.unlocked", "vault.grant_revoked", "vault.password_changed", "vault.grants_revoked"]
    {
        assert!(actions.iter().any(|a| a == expected), "{expected} missing from {actions:?}");
    }
    w.drop().await;
}

// ── The background pass ─────────────────────────────────────────────────────

#[tokio::test]
async fn the_pass_encrypts_existing_files_resumes_after_a_restart_and_decrypts_back() {
    let w = world!();
    let owner = w.user("user").await;
    let session = w.session(&owner).await;
    let mut files = Vec::new();
    for i in 0..5 {
        let bytes = format!("plaintext number {i} ").repeat(5000).into_bytes();
        let id = w.file(&owner, &format!("f{i}"), &format!("f{i}.txt"), "text/plain", &bytes).await;
        files.push((id, format!("f{i}"), bytes));
    }
    // One in the trash: encrypted too.
    sqlx::query(r#"update "uploads" set "deleted_at" = now() where "id" = $1"#)
        .bind(files[4].0)
        .execute(&w.pool)
        .await
        .unwrap();
    let grant = w.enable(&owner, &session).await;
    assert_eq!(w.vault_state(&owner).await.as_deref(), Some("encrypting"));

    // Interrupted after two files.
    assert_eq!(w.pass(2).await, 2);
    let (status, _) =
        w.rpc(&with(&[&session, &grant]), "192.0.2.1", "encryption", "loadEncryptionStatus", json!([])).await;
    assert_eq!(status["ok"]["state"], "encrypting");
    assert_eq!(status["ok"]["progress"], json!({ "done": 2, "total": 5 }));
    // Meanwhile every live file reads the same, encrypted yet or not, and links
    // already ask for the password.
    for (_, code, bytes) in &files[..4] {
        let r = w.get(&format!("/{owner}/{code}/raw"), &[("cookie", &grant)]).await;
        assert_eq!(r.body.as_ref(), bytes.as_slice(), "{code}");
        assert_eq!(w.get(&format!("/{owner}/{code}/raw"), &[]).await.status, StatusCode::UNAUTHORIZED, "{code}");
    }
    // A crash between "the row points at the new object" and "the old one is
    // gone" leaves a plaintext object registered for cleanup.
    let stray = format!("{owner}/stray-copy.txt");
    std::fs::write(w.storage_dir().join(&stray), b"plaintext left by a crash").unwrap();
    sqlx::query(r#"insert into "vault_cleanup" ("storage_key", "user_id") values ($1, $2)"#)
        .bind(&stray)
        .bind(&owner)
        .execute(&w.pool)
        .await
        .unwrap();

    // A restart: a new process, the same database and storage.
    let restarted = w.restarted();
    let moved = run_with(&restarted, true, &Stop::default(), u64::MAX).await.unwrap();
    assert_eq!(moved, 3);
    assert_eq!(w.vault_state(&owner).await.as_deref(), Some("on"));
    assert!(!w.storage_dir().join(&stray).exists(), "the crash's leftover was not deleted");
    let pending: i64 = sqlx::query_scalar(r#"select count(*) from "vault_cleanup""#).fetch_one(&w.pool).await.unwrap();
    assert_eq!(pending, 0);
    // Nothing readable is left in storage: every object is a row's ciphertext.
    let mut objects = Vec::new();
    for entry in std::fs::read_dir(w.storage_dir().join(&owner)).unwrap() {
        objects.push(std::fs::read(entry.unwrap().path()).unwrap());
    }
    assert_eq!(objects.len(), 5);
    for object in &objects {
        assert!(!String::from_utf8_lossy(object).contains("plaintext number"), "a plaintext object survived");
    }
    for (id, _, _) in &files {
        let (_, header, blurhash) = w.row(*id).await;
        assert!(header.is_some());
        assert_eq!(blurhash, None);
    }
    // A plaintext row that raced into an "on" vault sends it back to work.
    let mut tx = w.pool.begin().await.unwrap();
    modlight_server::vault::write::confirm_vault_for_write(&mut tx, &owner, false).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(w.vault_state(&owner).await.as_deref(), Some("encrypting"));
    w.pass(u64::MAX).await;
    assert_eq!(w.vault_state(&owner).await.as_deref(), Some("on"));

    // Off: needs the password; the files come back as they were.
    let (wrong, _) = w.rpc(&session, "192.0.2.1", "encryption", "disableEncryption", json!(["not the password"])).await;
    assert_eq!(wrong["ok"]["reason"], "wrong");
    let (off, _) = w.rpc(&session, "192.0.2.1", "encryption", "disableEncryption", json!([FILE_PASSWORD])).await;
    assert_eq!(off["ok"], json!({ "ok": true }));
    assert_eq!(w.vault_state(&owner).await.as_deref(), Some("decrypting"));
    // While decrypting nobody is asked any more.
    let (_, code, bytes) = &files[0];
    assert_eq!(w.get(&format!("/{owner}/{code}/raw"), &[]).await.body.as_ref(), bytes.as_slice());
    run_with(&w.restarted(), false, &Stop::default(), 2).await.unwrap();
    run_with(&w.restarted(), false, &Stop::default(), u64::MAX).await.unwrap();
    assert_eq!(w.vault_state(&owner).await, None, "the vault is deleted once every file is back");
    let grants: i64 = sqlx::query_scalar(r#"select count(*) from "vault_grants""#).fetch_one(&w.pool).await.unwrap();
    assert_eq!(grants, 0);
    for (id, code, bytes) in &files {
        let (key, header, _) = w.row(*id).await;
        assert_eq!(header, None);
        assert_eq!(&w.stored(&key), bytes, "{code}");
    }
    let (_, code, bytes) = &files[1];
    let open = w.get(&format!("/{owner}/{code}/raw"), &[]).await;
    assert_eq!(open.body.as_ref(), bytes.as_slice());
    assert_ne!(open.header("cache-control").as_deref(), Some("private, no-store"));
    w.drop().await;
}

// ── Rate limits ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn unlock_attempts_are_throttled_per_address_and_per_account() {
    let w = world!();
    let owner = w.user("user").await;
    let session = w.session(&owner).await;
    w.file(&owner, "pic", "a.png", "image/png", &png(1, 1)).await;
    w.enable(&owner, &session).await;
    let target = json!({ "tag": owner });
    for _ in 0..modlight_server::vault::unlock::PER_ADDRESS {
        let (r, _) =
            w.rpc("", "203.0.113.77", "encryption", "unlockOwnerFiles", json!([target, "guess guess", true])).await;
        assert_eq!(r["ok"]["reason"], "wrong");
    }
    // Spent: even the right password waits now.
    let (r, _) =
        w.rpc("", "203.0.113.77", "encryption", "unlockOwnerFiles", json!([target, FILE_PASSWORD, true])).await;
    assert_eq!(r["ok"]["reason"], "rate-limited");
    assert!(r["ok"]["retryAfter"].as_i64().unwrap() > 0);
    // Another address, and the owner in the panel, are not locked out.
    let (r, _) =
        w.rpc("", "203.0.113.78", "encryption", "unlockOwnerFiles", json!([target, FILE_PASSWORD, true])).await;
    assert_eq!(r["ok"], json!({ "ok": true }));
    let (r, _) = w.rpc(&session, "203.0.113.77", "encryption", "unlockMyFiles", json!([FILE_PASSWORD, true])).await;
    assert_eq!(r["ok"], json!({ "ok": true }));
    // The raw link's form says so too.
    let form = call(
        &w.app,
        Request::post("/api/encryption/unlock")
            .header("host", HOST)
            .header("x-real-ip", "203.0.113.77")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!("next=%2F{owner}%2Fpic%2Fraw&password=x")))
            .unwrap(),
    )
    .await;
    assert_eq!(form.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(form.text().contains("Too many attempts"));
    // The app's bucket: 429 with Retry-After.
    let token = create_api_token(&w.pool, &owner, "app", TokenScope::Read).await.unwrap();
    let mut last = None;
    for _ in 0..=modlight_server::vault::unlock::PER_OWNER {
        last = Some(
            call(
                &w.app,
                Request::post("/api/encryption/unlock")
                    .header("host", HOST)
                    .header("x-api-key", &token)
                    .header("content-type", "application/json")
                    .body(Body::from(json!({ "password": "guess guess" }).to_string()))
                    .unwrap(),
            )
            .await,
        );
    }
    let last = last.unwrap();
    assert_eq!(last.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(last.json()["error"], "rate_limited");
    assert!(last.header("retry-after").is_some());
    // Throttled attempts are audited once, not once per attempt.
    let audits: i64 = sqlx::query_scalar(
        r#"select count(*) from "audit_log" where "user_id" = $1 and "action" = 'vault.unlock_failed' and "detail"->>'rateLimited' = 'true'"#,
    )
    .bind(&owner)
    .fetch_one(&w.pool)
    .await
    .unwrap();
    assert_eq!(audits, 1);
    w.drop().await;
}

/// The writer's check: an encrypted object needs a vault that still exists.
#[tokio::test]
async fn an_encrypted_write_into_a_vault_that_is_gone_is_refused() {
    let w = world!();
    let owner = w.user("user").await;
    let mut tx = w.pool.begin().await.unwrap();
    assert!(modlight_server::vault::write::confirm_vault_for_write(&mut tx, &owner, true).await.is_err());
    tx.rollback().await.unwrap();
    let mut tx = w.pool.begin().await.unwrap();
    modlight_server::vault::write::confirm_vault_for_write(&mut tx, &owner, false).await.unwrap();
    tx.commit().await.unwrap();
    w.drop().await;
}
