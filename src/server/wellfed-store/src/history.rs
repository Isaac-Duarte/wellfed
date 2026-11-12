//! The mirrored event history: events and gaps, keyed by feeder, ring
//! epoch and sequence, with the feeder's cursor moved in the same
//! transaction as each insert so a reconnect never skips or repeats one.

use jiff::Timestamp;
use wellfed_api::{EventBody, EventKind, FeederId, Gap};

use crate::convert::{from_i64, millis, seconds, timestamp, to_i64};
use crate::projections::{Place, project};
use crate::{Error, Result, Store};

pub struct NewEvent<'a> {
    /// The feeder's ring epoch the number counts in.
    pub epoch: u32,
    pub seq: u64,
    pub at: Option<Timestamp>,
    pub received: Timestamp,
    /// The event, decoded once, for its kind and its projection; `None`
    /// for a body a newer feeder build added, which this server cannot
    /// read. That one is stored with no kind and projects nothing, so a
    /// later server reads it from its bytes.
    pub body: Option<EventBody>,
    /// The body as the feeder encoded it (postcard bytes, trailing zeros
    /// perhaps trimmed): what is stored.
    pub raw: &'a [u8],
}

/// The kind a body this server cannot read is stored under.
pub(crate) const UNREAD: &str = "unknown";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventRow {
    pub seq: u64,
    pub at: Option<Timestamp>,
    pub received: Timestamp,
    /// `None` for a body this server could not read when it came.
    pub kind: Option<EventKind>,
    pub body: Vec<u8>,
}

/// A stored kind, `None` for a body that came unread, and for a kind a
/// newer server stored that this one does not know: the row reads as
/// unread, as it would have had this server received it, rather than fail
/// every page it is on.
fn stored_kind(code: &str) -> Option<EventKind> {
    if code == UNREAD {
        return None;
    }
    EventKind::from_code(code)
}

impl Store {
    /// Record one event, its projection and the feeder's cursor past it,
    /// together. An event already held changes nothing; whether it was
    /// new. An event of another epoch than the cursor's moves the cursor
    /// to it.
    pub async fn record_event(&self, id: FeederId, event: NewEvent<'_>) -> Result<bool> {
        let epoch = event.epoch;
        let seq = to_i64(event.seq, "event sequence")?;
        let at = event.at.map(|at| at.as_second());
        let received = millis(event.received);
        let kind = event.body.map_or(UNREAD, |body| body.kind().code());

        let mut tx = self.writer.begin().await?;
        let done = sqlx::query!(
            "INSERT INTO events (feeder_id, epoch, seq, at_s, received_ms, kind, body)
             VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
            id.0,
            epoch,
            seq,
            at,
            received,
            kind,
            event.raw,
        )
        .execute(&mut *tx)
        .await?;
        let new = done.rows_affected() == 1;
        if new && let Some(body) = event.body {
            let place = Place {
                epoch,
                seq,
                at_s: at,
                received_ms: received,
            };
            project(&mut tx, id, place, body).await?;
        }
        advance(&mut tx, id, epoch, seq).await?;
        tx.commit().await?;
        Ok(new)
    }

