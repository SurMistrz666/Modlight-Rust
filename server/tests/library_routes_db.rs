//! The library route handlers end to end against a real PostgreSQL: the
//! upstream route tests (folders-api, uploads-api-check, duplicates-api,
//! api-route-auth, admin-disk-api) driven through `App` with a throwaway,
//! migrated database each. Upstream ran them against a parsing fake database
//! with the credential stubbed; here the credential is a real API token (or a
//! real session row) and the statements run against the real schema.
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
use modlight_server::api_token::{TokenScope, create_api_token};
use modlight_server::config::Config;
use modlight_server::http::App;
use modlight_server::paths::Paths;
use modlight_server::state::AppState;
use serde_json::{Value, json};
use sqlx::postgres::PgConnectOptions;
use sqlx::{ConnectOptions, PgPool};
use uuid::Uuid;

const SECRET: &str = "library-routes-test-secret-0123456789abcdef";

struct TempDb {
    admin: PgConnectOptions,
    name: String,
    pool: PgPool,
}

impl TempDb {
    async fn create() -> Option<TempDb> {
        let url = std::env::var("MODLIGHT_TEST_DATABASE_URL").ok()?;
        let admin = PgConnectOptions::from_str(&url).expect("MODLIGHT_TEST_DATABASE_URL");
        let name = format!("modlight_routes_{}_{}", std::process::id(), rand::random::<u32>());
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

macro_rules! db_or_skip {
    () => {
        match TempDb::create().await {
            Some(db) => db,
            None => {
                eprintln!("MODLIGHT_TEST_DATABASE_URL not set: skipped");
                return;
            }
        }
    };
}

/// The application on the temp database, with `extra` environment on top.
fn app_on(db: &TempDb, extra: &[(&str, &str)]) -> App {
    let storage = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("library-routes-storage");
    let mut vars: std::collections::HashMap<String, String> = [
        ("APP_URL", "https://host.example.com"),
        ("NODE_ENV", "production"),
        ("SKIP_ENV_VALIDATION", "1"),
        ("AUTH_SECRET", SECRET),
        ("STORAGE_LOCAL_PATH", storage.to_str().unwrap()),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    for (k, v) in extra {
        vars.insert(k.to_string(), v.to_string());
    }
    let config = Config::from_env_map(&vars).expect("config");
    App::new(AppState::new(config, Paths::crate_dir(), db.pool.clone()).expect("state"))
}

/// A user row the way Better Auth writes one; `id` doubles as the tag.
async fn user(db: &PgPool, id: &str, tag: Option<&str>, role: &str) {
    sqlx::query(
        r#"insert into "user" ("id", "name", "email", "email_verified", "created_at", "updated_at", "user_code", "role", "theme")
           values ($1, $1, $1 || '@example.test', true, now(), now(), $2, $3, 'modern')"#,
    )
    .bind(id)
    .bind(tag)
    .bind(role)
    .execute(db)
    .await
    .unwrap();
}

async fn token(db: &PgPool, user_id: &str, scope: TokenScope) -> String {
    create_api_token(db, user_id, "test", scope).await.unwrap()
}

async fn ban(db: &PgPool, user_id: &str) {
    sqlx::query(r#"update "user" set "banned" = true where "id" = $1"#).bind(user_id).execute(db).await.unwrap();
}

/// A session row and the cookie a browser would send for it.
async fn session_cookie(db: &PgPool, user_id: &str) -> String {
    let token = format!("tok{}", rand::random::<u64>());
    sqlx::query(
        r#"insert into "session" ("id", "token", "user_id", "expires_at", "created_at", "updated_at")
           values ($1, $2, $3, now() + interval '7 days', now(), now())"#,
    )
    .bind(format!("s{}", rand::random::<u64>()))
    .bind(&token)
    .bind(user_id)
    .execute(db)
    .await
    .unwrap();
    let mut mac = <hmac::Hmac<sha2::Sha256> as Mac>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(token.as_bytes());
    let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    let value = modlight_server::http::cookies::encode_uri_component(&format!("{token}.{sig}"));
    // APP_URL is https here, so Better Auth uses the `__Secure-` name.
    format!("__Secure-better-auth.session_token={value}")
}

async fn folder(db: &PgPool, owner: &str, name: &str, parent: Option<Uuid>) -> Uuid {
    sqlx::query_scalar(
        r#"insert into "folders" ("owner_id", "parent_id", "name", "slug") values ($1, $2, $3, lower($3)) returning "id""#,
    )
    .bind(owner)
    .bind(parent)
    .bind(name)
    .fetch_one(db)
    .await
    .unwrap()
}

#[derive(Default)]
struct Upload<'a> {
    checksum: Option<&'a str>,
    folder: Option<Uuid>,
    size: i64,
    deleted: bool,
    created: &'a str,
}

async fn upload(db: &PgPool, owner: &str, code: &str, u: Upload<'_>) -> Uuid {
    sqlx::query_scalar(
        r#"insert into "uploads" ("owner_id", "code", "filename", "ext", "mime_type", "size", "storage_key", "checksum",
                                  "delete_token", "folder_id", "deleted_at", "created_at")
           values ($1, $2, 'shot.png', 'png', 'image/png', $3, $1 || '/' || $2, $4, 'tok-' || $2, $5,
                   case when $6 then now() else null end, coalesce($7::timestamptz, now())) returning "id""#,
    )
    .bind(owner)
    .bind(code)
    .bind(if u.size == 0 { 1234 } else { u.size })
    .bind(u.checksum)
    .bind(u.folder)
    .bind(u.deleted)
    .bind(if u.created.is_empty() { None } else { Some(u.created) })
    .fetch_one(db)
    .await
    .unwrap()
}

/// A request with optional token, cookie and JSON body.
fn req(method: &str, uri: &str, token: Option<&str>, body: Option<Value>) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(uri).header("host", "host.example.com");
    if let Some(t) = token {
        b = b.header("x-api-key", t);
    }
    match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    }
}

