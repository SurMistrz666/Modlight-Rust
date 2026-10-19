//! The media serving routes and pages end to end against a real PostgreSQL and
//! a real storage directory: the upstream route tests (render-route,
//! render-headers, timeline-route, motion-route, preview-bots' route half,
//! tagless-redirect) driven through `App`, plus the security fixes of this port
//! (1: view limits, 5: delete token, 6: oEmbed) and `verifyMediaPassword`.
//! Upstream ran the route tests against mocked modules; here the rows, the
//! bytes, the tickets and the cookies are real.
//!
//! Runs only when `MODLIGHT_TEST_DATABASE_URL` names a server where the user
//! may create databases (see tests/db_compat.rs).

mod common;

use std::str::FromStr;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine;
use common::{Answer, call};
use hmac::Mac;
use modlight_server::config::Config;
use modlight_server::http::App;
use modlight_server::media::render_ticket::mint_render_ticket;
use modlight_server::paths::Paths;
use modlight_server::state::AppState;
use serde_json::{Value, json};
use sqlx::postgres::PgConnectOptions;
use sqlx::{ConnectOptions, PgPool};
use uuid::Uuid;

const SECRET: &str = "media-serving-test-secret-0123456789abcdef";

struct TempDb {
    admin: PgConnectOptions,
    name: String,
    pool: PgPool,
}

