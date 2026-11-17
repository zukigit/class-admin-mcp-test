/// Builds a query string from constant SQL fragments. Only use it with
/// compile-time constants: all values must go through `.bind()`.
macro_rules! sql {
    ($($t:tt)*) => { sqlx::AssertSqlSafe(format!($($t)*)) };
}

pub mod audit;
pub mod bookings;
pub mod classes;
pub mod courses;
pub mod revocations;

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

/// Opens (and creates if needed) the database and runs migrations.
pub async fn connect(path: &Path) -> anyhow::Result<SqlitePool> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).with_context(|| format!("creating database directory {}", dir.display()))?;
    }
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new().max_connections(8).connect_with(opts).await?;
    migrate(&pool).await?;
    Ok(pool)
}

/// In-memory database for tests.
pub async fn connect_memory() -> anyhow::Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
    // One connection: every connection to :memory: would be a separate database.
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(opts).await?;
    migrate(&pool).await?;
    Ok(pool)
}

async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("running database migrations")
}

/// Timestamp format used in the database: RFC 3339 UTC with seconds, so text order is time order.
pub fn ts(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn now_ts() -> String {
    ts(Utc::now())
}
