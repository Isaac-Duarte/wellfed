//! The embedded migrations, and the copy taken before any of them runs.
//!
//! A database whose record names a migration this build does not embed was
//! migrated by a newer build. It is opened read-only and nothing runs on it,
//! so going forward to that build again loses nothing (the upgrade rule's
//! "never write over newer data"). A build that stores a new value of a code
//! the server owns (a model, role, unit, job or state) adds a migration with
//! it, even an empty one, so an older build that would refuse the value
//! opens the database read-only instead (`tests/codes.rs` holds each set to
//! the newest migration, the JSON columns' through every type they nest).
//! Codes from the feeder's events grow with the device API and read as
//! unknown, and so does a JSON column's value an older build can't read.

use std::path::{Path, PathBuf};

use jiff::Timestamp;
use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, SqlitePool};

use crate::{Error, Result, Schema, copies};

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

/// Apply pending migrations, copying an existing database first. A copy a
/// start cut off left half-written goes first.
pub(crate) async fn run(writer: &SqlitePool, path: &Path, now: Timestamp) -> Result<()> {
    run_with(&MIGRATOR, writer, path, now).await
}

/// [`run`] with `migrator`'s migrations: a test gives it a second one, which
/// the embedded set gains only with the next schema change.
async fn run_with(
    migrator: &Migrator,
    writer: &SqlitePool,
    path: &Path,
    now: Timestamp,
) -> Result<()> {
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        copies::remove_partials(dir, &format!("{name}.before-"))?;
    }

    let mut connection = writer.acquire().await?;
    connection
        .ensure_migrations_table(&migrator.table_name)
        .await?;
    let applied: Vec<i64> = connection
        .list_applied_migrations(&migrator.table_name)
        .await?
        .into_iter()
        .map(|migration| migration.version)
        .collect();
    let pending = migrator
        .iter()
        .filter(|migration| !applied.contains(&migration.version))
        .map(|migration| migration.version)
        .max();

    if let (Some(version), false) = (pending, applied.is_empty()) {
        let copy = backup_path(path, version, now);
        let partial = copies::partial(&copy);
        let into = partial
            .to_str()
            .ok_or_else(|| Error::Path(partial.clone()))?;
        // `VACUUM INTO` takes a consistent copy of a live database. It is
        // not a table query, so it has no offline metadata to check.
        sqlx::query("VACUUM INTO ?")
            .bind(into)
            .execute(&mut *connection)
            .await?;
        copies::finish(&partial, &copy)?;
    }
    drop(connection);

    migrator.run(writer).await?;
    Ok(())
}

fn backup_path(path: &Path, version: i64, now: Timestamp) -> PathBuf {
    let now = now.as_millisecond();
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".before-{version}-{now}"));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    fn copies(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".before-"))
            .collect()
    }

    /// A database whose pending migration fails. Each attempt copies the
    /// database first, named for the newest pending migration and the time
    /// the caller gives, readable by the server's user alone, and a copy
    /// left by one attempt does not block the next. With nothing applied
    /// yet there is nothing to copy.
    #[tokio::test]
    async fn pending_migrations_copy_the_database_first_on_every_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let sources = dir.path().join("migrations");
        std::fs::create_dir(&sources).unwrap();
        std::fs::write(sources.join("1_first.sql"), "CREATE TABLE a (x INTEGER);").unwrap();
        let first = Migrator::new(sources.as_path()).await.unwrap();
        // The second makes the first's table again, so it always fails.
        std::fs::write(sources.join("2_second.sql"), "CREATE TABLE a (x INTEGER);").unwrap();
        let both = Migrator::new(sources.as_path()).await.unwrap();

        let path = dir.path().join("server.db");
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let writer = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        run_with(&first, &writer, &path, Timestamp::UNIX_EPOCH)
            .await
            .unwrap();
        assert!(
            copies(dir.path()).is_empty(),
            "a new database is not copied"
        );

        for (attempt, ms) in [(1, 1_790_000_000_001), (2, 1_790_000_000_002)] {
            let now = Timestamp::from_millisecond(ms).unwrap();
            let ran = run_with(&both, &writer, &path, now).await;
            assert!(matches!(ran, Err(Error::Migrate(_))), "{ran:?}");

            assert_eq!(copies(dir.path()).len(), attempt, "no partial copy left");
            let copy = dir.path().join(format!("server.db.before-2-{ms}"));
            let mode = std::fs::metadata(copy)
                .expect("named for the migration and the time given")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "the server's user's alone");
        }
    }
}
