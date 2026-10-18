//! Port of `migrate.ts`: drizzle's migrator plus the settings seed.
//!
//! Reproduces drizzle-orm 0.45.2 (`migrator.js` + `pg-core/dialect.js`)
//! exactly, so a database migrated by either implementation is accepted by the
//! other (PORT.md section 10):
//!
//! - migrations are read from `meta/_journal.json`, one `<tag>.sql` per entry;
//! - each file is split on `--> statement-breakpoint`;
//! - its hash is the SHA-256 hex of the file text, its `created_at` the
//!   journal's `when`;
//! - bookkeeping lives in `"drizzle"."__drizzle_migrations"` (`id serial`,
//!   `hash text`, `created_at bigint`), created if missing;
//! - a migration runs when there is no row yet or the newest row's
//!   `created_at` is lower than its `when`; all pending migrations run in ONE
//!   transaction, each statement as a simple query.
//!
//! Hashes are never compared, exactly like drizzle: only timestamps decide.

use std::path::Path;

use anyhow::{Context, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

#[derive(Debug, Deserialize)]
struct Journal {
    entries: Vec<JournalEntry>,
}

#[derive(Debug, Deserialize)]
struct JournalEntry {
    when: i64,
    tag: String,
    breakpoints: bool,
}

/// One migration as drizzle reads it.
#[derive(Debug, Clone)]
pub struct MigrationFile {
    pub tag: String,
    /// The statements, split on `--> statement-breakpoint` (pieces kept as-is).
    pub statements: Vec<String>,
    pub breakpoints: bool,
    /// Journal `when` (ms since epoch); stored as `created_at`.
    pub folder_millis: i64,
    /// SHA-256 hex of the whole file.
    pub hash: String,
}

/// `readMigrationFiles({ migrationsFolder })`.
pub fn read_migration_files(folder: &Path) -> anyhow::Result<Vec<MigrationFile>> {
    let journal_path = folder.join("meta").join("_journal.json");
    if !journal_path.is_file() {
        bail!("Can't find meta/_journal.json file");
    }
    let journal_text = std::fs::read_to_string(&journal_path).context("reading meta/_journal.json")?;
    let journal: Journal = serde_json::from_str(&journal_text).context("parsing meta/_journal.json")?;
    let mut out = Vec::with_capacity(journal.entries.len());
    for entry in journal.entries {
        let path = folder.join(format!("{}.sql", entry.tag));
        let query = std::fs::read_to_string(&path)
            .map_err(|_| anyhow::anyhow!("No file {} found in {} folder", path.display(), folder.display()))?;
        let statements = query.split("--> statement-breakpoint").map(str::to_string).collect();
        let hash = hex::encode(Sha256::digest(query.as_bytes()));
        out.push(MigrationFile {
            tag: entry.tag,
            statements,
            breakpoints: entry.breakpoints,
            folder_millis: entry.when,
            hash,
        });
    }
    Ok(out)
}

/// What a run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrateReport {
    /// Tags applied by this run, in order. Empty when the database was current.
    pub applied: Vec<String>,
}

/// `migrate(db, { migrationsFolder })` for node-postgres.
pub async fn migrate(pool: &PgPool, migrations: &[MigrationFile]) -> anyhow::Result<MigrateReport> {
    let mut conn = pool.acquire().await?;
    sqlx::raw_sql("CREATE SCHEMA IF NOT EXISTS \"drizzle\"").execute(&mut *conn).await?;
    sqlx::raw_sql(
        "\n\t\t\tCREATE TABLE IF NOT EXISTS \"drizzle\".\"__drizzle_migrations\" (\n\t\t\t\tid SERIAL PRIMARY KEY,\n\t\t\t\thash text NOT NULL,\n\t\t\t\tcreated_at bigint\n\t\t\t)\n\t\t",
    )
    .execute(&mut *conn)
    .await?;
    let last: Option<(i32, String, Option<i64>)> = sqlx::query_as(
        "select id, hash, created_at from \"drizzle\".\"__drizzle_migrations\" order by created_at desc limit 1",
    )
    .fetch_optional(&mut *conn)
    .await?;
    // `Number(lastDbMigration.created_at)`: NULL is 0.
    let last_millis = last.as_ref().map(|(_, _, created)| created.unwrap_or(0));

    let mut report = MigrateReport::default();
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    for migration in migrations {
        let pending = match last_millis {
            None => true,
            Some(last) => last < migration.folder_millis,
        };
        if !pending {
            continue;
        }
        for stmt in &migration.statements {
            sqlx::raw_sql(stmt)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("migration {} failed", migration.tag))?;
        }
        sqlx::query("insert into \"drizzle\".\"__drizzle_migrations\" (\"hash\", \"created_at\") values($1, $2)")
            .bind(&migration.hash)
            .bind(migration.folder_millis)
            .execute(&mut *tx)
            .await?;
        report.applied.push(migration.tag.clone());
    }
    tx.commit().await?;
    Ok(report)
}

/// Seed the settings an environment variable used to own.
///
/// `ON CONFLICT DO NOTHING` is what turns the variable into a STARTING VALUE
/// rather than a competing source of truth: from the first boot onwards the
/// row exists, the admin panel owns it, and changing the variable does nothing.
pub async fn seed(pool: &PgPool, registration_enabled: bool) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO settings (key, value) VALUES ('register_enabled', $1)\n   ON CONFLICT (key) DO NOTHING")
        .bind(if registration_enabled { "on" } else { "off" })
        .execute(pool)
        .await?;
    Ok(())
}

/// The whole `migrate.ts`: migrations, then the seed.
pub async fn run(pool: &PgPool, migrations_dir: &Path, registration_enabled: bool) -> anyhow::Result<MigrateReport> {
    let files = read_migration_files(migrations_dir)?;
    let report = migrate(pool, &files).await?;
    seed(pool, registration_enabled).await?;
    if report.applied.is_empty() {
        tracing::info!("database schema is current ({} migrations)", files.len());
    } else {
        tracing::info!("applied migrations: {}", report.applied.join(", "));
    }
    tracing::info!("✓ migrations applied");
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;

    #[test]
    fn reads_the_shipped_journal() {
        let files = read_migration_files(&Paths::crate_dir().migrations).unwrap();
        assert_eq!(files.len(), 14);
        assert_eq!(files[0].tag, "0000_init_folders_naming");
        assert_eq!(files[0].folder_millis, 1785441450828);
        assert_eq!(files[13].tag, "0013_upload_av_metadata");
        assert!(files.iter().all(|f| f.breakpoints && f.hash.len() == 64));
        // 0013 is two statements.
        assert_eq!(files[13].statements.len(), 2);
        // Timestamps strictly increase, which the "newest row" rule relies on.
        assert!(files.windows(2).all(|w| w[0].folder_millis < w[1].folder_millis));
    }

    #[test]
    fn hash_is_sha256_of_the_file_text() {
        let dir = Paths::crate_dir().migrations;
        let files = read_migration_files(&dir).unwrap();
        let text = std::fs::read(dir.join("0010_upload_checksum_index.sql")).unwrap();
        assert_eq!(files[10].hash, hex::encode(Sha256::digest(&text)));
    }

    #[test]
    fn missing_journal_is_drizzles_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_migration_files(dir.path()).unwrap_err();
        assert_eq!(err.to_string(), "Can't find meta/_journal.json file");
    }
}
