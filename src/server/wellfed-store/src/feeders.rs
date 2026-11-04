//! The home's adopted devices (the table keeps its first kind's name), with
//! each one's kind and model, its sealed key and cursor, and the last
//! status, settings and plans read from it.

use jiff::Timestamp;
use uuid::Uuid;
use wellfed_api::{ClientRole, DeviceKind, FeederId, HomeId, Model, Plans, Settings, Status};

use crate::convert::{cached, from_i64, millis, model, timestamp};
use crate::{Error, Result, Store};

/// The server's feeder key, sealed by the service.
#[derive(Clone, PartialEq, Eq)]
pub struct SealedKey {
    pub nonce: [u8; 12],
    pub sealed: Vec<u8>,
}

impl std::fmt::Debug for SealedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SealedKey").finish_non_exhaustive()
    }
}

pub struct NewFeeder<'a> {
    pub id: FeederId,
    pub home: HomeId,
    /// What its session at adoption named; its kind is the model's.
    pub model: Model,
    pub identity: [u8; 32],
    pub name: &'a str,
    pub role: ClientRole,
    pub address: Option<&'a str>,
    pub key: SealedKey,
    pub created: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeederRow {
    pub id: FeederId,
    pub home: HomeId,
    /// Which module of the device API it serves: `model`'s kind.
    pub kind: DeviceKind,
    pub model: Model,
    pub identity: [u8; 32],
    pub name: String,
    pub role: ClientRole,
    pub address: Option<String>,
    pub key: SealedKey,
    /// The newest event held.
    pub synced: Option<Synced>,
    /// When it was removed; `None` while it is mirrored.
    pub archived: Option<Timestamp>,
}

/// The newest event (or gap end) held, in the epoch it counts in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Synced {
    pub epoch: u32,
    pub seq: u64,
}

/// The last status, settings and plans read, and when.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FeederState {
    pub status: Option<(Status, Timestamp)>,
    pub settings: Option<(Settings, Timestamp)>,
    pub plans: Option<(Plans, Timestamp)>,
}

impl Store {
    /// Record an adopted feeder; [`Error::Duplicate`] if its identity is
    /// already adopted.
    pub async fn insert_feeder(&self, feeder: NewFeeder<'_>) -> Result<()> {
        let identity = feeder.identity.as_slice();
        let (kind, model) = (feeder.model.kind().name(), feeder.model.name());
        let role = feeder.role.code();
        let nonce = feeder.key.nonce.as_slice();
        let created = millis(feeder.created);
        let done = sqlx::query!(
            "INSERT INTO feeders (id, home_id, kind, model, identity, name, role, address,
                                  key_nonce, key_sealed, created_ms)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            feeder.id.0,
            feeder.home.0,
            kind,
            model,
            identity,
            feeder.name,
            role,
            feeder.address,
            nonce,
            feeder.key.sealed,
            created,
        )
        .execute(&self.writer)
        .await;

