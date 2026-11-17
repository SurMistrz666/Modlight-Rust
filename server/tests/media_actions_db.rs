//! `media::actions` against a real PostgreSQL: the permanent deletes and the
//! quota they give back. Runs only when `MODLIGHT_TEST_DATABASE_URL` names a
//! server where the user may create databases (see tests/db_compat.rs); each
//! test gets its own throwaway, migrated database.

use std::str::FromStr;

use modlight_server::media::actions::{DeletableMedia, delete_media_bulk, delete_media_hard};
use modlight_server::paths::Paths;
use modlight_server::storage::LocalDriver;
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
        let name = format!("modlight_mact_{}_{}", std::process::id(), rand::random::<u32>());
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

async fn user_with_quota(db: &PgPool, id: &str, used: i64) {
    sqlx::query(
        r#"insert into "user" ("id", "name", "email", "email_verified", "created_at", "updated_at", "user_code", "role", "used_quota")
           values ($1, $1, $1 || '@example.test', true, now(), now(), $1, 'user', $2)"#,
    )
    .bind(id)
    .bind(used)
    .execute(db)
    .await
    .unwrap();
}

async fn trashed_upload(db: &PgPool, owner: &str, code: &str, size: i64) -> DeletableMedia {
    let id: Uuid = sqlx::query_scalar(
        r#"insert into "uploads" ("owner_id", "code", "filename", "ext", "mime_type", "size", "storage_key", "delete_token", "deleted_at")
           values ($1, $2, $2 || '.png', 'png', 'image/png', $3, $1 || '/' || $2 || '.png', 'tok-' || $2, now()) returning "id""#,
    )
    .bind(owner)
    .bind(code)
    .bind(size)
    .fetch_one(db)
    .await
    .unwrap();
    DeletableMedia {
        id,
        owner_id: Some(owner.to_string()),
        size,
        storage_key: format!("{owner}/{code}.png"),
        thumb_key: None,
    }
}

async fn used_quota(db: &PgPool, id: &str) -> i64 {
    sqlx::query_scalar(r#"select "used_quota" from "user" where "id" = $1"#).bind(id).fetch_one(db).await.unwrap()
}

/// Two deletes of the same files, both having read the rows before either
/// removed them (a double-clicked "Empty trash", two API calls at once): the
/// quota goes down by what was freed, once. It went down twice, so every round
/// of upload, trash and a doubled empty gave the account free room past its
/// limit.
#[tokio::test]
async fn a_file_deleted_twice_is_refunded_once() {
    let t = db_or_skip!();
    let db = &t.pool;
    let dir = tempfile::tempdir().unwrap();
    let storage = LocalDriver::new(dir.path().to_str().unwrap());
    user_with_quota(db, "u1", 1000).await;

    let one = trashed_upload(db, "u1", "aaaaaaa", 300).await;
    delete_media_hard(db, &storage, &one).await.unwrap();
    delete_media_hard(db, &storage, &one).await.unwrap();
    assert_eq!(used_quota(db, "u1").await, 700);

    let rows = vec![trashed_upload(db, "u1", "bbbbbbb", 100).await, trashed_upload(db, "u1", "ccccccc", 200).await];
    let first = delete_media_bulk(db, &storage, &rows).await.unwrap();
    let second = delete_media_bulk(db, &storage, &rows).await.unwrap();
    assert_eq!(first.failed + second.failed, 0);
    assert_eq!(used_quota(db, "u1").await, 400);
    let left: i64 = sqlx::query_scalar(r#"select count(*) from "uploads""#).fetch_one(db).await.unwrap();
    assert_eq!(left, 0);
    t.drop().await;
}

fn put_object(storage: &LocalDriver, key: &str, body: &[u8]) {
    let path = storage.root().join(key);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// Point a row at a new object, as the file-vault pass (`switch_row`) or an
/// in-place conversion does between a delete's read and its `DELETE`.
async fn move_object(db: &PgPool, storage: &LocalDriver, media: &DeletableMedia, new_key: &str) {
    put_object(storage, new_key, b"the new bytes");
    sqlx::query(r#"update "uploads" set "storage_key" = $1 where "id" = $2"#)
        .bind(new_key)
        .bind(media.id)
        .execute(db)
        .await
        .unwrap();
}

/// A permanent delete that read the row just before the vault pass (or a
/// conversion) moved it to a new object: the row and the old object went, the
/// new object stayed in storage with nothing pointing at it (after a pass that
/// decrypts, the plaintext of a file deleted "forever"). Both shapes of the
/// delete now also remove the object the removed row pointed at.
#[tokio::test]
async fn a_delete_that_raced_a_rewrite_leaves_no_object_behind() {
    use modlight_server::storage::StorageDriver;
    let t = db_or_skip!();
    let db = &t.pool;
    let dir = tempfile::tempdir().unwrap();
    let storage = LocalDriver::new(dir.path().to_str().unwrap());
    user_with_quota(db, "u1", 1000).await;

    let one = trashed_upload(db, "u1", "ddddddd", 100).await;
    put_object(&storage, &one.storage_key, b"old");
    move_object(db, &storage, &one, "u1/ddddddd.0a1b2c3d4e5f.png").await;
    delete_media_hard(db, &storage, &one).await.unwrap();
    assert!(storage.stat(&one.storage_key).await.unwrap().is_none());
    assert!(storage.stat("u1/ddddddd.0a1b2c3d4e5f.png").await.unwrap().is_none(), "the new object was left");
    assert_eq!(used_quota(db, "u1").await, 900);

    let rows = vec![trashed_upload(db, "u1", "eeeeeee", 100).await, trashed_upload(db, "u1", "fffffff", 200).await];
    move_object(db, &storage, &rows[1], "u1/fffffff.0a1b2c3d4e5f.png").await;
    // A key another row still points at is never touched.
    let keeper = trashed_upload(db, "u1", "ggggggg", 50).await;
    move_object(db, &storage, &rows[0], &keeper.storage_key).await;
    let result = delete_media_bulk(db, &storage, &rows).await.unwrap();
    assert_eq!((result.deleted, result.failed), (2, 0));
    assert!(storage.stat("u1/fffffff.0a1b2c3d4e5f.png").await.unwrap().is_none(), "the new object was left");
    assert!(storage.stat(&keeper.storage_key).await.unwrap().is_some(), "a live object was deleted");
    assert_eq!(used_quota(db, "u1").await, 600);
    t.drop().await;
}
