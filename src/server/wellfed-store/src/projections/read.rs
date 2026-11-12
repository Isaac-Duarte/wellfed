//! Reading the projections back. Each read takes a window of the server's
//! receive times, `from` inclusive and `to` exclusive, and returns its rows
//! oldest first: the server's clock orders every epoch's events, where the
//! feeder's may have been invalid.

use jiff::Timestamp;
use wellfed_api::{Blocked, FeedOutcome, FeedReason, FeederId};

use super::{Boot, ClockSync, Feed, LockChange, Projected};
use crate::convert::{millis, seconds, source, timestamp};
use crate::{Error, Result, Store};

impl Store {
    /// The feeds received in `from..to`, each as its latest event.
    pub async fn feeds(
        &self,
        id: FeederId,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<Projected<Feed>>> {
        let (from, to) = (millis(from), millis(to));
        let rows = sqlx::query_as!(
            FeedRow,
            r#"SELECT epoch AS "epoch!", feed AS "feed!", seq AS "seq!", source, client, requested, completed, outcome, reason,
                    at_s, received_ms
             FROM feeds WHERE feeder_id = ? AND received_ms >= ? AND received_ms < ?
             ORDER BY received_ms, epoch, seq"#,
            id.0,
            from,
            to,
        )
        .fetch_all(&self.reader)
        .await?;

        rows.into_iter().map(Projected::try_from).collect()
    }

    /// The panel lock's changes received in `from..to`.
    pub async fn lock_changes(
        &self,
        id: FeederId,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<Projected<LockChange>>> {
        let (from, to) = (millis(from), millis(to));
        let rows = sqlx::query!(
            r#"SELECT epoch AS "epoch!", seq AS "seq!", locked, source, client, at_s, received_ms
             FROM lock_changes WHERE feeder_id = ? AND received_ms >= ? AND received_ms < ?
             ORDER BY received_ms, epoch, seq"#,
            id.0,
            from,
            to,
        )
        .fetch_all(&self.reader)
        .await?;

        rows.into_iter()
            .map(|row| {
                let value = lock_change(row.locked != 0, &row.source, row.client)?;
                projected(row.epoch, row.seq, row.at_s, row.received_ms, value)
            })
            .collect()
    }

    /// The boots received in `from..to`.
    pub async fn boots(
        &self,
        id: FeederId,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<Projected<Boot>>> {
        let (from, to) = (millis(from), millis(to));
        let rows = sqlx::query!(
            r#"SELECT epoch AS "epoch!", seq AS "seq!", bank, reset_flags, at_s, received_ms
             FROM boots WHERE feeder_id = ? AND received_ms >= ? AND received_ms < ?
             ORDER BY received_ms, epoch, seq"#,
            id.0,
            from,
            to,
        )
        .fetch_all(&self.reader)
        .await?;

        rows.into_iter()
            .map(|row| {
                let value = boot(row.bank, row.reset_flags)?;
                projected(row.epoch, row.seq, row.at_s, row.received_ms, value)
            })
            .collect()
    }

    /// The clock syncs received in `from..to`.
    pub async fn clock_syncs(
        &self,
        id: FeederId,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<Projected<ClockSync>>> {
        let (from, to) = (millis(from), millis(to));
        let rows = sqlx::query!(
            r#"SELECT epoch AS "epoch!", seq AS "seq!", source, client, jump_ms, at_s, received_ms
             FROM clock_syncs WHERE feeder_id = ? AND received_ms >= ? AND received_ms < ?
             ORDER BY received_ms, epoch, seq"#,
            id.0,
            from,
            to,
        )
        .fetch_all(&self.reader)
        .await?;

        rows.into_iter()
            .map(|row| {
                let value = clock_sync(&row.source, row.client, row.jump_ms)?;
                projected(row.epoch, row.seq, row.at_s, row.received_ms, value)
            })
            .collect()
    }

    /// The changes of the lasting reason the feeder cannot feed received in
    /// `from..to`: the new reason, or `None` once it could feed again.
    pub async fn blocked_changes(
        &self,
        id: FeederId,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<Projected<Option<Blocked>>>> {
        let (from, to) = (millis(from), millis(to));
        let rows = sqlx::query!(
            r#"SELECT epoch AS "epoch!", seq AS "seq!", reason, at_s, received_ms
             FROM blocked_changes WHERE feeder_id = ? AND received_ms >= ? AND received_ms < ?
             ORDER BY received_ms, epoch, seq"#,
            id.0,
            from,
            to,
        )
        .fetch_all(&self.reader)
        .await?;

        rows.into_iter()
            .map(|row| {
                let value = row.reason.as_deref().map(blocked);
                projected(row.epoch, row.seq, row.at_s, row.received_ms, value)
            })
            .collect()
    }
}

/// A `feeds` row as SQLite holds it.
pub(crate) struct FeedRow {
    pub(crate) epoch: i64,
    pub(crate) feed: i64,
    pub(crate) seq: i64,
    pub(crate) source: String,
    pub(crate) client: Option<Vec<u8>>,
    pub(crate) requested: i64,
    pub(crate) completed: i64,
    pub(crate) outcome: String,
    pub(crate) reason: String,
    pub(crate) at_s: Option<i64>,
    pub(crate) received_ms: i64,
}

impl TryFrom<FeedRow> for Projected<Feed> {
    type Error = Error;

    fn try_from(row: FeedRow) -> Result<Self> {
        let value = Feed {
            feed: u32::try_from(row.feed).map_err(|_| Error::Corrupt("feed number"))?,
            by: source(&row.source, row.client)?,
            requested: u8::try_from(row.requested)
                .map_err(|_| Error::Corrupt("requested portions"))?,
            completed: u8::try_from(row.completed)
                .map_err(|_| Error::Corrupt("completed portions"))?,
            // A code a newer server stored reads as unknown, as the device
            // API's own lenient enums do, rather than fail the whole read.
            outcome: FeedOutcome::from_code(&row.outcome).unwrap_or(FeedOutcome::Unknown),
            reason: FeedReason::from_code(&row.reason).unwrap_or(FeedReason::Unknown),
        };
        projected(row.epoch, row.seq, row.at_s, row.received_ms, value)
    }
}

pub(crate) fn projected<T>(
    epoch: i64,
    seq: i64,
    at_s: Option<i64>,
    received_ms: i64,
    value: T,
) -> Result<Projected<T>> {
    Ok(Projected {
        epoch: u32::try_from(epoch).map_err(|_| Error::Corrupt("event epoch"))?,
        seq: u64::try_from(seq).map_err(|_| Error::Corrupt("event sequence"))?,
        at: seconds(at_s)?,
        received: timestamp(received_ms)?,
        value,
    })
}

pub(crate) fn lock_change(
    locked: bool,
    source_kind: &str,
    client: Option<Vec<u8>>,
) -> Result<LockChange> {
    Ok(LockChange {
        locked,
        by: source(source_kind, client)?,
    })
}

pub(crate) fn boot(bank: i64, reset_flags: i64) -> Result<Boot> {
    Ok(Boot {
        bank: u8::try_from(bank).map_err(|_| Error::Corrupt("boot bank"))?,
        reset_flags: u32::try_from(reset_flags).map_err(|_| Error::Corrupt("reset flags"))?,
    })
}

pub(crate) fn clock_sync(
    source_kind: &str,
    client: Option<Vec<u8>>,
    jump_ms: Option<i64>,
) -> Result<ClockSync> {
    let jump_ms = jump_ms
        .map(|jump| i32::try_from(jump).map_err(|_| Error::Corrupt("clock jump")))
        .transpose()?;
    Ok(ClockSync {
        by: source(source_kind, client)?,
        jump_ms,
    })
}

/// A blocked reason as stored. A code a newer server stored reads as
/// unknown, so it is never refused.
pub(crate) fn blocked(reason: &str) -> Blocked {
    Blocked::from_code(reason).unwrap_or(Blocked::Unknown)
}