impl TempDb {
    async fn create() -> Option<TempDb> {
        let url = std::env::var("MODLIGHT_TEST_DATABASE_URL").ok()?;
        let admin = PgConnectOptions::from_str(&url).expect("MODLIGHT_TEST_DATABASE_URL");
        let name = format!("modlight_media_{}_{}", std::process::id(), rand::random::<u32>());
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

/// The app, a storage directory of its own, and a few helpers around both.
struct World {
    db: TempDb,
    app: App,
    storage: std::path::PathBuf,
}

impl World {
    async fn new() -> Option<World> {
        let db = TempDb::create().await?;
        let storage = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("media-serving-{}", db.name));
        std::fs::create_dir_all(&storage).unwrap();
        let vars: std::collections::HashMap<String, String> = [
            ("APP_URL", "https://host.example.com"),
            ("NODE_ENV", "production"),
            ("SKIP_ENV_VALIDATION", "1"),
            ("AUTH_SECRET", SECRET),
            ("STORAGE_LOCAL_PATH", storage.to_str().unwrap()),
            ("TRUST_PROXY", "true"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let config = Config::from_env_map(&vars).expect("config");
        let mut state = AppState::new(config, Paths::crate_dir(), db.pool.clone()).expect("state");
        // Storage is built from the process environment, like upstream's;
        // point it at this test's own directory.
        state.storage = modlight_server::storage::Storage::with_driver(std::sync::Arc::new(
            modlight_server::storage::LocalDriver::new(storage.to_str().unwrap()),
        ));
        let app = App::new(state);
        Some(World { db, app, storage })
    }

    fn pool(&self) -> &PgPool {
        &self.db.pool
    }

    async fn user(&self, id: &str, role: &str) {
        sqlx::query(
            r#"insert into "user" ("id", "name", "email", "email_verified", "created_at", "updated_at", "user_code", "role", "theme")
               values ($1, $1, $1 || '@example.test', true, now(), now(), $1, $2, 'modern')"#,
        )
        .bind(id)
        .bind(role)
        .execute(self.pool())
        .await
        .unwrap();
    }

    /// An upload with its bytes on disk; returns the id.
    async fn file(&self, owner: &str, code: &str, filename: &str, mime: &str, bytes: &[u8]) -> Uuid {
        let ext = filename.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        let key = format!("{owner}/{code}.{ext}");
        let path = self.storage.join(&key);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        sqlx::query_scalar(
            r#"insert into "uploads" ("owner_id", "code", "filename", "ext", "mime_type", "size", "storage_key", "delete_token", "checksum")
               values ($1, $2, $3, $4, $5, $6, $7, 'dt-' || $2, md5($2) || md5($2)) returning "id""#,
        )
        .bind(owner)
        .bind(code)
        .bind(filename)
        .bind(ext)
        .bind(mime)
        .bind(bytes.len() as i64)
        .bind(&key)
        .fetch_one(self.pool())
        .await
        .unwrap()
    }

    async fn set(&self, id: Uuid, assignments: &str) {
        sqlx::query(&format!(r#"update "uploads" set {assignments} where "id" = $1"#))
            .bind(id)
            .execute(self.pool())
            .await
            .unwrap();
    }

    async fn views(&self, id: Uuid) -> (i32, i32) {
        sqlx::query_as(r#"select "views", "downloads" from "uploads" where "id" = $1"#)
            .bind(id)
            .fetch_one(self.pool())
            .await
            .unwrap()
    }

    async fn get(&self, uri: &str, headers: &[(&str, &str)]) -> Answer {
        let mut b = Request::get(uri).header("host", "host.example.com");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        call(&self.app, b.body(Body::empty()).unwrap()).await
    }

    async fn post(&self, uri: &str) -> Answer {
        call(&self.app, Request::post(uri).header("host", "host.example.com").body(Body::empty()).unwrap()).await
    }

    async fn session_cookie(&self, user_id: &str) -> String {
        let token = format!("tok{}", rand::random::<u64>());
        sqlx::query(
            r#"insert into "session" ("id", "token", "user_id", "expires_at", "created_at", "updated_at")
               values ($1, $2, $3, now() + interval '7 days', now(), now())"#,
        )
        .bind(format!("s{}", rand::random::<u64>()))
        .bind(&token)
        .bind(user_id)
        .execute(self.pool())
        .await
        .unwrap();
        let mut mac = <hmac::Hmac<sha2::Sha256> as Mac>::new_from_slice(SECRET.as_bytes()).unwrap();
        mac.update(token.as_bytes());
        let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        let value = modlight_server::http::cookies::encode_uri_component(&format!("{token}.{sig}"));
        format!("__Secure-better-auth.session_token={value}")
    }

    async fn drop(self) {
        let _ = std::fs::remove_dir_all(&self.storage);
        self.db.drop().await;
    }
}

/// `name=value` pairs of a response's Set-Cookie headers.
fn cookies_of(a: &Answer) -> Vec<String> {
    a.headers.get_all("set-cookie").iter().map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string()).collect()
}

const DOCUMENT: &[u8] = b"<html><body><script>fetch('/api/me')</script></body></html>";

// ── /render (tests/unit/render-route.test.ts, render-headers.test.ts) ──────

#[tokio::test]
async fn the_rendered_document_comes_back_in_a_sandbox() {
    let Some(w) = World::new().await else { return eprintln!("skipped") };
    w.user("ToqOse", "user").await;
    let id = w.file("ToqOse", "abc123", "page.html", "text/html", DOCUMENT).await;
    let ticket = mint_render_ticket(SECRET, &id.to_string(), None);
    let url = format!("/ToqOse/abc123/render?t={}", modlight_server::http::cookies::encode_uri_component(&ticket));
    let a = w.get(&url, &[("sec-fetch-dest", "iframe")]).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(a.header("content-type").as_deref(), Some("text/html; charset=utf-8"));
    assert!(a.text().contains("<script>"));
    let csp = a.header("content-security-policy").expect("a document body without a sandbox policy is stored XSS");
    assert!(csp.starts_with("sandbox allow-scripts") && !csp.contains("allow-same-origin"));
    assert_eq!(a.header("x-content-type-options").as_deref(), Some("nosniff"));
    assert_eq!(a.header("cache-control").as_deref(), Some("private, no-store"));
    // The config rule's Referrer-Policy wins over the handler's, as upstream.
    assert_eq!(a.header("referrer-policy").as_deref(), Some("strict-origin-when-cross-origin"));
    assert!(a.header("vary").unwrap().contains("Cookie"));

    // Every shape it serves carries a sandbox.
    for (code, name, mime, ty) in [
        ("svg1", "f.svg", "image/svg+xml", "image/svg+xml; charset=utf-8"),
        ("xht1", "f.xhtml", "application/xhtml+xml", "text/html; charset=utf-8"),
    ] {
        let id = w.file("ToqOse", code, name, mime, DOCUMENT).await;
        let t = mint_render_ticket(SECRET, &id.to_string(), None);
        let a = w.get(&format!("/ToqOse/{code}/render?t={t}"), &[("sec-fetch-dest", "iframe")]).await;
        assert_eq!(a.status, StatusCode::OK, "{mime}");
        assert_eq!(a.header("content-type").as_deref(), Some(ty));
        assert!(a.header("content-security-policy").unwrap().contains("sandbox"));
    }
    // An HTML paste: its type is declared by its language.
    let paste = w.file("ToqOse", "pst1", "p.html", "text/plain; charset=utf-8", DOCUMENT).await;
    w.set(paste, r#""kind" = 'paste', "language" = 'html'"#).await;
    let t = mint_render_ticket(SECRET, &paste.to_string(), None);
    let a = w.get(&format!("/ToqOse/pst1/render?t={t}"), &[("sec-fetch-dest", "iframe")]).await;
    assert_eq!(a.status, StatusCode::OK);
    w.drop().await;
}

#[tokio::test]
async fn the_render_route_refuses_with_no_body_to_sandbox() {
    let Some(w) = World::new().await else { return eprintln!("skipped") };
    w.user("ToqOse", "user").await;
    w.user("SomeoneElse", "user").await;
    let id = w.file("ToqOse", "abc123", "page.html", "text/html", DOCUMENT).await;
    let t = mint_render_ticket(SECRET, &id.to_string(), None);
    let url = format!("/ToqOse/abc123/render?t={t}");

    // A top-level visit goes to the preview page; an unknown destination is refused.
    let a = w.get(&url, &[("sec-fetch-dest", "document")]).await;
    assert_eq!(a.status, StatusCode::FOUND);
    assert_eq!(a.header("location").as_deref(), Some("/ToqOse/abc123"));
    assert_eq!(w.get(&url, &[]).await.status, StatusCode::FORBIDDEN);
    // No ticket, or a forged one.
    assert_eq!(w.get("/ToqOse/abc123/render", &[("sec-fetch-dest", "iframe")]).await.status, StatusCode::FORBIDDEN);
    let forged = format!("/ToqOse/abc123/render?t={id}.9999999999.forged");
    assert_eq!(w.get(&forged, &[("sec-fetch-dest", "iframe")]).await.status, StatusCode::FORBIDDEN);
    // A ticket naming a file that is gone.
    let gone = mint_render_ticket(SECRET, "cccccccc-cccc-4ccc-8ccc-cccccccccccc", None);
    let a = w.get(&format!("/ToqOse/abc123/render?t={gone}"), &[("sec-fetch-dest", "iframe")]).await;
    assert_eq!(a.status, StatusCode::NOT_FOUND);
    // One file under another file's address.
    let other = w.file("ToqOse", "other99", "o.html", "text/html", DOCUMENT).await;
    let t2 = mint_render_ticket(SECRET, &other.to_string(), None);
    let a = w.get(&format!("/ToqOse/abc123/render?t={t2}"), &[("sec-fetch-dest", "iframe")]).await;
    assert_eq!(a.status, StatusCode::NOT_FOUND);
    // A path that only DECODES to /render.
    let a = w.get(&format!("/ToqOse/abc123/%72ender?t={t}"), &[("sec-fetch-dest", "iframe")]).await;
    assert_eq!(a.status, StatusCode::NOT_FOUND);
    assert!(!a.header("content-type").unwrap_or_default().contains("text/html"));
    // Under somebody else's tag.
    let a = w.get(&format!("/SomeoneElse/abc123/render?t={t}"), &[("sec-fetch-dest", "iframe")]).await;
    assert_eq!(a.status, StatusCode::NOT_FOUND);

    let frame = [("sec-fetch-dest", "iframe")];
    w.set(id, r#""published" = false"#).await;
    assert_eq!(w.get(&url, &frame).await.status, StatusCode::NOT_FOUND);
    w.set(id, r#""published" = true, "expires_at" = now() - interval '1 second'"#).await;
    assert_eq!(w.get(&url, &frame).await.status, StatusCode::NOT_FOUND);
    w.set(id, r#""expires_at" = null, "max_views" = 1, "views" = 2"#).await;
    assert_eq!(w.get(&url, &frame).await.status, StatusCode::NOT_FOUND);
    // Strictly greater: the view the page paid for still renders.
    w.set(id, r#""views" = 1"#).await;
    assert_eq!(w.get(&url, &frame).await.status, StatusCode::OK);
    w.set(id, r#""max_views" = null, "views" = 0, "password_hash" = 'scrypt:whatever'"#).await;
    assert_eq!(w.get(&url, &frame).await.status, StatusCode::UNAUTHORIZED);
    w.set(id, r#""password_hash" = null, "mime_type" = 'text/plain'"#).await;
    assert_eq!(w.get(&url, &frame).await.status, StatusCode::NOT_FOUND);
    w.set(id, r#""mime_type" = 'text/html', "size" = 6291456"#).await;
    assert_eq!(w.get(&url, &frame).await.status, StatusCode::PAYLOAD_TOO_LARGE);
    w.drop().await;
}

// ── timeline and motion (timeline-route.test.ts, motion-route.test.ts) ─────

#[tokio::test]
async fn timelines_and_motion_tiles_never_hand_back_the_file_or_spend_a_view() {
    let Some(w) = World::new().await else { return eprintln!("skipped") };
    w.user("tl", "user").await;
    let png = std::fs::read(Paths::crate_dir().public.join("apple-icon.png")).unwrap();
    let id = w.file("tl", "pic", "pic.png", "image/png", &png).await;
    w.set(id, r#""max_views" = 5"#).await;
    for action in ["timeline.vtt", "timeline.jpg"] {
        let a = w.get(&format!("/tl/pic/{action}"), &[]).await;
        assert_eq!(a.status.as_u16(), 415, "{action}");
        assert_eq!(a.header("x-timeline-refused").as_deref(), Some("not-video"));
        assert_eq!(a.header("cache-control").as_deref(), Some("no-store"));
        assert!(a.body.len() < 100);
    }
    let a = w.get("/tl/pic/raw?motion=1", &[]).await;
    assert_eq!(a.status.as_u16(), 415);
    assert_eq!(a.header("x-motion-refused").as_deref(), Some("not-animated"));
    assert_eq!(a.header("cache-control").as_deref(), Some("no-store"));
    // A value that merely mentions motion reaches the ordinary path.
    let a = w.get("/tl/pic/raw?motion=yes", &[]).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(a.body.len(), png.len());
    // Neither the previews nor the refusals spent anything; the plain read did.
    assert_eq!(w.views(id).await, (1, 0));
    // A gated file is refused before any work.
    w.set(id, r#""published" = false"#).await;
    assert_eq!(w.get("/tl/pic/timeline.vtt", &[]).await.status, StatusCode::NOT_FOUND);
    assert_eq!(w.get("/tl/pic/raw?motion=1", &[]).await.status, StatusCode::NOT_FOUND);
    w.set(id, r#""published" = true, "password_hash" = 'x'"#).await;
    assert_eq!(w.get("/tl/pic/timeline.jpg", &[]).await.status, StatusCode::UNAUTHORIZED);
    w.drop().await;
}

// ── raw, thumbnails, cache headers ──────────────────────────────────────────

#[tokio::test]
async fn raw_ranges_thumbnails_and_cache_policy() {
    let Some(w) = World::new().await else { return eprintln!("skipped") };
    w.user("rw", "user").await;
    let png = std::fs::read(Paths::crate_dir().public.join("apple-icon.png")).unwrap();
    let id = w.file("rw", "img", "Zdjęcie 1.png", "image/png", &png).await;
    let a = w.get("/rw/img/raw.png", &[]).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(a.header("content-disposition").as_deref(), Some("inline; filename*=UTF-8''Zdj%C4%99cie%201.png"));
    assert_eq!(a.header("cache-control").as_deref(), Some("public, max-age=31536000, immutable"));
    assert!(a.header("etag").is_some());
    assert_eq!(a.header("content-security-policy").unwrap().rsplit("; ").next(), Some("sandbox"));
    let a = w.get("/rw/img/raw", &[("range", "bytes=2-9")]).await;
    assert_eq!(a.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(a.header("content-range"), Some(format!("bytes 2-9/{}", png.len())));
    assert_eq!(&a.body[..], &png[2..10]);
    let a = w.get("/rw/img/raw?w=64", &[]).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(a.header("content-type").as_deref(), Some("image/webp"));
    assert_eq!(&a.body[8..12], b"WEBP");
    let a = w.get("/rw/img/download", &[]).await;
    assert!(a.header("content-disposition").unwrap().starts_with("attachment;"));
    // Statistics are written in the background; give them a moment.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(w.views(id).await, (2, 1));
    // Uploaded markup stays inert text on /raw.
    w.file("rw", "mk", "x.html", "text/html", DOCUMENT).await;
    let a = w.get("/rw/mk/raw", &[]).await;
    assert_eq!(a.header("content-type").as_deref(), Some("text/plain; charset=utf-8"));
    assert_eq!(a.header("x-content-type-options").as_deref(), Some("nosniff"));
    w.drop().await;
}

// ── Security fix 1 ──────────────────────────────────────────────────────────

#[tokio::test]
async fn fix1_every_bypass_of_the_audit_is_closed() {
    let Some(w) = World::new().await else { return eprintln!("skipped") };
    w.user("fx", "user").await;
    let bytes = b"one-time secret, many bytes long".to_vec();
    type Case<'a> = (&'a str, &'a [(&'a str, &'a str)], &'a str);
    let cases: [Case<'_>; 8] = [
        ("raw", &[("range", "bytes=1-")], ""),
        ("raw", &[("sec-fetch-dest", "image")], ""),
        ("raw", &[("sec-fetch-dest", "video")], ""),
        ("raw.txt", &[("range", "bytes=1-")], ""),
        ("download", &[("range", "bytes=1-")], ""),
        ("download", &[("sec-fetch-dest", "image")], "?w=1"),
        ("raw", &[], "?w=64"),
        ("raw", &[("user-agent", "Mozilla/5.0 (compatible; Discordbot/2.0)")], ""),
    ];
    for (i, (action, headers, query)) in cases.iter().enumerate() {
        let code = format!("c{i}");
        let id = w.file("fx", &code, "s.txt", "text/plain", &bytes).await;
        w.set(id, r#""max_views" = 1"#).await;
        let url = format!("/fx/{code}/{action}{query}");
        let first = w.get(&url, headers).await;
        assert!(first.status.is_success(), "{url}: the first read is the one view: {} {}", first.status, first.text());
        assert!(cookies_of(&first).iter().any(|c| c.starts_with(&format!("ml_view_{id}="))));
        for _ in 0..2 {
            assert_eq!(w.get(&url, headers).await.status, StatusCode::NOT_FOUND, "{url} {headers:?}");
        }
        assert_eq!(w.views(id).await.0, 1, "{url}");
    }
    w.drop().await;
}

#[tokio::test]
async fn fix1_the_legitimate_flows_ride_on_the_view_grant() {
    let Some(w) = World::new().await else { return eprintln!("skipped") };
    w.user("gr", "user").await;
    let png = std::fs::read(Paths::crate_dir().public.join("apple-icon.png")).unwrap();
    let id = w.file("gr", "pic", "p.png", "image/png", &png).await;
    w.set(id, r#""max_views" = 1"#).await;
    // The page counts the (last) view and hands out the grant...
    let page = w.get("/gr/pic", &[("user-agent", "Mozilla/5.0 Safari")]).await;
    assert_eq!(page.status, StatusCode::OK);
    let grant = cookies_of(&page).into_iter().find(|c| c.starts_with("ml_view_")).expect("grant");
    assert_eq!(w.views(id).await.0, 1);
    // ...which the page's own <img> presents, on the spent budget, as often as it reloads.
    for _ in 0..2 {
        let img = w.get("/gr/pic/raw.png", &[("sec-fetch-dest", "image"), ("cookie", &grant)]).await;
        assert_eq!(img.status, StatusCode::OK);
        assert_eq!(img.body.len(), png.len());
    }
    // A counted shape is still counted (and refused on a spent budget), grant or not.
    assert_eq!(w.get("/gr/pic/raw.png", &[("cookie", &grant)]).await.status, StatusCode::NOT_FOUND);
    assert_eq!(w.get("/gr/pic", &[("cookie", &grant)]).await.status, StatusCode::NOT_FOUND);
    // Without the grant, nothing.
    assert_eq!(w.get("/gr/pic/raw.png", &[("sec-fetch-dest", "image")]).await.status, StatusCode::NOT_FOUND);
    // A grant opens only its own file, and a tampered one opens nothing.
    let other = w.file("gr", "oth", "o.png", "image/png", &png).await;
    w.set(other, r#""max_views" = 1, "views" = 1"#).await;
    let forged = grant.replace(&id.to_string(), &other.to_string());
    assert_eq!(w.get("/gr/oth/raw", &[("range", "bytes=1-"), ("cookie", &forged)]).await.status, StatusCode::NOT_FOUND);
    assert_eq!(w.views(id).await.0, 1);
    w.drop().await;
}

#[tokio::test]
async fn fix1_preview_bots_get_no_content_of_a_capped_file() {
    let Some(w) = World::new().await else { return eprintln!("skipped") };
    w.user("bt", "user").await;
    let id = w.file("bt", "note", "n.txt", "text/plain", b"the secret is 42").await;
    w.set(id, r#""max_views" = 1"#).await;
    let a = w.get("/_page?path=%2Fbt%2Fnote", &[("user-agent", "Mozilla/5.0 (compatible; Twitterbot/1.0)")]).await;
    let viewer = &a.json()["props"]["viewer"];
    assert_eq!(viewer["textContent"], Value::Null);
    assert_eq!(viewer["renderPath"], Value::Null);
    // The card carries no excerpt of it either.
    let doc = w.get("/bt/note", &[("user-agent", "TelegramBot (like TwitterBot)")]).await.text();
    assert!(!doc.contains("the secret"), "{doc}");
    assert_eq!(w.views(id).await.0, 0);
    // A person pays the view and reads it.
    let a = w.get("/_page?path=%2Fbt%2Fnote", &[("user-agent", "Mozilla/5.0 Safari")]).await;
    assert_eq!(a.json()["props"]["viewer"]["textContent"], "the secret is 42");
    assert_eq!(w.views(id).await.0, 1);
    w.drop().await;
}

// ── Security fix 5: the delete page ─────────────────────────────────────────

#[tokio::test]
async fn fix5_delete_by_token() {
    let Some(w) = World::new().await else { return eprintln!("skipped") };
    w.user("dl", "user").await;
    let id = w.file("dl", "gone", "<img src=x onerror=alert(1)>.png", "image/png", b"x").await;
    for wrong in ["nope", "dt-gon", "dt-gonee", "DT-gone"] {
        assert_eq!(w.get(&format!("/dl/gone/delete/{wrong}"), &[]).await.status, StatusCode::NOT_FOUND, "{wrong}");
        assert_eq!(w.post(&format!("/dl/gone/delete/{wrong}")).await.status, StatusCode::NOT_FOUND);
    }
    let page = w.get("/dl/gone/delete/dt-gone", &[]).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.text().contains("&lt;img src=x onerror=alert(1)&gt;.png"));
    assert!(!page.text().contains("<img"));
    assert_eq!(w.post("/dl/gone/delete/dt-gone").await.status, StatusCode::OK);
    let left: i64 = sqlx::query_scalar(r#"select count(*) from "uploads" where "id" = $1"#)
        .bind(id)
        .fetch_one(w.pool())
        .await
        .unwrap();
    assert_eq!(left, 0);
    w.drop().await;
}

// ── Security fix 6: oEmbed ──────────────────────────────────────────────────

#[tokio::test]
async fn fix6_oembed_describes_a_capped_file_without_its_bytes() {
    let Some(w) = World::new().await else { return eprintln!("skipped") };
    w.user("oe", "user").await;
    let id = w.file("oe", "pic", "p.png", "image/png", b"x").await;
    w.set(id, r#""width" = 640, "height" = 480"#).await;
    let url = "/oembed?url=https%3A%2F%2Fhost.example.com%2Foe%2Fpic.png";
    let open = w.get(url, &[]).await.json();
    assert_eq!(open["type"], "photo");
    assert_eq!(open["url"], "https://host.example.com/oe/pic/raw.png");
    w.set(id, r#""max_views" = 3"#).await;
    let a = w.get(url, &[]).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(
        a.json(),
        json!({"version": "1.0", "title": "p.png", "provider_name": "example", "provider_url": "https://host.example.com",
               "cache_age": 86400, "type": "link"})
    );
    w.drop().await;
}

// ── verifyMediaPassword and the gate ────────────────────────────────────────

#[tokio::test]
async fn unlocking_a_file() {
    let Some(w) = World::new().await else { return eprintln!("skipped") };
    w.user("pw", "user").await;
    let id = w.file("pw", "lock", "l.png", "image/png", b"x").await;
    let hash = modlight_server::password::hash_password("sesame").await.unwrap();
    sqlx::query(r#"update "uploads" set "password_hash" = $2 where "id" = $1"#)
        .bind(id)
        .bind(&hash)
        .execute(w.pool())
        .await
        .unwrap();
    let page = w.get("/_page?path=%2Fpw%2Flock", &[]).await.json();
    assert_eq!(page["props"]["view"], "gate");
    assert_eq!(page["props"]["mediaId"], id.to_string());
    assert_eq!(page["props"]["onDefaultPassword"], false);
    assert_eq!(w.get("/pw/lock/raw", &[]).await.status, StatusCode::UNAUTHORIZED);

    let rpc = |args: Value, ip: &'static str| {
        Request::post("/_rpc/unlock/verifyMediaPassword")
            .header("host", "host.example.com")
            .header("origin", "https://host.example.com")
            .header("x-modlight-rpc", "1")
            .header("x-real-ip", ip)
            .header("content-type", "application/json")
            .body(Body::from(args.to_string()))
            .unwrap()
    };
    let bad = call(&w.app, rpc(json!([id.to_string(), "wrong"]), "192.0.2.1")).await;
    assert_eq!(bad.json(), json!({"ok": {"ok": false}}));
    let good = call(&w.app, rpc(json!([id.to_string(), "sesame"]), "192.0.2.1")).await;
    // A cookie write makes the envelope ask for a refetch (CONTRACT 7.1).
    assert_eq!(good.json()["ok"], json!({"ok": true, "remembered": {"$undefined": 1}}));
    let cookie = cookies_of(&good).into_iter().find(|c| c.starts_with("ml_unlock_")).unwrap();
    assert_eq!(w.get("/pw/lock/raw", &[("cookie", &cookie)]).await.status, StatusCode::OK);
    // Throttled per address and file, five attempts in the window.
    for _ in 0..5 {
        call(&w.app, rpc(json!([id.to_string(), "no"]), "192.0.2.9")).await;
    }
    let limited = call(&w.app, rpc(json!([id.to_string(), "sesame"]), "192.0.2.9")).await;
    assert_eq!(limited.json(), json!({"ok": {"ok": false, "error": "rate-limited"}}));
    // Ids are guarded before anything is counted or queried.
    let junk = call(&w.app, rpc(json!(["not-a-uuid", "x"]), "192.0.2.9")).await;
    assert_eq!(junk.json(), json!({"ok": {"ok": false}}));
    w.drop().await;
}

// ── The page: views, bots, tagless links ────────────────────────────────────

#[tokio::test]
async fn the_media_page() {
    let Some(w) = World::new().await else { return eprintln!("skipped") };
    w.user("pg", "user").await;
    let id = w.file("pg", "clip", "c.mp4", "video/mp4", b"not really a video").await;
    // A media bot is sent to the bytes; a markup bot gets the card; neither counts.
    let a = w.get("/pg/clip.mp4", &[("user-agent", "Mozilla/5.0 (compatible; Discordbot/2.0)")]).await;
    assert_eq!(a.status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(a.header("location").as_deref(), Some("https://host.example.com/pg/clip/raw.mp4"));
    let a = w.get("/pg/clip", &[("user-agent", "Mozilla/5.0 (compatible; Twitterbot/1.0)")]).await;
    assert_eq!(a.status, StatusCode::OK);
    assert!(a.text().contains("<meta property=\"og:video\" content=\"https://host.example.com/pg/clip/raw.mp4\"/>"));
    assert_eq!(w.views(id).await.0, 0);
    // A person counts; the owner does not.
    let page = w.get("/_page?path=%2Fpg%2Fclip", &[]).await.json();
    let viewer = &page["props"]["viewer"];
    assert_eq!(viewer["posterUrl"], "/pg/clip/raw?w=800");
    assert_eq!(viewer["timelineTrack"], "/pg/clip/timeline.vtt");
    assert_eq!(viewer["canEncode"], false);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(w.views(id).await.0, 1);
    let owner = w.session_cookie("pg").await;
    let page = w.get("/_page?path=%2Fpg%2Fclip", &[("cookie", &owner)]).await.json();
    assert_eq!(page["props"]["viewer"]["canEncode"], true);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(w.views(id).await.0, 1);

    // Tagless: an owner who opted out of the tag segment.
    sqlx::query(r#"update "user" set "skip_tag_in_url" = true where "id" = 'pg'"#).execute(w.pool()).await.unwrap();
    let page = w.get("/_page?path=%2Fclip.mp4", &[]).await.json();
    assert_eq!(page["route"], "/[userCode]");
    assert_eq!(page["props"]["viewer"]["publicUrl"], "https://host.example.com/pg/clip.mp4");
    // ...and its tagless actions are redirected with the query rebuilt.
    let a = w.get("/clip/raw.mp4?w=64&b=x%20y&10=a&2=b", &[]).await;
    assert_eq!(a.status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(a.header("location").as_deref(), Some("/pg/clip/raw.mp4?2=b&10=a&w=64&b=x+y"));
    w.drop().await;
}

// ── Encrypted uploads (encrypted-link, encrypted-upload: the server half) ──

#[tokio::test]
async fn an_encrypted_upload_is_handed_to_the_browser_untouched() {
    let Some(w) = World::new().await else { return eprintln!("skipped") };
    w.user("en", "user").await;
    let mut container = b"MLENC1\x00\x01".to_vec();
    container.extend(std::iter::repeat_n(7u8, 5000));
    let id = w.file("en", "box", "x.mlenc", "application/vnd.modlight.encrypted", &container).await;
    let page = w.get("/_page?path=%2Fen%2Fbox.mlenc", &[]).await.json();
    let props = &page["props"];
    assert_eq!(props["view"], "encrypted");
    // Relative, and nothing in it that could carry a key: the key lives in the
    // fragment, which no browser sends.
    assert_eq!(props["rawPath"], "/en/box/raw.mlenc");
    assert_eq!(props["cipherSize"], 5008);
    assert!(props.get("viewer").is_none());
    // The page itself counts nothing; the viewer's first ranged read does.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(w.views(id).await.0, 0);
    let head = w.get("/en/box/raw.mlenc", &[("range", "bytes=0-4095")]).await;
    assert_eq!(head.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(head.header("content-type").as_deref(), Some("application/octet-stream"));
    assert!(head.header("content-disposition").unwrap().starts_with("attachment;"));
    // No card picture, no snippet: a container describes nothing about itself.
    let doc = w.get("/en/box.mlenc", &[("user-agent", "Mozilla/5.0 (compatible; Twitterbot/1.0)")]).await.text();
    assert!(doc.contains("/embed/v1/binary-FILE.png"));
    w.drop().await;
}
