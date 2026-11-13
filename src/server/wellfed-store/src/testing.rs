//! Store faults for the tests of the crates above, which may not reach the
//! database themselves: only this crate holds SQL. Built only with the
//! `testing` feature, which no shipping crate enables.

use std::path::Path;

use sqlx::Connection;
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};

use crate::Result;

/// From now on the database at `path` refuses [`Store::install_offered`]
/// (and any other write to the `offered` column), through a trigger kept in
/// the file, so a store already open there and one reopened later both
/// fail it while every other write goes through.
///
/// [`Store::install_offered`]: crate::Store::install_offered
pub async fn refuse_offer_notes(path: &Path) -> Result<()> {
    let options = SqliteConnectOptions::new().filename(path);
    let mut connection = SqliteConnection::connect_with(&options).await?;

    sqlx::query(
        "CREATE TRIGGER refuse_offer_notes BEFORE UPDATE OF offered ON installs
         BEGIN SELECT RAISE(ABORT, 'offer notes refused'); END",
    )
    .execute(&mut connection)
    .await?;

    connection.close().await?;
    Ok(())
}
