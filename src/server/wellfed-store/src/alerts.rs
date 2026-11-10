//! Each feeder's alerts as the server keeps them: one JSON document per
//! feeder, which the mirror writes and decodes and versions by its
//! `format`, so the store only keeps it whole. Two tasks write it (the
//! feeder's alert reader and the webhook), so every write names the
//! version it read and is refused when another write came first.

use jiff::Timestamp;
use wellfed_api::FeederId;

use crate::convert::{millis, timestamp};
use crate::{Error, Result, Store};

/// One feeder's kept alerts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertsRow {
    pub feeder: FeederId,
    /// The document's own layout, as its writer numbers it.
    pub format: u32,
    pub state: String,
    /// The row's writes so far: the version a write names.
    pub version: Version,
    pub updated: Timestamp,
}

/// A row's write count, for [`Store::put_feeder_alerts`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Version(i64);

/// What a write of the kept alerts expects to find.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
    /// No row yet.
    Absent,
    /// The row at this version.
    At(Version),
}

impl Store {
    /// The feeder's kept alerts, or `None` before any.
    pub async fn feeder_alerts(&self, id: FeederId) -> Result<Option<AlertsRow>> {
        let row = sqlx::query!(
            r#"SELECT format, state, version, updated_ms FROM feeder_alerts WHERE feeder_id = ?"#,
            id.0,
        )
        .fetch_optional(&self.reader)
        .await?;
        row.map(|row| {
            Ok(AlertsRow {
                feeder: id,
                format: u32::try_from(row.format).map_err(|_| Error::Corrupt("alerts format"))?,
                state: row.state,
                version: Version(row.version),
                updated: timestamp(row.updated_ms)?,
            })
        })
        .transpose()
    }

    /// Every feeder's kept alerts, by feeder.
    pub async fn every_feeder_alerts(&self) -> Result<Vec<AlertsRow>> {
        let rows = sqlx::query!(
            r#"SELECT feeder_id AS "feeder_id: uuid::Uuid", format, state, version, updated_ms
               FROM feeder_alerts ORDER BY feeder_id"#
        )
        .fetch_all(&self.reader)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(AlertsRow {
                    feeder: FeederId(row.feeder_id),
                    format: u32::try_from(row.format)
                        .map_err(|_| Error::Corrupt("alerts format"))?,
                    state: row.state,
                    version: Version(row.version),
                    updated: timestamp(row.updated_ms)?,
                })
            })
            .collect()
    }

    /// Drop the feeder's kept alerts, so it starts afresh.
    pub async fn delete_feeder_alerts(&self, id: FeederId) -> Result<()> {
        sqlx::query!("DELETE FROM feeder_alerts WHERE feeder_id = ?", id.0)
            .execute(&self.writer)
            .await?;
        Ok(())
    }

    /// Write the feeder's kept alerts over what `expect` names: whether it
    /// was written. `false` means another write came first, and the caller
    /// reads the row again and redoes its change.
    pub async fn put_feeder_alerts(
        &self,
        id: FeederId,
        expect: Expect,
        format: u32,
        state: &str,
        at: Timestamp,
    ) -> Result<bool> {
        let format = i64::from(format);
        let at = millis(at);
        let written = match expect {
            Expect::Absent => sqlx::query!(
                "INSERT INTO feeder_alerts (feeder_id, format, state, version, updated_ms)
                 VALUES (?, ?, ?, 1, ?) ON CONFLICT (feeder_id) DO NOTHING",
                id.0,
                format,
                state,
                at,
            )
            .execute(&self.writer)
            .await?
            .rows_affected(),
            Expect::At(Version(version)) => sqlx::query!(
                "UPDATE feeder_alerts SET format = ?, state = ?, version = version + 1,
                        updated_ms = ?
                 WHERE feeder_id = ? AND version = ?",
                format,
                state,
                at,
                id.0,
                version,
            )
            .execute(&self.writer)
            .await?
            .rows_affected(),
        };
        Ok(written == 1)
    }
}
