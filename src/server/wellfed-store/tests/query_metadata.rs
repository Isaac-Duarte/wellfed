//! The committed query metadata (`.sqlx/`) still describes the schema the
//! migrations build. The query macros compile against that metadata alone,
//! so a migration that changes a column's type or nullability without
//! touching a query's text would build against stale metadata and decode
//! wrongly at run time. Each recorded query is described again against a
//! freshly migrated database and must match what was recorded;
//! `make sqlx-prepare` rebuilds the metadata after a change.

use std::fs;
use std::path::Path;

use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{AssertSqlSafe, ConnectOptions, Executor, SqlSafeStr};

static MIGRATOR: Migrator = sqlx::migrate!();

#[tokio::test]
async fn the_query_metadata_describes_the_migrated_schema() {
    let mut connection = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true)
        .connect()
        .await
        .unwrap();
    MIGRATOR.run(&mut connection).await.unwrap();

    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(".sqlx");
    let mut stale = Vec::new();
    let mut read = 0;
    for entry in fs::read_dir(&directory).unwrap() {
        let path = entry.unwrap().path();
        let recorded: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let query = recorded["query"].as_str().unwrap().to_owned();

        let described = (&mut connection)
            .describe(AssertSqlSafe(query.clone()).into_sql_str())
            .await;
        let now = match described {
            Ok(describe) => serde_json::to_value(&describe).unwrap(),
            Err(error) => Value::String(error.to_string()),
        };
        if now != recorded["describe"] {
            stale.push(format!("{}: {query}", path.display()));
        }
        read += 1;
    }

    assert!(read > 0, "no query metadata in {}", directory.display());
    assert!(
        stale.is_empty(),
        "query metadata no longer matches the migrated schema; run `make sqlx-prepare`:\n{}",
        stale.join("\n")
    );
}
