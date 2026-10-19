//! The library server actions end to end (`POST /_rpc/...` through the whole
//! front door) against a real PostgreSQL and a real storage directory. Runs
//! only when `MODLIGHT_TEST_DATABASE_URL` names a server where the user may
//! create databases (see tests/db_compat.rs); each test gets its own
//! throwaway, migrated database.
//!
//! These replace upstream's mock-based unit tests of the actions
//! (duplicates-trash, duplicate-reminder's keepDuplicateSets and
//! countDuplicateSetsAgain, the storage-orphan-scan integration test) and
//! cover the two exports the reference never registered as actions
//! (`folders.moveFolder`, `folders.countUnfiledUploads`: no client component
//! imports them, so tests/parity/library_actions.py cannot reach them there).

mod common;

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use modlight_server::config::Config;
use modlight_server::http::App;
use modlight_server::paths::Paths;
use modlight_server::state::AppState;
use modlight_server::storage::{LocalDriver, Storage};
use serde_json::{Value, json};
use sqlx::postgres::PgConnectOptions;
use sqlx::{ConnectOptions, PgPool};
use uuid::Uuid;

const SECRET: &str = "library-actions-test-secret-0123456789abcdef";
const HOST: &str = "127.0.0.1:8421";

struct Harness {
    admin: PgConnectOptions,
    name: String,
    pool: PgPool,
    app: App,
    storage: tempfile::TempDir,
}

impl Harness {
    async fn new() -> Option<Harness> {
        let url = std::env::var("MODLIGHT_TEST_DATABASE_URL").ok()?;
        let admin = PgConnectOptions::from_str(&url).expect("MODLIGHT_TEST_DATABASE_URL");
        let name = format!("modlight_libact_{}_{}", std::process::id(), rand::random::<u32>());
        let mut conn = admin.connect().await.expect("connect to the test server");
        sqlx::query(&format!("CREATE DATABASE \"{name}\"")).execute(&mut conn).await.expect("create database");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect_with(admin.clone().database(&name).options([("TimeZone", "UTC")]))
            .await
            .expect("connect to the temp database");
        modlight_server::db::migrate::run(&pool, &Paths::crate_dir().migrations, false).await.expect("migrate");

        let storage = tempfile::tempdir().unwrap();
        let root = storage.path().to_str().unwrap().to_string();
        let vars: HashMap<String, String> = [
            ("APP_URL", "http://127.0.0.1:8421"),
            ("AUTH_SECRET", SECRET),
            ("TRUST_PROXY", "true"),
            ("STORAGE_DRIVER", "local"),
            ("STORAGE_LOCAL_PATH", root.as_str()),
            ("SKIP_ENV_VALIDATION", "1"),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
        let config = Config::from_env_map(&vars).expect("config");
        let mut state = AppState::new(config, Paths::crate_dir(), pool.clone()).expect("state");
        state.storage = Storage::with_driver(Arc::new(LocalDriver::new(&root)));
        Some(Harness { admin, name, pool, app: App::new(state), storage })
    }

    async fn drop(self) {
        self.pool.close().await;
        if let Ok(mut conn) = self.admin.connect().await {
            let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)", self.name))
                .execute(&mut conn)
                .await;
        }
    }

