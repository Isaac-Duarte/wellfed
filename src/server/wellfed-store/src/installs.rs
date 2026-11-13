//! Installs: one row per install an owner asked for, a release's or a
//! stock restore's, rewritten as it progresses and once it ends.

use jiff::Timestamp;
use uuid::Uuid;
use wellfed_api::{
    FeederId, Hex, Install, InstallFailure, InstallId, InstallState, InstallTarget, UserId,
};

use crate::convert::{lenient, millis, timestamp};
use crate::{Error, Result, Store};

impl Store {
    /// Keep `install` as it stands now: a new row, or its row rewritten.
    pub async fn save_install(&self, install: &Install) -> Result<()> {
        let id = install.id.0;
        let feeder = install.feeder.0;
        let target = install.target.code();
        // A target a newer server added is never one this server starts.
        let name = install
            .target
            .name()
            .ok_or(Error::Unknown("install target"))?;
        let name = name.0.as_slice();
        let user = install.by.0;
        let started = millis(install.started);
        let state = install.state.code();
        let failure = install
            .failure
            .map(|failure| serde_json::to_string(&failure))
            .transpose()
            .map_err(|e| Error::Json("install failure", e))?;
        let finished = install.finished.map(millis);

        sqlx::query!(
            "INSERT INTO installs (id, feeder_id, target, release_id, version, user_id,
                                   started_ms, state, written, image_len, failure, finished_ms)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (id) DO UPDATE SET
                 state = excluded.state,
                 written = excluded.written,
                 failure = excluded.failure,
                 finished_ms = excluded.finished_ms",
            id,
            feeder,
            target,
            name,
            install.version,
            user,
            started,
            state,
            install.written,
            install.image_len,
            failure,
            finished,
        )
        .execute(&self.writer)
        .await?;
        Ok(())
    }

    /// The feeder's most recent install, running or not.
    pub async fn latest_install(&self, feeder: FeederId) -> Result<Option<Install>> {
        let feeder = feeder.0;
        let row = sqlx::query_as!(
            Raw,
            r#"SELECT id AS "id: Uuid", feeder_id AS "feeder_id: Uuid", target, release_id,
                      version, user_id AS "user_id: Uuid", started_ms, state, written,
                      image_len, failure, finished_ms
               FROM installs WHERE feeder_id = ?
               ORDER BY started_ms DESC, rowid DESC LIMIT 1"#,
            feeder
        )
        .fetch_optional(&self.reader)
        .await?;
        row.map(Install::try_from).transpose()
    }

    /// The feeder's most recent stock restore, running or not.
    pub async fn latest_restore(&self, feeder: FeederId) -> Result<Option<Install>> {
        let feeder = feeder.0;
        let stock = InstallTarget::STOCK;
        let row = sqlx::query_as!(
            Raw,
            r#"SELECT id AS "id: Uuid", feeder_id AS "feeder_id: Uuid", target, release_id,
                      version, user_id AS "user_id: Uuid", started_ms, state, written,
                      image_len, failure, finished_ms
               FROM installs WHERE feeder_id = ? AND target = ?
               ORDER BY started_ms DESC, rowid DESC LIMIT 1"#,
            feeder,
            stock
        )
        .fetch_optional(&self.reader)
        .await?;
        row.map(Install::try_from).transpose()
    }

    /// Note that the feeder took `id`'s offer. Kept beside the row, not
    /// in it: only a start reads it, to tell an install that changed
    /// nothing from one that may have.
    pub async fn install_offered(&self, id: InstallId) -> Result<()> {
        let id = id.0;
        sqlx::query!("UPDATE installs SET offered = 1 WHERE id = ?", id)
            .execute(&self.writer)
            .await?;
        Ok(())
    }

    /// End every install a stopped server left running at `at`: only the
    /// server that started one can run it, so at a start none is. One whose
    /// offer the feeder took ends uncertain (`unreachable`, `offered`),
    /// since bank 1 may be partly written or activated; any other
    /// `interrupted`, having changed nothing; one whose trial it followed
    /// `activated`, the verdict unseen. How many there were.
    pub async fn interrupt_installs(&self, at: Timestamp) -> Result<u64> {
        let running = InstallState::Running.code();
        let failed = InstallState::Failed.code();
        let json = |failure: InstallFailure| {
            serde_json::to_string(&failure).map_err(|e| Error::Json("install failure", e))
        };
        let interrupted = json(InstallFailure::Interrupted)?;
        let uncertain = json(InstallFailure::Unreachable { offered: true })?;
        let at = millis(at);
        let done = sqlx::query!(
            "UPDATE installs
             SET state = ?, failure = CASE WHEN offered THEN ? ELSE ? END, finished_ms = ?
             WHERE state = ?",
            failed,
            uncertain,
            interrupted,
            at,
            running,
        )
        .execute(&self.writer)
        .await?;

        // Activated, its trial followed by a server that stopped: the
        // verdict was never seen, which `activated` with an end says.
        let confirming = InstallState::Confirming.code();
        let activated = InstallState::Activated.code();
        let unseen = sqlx::query!(
            "UPDATE installs SET state = ?, finished_ms = ? WHERE state = ?",
            activated,
            at,
            confirming,
        )
        .execute(&self.writer)
        .await?;
        Ok(done.rows_affected() + unseen.rows_affected())
    }
}

/// A row as SQLite holds it.
pub(crate) struct Raw {
    pub(crate) id: Uuid,
    pub(crate) feeder_id: Uuid,
    pub(crate) target: String,
    pub(crate) release_id: Vec<u8>,
    pub(crate) version: i64,
    pub(crate) user_id: Uuid,
    pub(crate) started_ms: i64,
    pub(crate) state: String,
    pub(crate) written: i64,
    pub(crate) image_len: i64,
    pub(crate) failure: Option<String>,
    pub(crate) finished_ms: Option<i64>,
}

impl TryFrom<Raw> for Install {
    type Error = Error;

    fn try_from(raw: Raw) -> Result<Self> {
        let small = |value: i64, what| u32::try_from(value).map_err(|_| Error::Corrupt(what));
        let failure = raw
            .failure
            .map(|json| lenient(&json, InstallFailure::Unknown, "install failure"))
            .transpose()?;
        let name = Hex(raw
            .release_id
            .try_into()
            .map_err(|_| Error::Corrupt("install target's name"))?);

        Ok(Self {
            id: InstallId(raw.id),
            feeder: FeederId(raw.feeder_id),
            target: InstallTarget::from_code(&raw.target, name)
                .ok_or(Error::Corrupt("install target"))?,
            version: small(raw.version, "install version")?,
            by: UserId(raw.user_id),
            started: timestamp(raw.started_ms)?,
            state: InstallState::from_code(&raw.state).ok_or(Error::Corrupt("install state"))?,
            written: small(raw.written, "install progress")?,
            image_len: small(raw.image_len, "install length")?,
            failure,
            finished: raw.finished_ms.map(timestamp).transpose()?,
        })
    }
}
