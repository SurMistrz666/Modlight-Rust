//! Races in whole flows (hunt 2026-11-17, round 2b) against a real
//! PostgreSQL: invitations, the folder tree, the quota recalculation. Runs
//! only when `MODLIGHT_TEST_DATABASE_URL` names a server where the user may
//! create databases (see tests/db_compat.rs); each test gets its own
//! throwaway, migrated database.

use std::str::FromStr;
use std::time::Duration;

use modlight_server::paths::Paths;
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
        let name = format!("modlight_flows_{}_{}", std::process::id(), rand::random::<u32>());
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

async fn user(db: &PgPool, id: &str, used: i64) {
    sqlx::query(
        r#"insert into "user" ("id", "name", "email", "email_verified", "created_at", "updated_at", "user_code", "role", "used_quota")
           values ($1, $1, $1 || '@example.test', false, now(), now(), $1, 'user', $2)"#,
    )
    .bind(id)
    .bind(used)
    .execute(db)
    .await
    .unwrap();
}

/// Wait until some other connection of this database is blocked on a lock:
/// the operation under test has reached the point where it waits for the
/// transaction the test holds.
async fn until_someone_waits_on_a_lock(db: &PgPool) {
    for _ in 0..500 {
        let waiting: i64 = sqlx::query_scalar(
            r#"select count(*) from pg_stat_activity
               where datname = current_database() and wait_event_type = 'Lock' and pid <> pg_backend_pid()"#,
        )
        .fetch_one(db)
        .await
        .unwrap();
        if waiting > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("nothing ever waited on a lock");
}

// ---------------------------------------------------------------------------
// FLOW-N4: an invitation is accepted once
// ---------------------------------------------------------------------------

/// Two acceptances of one invitation link at the same moment (a double click,
/// or the link in two hands): both read it before either revoked it, both
/// set "their" password and both succeeded. One wins now.
#[tokio::test]
async fn an_invitation_is_accepted_once() {
    use modlight_server::invites::{AcceptInviteResult, accept_invite, create_invite};
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "invited", 0).await;
    let token = create_invite(db, "invited").await.unwrap();
    let (a, b) =
        tokio::join!(accept_invite(db, &token, "first password"), accept_invite(db, &token, "second password"));
    let wins = [a.unwrap(), b.unwrap()].iter().filter(|r| matches!(r, AcceptInviteResult::Ok { .. })).count();
    assert_eq!(wins, 1, "both acceptances succeeded");
    // And the link is spent.
    let again = accept_invite(db, &token, "third password").await.unwrap();
    assert_eq!(again, AcceptInviteResult::Invalid);
    t.drop().await;
}

// ---------------------------------------------------------------------------
// FLOW-N2: the folder tree changes under one lock
// ---------------------------------------------------------------------------

/// Another change of the tree in progress, as the fixed code makes one: it
/// holds the owner's folder rows and has written, not yet committed.
async fn tree_change_in_progress(db: &PgPool, owner: &str) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = db.begin().await.unwrap();
    sqlx::query(r#"select "id" from "folders" where "owner_id" = $1 order by "id" for update"#)
        .bind(owner)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx
}