    /// Record that `from..=to` are lost, and move the cursor past them.
    pub async fn record_gap(
        &self,
        id: FeederId,
        epoch: u32,
        from: u64,
        to: u64,
        received: Timestamp,
    ) -> Result<()> {
        let from = to_i64(from, "gap start")?;
        let to = to_i64(to, "gap end")?;
        let received = millis(received);

        let mut tx = self.writer.begin().await?;
        sqlx::query!(
            "INSERT INTO gaps (feeder_id, epoch, from_seq, to_seq, received_ms) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT DO NOTHING",
            id.0,
            epoch,
            from,
            to,
            received,
        )
        .execute(&mut *tx)
        .await?;
        advance(&mut tx, id, epoch, to).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Up to `limit` events of `epoch` after `after` (from the oldest when
    /// `None`), oldest first, of `kind` only when given.
    pub async fn events(
        &self,
        id: FeederId,
        epoch: u32,
        after: Option<u64>,
        limit: u32,
        kind: Option<EventKind>,
    ) -> Result<Vec<EventRow>> {
        let after = after.map_or(Ok(-1), |after| to_i64(after, "cursor"))?;
        let kind = kind.map(EventKind::code);
        let rows = sqlx::query!(
            r#"SELECT seq, at_s, received_ms, kind, body FROM events
               WHERE feeder_id = ?1 AND epoch = ?2 AND seq > ?3 AND (?4 IS NULL OR kind = ?4)
               ORDER BY seq LIMIT ?5"#,
            id.0,
            epoch,
            after,
            kind,
            limit,
        )
        .fetch_all(&self.reader)
        .await?;

        rows.into_iter()
            .map(|row| event(row.seq, row.at_s, row.received_ms, &row.kind, row.body))
            .collect()
    }

    /// The events of `kind` received in `from..to`, of every epoch, oldest
    /// first by the server's clock (the feeder's may have been invalid),
    /// each with its epoch: the kinds no projection holds, such as missed
    /// meals, read by time.
    pub async fn events_received(
        &self,
        id: FeederId,
        kind: EventKind,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<(u32, EventRow)>> {
        let (from, to) = (millis(from), millis(to));
        let kind = kind.code();
        let rows = sqlx::query!(
            r#"SELECT epoch, seq, at_s, received_ms, kind, body FROM events
               WHERE feeder_id = ? AND kind = ? AND received_ms >= ? AND received_ms < ?
               ORDER BY received_ms, epoch, seq"#,
            id.0,
            kind,
            from,
            to,
        )
        .fetch_all(&self.reader)
        .await?;

        rows.into_iter()
            .map(|row| {
                let epoch = event_epoch(row.epoch)?;
                let event = event(row.seq, row.at_s, row.received_ms, &row.kind, row.body)?;
                Ok((epoch, event))
            })
            .collect()
    }

    /// The gaps of `epoch` that end after `after` and at or before
    /// `through`.
    pub async fn gaps(
        &self,
        id: FeederId,
        epoch: u32,
        after: Option<u64>,
        through: u64,
    ) -> Result<Vec<Gap>> {
        let after = after.map_or(Ok(-1), |after| to_i64(after, "cursor"))?;
        let through = to_i64(through, "cursor")?;
        let rows = sqlx::query!(
            "SELECT from_seq, to_seq, received_ms FROM gaps
             WHERE feeder_id = ? AND epoch = ? AND to_seq > ? AND to_seq <= ? ORDER BY from_seq",
            id.0,
            epoch,
            after,
            through,
        )
        .fetch_all(&self.reader)
        .await?;

        rows.into_iter()
            .map(|row| gap(epoch, row.from_seq, row.to_seq, row.received_ms))
            .collect()
    }
}

/// Move the cursor up to `seq`, never back within one epoch; a new epoch
/// starts the cursor again.
async fn advance(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: FeederId,
    epoch: u32,
    seq: i64,
) -> Result<()> {
    sqlx::query!(
        "UPDATE feeders
         SET synced = CASE WHEN epoch IS ?1 THEN MAX(COALESCE(synced, -1), ?2) ELSE ?2 END,
             epoch = ?1
         WHERE id = ?3",
        epoch,
        seq,
        id.0
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// A stored epoch: the feeder's, which is 32 bits.
pub(crate) fn event_epoch(epoch: i64) -> Result<u32> {
    u32::try_from(epoch).map_err(|_| Error::Corrupt("event epoch"))
}

/// An `events` row as the readers take it.
pub(crate) fn event(
    seq: i64,
    at_s: Option<i64>,
    received_ms: i64,
    kind: &str,
    body: Vec<u8>,
) -> Result<EventRow> {
    Ok(EventRow {
        seq: from_i64(seq, "event sequence")?,
        at: seconds(at_s)?,
        received: timestamp(received_ms)?,
        kind: stored_kind(kind),
        body,
    })
}

/// A `gaps` row of `epoch` as the readers take it.
pub(crate) fn gap(epoch: u32, from_seq: i64, to_seq: i64, received_ms: i64) -> Result<Gap> {
    Ok(Gap {
        epoch,
        from: from_i64(from_seq, "gap start")?,
        to: from_i64(to_seq, "gap end")?,
        received: timestamp(received_ms)?,
    })
}