        match done {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                Err(Error::Duplicate("feeder"))
            }
            Err(error) => Err(error.into()),
        }
    }

    pub async fn feeders(&self) -> Result<Vec<FeederRow>> {
        let rows = sqlx::query_as!(
            Selected,
            r#"SELECT id AS "id: Uuid", home_id AS "home_id: Uuid", kind, model, identity, name,
                      role, address, key_nonce, key_sealed, synced, epoch, archived_ms
               FROM feeders ORDER BY created_ms, id"#
        )
        .fetch_all(&self.reader)
        .await?;

        rows.into_iter().map(feeder_row).collect()
    }

    pub async fn feeder(&self, id: FeederId) -> Result<Option<FeederRow>> {
        let row = sqlx::query_as!(
            Selected,
            r#"SELECT id AS "id: Uuid", home_id AS "home_id: Uuid", kind, model, identity, name,
                      role, address, key_nonce, key_sealed, synced, epoch, archived_ms
               FROM feeders WHERE id = ?"#,
            id.0
        )
        .fetch_optional(&self.reader)
        .await?;

        row.map(feeder_row).transpose()
    }

    /// The feeder with this identity, adopted or archived.
    pub async fn feeder_by_identity(&self, identity: &[u8; 32]) -> Result<Option<FeederId>> {
        let identity = identity.as_slice();
        let row = sqlx::query!(
            r#"SELECT id AS "id: Uuid" FROM feeders WHERE identity = ?"#,
            identity
        )
        .fetch_optional(&self.reader)
        .await?;
        Ok(row.map(|row| FeederId(row.id)))
    }

    /// Bring an archived feeder back under a new key: its id, history and
    /// journal continue. Whether an archived feeder was there to take it.
    pub async fn readopt_feeder(&self, feeder: NewFeeder<'_>) -> Result<bool> {
        let (kind, model) = (feeder.model.kind().name(), feeder.model.name());
        let role = feeder.role.code();
        let nonce = feeder.key.nonce.as_slice();
        let done = sqlx::query!(
            "UPDATE feeders SET home_id = ?, kind = ?, model = ?, name = ?, role = ?, address = ?,
                 key_nonce = ?, key_sealed = ?, archived_ms = NULL
             WHERE id = ? AND archived_ms IS NOT NULL",
            feeder.home.0,
            kind,
            model,
            feeder.name,
            role,
            feeder.address,
            nonce,
            feeder.key.sealed,
            feeder.id.0,
        )
        .execute(&self.writer)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Archive a removed feeder. Whether it was mirrored until now.
    pub async fn archive_feeder(&self, id: FeederId, at: Timestamp) -> Result<bool> {
        let at = millis(at);
        let done = sqlx::query!(
            "UPDATE feeders SET archived_ms = ? WHERE id = ? AND archived_ms IS NULL",
            at,
            id.0
        )
        .execute(&self.writer)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Where the feeder was last reached.
    pub async fn set_address(&self, id: FeederId, address: &str) -> Result<()> {
        sqlx::query!("UPDATE feeders SET address = ? WHERE id = ?", address, id.0)
            .execute(&self.writer)
            .await?;
        Ok(())
    }

    pub async fn record_status(&self, id: FeederId, status: &Status, at: Timestamp) -> Result<()> {
        let json = serde_json::to_string(status).map_err(|e| Error::Json("status", e))?;
        let at = millis(at);
        sqlx::query!(
            "INSERT INTO feeder_state (feeder_id, status, status_ms) VALUES (?, ?, ?)
             ON CONFLICT (feeder_id) DO UPDATE SET status = excluded.status, status_ms = excluded.status_ms",
            id.0,
            json,
            at,
        )
        .execute(&self.writer)
        .await?;
        Ok(())
    }

    pub async fn record_settings(
        &self,
        id: FeederId,
        settings: &Settings,
        at: Timestamp,
    ) -> Result<()> {
        let json = serde_json::to_string(settings).map_err(|e| Error::Json("settings", e))?;
        let at = millis(at);
        sqlx::query!(
            "INSERT INTO feeder_state (feeder_id, settings, settings_ms) VALUES (?, ?, ?)
             ON CONFLICT (feeder_id) DO UPDATE SET settings = excluded.settings, settings_ms = excluded.settings_ms",
            id.0,
            json,
            at,
        )
        .execute(&self.writer)
        .await?;
        Ok(())
    }

    pub async fn record_plans(&self, id: FeederId, plans: &Plans, at: Timestamp) -> Result<()> {
        let json = serde_json::to_string(plans).map_err(|e| Error::Json("plans", e))?;
        let at = millis(at);
        sqlx::query!(
            "INSERT INTO feeder_state (feeder_id, plans, plans_ms) VALUES (?, ?, ?)
             ON CONFLICT (feeder_id) DO UPDATE SET plans = excluded.plans, plans_ms = excluded.plans_ms",
            id.0,
            json,
            at,
        )
        .execute(&self.writer)
        .await?;
        Ok(())
    }

    pub async fn state(&self, id: FeederId) -> Result<FeederState> {
        let row = sqlx::query!(
            "SELECT status, status_ms, settings, settings_ms, plans, plans_ms
             FROM feeder_state WHERE feeder_id = ?",
            id.0
        )
        .fetch_optional(&self.reader)
        .await?;

        let Some(row) = row else {
            return Ok(FeederState::default());
        };
        Ok(FeederState {
            status: cached(row.status, row.status_ms, "status")?,
            settings: cached(row.settings, row.settings_ms, "settings")?,
            plans: cached(row.plans, row.plans_ms, "plans")?,
        })
    }
}

/// A device's columns, as both reads select them.
struct Selected {
    id: Uuid,
    home_id: Uuid,
    kind: String,
    model: String,
    identity: Vec<u8>,
    name: String,
    role: String,
    address: Option<String>,
    key_nonce: Vec<u8>,
    key_sealed: Vec<u8>,
    synced: Option<i64>,
    epoch: Option<i64>,
    archived_ms: Option<i64>,
}

fn feeder_row(row: Selected) -> Result<FeederRow> {
    let model = model(&row.model)?;
    // The kind is the model's, kept beside it for queries.
    if row.kind != model.kind().name() {
        return Err(Error::Corrupt("device kind"));
    }

    Ok(FeederRow {
        id: FeederId(row.id),
        home: HomeId(row.home_id),
        kind: model.kind(),
        model,
        identity: row
            .identity
            .as_slice()
            .try_into()
            .map_err(|_| Error::Corrupt("identity"))?,
        name: row.name,
        role: ClientRole::from_code(&row.role).ok_or(Error::Corrupt("feeder role"))?,
        address: row.address,
        key: SealedKey {
            nonce: row
                .key_nonce
                .as_slice()
                .try_into()
                .map_err(|_| Error::Corrupt("key nonce"))?,
            sealed: row.key_sealed,
        },
        synced: match (row.synced, row.epoch) {
            (Some(seq), Some(epoch)) => Some(Synced {
                epoch: u32::try_from(epoch).map_err(|_| Error::Corrupt("epoch"))?,
                seq: from_i64(seq, "cursor")?,
            }),
            (None, _) => None,
            (Some(_), None) => return Err(Error::Corrupt("cursor without its epoch")),
        },
        archived: row.archived_ms.map(timestamp).transpose()?,
    })
}
