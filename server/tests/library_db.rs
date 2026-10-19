//! The library port against a real PostgreSQL: folders, listings, upload
//! operations, duplicates, the trash. Runs only when
//! `MODLIGHT_TEST_DATABASE_URL` names a server where the user may create
//! databases (see tests/db_compat.rs); each test gets its own throwaway,
//! migrated database.
//!
//! These replace upstream's mock-based unit tests of the same modules
//! (folders-core, duplicates, empty-trash, uploads-api-*): the statements are
//! the thing worth testing, so they run against the real schema.

use std::str::FromStr;
use std::sync::Arc;

use modlight_server::db::models::JsDate;
use modlight_server::paths::Paths;
use modlight_server::storage::{LocalDriver, StorageDriver};
use serde_json::json;
use sqlx::postgres::PgConnectOptions;
use sqlx::{ConnectOptions, PgPool};
use uuid::Uuid;

struct TempDb {
    admin: PgConnectOptions,
    name: String,
    pool: PgPool,
}

impl TempDb {
    async fn create() -> Option<TempDb> {
        let url = std::env::var("MODLIGHT_TEST_DATABASE_URL").ok()?;
        let admin = PgConnectOptions::from_str(&url).expect("MODLIGHT_TEST_DATABASE_URL");
        let name = format!("modlight_lib_{}_{}", std::process::id(), rand::random::<u32>());
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

async fn user(db: &PgPool, id: &str, tag: &str) {
    sqlx::query(
        r#"insert into "user" ("id", "name", "email", "email_verified", "created_at", "updated_at", "user_code", "role")
           values ($1, $1, $1 || '@example.test', true, now(), now(), $2, 'user')"#,
    )
    .bind(id)
    .bind(tag)
    .execute(db)
    .await
    .unwrap();
}

async fn upload(db: &PgPool, owner: &str, code: &str, size: i64, checksum: Option<&str>, folder: Option<Uuid>) -> Uuid {
    sqlx::query_scalar(
        r#"insert into "uploads" ("owner_id", "code", "filename", "ext", "mime_type", "size", "storage_key", "checksum", "delete_token", "folder_id")
           values ($1, $2, $2 || '.png', 'png', 'image/png', $3, $1 || '/' || $2 || '.png', $4, 'tok-' || $2, $5) returning "id""#,
    )
    .bind(owner)
    .bind(code)
    .bind(size)
    .bind(checksum)
    .bind(folder)
    .fetch_one(db)
    .await
    .unwrap()
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

#[tokio::test]
async fn folders_core() {
    use modlight_server::folders::*;
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "u1", "alice").await;
    user(db, "u2", "bob").await;

    let docs = create_folder_for(db, "u1", Some(&json!("  Docs  ")), None).await.unwrap();
    assert_eq!((docs.name.as_str(), docs.slug.as_str()), ("Docs", "docs"));
    let docs2 = create_folder_for(db, "u1", Some(&json!("docs")), None).await.unwrap();
    assert_eq!(docs2.slug, "docs-2");
    let child = create_folder_for(db, "u1", Some(&json!("Źródła")), Some(&docs.id.to_string())).await.unwrap();
    assert_eq!(child.slug, "zrodla");
    // Another account's folder is "not found", as is garbage.
    for bad in [docs.id.to_string(), "nope".to_string()] {
        let err = create_folder_for(db, "u2", Some(&json!("x")), Some(&bad)).await.unwrap_err();
        assert!(matches!(err, FolderError::Op(FolderOpError { reason: FolderIssue::NotFound, .. })));
    }
    // An empty-string parent is not normalised: Postgres refuses it.
    assert!(matches!(create_folder_for(db, "u1", None, Some("")).await, Err(FolderError::Other(_))));
    // Renaming to the same name moves the slug, as upstream.
    let renamed = rename_folder_for(db, "u1", &docs.id.to_string(), Some(&json!("Docs"))).await.unwrap();
    assert_eq!(renamed.slug, "docs-3");
    assert!(matches!(
        rename_folder_for(db, "u1", &docs.id.to_string(), Some(&json!(" "))).await,
        Err(FolderError::Op(FolderOpError { reason: FolderIssue::InvalidName, .. }))
    ));
    // Cycles.
    let err = move_folder_for(db, "u1", &docs.id.to_string(), Some(&child.id.to_string())).await.unwrap_err();
    assert!(matches!(err, FolderError::Op(FolderOpError { reason: FolderIssue::Cycle, .. })));
    // Depth ceiling: a chain of 8 refuses a 9th.
    let mut parent = docs2.id.to_string();
    for i in 0..7 {
        parent =
            create_folder_for(db, "u1", Some(&json!(format!("d{i}"))), Some(&parent)).await.unwrap().id.to_string();
    }
    let err = create_folder_for(db, "u1", Some(&json!("deep")), Some(&parent)).await.unwrap_err();
    assert_eq!(err.to_string(), "Folders can be nested at most 8 levels deep");

    // Files: move in, list, delete folder only, delete with contents.
    let a = upload(db, "u1", "aaaa", 10, None, None).await;
    let b = upload(db, "u1", "bbbb", 20, None, None).await;
    let theirs = upload(db, "u2", "cccc", 30, None, None).await;
    let ids = vec![json!(a.to_string()), json!(theirs.to_string()), json!("junk")];
    assert_eq!(move_uploads_to_folder_for(db, "u1", &ids, Some(&child.id.to_string())).await.unwrap(), 1);
    assert_eq!(count_unfiled_uploads_for(db, "u1").await.unwrap(), 1);
    let tree = modlight_server::data::folders::folder_tree(db, "u1").await.unwrap();
    let root = tree.iter().find(|n| n.id == docs.id).unwrap();
    assert_eq!((root.count, root.total_count, root.total_size), (0, 1, 10));
    let crumbs = modlight_server::data::folders::folder_path(db, "u1", &child.id.to_string()).await.unwrap();
    assert_eq!(crumbs.iter().map(|c| c.slug.as_str()).collect::<Vec<_>>(), vec!["docs-3", "zrodla"]);

    let moved = move_unfiled_to_folder_for(db, "u1", None).await.unwrap();
    assert_eq!((moved.moved, moved.folder_name.as_str()), (1, "Wszystko"));
    let again = move_unfiled_to_folder_for(db, "u1", Some(&json!("wszystko"))).await.unwrap();
    assert_eq!((again.moved, again.folder_id, again.folder_name.as_str()), (0, moved.folder_id, "Wszystko"));

    let plain = delete_folder_for(db, None, "u1", &moved.folder_id.to_string(), false).await.unwrap();
    assert_eq!((plain.folders, plain.trashed), (1, 0));
    let folder_of_b: Option<Uuid> =
        sqlx::query_scalar(r#"select "folder_id" from "uploads" where "id" = $1"#).bind(b).fetch_one(db).await.unwrap();
    assert_eq!(folder_of_b, None);
    let deep = delete_folder_for(db, None, "u1", &docs.id.to_string(), true).await.unwrap();
    assert_eq!((deep.folders, deep.trashed), (2, 1));
    let trashed: Option<JsDate> = sqlx::query_scalar(r#"select "deleted_at" from "uploads" where "id" = $1"#)
        .bind(a)
        .fetch_one(db)
        .await
        .unwrap();
    assert!(trashed.is_some());
    t.drop().await;
}

#[tokio::test]
async fn listing_and_operations() {
    use modlight_server::data::uploads::*;
    use modlight_server::uploads::*;
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "u1", "alice").await;
    user(db, "u2", "bob").await;
    let a = upload(db, "u1", "alpha", 100, None, None).await;
    let _b = upload(db, "u1", "beta", 300, None, None).await;
    let theirs = upload(db, "u2", "gamma", 5, None, None).await;

    let page =
        list_uploads(db, &ListParams { user_id: "u1".into(), sort: Some("largest".into()), ..Default::default() })
            .await
            .unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.items[0].code, "beta");
    let found =
        list_uploads(db, &ListParams { user_id: "u1".into(), search: Some("ALP".into()), ..Default::default() })
            .await
            .unwrap();
    assert_eq!(found.total, 1);
    let images = list_uploads(
        db,
        &ListParams { user_id: "u1".into(), kind: Some("image".into()), min_size: Some(200.0), ..Default::default() },
    )
    .await
    .unwrap();
    assert_eq!(images.total, 1);
    // A non-uuid folder id is a Postgres error, as upstream.
    assert!(
        list_uploads(
            db,
            &ListParams { user_id: "u1".into(), folder: FolderFilter::Folder("x".into()), ..Default::default() }
        )
        .await
        .is_err()
    );
    assert_eq!(kind_counts(db, "u1").await.unwrap()["image"], 2);
    let stats = user_stats(db, "u1").await.unwrap();
    assert_eq!((stats.count, stats.total_size), (2, 400));

    let me =
        UploadActor { user_id: "u1".into(), email: Some("u1@example.test".into()), is_admin: false, ip: "test".into() };
    let admin = UploadActor { user_id: "u1".into(), is_admin: true, ..me.clone() };
    // Somebody else's id is "Not found" for a user and reachable for an admin.
    assert!(get_actionable_upload(db, &me, Some(&json!(theirs.to_string()))).await.is_err());
    let rows = set_published_for(db, &admin, &[json!(theirs.to_string())], false).await.unwrap();
    assert!(!rows[0].published);
    let audited: i64 =
        sqlx::query_scalar(r#"select count(*) from "audit_log" where "action" = 'admin.upload.published'"#)
            .fetch_one(db)
            .await
            .unwrap();
    assert_eq!(audited, 1);

    let v = set_vanity_for(db, &me, Some(&json!(a.to_string())), Some(&json!("My Shot!"))).await.unwrap();
    assert_eq!(v.code, "my-shot");
    let err = set_vanity_for(db, &me, Some(&json!(a.to_string())), Some(&json!("login"))).await.unwrap_err();
    assert_eq!(err.to_string(), "\"login\" is reserved by the app");
    let err = set_vanity_for(db, &me, Some(&json!(a.to_string())), Some(&json!("beta"))).await.unwrap_err();
    assert_eq!(err.to_string(), "That URL is already taken");

    let rows = set_max_views_for(db, &me, &[json!(a.to_string())], Some(&json!(0))).await.unwrap();
    assert_eq!(rows[0].max_views, Some(1));
    assert!(set_max_views_for(db, &me, &[json!(a.to_string())], Some(&json!(1e20))).await.is_err());
    let rows = set_password_for(db, &me, &[json!(a.to_string())], Some(&json!("hunter2"))).await.unwrap();
    assert!(rows[0].password_hash.as_deref().unwrap().starts_with("scrypt$"));
    let rows =
        set_expiry_for(db, &me, &[json!(a.to_string())], Some(&json!("2030-01-01T00:00:00.000Z"))).await.unwrap();
    assert_eq!(rows[0].expires_at.unwrap().to_iso_string(), "2030-01-01T00:00:00.000Z");

    // Trash: totals and the empty, with the bytes and the quota.
    let dir = tempfile::tempdir().unwrap();
    let storage: Arc<dyn StorageDriver> = Arc::new(LocalDriver::new(dir.path().to_str().unwrap()));
    sqlx::query(r#"update "uploads" set "deleted_at" = now() where "owner_id" = 'u1'"#).execute(db).await.unwrap();
    sqlx::query(r#"update "user" set "used_quota" = 1000 where "id" = 'u1'"#).execute(db).await.unwrap();
    let totals = trash_totals_for(db, &me, "alice").await.unwrap();
    assert_eq!((totals.files, totals.bytes), (2, 400));
    assert!(trash_totals_for(db, &me, "bob").await.is_err());
    let emptied = empty_trash_for(db, storage.as_ref(), &me, "alice").await.unwrap();
    assert_eq!((emptied.files, emptied.bytes, emptied.failed), (2, 400, 0));
    let used: i64 =
        sqlx::query_scalar(r#"select "used_quota" from "user" where "id" = 'u1'"#).fetch_one(db).await.unwrap();
    assert_eq!(used, 600);
    t.drop().await;
}

#[tokio::test]
async fn duplicates_scan_and_dismissals() {
    use modlight_server::data::duplicates::*;
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "u1", "alice").await;
    let sum = "a".repeat(64);
    let other = "b".repeat(64);
    upload(db, "u1", "one", 10, Some(&sum), None).await;
    upload(db, "u1", "two", 10, Some(&sum), None).await;
    upload(db, "u1", "three", 7, Some(&other), None).await;
    upload(db, "u1", "four", 7, Some(&other), None).await;
    upload(db, "u1", "five", 7, None, None).await;
    let config = modlight_server::Config::from_env_map(&Default::default()).unwrap();
    let scan = scan_duplicates(db, &config, "u1").await.unwrap();
    assert_eq!((scan.total_groups, scan.total_files, scan.reclaimable, scan.unchecksummed), (2, 4, 17, 1));
    assert_eq!(scan.groups[0].checksum, sum);
    assert_eq!(scan.groups[0].files[0].url, "http://localhost:8420/alice/one.png");
    let kept = dismiss_checksums(db, "u1", &[DismissalInput { checksum: sum.clone(), copies: 2.0, bytes: 20.0 }])
        .await
        .unwrap();
    assert_eq!(kept, 1);
    let scan = scan_duplicates(db, &config, "u1").await.unwrap();
    assert_eq!((scan.active.groups, scan.dismissed_groups), (1, 1));
    assert!(scan.groups.last().unwrap().dismissed);
    let (totals, due) = duplicate_reminder(db, "u1").await.unwrap();
    assert_eq!((totals.groups, totals.redundant, due), (1, 1, false));
    assert_eq!(restore_checksums(db, "u1", std::slice::from_ref(&sum)).await.unwrap(), 1);
    t.drop().await;
}
