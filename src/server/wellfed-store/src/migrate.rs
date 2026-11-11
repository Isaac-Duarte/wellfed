//! The embedded migrations, and the copy taken before any of them runs.
//!
//! A database whose record names a migration this build does not embed was
//! migrated by a newer build. It is opened read-only and nothing runs on it,
//! so going forward to that build again loses nothing (the upgrade rule's
//! "never write over newer data").

use std::path::{Path, PathBuf};

use jiff::Timestamp;
use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, SqlitePool};

use crate::{Result, Schema};

static MIGRATOR: Migrator = sqlx::migrate!();

/// Whether the database at `path` was migrated by a newer build: read on a
/// read-only connection, so a newer database is never written, not even by
/// the migrations table's creation. A database not there yet is not newer.
pub(crate) async fn schema(path: &Path) -> Result<Schema> {
    if !path.exists() {
        return Ok(Schema::Current);
    }

    let mut connection = read_only(path).connect().await?;
    // Not table queries with offline metadata: the migrations table is
    // sqlx's (under its default name, which `MIGRATOR` keeps), and may not
    // exist yet.
    let recorded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(&mut connection)
    .await?;
    let applied: Vec<i64> = if recorded == 0 {
        Vec::new()
    } else {
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations")
            .fetch_all(&mut connection)
            .await?
    };
    connection.close().await?;

    let embedded = MIGRATOR
        .iter()
        .map(|migration| migration.version)
        .max()
        .unwrap_or(0);
    let unknown = applied
        .into_iter()
        .filter(|version| !MIGRATOR.version_exists(*version))
        .max();
    Ok(unknown.map_or(Schema::Current, |applied| Schema::Newer {
        applied,
        embedded,
    }))
}

/// Options that only read the database at `path`, and set nothing on it:
/// not even the journal mode, which a read-only connection cannot change.
pub(crate) fn read_only(path: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .busy_timeout(crate::BUSY)
}

/// Apply pending migrations, copying an existing database first.
pub(crate) async fn run(writer: &SqlitePool, path: &Path, now: Timestamp) -> Result<()> {
    let mut connection = writer.acquire().await?;
    connection
        .ensure_migrations_table(&MIGRATOR.table_name)
        .await?;
    let applied: Vec<i64> = connection
        .list_applied_migrations(&MIGRATOR.table_name)
        .await?
        .into_iter()
        .map(|migration| migration.version)
        .collect();
    let pending = MIGRATOR
        .iter()
        .filter(|migration| !applied.contains(&migration.version))
        .map(|migration| migration.version)
        .max();

    if let (Some(version), false) = (pending, applied.is_empty()) {
        let copy = backup_path(path, version, now);
        // `VACUUM INTO` takes a consistent copy of a live database. It is
        // not a table query, so it has no offline metadata to check.
        sqlx::query("VACUUM INTO ?")
            .bind(copy.to_string_lossy().into_owned())
            .execute(&mut *connection)
            .await?;
    }
    drop(connection);

    MIGRATOR.run(writer).await?;
    Ok(())
}

fn backup_path(path: &Path, version: i64, now: Timestamp) -> PathBuf {
    let now = now.as_millisecond();
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".before-{version}-{now}"));
    path.with_file_name(name)
}
