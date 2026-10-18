//! Port of `lib/db/index.ts`: the single pooled connection to PostgreSQL.
//!
//! Upstream pool settings are kept: at most 10 connections, idle connections
//! closed after 30 s, 10 s to obtain one. Queries are raw SQL (PORT.md
//! section 4) against the rows in [`models`].
//!
//! Every connection runs with `TimeZone=UTC`. The Better Auth tables use
//! `timestamp` without a zone and drizzle stores UTC wall time in them; pinning
//! the session zone is what makes `now()` defaults, comparisons and the
//! `timestamptz` parameters [`types::JsDate`] binds agree with that.

pub mod migrate;
pub mod models;
pub mod types;

use std::str::FromStr;
use std::time::Duration;

use anyhow::Context;
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

/// Parse a node-postgres style connection string into sqlx options.
pub fn connect_options(url: &str) -> anyhow::Result<PgConnectOptions> {
    let options = if url.is_empty() {
        // node-postgres falls back to the PG* environment variables.
        PgConnectOptions::new()
    } else {
        PgConnectOptions::from_str(url).context("DATABASE_URL is not a valid PostgreSQL connection string")?
    };
    Ok(options.options([("TimeZone", "UTC")]))
}

fn pool_options() -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(10)
        .idle_timeout(Duration::from_secs(30))
        .acquire_timeout(Duration::from_secs(10))
}

/// A pool that connects on first use, like `new Pool()` upstream: the server
/// starts (and /api/health can report `database: fail`) even when the
/// database is down.
pub fn connect_lazy(url: &str) -> anyhow::Result<PgPool> {
    Ok(pool_options().connect_lazy_with(connect_options(url)?))
}

/// A pool with one connection established up front (the migrator wants to
/// fail fast, as `migrate.ts` did).
pub async fn connect(url: &str, max_connections: u32) -> anyhow::Result<PgPool> {
    pool_options()
        .max_connections(max_connections)
        .connect_with(connect_options(url)?)
        .await
        .context("could not connect to the database")
}