fn with(mut r: Request<Body>, name: &'static str, value: &str) -> Request<Body> {
    r.headers_mut().insert(name, value.parse().unwrap());
    r
}

fn sha(n: u32) -> String {
    format!("{n:064x}")
}

// ── /api/folders (tests/unit/folders-api.test.ts) ───────────────────────────

#[tokio::test]
async fn folder_writes_refuse_before_the_body_and_mark_refusals_private() {
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "fw1", Some("fw1"), "user").await;
    let good = token(db, "fw1", TokenScope::Full).await;
    let app = app_on(&t, &[]);
    for method in ["POST", "PATCH", "DELETE"] {
        let body = json!({ "id": Uuid::nil(), "name": "x" });
        let none = call(&app, req(method, "/api/folders", None, Some(body.clone()))).await;
        assert_eq!(none.status, StatusCode::UNAUTHORIZED, "{method}");
        assert_eq!(none.json(), json!({ "message": "Token not specified or not found." }));
        assert_eq!(none.header("cache-control").as_deref(), Some("private, no-store"));
        let revoked = call(&app, req(method, "/api/folders", Some("revoked"), Some(body.clone()))).await;
        assert_eq!(revoked.status, StatusCode::UNAUTHORIZED);
        // Not JSON and no credential: 401, not 415 (the body is never read).
        let plain = Request::builder()
            .method(method)
            .uri("/api/folders")
            .header("content-type", "text/plain")
            .body(Body::from("not json"))
            .unwrap();
        assert_eq!(call(&app, plain).await.status, StatusCode::UNAUTHORIZED);
        let typed =
            with(req(method, "/api/folders", Some(&good), Some(body)), "content-type", "text/plain;charset=UTF-8");
        assert_eq!(call(&app, typed).await.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
    // A banned account keeps its tokens and loses the use of them.
    ban(db, "fw1").await;
    let banned = call(&app, req("POST", "/api/folders", Some(&good), Some(json!({ "name": "x" })))).await;
    assert_eq!(banned.status, StatusCode::UNAUTHORIZED);
    t.drop().await;
}

#[tokio::test]
async fn folders_post_creates_in_the_listing_shape_and_guards_the_tree() {
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "fp1", Some("fp1"), "user").await;
    user(db, "fp2", Some("fp2"), "user").await;
    let me = token(db, "fp1", TokenScope::Write).await;
    let mine = folder(db, "fp1", "Mine", None).await;
    let theirs = folder(db, "fp2", "Theirs", None).await;
    let app = app_on(&t, &[]);
    let post = |body: Value| call(&app, req("POST", "/api/folders", Some(&me), Some(body)));

    let created = post(json!({ "name": "Holiday Photos" })).await;
    assert_eq!(created.status, StatusCode::CREATED);
    let f = &created.json()["folder"];
    let keys: Vec<&String> = f.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["id", "name", "slug", "parentId", "isPublic", "fileCount", "size", "sortOrder"]);
    assert_eq!(
        (&f["name"], &f["slug"], &f["parentId"], &f["isPublic"], &f["fileCount"], &f["size"]),
        (&json!("Holiday Photos"), &json!("holiday-photos"), &Value::Null, &json!(false), &json!(0), &json!(0))
    );

    let nested = post(json!({ "name": "Child", "parentId": mine })).await;
    assert_eq!(nested.status, StatusCode::CREATED);
    assert_eq!(nested.json()["folder"]["parentId"], json!(mine));

    // Another account's parent and a malformed one: the same 404, no row.
    for parent in [json!(theirs), json!("nope")] {
        let refused = post(json!({ "name": "Nope", "parentId": parent })).await;
        assert_eq!(refused.status, StatusCode::NOT_FOUND);
        assert_eq!(refused.json()["reason"], "not-found");
    }
    let count: i64 =
        sqlx::query_scalar(r#"select count(*) from "folders" where "name" = 'Nope'"#).fetch_one(db).await.unwrap();
    assert_eq!(count, 0);

    // The depth ceiling still bites once the session is out of the picture.
    let mut parent: Option<String> = None;
    for i in 0..modlight_server::folders::MAX_FOLDER_DEPTH {
        let res = post(json!({ "name": format!("Level {}", i + 1), "parentId": parent })).await;
        assert_eq!(res.status, StatusCode::CREATED);
        parent = res.json()["folder"]["id"].as_str().map(str::to_string);
    }
    let deep = post(json!({ "name": "Too deep", "parentId": parent })).await;
    assert_eq!(deep.status, StatusCode::CONFLICT);
    assert_eq!(deep.json()["reason"], "too-deep");

    assert_eq!(post(json!({})).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(post(json!({ "name": 7 })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(post(json!({ "name": "x", "parentId": 7 })).await.status, StatusCode::BAD_REQUEST);
    let top = post(json!({ "name": "Top", "parentId": "root" })).await;
    assert_eq!((top.status, top.json()["folder"]["parentId"].clone()), (StatusCode::CREATED, Value::Null));
    assert_eq!(post(json!(["name"])).await.status, StatusCode::BAD_REQUEST);
    let big = with(
        req("POST", "/api/folders", Some(&me), Some(json!({ "name": "x".repeat(64 * 1024) }))),
        "content-length",
        &(64 * 1024).to_string(),
    );
    assert_eq!(call(&app, big).await.status, StatusCode::PAYLOAD_TOO_LARGE);
    t.drop().await;
}

#[tokio::test]
async fn folders_patch_renames_moves_and_publishes() {
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "fq1", Some("fq1"), "user").await;
    user(db, "fq2", Some("fq2"), "user").await;
    let me = token(db, "fq1", TokenScope::Full).await;
    let mine = folder(db, "fq1", "Mine", None).await;
    let theirs = folder(db, "fq2", "Theirs", None).await;
    let child = folder(db, "fq1", "Child", Some(mine)).await;
    let app = app_on(&t, &[]);
    let patch = |body: Value| call(&app, req("PATCH", "/api/folders", Some(&me), Some(body)));
    let parent_of = |id: Uuid| async move {
        sqlx::query_scalar::<_, Option<Uuid>>(r#"select "parent_id" from "folders" where "id" = $1"#)
            .bind(id)
            .fetch_one(db)
            .await
            .unwrap()
    };

    let renamed = patch(json!({ "id": mine, "name": "Renamed" })).await;
    assert_eq!(renamed.status, StatusCode::OK);
    assert_eq!(
        (renamed.json()["folder"]["name"].clone(), renamed.json()["folder"]["slug"].clone()),
        (json!("Renamed"), json!("renamed"))
    );

    // An absent parentId leaves the parent alone; null moves to the root;
    // the sentinel too.
    patch(json!({ "id": child, "name": "Still nested" })).await;
    assert_eq!(parent_of(child).await, Some(mine));
    assert_eq!(patch(json!({ "id": child, "parentId": null })).await.json()["folder"]["parentId"], Value::Null);
    assert_eq!(parent_of(child).await, None);
    patch(json!({ "id": child, "parentId": mine })).await;
    assert_eq!(patch(json!({ "id": child, "parentId": "root" })).await.json()["folder"]["parentId"], Value::Null);
    patch(json!({ "id": child, "parentId": mine })).await;

    let on = patch(json!({ "id": mine, "isPublic": true })).await;
    assert_eq!(on.json()["folder"]["isPublic"], true);
    assert_eq!(patch(json!({ "id": mine, "isPublic": false })).await.json()["folder"]["isPublic"], false);

    // Rename and move together: the slug is re-derived from the NEW name in
    // the NEW parent ("mine" is taken at the root by... nothing yet, so make it so).
    folder(db, "fq1", "Mine", None).await;
    let both = patch(json!({ "id": child, "name": "Mine", "parentId": null })).await;
    assert_eq!(
        (both.json()["folder"]["name"].clone(), both.json()["folder"]["slug"].clone()),
        (json!("Mine"), json!("mine-2"))
    );
    patch(json!({ "id": child, "parentId": mine })).await;

    let cycle = patch(json!({ "id": mine, "parentId": child })).await;
    assert_eq!((cycle.status, cycle.json()["reason"].clone()), (StatusCode::CONFLICT, json!("cycle")));
    assert_eq!(parent_of(mine).await, None);
    assert_eq!(patch(json!({ "id": mine, "parentId": mine })).await.status, StatusCode::CONFLICT);

    let foreign = patch(json!({ "id": theirs, "name": "Hijacked" })).await;
    assert_eq!(foreign.status, StatusCode::NOT_FOUND);
    let name: String =
        sqlx::query_scalar(r#"select "name" from "folders" where "id" = $1"#).bind(theirs).fetch_one(db).await.unwrap();
    assert_eq!(name, "Theirs");

    let blank = patch(json!({ "id": mine, "name": "   " })).await;
    assert_eq!((blank.status, blank.json()["reason"].clone()), (StatusCode::BAD_REQUEST, json!("invalid-name")));
    assert_eq!(patch(json!({ "id": mine })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(patch(json!({ "id": mine, "isPublic": "yes" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(patch(json!({ "id": mine, "name": 7 })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(patch(json!({ "name": "x" })).await.status, StatusCode::BAD_REQUEST);

    // A move past the depth ceiling.
    let mut parent: Option<Uuid> = None;
    for i in 0..modlight_server::folders::MAX_FOLDER_DEPTH {
        parent = Some(folder(db, "fq1", &format!("L{i}"), parent).await);
    }
    let deep = patch(json!({ "id": mine, "parentId": parent })).await;
    assert_eq!((deep.status, deep.json()["reason"].clone()), (StatusCode::CONFLICT, json!("too-deep")));
    t.drop().await;
}

#[tokio::test]
async fn folders_delete_defaults_to_the_safe_mode() {
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "fd1", Some("fd1"), "user").await;
    user(db, "fd2", Some("fd2"), "user").await;
    let me = token(db, "fd1", TokenScope::Full).await;
    let app = app_on(&t, &[]);
    let delete = |body: Value| call(&app, req("DELETE", "/api/folders", Some(&me), Some(body)));

    let mine = folder(db, "fd1", "Mine", None).await;
    let child = folder(db, "fd1", "Child", Some(mine)).await;
    let file = upload(db, "fd1", "f1", Upload { folder: Some(mine), size: 100, ..Default::default() }).await;
    let safe = delete(json!({ "id": mine })).await;
    assert_eq!(safe.status, StatusCode::OK);
    assert_eq!(safe.json(), json!({ "message": "OK", "withContents": false, "folders": 1, "trashed": 0 }));
    let (folder_id, deleted): (Option<Uuid>, bool) =
        sqlx::query_as(r#"select "folder_id", "deleted_at" is not null from "uploads" where "id" = $1"#)
            .bind(file)
            .fetch_one(db)
            .await
            .unwrap();
    assert_eq!((folder_id, deleted), (None, false));
    let child_parent: Option<Uuid> = sqlx::query_scalar(r#"select "parent_id" from "folders" where "id" = $1"#)
        .bind(child)
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(child_parent, None);

    let mine = folder(db, "fd1", "Mine2", None).await;
    folder(db, "fd1", "Child2", Some(mine)).await;
    let file = upload(db, "fd1", "f2", Upload { folder: Some(mine), size: 100, ..Default::default() }).await;
    assert_eq!(delete(json!({ "id": mine, "withContents": "true" })).await.status, StatusCode::BAD_REQUEST);
    let full = delete(json!({ "id": mine, "withContents": true })).await;
    assert_eq!(full.json(), json!({ "message": "OK", "withContents": true, "folders": 2, "trashed": 1 }));
    let deleted: bool = sqlx::query_scalar(r#"select "deleted_at" is not null from "uploads" where "id" = $1"#)
        .bind(file)
        .fetch_one(db)
        .await
        .unwrap();
    assert!(deleted);

    let theirs = folder(db, "fd2", "Theirs", None).await;
    let foreign = delete(json!({ "id": theirs, "withContents": true })).await;
    assert_eq!((foreign.status, foreign.json()["reason"].clone()), (StatusCode::NOT_FOUND, json!("not-found")));
    assert_eq!(delete(json!({})).await.status, StatusCode::BAD_REQUEST);
    t.drop().await;
}

#[tokio::test]
async fn folders_get_lists_direct_counts_and_the_unfiled_shelf() {
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "fg1", Some("fg1"), "user").await;
    user(db, "fg2", Some("fg2"), "user").await;
    let reader = token(db, "fg1", TokenScope::Read).await;
    let uploader = token(db, "fg1", TokenScope::Upload).await;
    let mine = folder(db, "fg1", "Mine", None).await;
    let inner = folder(db, "fg1", "Inner", Some(mine)).await;
    folder(db, "fg2", "Theirs", None).await;
    upload(db, "fg1", "a", Upload { folder: Some(mine), size: 100, ..Default::default() }).await;
    upload(db, "fg1", "b", Upload { folder: Some(inner), size: 5, ..Default::default() }).await;
    upload(db, "fg1", "c", Upload { folder: Some(mine), size: 50, deleted: true, ..Default::default() }).await;
    upload(db, "fg1", "d", Upload { size: 7, ..Default::default() }).await;
    let app = app_on(&t, &[]);

    let scoped = call(&app, req("GET", "/api/folders", Some(&uploader), None)).await;
    assert_eq!(scoped.status, StatusCode::FORBIDDEN);
    assert_eq!(scoped.json()["required_scope"], "read");

    let res = call(&app, req("GET", "/api/folders", Some(&reader), None)).await;
    assert_eq!(res.status, StatusCode::OK);
    let body = res.json();
    assert_eq!(body["rootValue"], "root");
    assert_eq!(body["unfiled"], json!({ "fileCount": 1, "size": 7 }));
    let folders = body["folders"].as_array().unwrap();
    assert_eq!(folders.len(), 2);
    let by_name = |n: &str| folders.iter().find(|f| f["name"] == n).unwrap().clone();
    assert_eq!((by_name("Mine")["fileCount"].clone(), by_name("Mine")["size"].clone()), (json!(1), json!(100)));
    assert_eq!(by_name("Inner")["parentId"], json!(mine));

    // A session works too (the lightweight caller: no ban check, as upstream).
    let cookie = session_cookie(db, "fg1").await;
    let session = call(&app, with(req("GET", "/api/folders", None, None), "cookie", &cookie)).await;
    assert_eq!(session.status, StatusCode::OK);
    // A present-but-wrong token never falls through to that session.
    let wrong = call(&app, with(req("GET", "/api/folders", Some("nope"), None), "cookie", &cookie)).await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    t.drop().await;
}

// ── POST /api/uploads/check (tests/unit/uploads-api-check.test.ts) ──────────

#[tokio::test]
async fn uploads_check_matches_only_the_callers_live_files() {
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "ck1", Some("Tagged"), "user").await;
    user(db, "ck2", Some("Other"), "user").await;
    let me = token(db, "ck1", TokenScope::Full).await;
    let screens = folder(db, "ck1", "Screenshots", None).await;
    let (mine, theirs, trashed, absent) = (sha(1), sha(2), sha(3), sha(4));
    let created = "2026-01-01T00:00:00.000Z";
    upload(
        db,
        "ck1",
        "code0001",
        Upload { checksum: Some(&mine), folder: Some(screens), created, ..Default::default() },
    )
    .await;
    upload(db, "ck2", "code0002", Upload { checksum: Some(&theirs), created, ..Default::default() }).await;
    upload(db, "ck1", "code0003", Upload { checksum: Some(&trashed), deleted: true, created, ..Default::default() })
        .await;
    let app = app_on(&t, &[]);
    let post = |body: Value| call(&app, req("POST", "/api/uploads/check", Some(&me), Some(body)));

    let res = post(json!({ "checksums": [mine] })).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header("cache-control").as_deref(), Some("private, no-store"));
    let body = res.json();
    let result = &body["results"][0];
    assert_eq!(result["status"], "duplicate");
    let m = &result["matches"][0];
    assert_eq!(
        (&m["code"], &m["filename"], &m["size"], &m["folderId"], &m["folderName"], &m["createdAt"]),
        (&json!("code0001"), &json!("shot.png"), &json!(1234), &json!(screens), &json!("Screenshots"), &json!(created))
    );
    assert!(m["url"].as_str().unwrap().contains("/Tagged/code0001"));

    // Another account's checksum and a trashed file are not duplicates.
    for checksum in [&theirs, &trashed] {
        let r = post(json!({ "checksums": [checksum] })).await.json();
        assert_ne!(r["results"][0]["status"], "duplicate");
        assert_eq!(r["results"][0]["matches"], json!([]));
    }

    // Order kept, repeats collapsed.
    let order = post(json!({ "checksums": [absent, mine] })).await.json();
    assert_eq!(order["results"][0]["checksum"], json!(absent));
    assert_eq!(order["results"][1]["status"], "duplicate");
    let twice = post(json!({ "checksums": [mine, mine.to_uppercase()] })).await.json();
    assert_eq!(twice["requested"], 1);

    // `new` only when every live file carries a checksum.
    let clean = post(json!({ "checksums": [absent] })).await.json();
    assert_eq!((clean["unchecksummed"].clone(), clean["results"][0]["status"].clone()), (json!(0), json!("new")));
    upload(db, "ck2", "code0005", Upload::default()).await;
    upload(db, "ck1", "code0006", Upload { deleted: true, ..Default::default() }).await;
    assert_eq!(post(json!({ "checksums": [absent] })).await.json()["results"][0]["status"], "new");
    upload(db, "ck1", "code0004", Upload::default()).await;
    let blind = post(json!({ "checksums": [absent] })).await.json();
    assert_eq!((blind["unchecksummed"].clone(), blind["results"][0]["status"].clone()), (json!(1), json!("unknown")));
    assert_eq!(post(json!({ "checksums": [mine] })).await.json()["results"][0]["status"], "duplicate");
    t.drop().await;
}

#[tokio::test]
async fn uploads_check_refuses_bad_lists_and_credentials() {
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "cr1", Some("cr1"), "user").await;
    let me = token(db, "cr1", TokenScope::Read).await;
    let app = app_on(&t, &[]);
    let post = |body: Value| call(&app, req("POST", "/api/uploads/check", Some(&me), Some(body)));
    let mine = sha(1);

    let reason = |a: Answer| (a.status, a.json()["reason"].clone());
    assert_eq!(
        reason(post(json!({ "checksums": [mine, "not-a-checksum"] })).await),
        (StatusCode::BAD_REQUEST, json!("malformed"))
    );
    assert_eq!(post(json!({ "checksums": [&mine[..63]] })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(post(json!({ "checksums": ["z".repeat(64)] })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(post(json!({ "checksums": [format!("{mine} ")] })).await.status, StatusCode::OK);
    for empty in [json!({ "checksums": [] }), json!({ "checksums": "abc" }), json!({})] {
        assert_eq!(reason(post(empty).await).1, json!("empty"));
    }
    let too_many: Vec<String> = (0..501).map(|i| sha(i + 10)).collect();
    assert_eq!(reason(post(json!({ "checksums": too_many })).await), (StatusCode::BAD_REQUEST, json!("too-many")));
    // A full batch is ~33 KB: the body ceiling must not refuse it.
    let full: Vec<String> = (0..500).map(|i| sha(i + 10)).collect();
    let ok = post(json!({ "checksums": full })).await;
    assert_eq!((ok.status, ok.json()["requested"].clone()), (StatusCode::OK, json!(500)));

    let typed = with(
        req("POST", "/api/uploads/check", Some(&me), Some(json!({ "checksums": [mine] }))),
        "content-type",
        "text/plain",
    );
    assert_eq!(call(&app, typed).await.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(
        call(&app, req("POST", "/api/uploads/check", Some("nope"), Some(json!({ "checksums": [mine] })))).await.status,
        StatusCode::UNAUTHORIZED
    );
    ban(db, "cr1").await;
    assert_eq!(post(json!({ "checksums": [mine] })).await.status, StatusCode::UNAUTHORIZED);
    t.drop().await;
}

// ── /api/duplicates (tests/unit/duplicates-api.test.ts) ─────────────────────

#[tokio::test]
async fn duplicates_scan_carries_the_reminder_and_its_thresholds() {
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "du1", Some("du1"), "user").await;
    user(db, "du2", Some("du2"), "user").await;
    let me = token(db, "du1", TokenScope::Read).await;
    let app = app_on(&t, &[]);
    assert_eq!(call(&app, req("GET", "/api/duplicates", Some("nope"), None)).await.status, StatusCode::UNAUTHORIZED);

    let set = sha(0xA);
    for i in 0..11 {
        upload(db, "du1", &format!("d{i}"), Upload { checksum: Some(&set), size: 10, ..Default::default() }).await;
    }
    upload(db, "du1", "loose", Upload::default()).await;
    // Another account's copies of the same bytes are not in this scan.
    upload(db, "du2", "x1", Upload { checksum: Some(&set), size: 10, ..Default::default() }).await;
    upload(db, "du2", "x2", Upload { checksum: Some(&set), size: 10, ..Default::default() }).await;

    let body = call(&app, req("GET", "/api/duplicates", Some(&me), None)).await.json();
    let keys: Vec<&String> = body.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "groups",
            "totalGroups",
            "totalFiles",
            "reclaimable",
            "dismissedGroups",
            "active",
            "truncated",
            "unchecksummed",
            "scannedAt",
            "due",
            "thresholds"
        ]
    );
    assert_eq!(body["unchecksummed"], 1);
    assert_eq!(body["totalFiles"], 11);
    // 10 redundant copies sit ON the threshold: "more than", not "at least".
    assert_eq!(body["active"]["redundant"], 10);
    assert_eq!(body["due"], false);
    assert_eq!(body["thresholds"], json!({ "redundant": 10, "bytes": 1024 * 1024 * 1024 }));
    upload(db, "du1", "d11", Upload { checksum: Some(&set), size: 10, ..Default::default() }).await;
    assert_eq!(call(&app, req("GET", "/api/duplicates", Some(&me), None)).await.json()["due"], true);

    // A library made entirely of kept sets is not due, however big.
    sqlx::query(
        r#"insert into "duplicate_dismissals" ("owner_id", "checksum", "copies", "bytes") values ('du1', $1, 12, 120)"#,
    )
    .bind(&set)
    .execute(db)
    .await
    .unwrap();
    let kept = call(&app, req("GET", "/api/duplicates", Some(&me), None)).await.json();
    assert_eq!((kept["due"].clone(), kept["dismissedGroups"].clone()), (json!(false), json!(1)));
    t.drop().await;
}

#[tokio::test]
async fn duplicates_decisions_are_scoped_normalised_and_audited() {
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "dv1", Some("dv1"), "user").await;
    user(db, "dv2", Some("dv2"), "user").await;
    let me = token(db, "dv1", TokenScope::Write).await;
    let reader = token(db, "dv1", TokenScope::Read).await;
    let app = app_on(&t, &[]);
    let post = |body: Value| call(&app, req("POST", "/api/duplicates", Some(&me), Some(body)));
    let (digest, other) = ("a".repeat(64), "b".repeat(64));
    let rows = || async {
        sqlx::query_as::<_, (String, String, i32, i64)>(
            r#"select "owner_id", "checksum", "copies", "bytes" from "duplicate_dismissals" order by "owner_id", "checksum""#,
        )
        .fetch_all(db)
        .await
        .unwrap()
    };

    assert_eq!(
        call(&app, req("POST", "/api/duplicates", Some(&reader), Some(json!({ "countAgain": [digest] })))).await.status,
        StatusCode::FORBIDDEN
    );
    for body in [json!({}), json!({ "keep": [{ "checksum": digest }], "countAgain": [digest] })] {
        let res = post(body).await;
        assert_eq!((res.status, res.json()["reason"].clone()), (StatusCode::BAD_REQUEST, json!("one-verb")));
    }
    let typed = with(
        req("POST", "/api/duplicates", Some(&me), Some(json!({ "countAgain": [digest] }))),
        "content-type",
        "text/plain",
    );
    assert_eq!(call(&app, typed).await.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(post(json!({ "countAgain": ["not-a-digest"] })).await.json()["reason"], "malformed");
    assert!(rows().await.is_empty());

    let kept = post(json!({ "keep": [{ "checksum": digest.to_uppercase(), "copies": 4, "bytes": 900 }] })).await;
    assert_eq!(kept.json(), json!({ "message": "OK", "kept": 1, "checksums": [digest] }));
    assert_eq!(rows().await, vec![("dv1".into(), digest.clone(), 4, 900)]);

    // Matched back by digest, not position; nonsense counts read as none.
    post(json!({ "keep": [{ "checksum": other, "copies": 2, "bytes": 20 }, { "checksum": digest, "copies": -3 }] }))
        .await;
    assert_eq!(rows().await, vec![("dv1".into(), digest.clone(), 0, 0), ("dv1".into(), other.clone(), 2, 20)]);

    // Another account's decision is out of reach.
    sqlx::query(r#"insert into "duplicate_dismissals" ("owner_id", "checksum") values ('dv2', $1)"#)
        .bind(&digest)
        .execute(db)
        .await
        .unwrap();
    let restored = post(json!({ "countAgain": [digest] })).await;
    assert_eq!(restored.json(), json!({ "message": "OK", "restored": 1 }));
    assert_eq!(rows().await, vec![("dv1".into(), other.clone(), 2, 20), ("dv2".into(), digest.clone(), 0, 0)]);

    let actions: Vec<(String, Value)> =
        sqlx::query_as(r#"select "action", "detail" from "audit_log" where "user_id" = 'dv1' order by "created_at""#)
            .fetch_all(db)
            .await
            .unwrap();
    assert_eq!(
        actions,
        vec![
            ("uploads.duplicates_kept".to_string(), json!({ "count": 1, "requested": 1, "via": "api" })),
            ("uploads.duplicates_kept".to_string(), json!({ "count": 2, "requested": 2, "via": "api" })),
            ("uploads.duplicates_counted_again".to_string(), json!({ "count": 1, "requested": 1, "via": "api" })),
        ]
    );
    t.drop().await;
}

// ── /api/shortlinks and /s (api-route-auth.test.ts, lib/shortlinks.ts) ──────

#[tokio::test]
async fn shortlinks_refuse_with_ok_false_and_mint_through_the_shared_core() {
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "sl1", Some("tag1"), "user").await;
    user(db, "sl2", None, "user").await;
    let me = token(db, "sl1", TokenScope::Write).await;
    let reader = token(db, "sl1", TokenScope::Read).await;
    let tagless = token(db, "sl2", TokenScope::Full).await;
    let app = app_on(&t, &[]);
    let post = |tok: &str, body: Value| call(&app, req("POST", "/api/shortlinks", Some(tok), Some(body)));

    let none = call(&app, req("GET", "/api/shortlinks", None, None)).await;
    assert_eq!(none.json(), json!({ "ok": false, "message": "Token not specified or not found." }));
    let scoped = post(&reader, json!({ "target": "https://example.com" })).await;
    assert_eq!(scoped.status, StatusCode::FORBIDDEN);
    assert_eq!(scoped.json()["ok"], false);
    assert_eq!(scoped.json()["reason"], "insufficient_scope");
    // Credential before body; Content-Type is the CSRF control.
    let anonymous = Request::builder()
        .method("POST")
        .uri("/api/shortlinks")
        .header("content-type", "text/plain")
        .body(Body::from("not json at all"))
        .unwrap();
    assert_eq!(call(&app, anonymous).await.status, StatusCode::UNAUTHORIZED);
    let typed = with(
        req("POST", "/api/shortlinks", Some(&me), Some(json!({ "target": "https://evil.example" }))),
        "content-type",
        "text/plain;charset=UTF-8",
    );
    let typed = call(&app, typed).await;
    assert_eq!(typed.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(
        typed.json(),
        json!({ "ok": false, "reason": "error", "message": "Content-Type must be application/json." })
    );

    let js = post(&me, json!({ "target": "javascript:alert(1)" })).await;
    assert_eq!((js.status, js.json()), (StatusCode::BAD_REQUEST, json!({ "ok": false, "reason": "invalid-url" })));
    let made = post(&me, json!({ "target": "example.com/a", "customCode": "My Code", "password": "pw" })).await;
    assert_eq!(made.status, StatusCode::CREATED);
    assert_eq!(
        made.json(),
        json!({ "ok": true, "code": "tag1/my-code", "url": "https://host.example.com/s/tag1/my-code" })
    );
    let taken = post(&me, json!({ "target": "example.com/b", "customCode": "my code" })).await;
    assert_eq!((taken.status, taken.json()["reason"].clone()), (StatusCode::CONFLICT, json!("code-taken")));
    let root = post(&me, json!({ "target": "example.com/b", "useUserCode": false })).await;
    assert_eq!((root.status, root.json()["reason"].clone()), (StatusCode::FORBIDDEN, json!("root-not-allowed")));
    let no_tag = post(&tagless, json!({ "target": "example.com/c" })).await;
    assert_eq!((no_tag.status, no_tag.json()["reason"].clone()), (StatusCode::UNPROCESSABLE_ENTITY, json!("no-tag")));

    let list = call(&app, req("GET", "/api/shortlinks", Some(&reader), None)).await.json();
    let link = &list["links"][0];
    let keys: Vec<&String> = link.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["id", "code", "url", "target", "clicks", "hasPassword", "lastUsedAt", "expiresAt", "createdAt"]);
    assert_eq!((link["target"].clone(), link["hasPassword"].clone()), (json!("https://example.com/a"), json!(true)));
    t.drop().await;
}

#[tokio::test]
async fn short_links_redirect_on_both_hosts_and_count_clicks() {
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "rd1", Some("tag1"), "user").await;
    let link_id: Uuid = sqlx::query_scalar(
        r#"insert into "short_links" ("owner_id", "code", "target") values ('rd1', 'tag1/promo', 'https://example.com/promo') returning "id""#,
    )
    .fetch_one(db)
    .await
    .unwrap();
    sqlx::query(r#"insert into "short_links" ("owner_id", "code", "target", "password_hash") values ('rd1', 'root1', 'https://example.com/r', 'x')"#)
        .execute(db)
        .await
        .unwrap();
    let app = app_on(&t, &[("SHORTLINK_URL", "https://s.example.com")]);
    let get = |host: &'static str, path: &str| {
        call(&app, Request::get(path).header("host", host).body(Body::empty()).unwrap())
    };

    let main = get("host.example.com", "/s/tag1/promo").await;
    assert_eq!((main.status, main.header("location")), (StatusCode::FOUND, Some("https://example.com/promo".into())));
    // proxy.ts rewrites the shortener host's bare path to /s/...
    let short = get("s.example.com", "/tag1/promo").await;
    assert_eq!((short.status, short.header("location")), (StatusCode::FOUND, Some("https://example.com/promo".into())));
    let missing = get("s.example.com", "/nothing").await;
    assert_eq!((missing.status, missing.text()), (StatusCode::NOT_FOUND, "Not found".to_string()));
    assert_eq!(missing.header("content-type").as_deref(), Some("text/plain;charset=UTF-8"));
    // A password-protected link sends a visitor without a token to the gate.
    let gated = get("host.example.com", "/s/root1").await;
    assert_eq!(gated.header("location").as_deref(), Some("https://host.example.com/s/root1/unlock"));

    // Clicks are counted in the background.
    let mut clicks = 0;
    for _ in 0..50 {
        clicks = sqlx::query_scalar(r#"select "clicks" from "short_links" where "id" = $1"#)
            .bind(link_id)
            .fetch_one(db)
            .await
            .unwrap();
        if clicks == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(clicks, 2);
    t.drop().await;
}

// ── GET /api/admin/disk (tests/unit/admin-disk-api.test.ts) ─────────────────

#[tokio::test]
async fn admin_disk_report_has_the_client_contract_and_is_paced() {
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "ad-admin", Some("adm"), "admin").await;
    user(db, "ad-user", Some("usr"), "user").await;
    let admin = token(db, "ad-admin", TokenScope::Full).await;
    let admin_write = token(db, "ad-admin", TokenScope::Write).await;
    let plain = token(db, "ad-user", TokenScope::Full).await;
    upload(db, "ad-user", "big", Upload { size: 5000, ..Default::default() }).await;
    upload(db, "ad-user", "bin", Upload { size: 70, deleted: true, ..Default::default() }).await;
    let app = app_on(&t, &[]);
    let get = |tok: &str| call(&app, req("GET", "/api/admin/disk", Some(tok), None));

    let hidden = get(&plain).await;
    assert_eq!((hidden.status, hidden.text()), (StatusCode::NOT_FOUND, "Not found".to_string()));
    assert_eq!(hidden.header("cache-control"), None);
    let narrow = get(&admin_write).await;
    assert_eq!((narrow.status, narrow.json()["required_scope"].clone()), (StatusCode::FORBIDDEN, json!("full")));

    let res = get(&admin).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header("cache-control").as_deref(), Some("private, no-store"));
    let body = res.json();
    let keys: Vec<&String> = body.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "driver",
            "local",
            "liveFiles",
            "liveBytes",
            "trashedFiles",
            "trashedBytes",
            "disk",
            "minFreeBytes",
            "kinds",
            "kindBytes",
            "accounts",
            "biggest",
            "cold",
            "coldTotals",
            "trash",
            "trashRetentionDays",
            "trashDueSoonDays"
        ]
    );
    assert_eq!((body["liveFiles"].clone(), body["liveBytes"].clone()), (json!(1), json!(5000)));
    assert_eq!((body["trashedFiles"].clone(), body["trashedBytes"].clone()), (json!(1), json!(70)));
    assert_eq!((body["driver"].clone(), body["local"].clone()), (json!("local"), json!(true)));
    assert_eq!(body["kinds"], json!([{ "kind": "image", "files": 1, "bytes": 5000 }]));
    assert_eq!(body["accounts"][0]["tag"], "usr");
    assert_eq!(body["accounts"][0]["liveBytes"], 5000);
    assert_eq!(body["biggest"][0]["code"], "big");
    assert_eq!(body["biggest"][0]["lastAccessedAt"], Value::Null);
    assert_eq!(body["trash"]["files"], 1);
    assert_eq!((body["trashRetentionDays"].clone(), body["trashDueSoonDays"].clone()), (json!(30), json!(7)));

    // 30 per minute per account, then 429 with retry-after (one call above).
    for _ in 0..29 {
        assert_eq!(get(&admin).await.status, StatusCode::OK);
    }
    let paced = get(&admin).await;
    assert_eq!(paced.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(paced.json(), json!({ "message": "Too many requests.", "reason": "rate_limited" }));
    let after: i64 = paced.header("retry-after").unwrap().parse().unwrap();
    assert!((1..=60).contains(&after));

    // A signed-in administrator, and a signed-in user.
    let cookie = session_cookie(db, "ad-user").await;
    let user_session = call(&app, with(req("GET", "/api/admin/disk", None, None), "cookie", &cookie)).await;
    assert_eq!(user_session.status, StatusCode::NOT_FOUND);
    t.drop().await;
}

// ── Replacing an avatar or a wallpaper deletes the previous object ──────────
// Fix 7b (SKELETON.md 9): upstream's `UPDATE ... RETURNING` read the NEW key,
// so the previous object stayed in storage forever.

const TINY_PNG: &str = "89504e470d0a1a0a0000000d4948445200000004000000040802000000269309290000001049444154789c6338212707470cc47100b16310413b4b63820000000049454e44ae426082";

fn stored(root: &std::path::Path, prefix: &str, user_id: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join(prefix))
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.retain(|n| n.starts_with(&format!("{user_id}-")));
    names.sort();
    names
}

fn image_post(uri: &str, cookie: &str) -> Request<Body> {
    let bytes = hex::decode(TINY_PNG).unwrap();
    Request::post(uri)
        .header("host", "host.example.com")
        .header("cookie", cookie)
        .header("content-type", "image/png")
        .header("content-length", bytes.len().to_string())
        .body(Body::from(bytes))
        .unwrap()
}

#[tokio::test]
async fn replacing_avatar_or_wallpaper_deletes_the_previous_object() {
    let db = db_or_skip!();
    // The storage driver reads the process environment (as upstream); give
    // this app a private directory instead.
    let root = tempfile::tempdir().unwrap();
    let vars: std::collections::HashMap<String, String> = [
        ("APP_URL", "https://host.example.com"),
        ("NODE_ENV", "production"),
        ("SKIP_ENV_VALIDATION", "1"),
        ("AUTH_SECRET", SECRET),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let mut state = AppState::new(Config::from_env_map(&vars).unwrap(), Paths::crate_dir(), db.pool.clone()).unwrap();
    state.storage = modlight_server::storage::Storage::with_driver(std::sync::Arc::new(
        modlight_server::storage::local::LocalDriver::new(root.path().to_str().unwrap()),
    ));
    let app = App::new(state);
    let root = root.path();
    let id = format!("repl{}", rand::random::<u32>());
    user(&db.pool, &id, Some(&id), "user").await;
    let cookie = session_cookie(&db.pool, &id).await;

    for (route, column, prefix) in
        [("/api/avatar", "image", "avatars"), ("/api/background", "background_key", "backgrounds")]
    {
        let first = call(&app, image_post(route, &cookie)).await;
        assert_eq!(first.status, StatusCode::OK, "{route}: {}", first.text());
        let first_key: String = sqlx::query_scalar(&format!(r#"select "{column}" from "user" where "id" = $1"#))
            .bind(&id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(stored(root, prefix, &id).len(), 1, "{route}");

        let second = call(&app, image_post(route, &cookie)).await;
        assert_eq!(second.status, StatusCode::OK, "{route}: {}", second.text());
        let second_key: String = sqlx::query_scalar(&format!(r#"select "{column}" from "user" where "id" = $1"#))
            .bind(&id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_ne!(first_key, second_key);
        // Only the current object is left.
        let left = stored(root, prefix, &id);
        assert_eq!(left.len(), 1, "{route}: {left:?}");
        assert!(second_key.ends_with(&left[0]), "{route}: {second_key} vs {left:?}");

        // A stored key naming ANOTHER account's object is never deleted.
        let victim = format!("{prefix}/someoneelse-0000.webp");
        let victim_path = root.join(&victim);
        std::fs::write(&victim_path, b"not yours").unwrap();
        sqlx::query(&format!(r#"update "user" set "{column}" = $1 where "id" = $2"#))
            .bind(&victim)
            .bind(&id)
            .execute(&db.pool)
            .await
            .unwrap();
        let third = call(&app, image_post(route, &cookie)).await;
        assert_eq!(third.status, StatusCode::OK, "{route}: {}", third.text());
        assert!(victim_path.exists(), "{route}: another account's object was deleted");
        std::fs::remove_file(&victim_path).unwrap();

        // Removing deletes the current object too (the second one stays: the
        // test pointed the column away from it by hand).
        let removed = call(
            &app,
            Request::delete(route)
                .header("host", "host.example.com")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(removed.status, StatusCode::OK, "{route}: {}", removed.text());
        assert_eq!(stored(root, prefix, &id), vec![left[0].clone()], "{route}");
    }
    db.drop().await;
}
