//! Reading again the events a server of an older device API revision
//! stored unread, once the revision rises ([`super::Store::refresh_projections`]).

use sqlx::SqliteConnection;
use uuid::Uuid;
use wellfed_api::{EventBody, FeederId};

use super::{Place, Refresh, project};
use crate::{Error, Result};

/// Read the rows stored unread again: each that `decode` now reads takes
/// its kind and its projection, in history order.
pub(super) async fn reread(
    tx: &mut SqliteConnection,
    decode: &impl Fn(&[u8]) -> Option<EventBody>,
) -> Result<Refresh> {
    let rows = sqlx::query!(
        r#"SELECT feeder_id, epoch, seq, at_s, received_ms, body FROM events
           WHERE kind = ? ORDER BY feeder_id, epoch, seq"#,
        crate::history::UNREAD,
    )
    .fetch_all(&mut *tx)
    .await?;

    let (mut read, mut unread) = (0, 0);
    for row in rows {
        let Some(body) = decode(&row.body) else {
            unread += 1;
            continue;
        };
        take_kind(tx, &row.feeder_id, row.epoch, row.seq, &body).await?;
        let id = Uuid::from_slice(&row.feeder_id).map_err(|_| Error::Corrupt("feeder id"))?;
        let place = Place {
            epoch: u32::try_from(row.epoch).map_err(|_| Error::Corrupt("event epoch"))?,
            seq: row.seq,
            at_s: row.at_s,
            received_ms: row.received_ms,
        };
        project(tx, FeederId(id), place, body).await?;
        read += 1;
    }
    Ok(Refresh::Reread { read, unread })
}

/// Give a row stored unread the kind its body now reads as.
pub(super) async fn take_kind(
    conn: &mut SqliteConnection,
    feeder: &[u8],
    epoch: i64,
    seq: i64,
    body: &EventBody,
) -> Result<()> {
    let kind = body.kind().code();
    sqlx::query!(
        "UPDATE events SET kind = ? WHERE feeder_id = ? AND epoch = ? AND seq = ?",
        kind,
        feeder,
        epoch,
        seq,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}
