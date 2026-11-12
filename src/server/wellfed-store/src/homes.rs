//! The home the server serves, which every device belongs to. One server
//! serves one home for now: setup writes it with the first owner
//! (`users.rs`), and it is never removed.

use jiff::Timestamp;
use sqlx::SqliteExecutor;
use uuid::Uuid;
use wellfed_api::{Home, HomeId, TimeFormat, Units};

use crate::convert::millis;
use crate::{Error, Result, Store};

/// Write `home`, made at `created`, and answer the id stored. The server
/// has one home, so over one already there this writes its settings and
/// keeps that home's id and age: the id answered is the stored one, which
/// is then not `home.id`.
pub(crate) async fn insert_home<'e>(
    executor: impl SqliteExecutor<'e>,
    home: &Home,
    created: Timestamp,
) -> Result<HomeId> {
    let (units, format) = (home.units.code(), home.time_format.code());
    let created = millis(created);
    let stored = sqlx::query!(
        r#"INSERT INTO homes (id, name, units, time_format, zone, created_ms)
           VALUES (?, ?, ?, ?, ?, ?)
           ON CONFLICT (single) DO UPDATE SET name = excluded.name, units = excluded.units,
               time_format = excluded.time_format, zone = excluded.zone
           RETURNING id AS "id: Uuid""#,
        home.id.0,
        home.name,
        units,
        format,
        home.zone,
        created,
    )
    .fetch_one(executor)
    .await?;
    Ok(HomeId(stored.id))
}

impl Store {
    /// Write the home before any owner: setup writes it with the first
    /// owner ([`Self::create_first_owner`]), and takes over one written
    /// here, so this is for a store that starts from its devices, as the
    /// tests do.
    pub async fn create_home(&self, home: &Home, created: Timestamp) -> Result<HomeId> {
        insert_home(&self.writer, home, created).await
    }

    /// The home, once setup has written it.
    pub async fn home(&self) -> Result<Option<Home>> {
        let row = sqlx::query!(
            r#"SELECT id AS "id: Uuid", name, units, time_format, zone
               FROM homes ORDER BY created_ms, id LIMIT 1"#
        )
        .fetch_optional(&self.reader)
        .await?;

        row.map(|row| home(row.id, row.name, &row.units, &row.time_format, row.zone))
            .transpose()
    }

    /// Store a change to `home`, by its id: [`Error::Missing`] when no
    /// stored home has it, and nothing is written.
    pub async fn set_home(&self, home: &Home) -> Result<()> {
        let (units, format) = (home.units.code(), home.time_format.code());
        let done = sqlx::query!(
            "UPDATE homes SET name = ?, units = ?, time_format = ?, zone = ? WHERE id = ?",
            home.name,
            units,
            format,
            home.zone,
            home.id.0,
        )
        .execute(&self.writer)
        .await?;
        wellfed_formats::require(done.rows_affected() == 1, || Error::Missing("home"))
    }
}

/// A `homes` row as the home it stores.
pub(crate) fn home(
    id: Uuid,
    name: String,
    units: &str,
    time_format: &str,
    zone: String,
) -> Result<Home> {
    Ok(Home {
        id: HomeId(id),
        name,
        units: Units::from_code(units).ok_or(Error::Corrupt("units"))?,
        time_format: TimeFormat::from_code(time_format).ok_or(Error::Corrupt("time format"))?,
        zone,
    })
}