/// "Delete with contents" while a subfolder (with a file in it) was being
/// created inside: the subtree was read before the new folder existed, so it
/// survived with a parent that no longer exists, and its file stayed live.
#[tokio::test]
async fn a_folder_created_inside_a_subtree_being_deleted_goes_with_it() {
    use modlight_server::folders::{create_folder_for, delete_folder_for};
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "u1", 0).await;
    let x = create_folder_for(db, "u1", Some(&serde_json::json!("X")), None).await.unwrap();

    let mut other = tree_change_in_progress(db, "u1").await;
    let child: uuid::Uuid = sqlx::query_scalar(
        r#"insert into "folders" ("owner_id", "parent_id", "name", "slug") values ('u1', $1, 'C', 'c') returning "id""#,
    )
    .bind(x.id)
    .fetch_one(&mut *other)
    .await
    .unwrap();
    let file: uuid::Uuid = sqlx::query_scalar(
        r#"insert into "uploads" ("owner_id", "code", "filename", "ext", "mime_type", "size", "storage_key", "delete_token", "folder_id")
           values ('u1', 'cfile01', 'c.png', 'png', 'image/png', 10, 'u1/cfile01.png', 'tok', $1) returning "id""#,
    )
    .bind(child)
    .fetch_one(&mut *other)
    .await
    .unwrap();

    let (pool, id) = (db.clone(), x.id.to_string());
    let deleting =
        tokio::spawn(async move { delete_folder_for(&pool, None, "u1", &id, true).await.map(|r| r.trashed) });
    until_someone_waits_on_a_lock(db).await;
    other.commit().await.unwrap();
    assert_eq!(deleting.await.unwrap().unwrap(), 1, "the file in the new subfolder was not trashed");

    let left: i64 = sqlx::query_scalar(r#"select count(*) from "folders""#).fetch_one(db).await.unwrap();
    assert_eq!(left, 0, "a folder survived its deleted parent");
    let trashed: bool = sqlx::query_scalar(r#"select "deleted_at" is not null from "uploads" where "id" = $1"#)
        .bind(file)
        .fetch_one(db)
        .await
        .unwrap();
    assert!(trashed);
    t.drop().await;
}

/// A into B and B into A at the same moment: each checked the tree before the
/// other's move landed, both passed, and the two folders formed a cycle (and
/// dropped out of the tree). The second move now sees the first and refuses.
#[tokio::test]
async fn two_moves_at_once_cannot_make_a_cycle() {
    use modlight_server::folders::{create_folder_for, move_folder_for};
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "u1", 0).await;
    let a = create_folder_for(db, "u1", Some(&serde_json::json!("A")), None).await.unwrap();
    let b = create_folder_for(db, "u1", Some(&serde_json::json!("B")), None).await.unwrap();

    let mut other = tree_change_in_progress(db, "u1").await;
    sqlx::query(r#"update "folders" set "parent_id" = $1 where "id" = $2"#)
        .bind(b.id)
        .bind(a.id)
        .execute(&mut *other)
        .await
        .unwrap();

    let (pool, b_id, a_id) = (db.clone(), b.id.to_string(), a.id.to_string());
    let moving = tokio::spawn(async move { move_folder_for(&pool, "u1", &b_id, Some(&a_id)).await.map(|_| ()) });
    until_someone_waits_on_a_lock(db).await;
    other.commit().await.unwrap();
    let err = moving.await.unwrap().expect_err("the second move made a cycle");
    assert!(format!("{err:?}").contains("Cycle"), "{err:?}");
    let b_parent: Option<uuid::Uuid> = sqlx::query_scalar(r#"select "parent_id" from "folders" where "id" = $1"#)
        .bind(b.id)
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(b_parent, None);
    t.drop().await;
}

// ---------------------------------------------------------------------------
// FLOW-N3: the quota recalculation
// ---------------------------------------------------------------------------

async fn used_quota(db: &PgPool, id: &str) -> i64 {
    sqlx::query_scalar(r#"select "used_quota" from "user" where "id" = $1"#).bind(id).fetch_one(db).await.unwrap()
}

async fn file(db: &PgPool, owner: &str, code: &str, size: i64) -> uuid::Uuid {
    sqlx::query_scalar(
        r#"insert into "uploads" ("owner_id", "code", "filename", "ext", "mime_type", "size", "storage_key", "delete_token")
           values ($1, $2, $2 || '.png', 'png', 'image/png', $3, $1 || '/' || $2 || '.png', 'tok-' || $2) returning "id""#,
    )
    .bind(owner)
    .bind(code)
    .bind(size)
    .fetch_one(db)
    .await
    .unwrap()
}

/// A permanent delete (row and refund in one transaction) committed while the
/// recalculation ran: it had summed the rows before and wrote its total after,
/// so the refund was overwritten and the account counted a file it no longer
/// had.
#[tokio::test]
async fn a_refund_made_while_the_quota_is_recounted_is_kept() {
    use modlight_server::data::admin::recalculate_all_quotas;
    let t = db_or_skip!();
    let db = &t.pool;
    user(db, "recount", 999).await; // drifted: the rows hold 300
    let small = file(db, "recount", "small01", 100).await;
    file(db, "recount", "large01", 200).await;

    let mut deleting = db.begin().await.unwrap();
    sqlx::query(r#"delete from "uploads" where "id" = $1"#).bind(small).execute(&mut *deleting).await.unwrap();
    sqlx::query(r#"update "user" set "used_quota" = "used_quota" - 100 where "id" = 'recount'"#)
        .execute(&mut *deleting)
        .await
        .unwrap();

    let pool = db.clone();
    let recount = tokio::spawn(async move { recalculate_all_quotas(&pool).await.map(|r| r.corrected) });
    until_someone_waits_on_a_lock(db).await;
    deleting.commit().await.unwrap();
    recount.await.unwrap().unwrap();
    assert_eq!(used_quota(db, "recount").await, 200, "the refund was overwritten");
    t.drop().await;
}

/// An upload in progress has reserved its bytes, and has no row yet: the
/// recalculation keeps the reservation (it counted the rows alone, so the
/// account went over its quota once the file landed).
#[tokio::test]
async fn a_recount_keeps_what_uploads_in_progress_reserved() {
    use modlight_server::data::admin::recalculate_all_quotas;
    use modlight_server::upload::finalize::{release_quota, reserve_quota};
    let t = db_or_skip!();
    let db = &t.pool;
    // An id no other test of this binary uses: reservations are per process.
    user(db, "in-flight-owner", 0).await;
    file(db, "in-flight-owner", "landed1", 300).await;
    sqlx::query(r#"update "user" set "used_quota" = 300 where "id" = 'in-flight-owner'"#).execute(db).await.unwrap();
    reserve_quota(db, "in-flight-owner", 500).await.unwrap();
    assert_eq!(used_quota(db, "in-flight-owner").await, 800);
    let result = recalculate_all_quotas(db).await.unwrap();
    assert_eq!(result.corrected, 0, "{:?}", result.changes);
    assert_eq!(used_quota(db, "in-flight-owner").await, 800, "the reservation was dropped");
    release_quota(db, "in-flight-owner", 500).await.unwrap();
    recalculate_all_quotas(db).await.unwrap();
    assert_eq!(used_quota(db, "in-flight-owner").await, 300);
    t.drop().await;
}