    async fn user(&self, id: &str, tag: &str, role: &str) {
        sqlx::query(
            r#"insert into "user" ("id", "name", "email", "email_verified", "created_at", "updated_at", "user_code", "role", "theme")
               values ($1, $1, $1 || '@example.test', true, now(), now(), $2, $3, 'modern')"#,
        )
        .bind(id)
        .bind(tag)
        .bind(role)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    /// A live session row and the signed cookie value a browser would send.
    async fn session(&self, user_id: &str) -> String {
        let token = format!("tok{}", Uuid::new_v4().simple());
        sqlx::query(
            r#"insert into "session" ("id", "token", "user_id", "expires_at", "created_at", "updated_at")
               values ($1, $2, $3, (now() at time zone 'utc') + interval '7 days', now() at time zone 'utc', now() at time zone 'utc')"#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&token)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .unwrap();
        modlight_server::auth::cookies::sign_cookie_value(&token, SECRET)
    }

    async fn upload(&self, owner: Option<&str>, code: &str, size: i64, extra: &str) -> Uuid {
        let id: Uuid = sqlx::query_scalar(
            r#"insert into "uploads" ("owner_id", "code", "filename", "ext", "mime_type", "size", "storage_key", "delete_token")
               values ($1, $2, $2 || '.png', 'png', 'image/png', $3, coalesce($1, 'gone') || '/' || $2 || '.png', 'tok-' || $2)
               returning "id""#,
        )
        .bind(owner)
        .bind(code)
        .bind(size)
        .fetch_one(&self.pool)
        .await
        .unwrap();
        if !extra.is_empty() {
            sqlx::query(&format!(r#"update "uploads" set {extra} where "id" = $1"#))
                .bind(id)
                .execute(&self.pool)
                .await
                .unwrap();
        }
        id
    }

    /// One action call; the envelope as JSON (development mode: real messages).
    async fn rpc(&self, cookie: Option<&str>, module: &str, export: &str, args: Value) -> Value {
        self.rpc_full(cookie, module, export, args).await.0
    }

    async fn rpc_full(&self, cookie: Option<&str>, module: &str, export: &str, args: Value) -> (Value, Vec<String>) {
        let mut req = Request::post(format!("/_rpc/{module}/{export}"))
            .header("host", HOST)
            .header("origin", format!("http://{HOST}"))
            .header("x-modlight-rpc", "1")
            .header("x-real-ip", "203.0.113.9")
            .header("content-type", "application/json");
        if let Some(c) = cookie {
            req = req.header("cookie", format!("better-auth.session_token={c}"));
        }
        let a = common::call(&self.app, req.body(Body::from(args.to_string())).unwrap()).await;
        let cookies = a.headers.get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect();
        (a.json(), cookies)
    }

    async fn scalar_i64(&self, sql: &str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(&self.pool).await.unwrap()
    }

    async fn audits(&self, action: &str) -> Vec<(Option<String>, Value)> {
        sqlx::query_as(r#"select "user_id", "detail" from "audit_log" where "action" = $1 order by "created_at""#)
            .bind(action)
            .fetch_all(&self.pool)
            .await
            .unwrap()
    }

    fn put(&self, key: &str, data: &[u8], age_seconds: u64) {
        let path = self.storage.path().join(key);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, data).unwrap();
        if age_seconds > 0 {
            let when = std::time::SystemTime::now() - std::time::Duration::from_secs(age_seconds);
            std::fs::File::options().write(true).open(&path).unwrap().set_modified(when).unwrap();
        }
    }

    fn exists(&self, key: &str) -> bool {
        self.storage.path().join(key).is_file()
    }
}

fn sha(c: char) -> String {
    std::iter::repeat_n(c, 64).collect()
}

// ── duplicates (tests/unit/duplicates-trash.test.ts, duplicate-reminder.test.ts) ──

#[tokio::test]
async fn trash_duplicates_scopes_the_statement_to_the_caller() {
    let Some(h) = Harness::new().await else { return };
    h.user("me", "me", "user").await;
    h.user("them", "them", "user").await;
    let me = h.session("me").await;
    let mine = h.upload(Some("me"), "abc", 100, "").await;
    let mine2 = h.upload(Some("me"), "abd", 250, "").await;
    let trashed = h.upload(Some("me"), "abe", 7, "deleted_at = '2026-01-01T00:00:00Z'").await;
    let theirs = h.upload(Some("them"), "xyz", 999, "").await;

    // Signed out: refused before any statement.
    assert_eq!(
        h.rpc(None, "duplicates", "trashDuplicates", json!([[mine]])).await,
        json!({ "ok": { "ok": false, "error": "unauthorized" } })
    );
    // Bad lists answer with data, not a throw.
    assert_eq!(
        h.rpc(Some(&me), "duplicates", "trashDuplicates", json!([["not-a-uuid"]])).await["ok"],
        json!({ "ok": false, "error": "malformed" })
    );
    assert_eq!(h.rpc(Some(&me), "duplicates", "trashDuplicates", json!([[]])).await["ok"]["error"], "empty");

    // Another account's well-formed id: the WHERE is what stops it.
    let out = h.rpc(Some(&me), "duplicates", "trashDuplicates", json!([[theirs]])).await;
    assert_eq!(out["ok"], json!({ "ok": true, "trashed": 0, "bytes": 0, "skipped": 1 }));
    assert_eq!(out["revalidate"], true);
    assert_eq!(
        h.scalar_i64(r#"select count(*) from "uploads" where "deleted_at" is null and "owner_id" = 'them'"#).await,
        1
    );
    assert!(h.audits("uploads.duplicates_trashed").await.is_empty(), "nothing happened, nothing claimed");

    // Counted from what the statement returned, not from what was asked.
    let before: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar(r#"select "deleted_at" from "uploads" where "id" = $1"#)
            .bind(trashed)
            .fetch_one(&h.pool)
            .await
            .unwrap();
    let out = h
        .rpc(
            Some(&me),
            "duplicates",
            "trashDuplicates",
            json!([[mine, mine2.to_string().to_uppercase(), trashed, theirs]]),
        )
        .await;
    assert_eq!(out["ok"], json!({ "ok": true, "trashed": 2, "bytes": 350, "skipped": 2 }));
    let after: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar(r#"select "deleted_at" from "uploads" where "id" = $1"#)
            .bind(trashed)
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(before, after, "an already-trashed row is not re-stamped");
    let audits = h.audits("uploads.duplicates_trashed").await;
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].0.as_deref(), Some("me"));
    assert_eq!(audits[0].1["count"], 2);
    assert_eq!(audits[0].1["bytes"], 350);
    assert_eq!(audits[0].1["requested"], 4);
    h.drop().await;
}

#[tokio::test]
async fn keep_and_count_again_are_keyed_by_owner_and_checksum() {
    let Some(h) = Harness::new().await else { return };
    h.user("me", "me", "user").await;
    h.user("them", "them", "user").await;
    let me = h.session("me").await;
    let them = h.session("them").await;

    assert_eq!(
        h.rpc(None, "duplicates", "keepDuplicateSets", json!([[{ "checksum": sha('a'), "copies": 1, "bytes": 1 }]]))
            .await,
        json!({ "ok": { "ok": false, "error": "unauthorized" } })
    );
    assert_eq!(
        h.rpc(Some(&me), "duplicates", "keepDuplicateSets", json!([[{ "checksum": "nope", "copies": 1, "bytes": 1 }]]))
            .await["ok"],
        json!({ "ok": false, "error": "malformed" })
    );
    assert_eq!(h.rpc(Some(&me), "duplicates", "keepDuplicateSets", json!([[]])).await["ok"]["error"], "empty");
    assert_eq!(h.scalar_i64(r#"select count(*) from "duplicate_dismissals""#).await, 0);
    assert!(h.audits("uploads.duplicates_kept").await.is_empty());

    // Normalised and collapsed: a double click must not become a 500.
    let out = h
        .rpc(
            Some(&me),
            "duplicates",
            "keepDuplicateSets",
            json!([[{ "checksum": sha('A'), "copies": 2, "bytes": 20 }, { "checksum": sha('a'), "copies": 2, "bytes": 20 }]]),
        )
        .await;
    assert_eq!(out["ok"], json!({ "ok": true, "kept": 1, "checksums": [sha('a')] }));
    let row: (String, String, i32, i64) =
        sqlx::query_as(r#"select "owner_id", "checksum", "copies", "bytes" from "duplicate_dismissals""#)
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(row, ("me".into(), sha('a'), 2, 20));
    // Counts past the column's range fail like the JavaScript number did.
    let out =
        h.rpc(Some(&me), "duplicates", "keepDuplicateSets", json!([[{ "checksum": sha('b'), "copies": 1e12 }]])).await;
    assert_eq!(out["ok"], json!({ "ok": false, "error": "failed" }));

    // The undo is scoped to the caller: a checksum is a global fact about bytes.
    let out = h.rpc(Some(&them), "duplicates", "countDuplicateSetsAgain", json!([[sha('a')]])).await;
    assert_eq!(out["ok"], json!({ "ok": true, "restored": 0 }));
    assert_eq!(h.scalar_i64(r#"select count(*) from "duplicate_dismissals""#).await, 1);
    assert_eq!(
        h.rpc(Some(&me), "duplicates", "countDuplicateSetsAgain", json!([["not-a-digest"]])).await["ok"],
        json!({ "ok": false, "error": "malformed" })
    );
    let out = h.rpc(Some(&me), "duplicates", "countDuplicateSetsAgain", json!([[sha('A')]])).await;
    assert_eq!(out["ok"], json!({ "ok": true, "restored": 1 }));
    assert_eq!(h.scalar_i64(r#"select count(*) from "duplicate_dismissals""#).await, 0);
    assert_eq!(h.audits("uploads.duplicates_counted_again").await.len(), 2);
    h.drop().await;
}

// ── admin: storage scan (tests/integration/storage-orphan-scan.test.ts) ─────

#[tokio::test]
async fn the_storage_scan_reports_only_unreferenced_old_objects() {
    let Some(h) = Harness::new().await else { return };
    h.user("adm", "adm", "admin").await;
    h.user("alicja", "alicja", "user").await;
    let adm = h.session("adm").await;
    let scan = || async { h.rpc(Some(&adm), "admin", "runStorageScan", json!([])).await["ok"].clone() };

    // Per-account decoration and cached conversions are never even opened.
    h.put("avatars/user_1-abc123.webp", b"a", 7 * 86400);
    h.put("backgrounds/user_1-def456.webp", b"b", 7 * 86400);
    h.put("derived/some-upload-id/webp-256.webp", b"d", 7 * 86400);
    let r = scan().await;
    assert_eq!((r["supported"].clone(), r["driver"].clone()), (json!(true), json!("local")));
    assert_eq!((r["scanned"].clone(), r["orphans"].clone(), r["sample"].clone()), (json!(0), json!(0), json!([])));

    // A file no row points at, beside the excluded trees; a nested directory
    // named like one of them is NOT exempt.
    h.put("alicja/left-behind.png", b"12345", 7 * 86400);
    h.put("alicja/avatars/sneaky.png", b"xy", 7 * 86400);
    let r = scan().await;
    assert_eq!(r["orphans"], 2);
    assert_eq!(r["orphanBytes"], 7);
    let keys: Vec<&str> = r["sample"].as_array().unwrap().iter().map(|s| s["key"].as_str().unwrap()).collect();
    // Depth first from a stack: a directory's own files before its subdirectories'.
    assert_eq!(keys, ["alicja/left-behind.png", "alicja/avatars/sneaky.png"]);

    // Referenced keys (storage AND thumbnail) say nothing; a fresh object is
    // only counted as skipped (it may be mid-finalize).
    h.upload(Some("alicja"), "kept", 5, "thumb_key = 'thumbs/alicja/kept.webp'").await;
    h.put("alicja/kept.png", b"kept!", 7 * 86400);
    h.put("thumbs/alicja/kept.webp", b"t", 7 * 86400);
    h.put("alicja/in-flight.png", b"new", 0);
    let r = scan().await;
    assert_eq!(r["scanned"], 5);
    assert_eq!(r["orphans"], 2);
    assert_eq!(r["skippedRecent"], 1);
    assert_eq!(r["truncated"], false);

    // Not an administrator: redirected before the walk.
    let user = h.session("alicja").await;
    assert_eq!(h.rpc(Some(&user), "admin", "runStorageScan", json!([])).await, json!({ "redirect": "/dashboard" }));
    assert_eq!(h.rpc(None, "admin", "runStorageScan", json!([])).await, json!({ "redirect": "/login" }));
    h.drop().await;
}

// ── admin: orphans and account deletion ─────────────────────────────────────

#[tokio::test]
async fn deleting_an_account_keeps_or_destroys_its_files() {
    let Some(h) = Harness::new().await else { return };
    h.user("adm", "adm", "admin").await;
    h.user("keep", "keep", "user").await;
    h.user("gone", "gone", "user").await;
    let adm = h.session("adm").await;
    h.session("keep").await;
    for (i, size) in [10i64, 20, 30].iter().enumerate() {
        h.upload(Some("keep"), &format!("k{i}"), *size, "").await;
        h.put(&format!("keep/k{i}.png"), b"keep", 7200);
    }
    let mut gone_files: Vec<Uuid> = Vec::new();
    for i in 0..3 {
        gone_files.push(h.upload(Some("gone"), &format!("g{i}"), 100, "").await);
        h.put(&format!("gone/g{i}.png"), b"gone", 7200);
    }
    sqlx::query(
        r#"insert into "verification" ("id", "identifier", "value", "expires_at", "created_at", "updated_at")
           values ('v1', 'invite:abc', 'gone', now() + interval '1 day', now(), now()),
                  ('v2', 'reset:abc', 'gone', now() + interval '1 day', now(), now())"#,
    )
    .execute(&h.pool)
    .await
    .unwrap();

    // Refusals first: nothing is touched.
    let refuse = |reason: &str| json!({ "ok": { "ok": false, "reason": reason } });
    assert_eq!(h.rpc(Some(&adm), "admin", "deleteUserAccount", json!(["", "delete"])).await, refuse("not-found"));
    assert_eq!(h.rpc(Some(&adm), "admin", "deleteUserAccount", json!(["adm", "delete"])).await, refuse("self"));
    assert_eq!(h.rpc(Some(&adm), "admin", "deleteUserAccount", json!(["nobody", "delete"])).await, refuse("not-found"));
    let impact = h.rpc(Some(&adm), "admin", "loadDeletionImpact", json!(["gone"])).await["ok"].clone();
    assert_eq!(impact["files"], 3);
    assert_eq!(impact["bytes"], 300);
    assert_eq!(h.rpc(Some(&adm), "admin", "loadDeletionImpact", json!([""])).await["ok"], Value::Null);

    // "delete": bytes, rows, sessions, accounts, the user; only invitations of it.
    let out = h.rpc(Some(&adm), "admin", "deleteUserAccount", json!(["gone", "delete"])).await;
    assert_eq!(out["ok"], json!({ "ok": true, "deleted": 3, "bytes": 300, "failed": 0, "orphaned": 0 }));
    assert_eq!(out["revalidate"], true);
    assert!((0..3).all(|i| !h.exists(&format!("gone/g{i}.png"))));
    let left: i64 = sqlx::query_scalar(r#"select count(*) from "uploads" where "id" = any($1)"#)
        .bind(&gone_files)
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
    assert_eq!(h.scalar_i64(r#"select count(*) from "user" where "id" = 'gone'"#).await, 0);
    assert_eq!(
        h.scalar_i64(r#"select count(*) from "verification" where "value" = 'gone'"#).await,
        1,
        "only invite: rows go"
    );

    // "keep": the rows become orphans, owned by nobody, bytes untouched.
    let out = h.rpc(Some(&adm), "admin", "deleteUserAccount", json!(["keep", "keep"])).await;
    assert_eq!(out["ok"], json!({ "ok": true, "deleted": 0, "bytes": 0, "failed": 0, "orphaned": 3 }));
    assert_eq!(h.scalar_i64(r#"select count(*) from "session" where "user_id" = 'keep'"#).await, 0);
    let orphans = h.rpc(Some(&adm), "admin", "loadOrphans", json!([])).await["ok"].clone();
    assert_eq!(orphans["summary"]["files"], 3);
    assert_eq!(orphans["summary"]["bytes"], 60);
    assert_eq!(orphans["items"].as_array().unwrap().len(), 3);

    // One by one; the ownerless predicate is in the WHERE.
    let first = orphans["items"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(h.rpc(Some(&adm), "admin", "deleteOrphan", json!([first])).await["ok"], json!({ "ok": true }));
    assert_eq!(h.rpc(Some(&adm), "admin", "deleteOrphan", json!([first])).await["ok"], json!({ "ok": false }));
    assert!(!h.exists("keep/k0.png"));
    assert!(h.rpc(Some(&adm), "admin", "deleteOrphan", json!(["nope"])).await["error"].is_object());

    // The typed count gate, re-read on the server.
    let purge = |c: Value| h.rpc(Some(&adm), "admin", "purgeAllOrphans", json!([c]));
    assert_eq!(purge(json!("3")).await["ok"]["reason"], "confirmation");
    assert!(purge(json!(2)).await["error"].is_object(), "a non-string confirmation is a TypeError");
    let out = purge(json!(" 2 ")).await;
    assert_eq!(out["ok"], json!({ "ok": true, "deleted": 2, "bytes": 50, "failed": 0 }));
    assert!(!h.exists("keep/k1.png") && !h.exists("keep/k2.png"));
    assert_eq!(
        purge(json!("0")).await["ok"],
        json!({ "ok": false, "reason": "empty", "deleted": 0, "bytes": 0, "failed": 0 })
    );
    h.drop().await;
}

#[tokio::test]
async fn recalculating_quotas_corrects_every_drifted_counter() {
    let Some(h) = Harness::new().await else { return };
    h.user("adm", "adm", "admin").await;
    h.user("high", "high", "user").await;
    h.user("null", "null", "user").await;
    let adm = h.session("adm").await;
    h.upload(Some("high"), "h1", 40, "").await;
    h.upload(Some("null"), "n1", 7, "deleted_at = now()").await;
    h.upload(None, "o1", 1000, "").await;
    sqlx::query(r#"update "user" set "used_quota" = 999 where "id" = 'high'"#).execute(&h.pool).await.unwrap();
    sqlx::query(r#"update "user" set "used_quota" = null where "id" = 'null'"#).execute(&h.pool).await.unwrap();
    let out = h.rpc(Some(&adm), "admin", "recalculateEveryQuota", json!([])).await;
    let mut ok = out["ok"].clone();
    ok["changes"].as_array_mut().unwrap().sort_by_key(|c| c["id"].as_str().unwrap().to_string());
    assert_eq!(
        ok,
        json!({ "accounts": 3, "corrected": 2, "drift": -952, "changes": [
            { "id": "high", "email": "high@example.test", "before": 999, "after": 40 },
            { "id": "null", "email": "null@example.test", "before": 0, "after": 7 },
        ] })
    );
    assert_eq!(out["revalidate"], true);
    h.drop().await;
}

// ── uploads ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn upload_actions_reach_own_rows_and_audit_an_admins_touch() {
    let Some(h) = Harness::new().await else { return };
    h.user("me", "me", "user").await;
    h.user("them", "them", "user").await;
    h.user("adm", "adm", "admin").await;
    sqlx::query(r#"update "user" set "used_quota" = 500 where "id" = 'them'"#).execute(&h.pool).await.unwrap();
    let me = h.session("me").await;
    let adm = h.session("adm").await;
    let mine = h.upload(Some("me"), "mine", 10, "").await;
    let theirs = h.upload(Some("them"), "theirs", 200, "").await;
    h.put("them/theirs.png", b"bytes", 0);

    let err = |v: Value| v["error"]["message"].as_str().unwrap_or("").to_string();
    assert_eq!(err(h.rpc(None, "uploads", "togglePublish", json!([mine])).await), "Unauthorized");
    let out = h.rpc(Some(&me), "uploads", "togglePublish", json!([mine])).await;
    assert_eq!(out, json!({ "ok": { "published": false }, "revalidate": true }));
    assert_eq!(err(h.rpc(Some(&me), "uploads", "togglePublish", json!([theirs])).await), "Not found");
    assert_eq!(err(h.rpc(Some(&me), "uploads", "deleteUploadForever", json!([mine])).await), "Not found");
    assert_eq!(h.rpc(Some(&me), "uploads", "trashSummary", json!([])).await["ok"], json!({ "files": 0, "bytes": 0 }));
    assert_eq!(err(h.rpc(Some(&me), "uploads", "trashSummary", json!(["them"])).await), "Not found");
    assert_eq!(
        err(h.rpc(Some(&me), "uploads", "setVanity", json!([mine, "login"])).await),
        "\"login\" is reserved by the app"
    );

    // An administrator reaches another account's file, and leaves a trace.
    assert_eq!(h.rpc(Some(&adm), "uploads", "trashUpload", json!([theirs])).await["ok"], json!({ "$undefined": 1 }));
    assert_eq!(
        h.rpc(Some(&adm), "uploads", "trashSummary", json!(["them"])).await["ok"],
        json!({ "files": 1, "bytes": 200 })
    );
    let out = h.rpc(Some(&adm), "uploads", "emptyTrash", json!([" them "])).await;
    assert_eq!(out["ok"], json!({ "deleted": 1, "bytes": 200, "failed": 0 }));
    assert!(!h.exists("them/theirs.png"));
    assert_eq!(h.scalar_i64(r#"select "used_quota" from "user" where "id" = 'them'"#).await, 300);
    let trashed = h.audits("admin.upload.trashed").await;
    assert_eq!(trashed.len(), 1);
    assert_eq!(trashed[0].1["ownerId"], "them");
    assert_eq!(trashed[0].1["byEmail"], "adm@example.test");
    let emptied = h.audits("admin.trash.emptied").await;
    assert_eq!(
        emptied[0].1,
        json!({ "by": "adm", "byEmail": "adm@example.test", "ownerId": "them", "ownerTag": "them", "files": 1, "bytes": 200, "failed": 0 })
    );
    // Own rows leave no audit entry.
    h.rpc(Some(&me), "uploads", "setUploadMaxViews", json!([mine, 2.5])).await;
    assert!(h.audits("admin.upload.link_changed").await.is_empty());
    assert_eq!(h.scalar_i64(&format!(r#"select "max_views"::bigint from "uploads" where "id" = '{mine}'"#)).await, 2);
    h.drop().await;
}

// ── folders (including the two exports the reference never registered) ─────

#[tokio::test]
async fn folder_actions_move_count_and_list() {
    let Some(h) = Harness::new().await else { return };
    h.user("me", "me", "user").await;
    let me = h.session("me").await;
    let create = |name: &str, parent: Value| {
        let name = name.to_string();
        let me = me.clone();
        let h = &h;
        async move { h.rpc(Some(&me), "folders", "createFolder", json!([name, parent])).await }
    };
    let a = create("A", Value::Null).await;
    assert_eq!(a["ok"]["createdAt"].as_object().map(|o| o.contains_key("$date")), Some(true), "createdAt is a Date");
    let a_id = a["ok"]["id"].as_str().unwrap().to_string();
    let b_id = create("B", json!(a_id)).await["ok"]["id"].as_str().unwrap().to_string();
    let c_id = create("C", Value::Null).await["ok"]["id"].as_str().unwrap().to_string();

    let err = |v: Value| v["error"]["message"].as_str().unwrap_or("").to_string();
    assert_eq!(
        err(h.rpc(Some(&me), "folders", "moveFolder", json!([a_id, b_id])).await),
        "A folder cannot be moved inside itself"
    );
    assert_eq!(
        err(h.rpc(Some(&me), "folders", "moveFolder", json!([a_id, a_id])).await),
        "A folder cannot be moved inside itself"
    );
    assert_eq!(err(h.rpc(Some(&me), "folders", "moveFolder", json!(["nope", null])).await), "Folder not found");
    // Moving within the same parent moves the slug (upstream quirk).
    let out = h.rpc(Some(&me), "folders", "moveFolder", json!([b_id, a_id])).await;
    assert_eq!(out, json!({ "ok": { "$undefined": 1 }, "revalidate": true }));
    let slug: String = sqlx::query_scalar(&format!(r#"select "slug" from "folders" where "id" = '{b_id}'"#))
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(slug, "b-2");
    h.rpc(Some(&me), "folders", "moveFolder", json!([b_id, c_id])).await;
    let parent: Uuid = sqlx::query_scalar(&format!(r#"select "parent_id" from "folders" where "id" = '{b_id}'"#))
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(parent.to_string(), c_id);

    h.upload(Some("me"), "u1", 1, "").await;
    h.upload(Some("me"), "u2", 1, "").await;
    h.upload(Some("me"), "u3", 1, "deleted_at = now()").await;
    assert_eq!(h.rpc(Some(&me), "folders", "countUnfiledUploads", json!([])).await["ok"], 2);
    let moved = h.rpc(Some(&me), "folders", "moveUnfiledToFolder", json!([])).await["ok"].clone();
    assert_eq!(moved["moved"], 2);
    assert_eq!(moved["folderName"], "Wszystko");
    assert_eq!(h.rpc(Some(&me), "folders", "countUnfiledUploads", json!([])).await["ok"], 0);

    // listFolders: every folder, and the count upstream's query really answers (0).
    let list = h.rpc(Some(&me), "folders", "listFolders", json!([])).await["ok"].clone();
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 4);
    assert!(list.iter().all(|f| f["count"] == 0));
    let wszystko = list.iter().find(|f| f["name"] == "Wszystko").unwrap();
    assert_eq!(wszystko["parentId"], Value::Null);
    assert_eq!(wszystko["isPublic"], false);
    h.drop().await;
}

// ── pastes, short links and their unlock cookie ─────────────────────────────

#[tokio::test]
async fn pastes_write_bytes_count_quota_and_stamp_passwords() {
    let Some(h) = Harness::new().await else { return };
    h.user("me", "me", "user").await;
    sqlx::query(r#"update "user" set "max_quota" = 20, "used_quota" = 5 where "id" = 'me'"#)
        .execute(&h.pool)
        .await
        .unwrap();
    let me = h.session("me").await;
    let out = h
        .rpc(Some(&me), "pastes", "createPaste", json!([{ "content": "<b>hi</b>", "language": "HTML", "maxViews": 0 }]))
        .await;
    let ok = out["ok"].clone();
    let code = ok["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 10);
    assert_eq!(ok["url"], format!("http://127.0.0.1:8421/me/{code}.html"));
    assert_eq!(ok["rawUrl"], format!("http://127.0.0.1:8421/me/{code}/raw.html"));
    assert_eq!(
        (ok["expiresAt"].clone(), ok["maxViews"].clone(), ok["hasPassword"].clone()),
        (Value::Null, Value::Null, json!(false))
    );
    assert_eq!(std::fs::read(h.storage.path().join(format!("me/{code}.html"))).unwrap(), b"<b>hi</b>");
    let row: (String, String, String, i64, String) = sqlx::query_as(
        r#"select "filename", "mime_type", "language", "size", "checksum" from "uploads" where "code" = $1"#,
    )
    .bind(&code)
    .fetch_one(&h.pool)
    .await
    .unwrap();
    assert_eq!(row.0, "paste.html");
    assert_eq!(row.1, "text/html");
    assert_eq!(row.2, "html");
    assert_eq!(row.3, 9);
    assert_eq!(row.4, modlight_server::media::derived_cache::content_hash(b"<b>hi</b>"));
    assert_eq!(h.scalar_i64(r#"select "used_quota" from "user" where "id" = 'me'"#).await, 14);

    // The session's own quota numbers decide (read fresh on every call).
    let err = |v: Value| v["error"]["message"].as_str().unwrap_or("").to_string();
    assert_eq!(
        err(h.rpc(Some(&me), "pastes", "createPaste", json!([{ "content": "1234567" }])).await),
        "Quota exceeded"
    );
    assert_eq!(err(h.rpc(Some(&me), "pastes", "createPaste", json!([{ "content": " \n " }])).await), "Paste is empty");

    // With no password typed, the account default is stamped with its generation.
    let stamp = modlight_server::password::hash_password("dflt").await.unwrap();
    sqlx::query(r#"update "user" set "default_password_hash" = $1, "default_password_version" = 4, "max_quota" = -1 where "id" = 'me'"#)
        .bind(&stamp)
        .execute(&h.pool)
        .await
        .unwrap();
    let ok = h.rpc(Some(&me), "pastes", "createPaste", json!([{ "content": "d", "expiresInMinutes": 30 }])).await["ok"]
        .clone();
    assert_eq!(ok["hasPassword"], true);
    let (hash, version, minutes): (Option<String>, Option<i32>, f64) = sqlx::query_as(
        r#"select "password_hash", "password_version", extract(epoch from "expires_at" - now())::float8 / 60 from "uploads" where "code" = $1"#,
    )
    .bind(ok["code"].as_str().unwrap())
    .fetch_one(&h.pool)
    .await
    .unwrap();
    assert_eq!((hash.as_deref(), version), (Some(stamp.as_str()), Some(4)));
    assert!((minutes - 30.0).abs() < 1.0);
    let ok =
        h.rpc(Some(&me), "pastes", "createPaste", json!([{ "content": "o", "password": "mine" }])).await["ok"].clone();
    let version: Option<i32> = sqlx::query_scalar(r#"select "password_version" from "uploads" where "code" = $1"#)
        .bind(ok["code"].as_str().unwrap())
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(version, None, "a typed password stores no generation");
    h.drop().await;
}

#[tokio::test]
async fn short_links_and_the_unlock_cookie() {
    let Some(h) = Harness::new().await else { return };
    h.user("me", "me", "user").await;
    let me = h.session("me").await;
    let out = h
        .rpc(
            Some(&me),
            "shortlinks",
            "createShortLink",
            json!([{ "target": "example.com", "customCode": "Gate", "password": "  pw  " }]),
        )
        .await;
    assert_eq!(
        out,
        json!({ "ok": { "ok": true, "code": "me/gate", "url": "http://127.0.0.1:8421/s/me/gate" }, "revalidate": true })
    );
    assert_eq!(
        h.rpc(Some(&me), "shortlinks", "createShortLink", json!([{ "target": "javascript:alert(1)" }])).await["ok"],
        json!({ "ok": false, "reason": "invalid-url" })
    );
    let listed = h.rpc(Some(&me), "shortlinks", "listShortLinks", json!([])).await["ok"].clone();
    assert!(listed[0]["createdAt"]["$date"].is_string(), "dates travel as Dates");
    let id = listed[0]["id"].as_str().unwrap().to_string();

    // Created trimmed, verified untrimmed (upstream quirk kept).
    let verify = |pw: &str| {
        let pw = pw.to_string();
        let h = &h;
        async move { h.rpc_full(None, "shortlink-unlock", "verifyShortlinkPassword", json!(["me/gate", pw])).await }
    };
    assert_eq!(verify("  pw  ").await.0, json!({ "ok": { "ok": false } }));
    let (out, cookies) = verify("pw").await;
    assert_eq!(out, json!({ "ok": { "ok": true }, "revalidate": true }), "a cookie set is a revalidation");
    assert_eq!(cookies.len(), 1);
    let cookie = &cookies[0];
    assert!(cookie.starts_with(&format!("ml_sl_{id}=")), "{cookie}");
    assert!(
        cookie.contains("; Path=/") && cookie.contains("; Max-Age=3600") && cookie.contains("; HttpOnly"),
        "{cookie}"
    );
    assert!(cookie.ends_with("; SameSite=lax"), "{cookie}");
    let token = cookie.split(';').next().unwrap().split_once('=').unwrap().1;
    assert!(modlight_server::password::verify_unlock_token(
        SECRET,
        Some(token),
        modlight_server::password::UnlockScope::Shortlink,
        &id
    ));

    // Retarget keeps the code; another account cannot reach it.
    let out = h.rpc(Some(&me), "shortlinks", "retargetShortLink", json!([id, "example.org/new"])).await;
    assert_eq!(out["ok"], json!({ "ok": true, "target": "https://example.org/new" }));
    h.user("them", "them", "user").await;
    let them = h.session("them").await;
    assert_eq!(
        h.rpc(Some(&them), "shortlinks", "retargetShortLink", json!([id, "example.net"])).await["ok"],
        json!({ "ok": false, "reason": "not-found" })
    );
    h.rpc(Some(&them), "shortlinks", "deleteShortLink", json!([id])).await;
    assert_eq!(h.scalar_i64(r#"select count(*) from "short_links""#).await, 1);
    h.rpc(Some(&me), "shortlinks", "deleteShortLink", json!([id])).await;
    assert_eq!(h.scalar_i64(r#"select count(*) from "short_links""#).await, 0);
    h.drop().await;
}
