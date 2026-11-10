//! What the server keeps of each feeder's alerts, in the store's
//! `feeder_alerts`: the table it last read, and what it has made of the
//! tables since, through the same [`Watch`] a phone keeps, so the server
//! opens, holds and ends each alert by the phone's rules and says the
//! feeder is unreachable by the phone's rule too.
//!
//! Two tasks write it: the feeder's alert reader ([`crate::alerts`]) and the
//! webhook, which confirms each change once it has delivered it. Each
//! change reads the row, changes it and writes it over the version it
//! read, and starts again when another write came first ([`change`]), so
//! neither loses the other's.
//!
//! The kept alerts are the server's own judgement, rebuilt from the next
//! read: a row this build can't read is started afresh, not refused. The
//! first table after a fresh start (a feeder just adopted, a store
//! imported, a row reset) is taken quietly ([`Kept::take`]): what is open
//! then is kept as already told, so the webhook neither announces it nor
//! sends it a second time, and still sends its clear. What a reset costs
//! is the clear of an alert that ended before the next read.
//!
//! The row's JSON is [`FORMAT`] 1, pinned by `tests/kept.rs` against a
//! committed sample: it is made of the phone's [`Watch`] and the model's
//! [`AlertsView`], and a change to either changes what this row holds.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tracing::warn;
use wellfed_api::{AlertsRead, FeederAlerts, FeederId};
use wellfed_client::alerts::{
    Change, Context, Cursor, Delivery, Notice, NoticeKey, Tracked, Watch,
};
use wellfed_model::AlertsView;
use wellfed_store::{Expect, Store};

/// The layout of [`Kept`] this build writes and reads.
pub const FORMAT: u32 = 1;

/// How many times a change is tried when other writes keep coming first:
/// far more than the two writers can race.
const TRIES: usize = 8;

/// One feeder's kept alerts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Kept {
    /// The table as last read; `None` before the first read.
    pub table: Option<ReadTable>,
    /// The alerts the server holds open and their changes still to deliver,
    /// as a phone's watch keeps them.
    pub watch: Watch,
    /// The alerts the webhook was told opened and not yet told cleared, so
    /// a clear names what it ends.
    pub hooked: Vec<Tracked>,
}

/// One read of the feeder's table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadTable {
    pub cursor: Cursor,
    pub view: AlertsView,
    /// Whether the status read with it showed a valid clock.
    pub clock_valid: bool,
    /// When the server read it.
    pub at: Timestamp,
}

#[derive(Debug, thiserror::Error)]
pub enum KeptError {
    #[error(transparent)]
    Store(#[from] wellfed_store::Error),
    #[error("encode the kept alerts")]
    Encode(#[source] serde_json::Error),
    #[error("the kept alerts kept changing under the write")]
    Contended,
}

impl Kept {
    /// Take the table `table`. The first after a fresh start seeds the
    /// alerts: each one open is confirmed as told and noted for its clear,
    /// with nothing to deliver, as Home Assistant seeds on its first table.
    /// Only the table's own alerts are seeded: the server's judgement that
    /// the feeder is unreachable is its own, and goes out as it is made.
    pub fn take(&mut self, table: ReadTable, context: &Context<'_>) {
        let first = self.table.is_none();
        self.watch.apply(table.cursor, &table.view, context);
        self.table = Some(table);
        if !first {
            return;
        }

        for change in self.watch.changes() {
            let Change::Post(tracked) = change else {
                continue;
            };
            if !matches!(tracked.key, NoticeKey::Alert { .. }) {
                continue;
            }
            self.watch.confirm(&change);
            self.hooked.retain(|hooked| hooked.key != tracked.key);
            if tracked.delivery != Delivery::Quiet {
                self.hooked.push(tracked);
            }
        }
    }

    /// Since when the server has judged the feeder unreachable, if it has.
    pub fn unreachable(&self) -> Option<Timestamp> {
        self.watch
            .open
            .iter()
            .find_map(|tracked| match tracked.notice {
                Notice::Unreachable { since } if tracked.key == NoticeKey::Unreachable => {
                    Some(since)
                }
                _ => None,
            })
    }

    /// What `GET /feeders/{id}/alerts` answers.
    pub fn body(&self) -> FeederAlerts {
        let read = self.table.as_ref().map(|table| AlertsRead {
            epoch: table.cursor.epoch,
            boot: table.cursor.boot,
            generation: table.cursor.generation,
            more: table.view.more,
            settling: table.view.settling,
            unread: table.view.unread,
            clock_valid: table.clock_valid,
            at: table.at,
            // The service sets the later check it keeps in memory.
            checked: table.at,
        });
        FeederAlerts {
            read,
            alerts: self
                .table
                .as_ref()
                .map(|table| table.view.alerts.clone())
                .unwrap_or_default(),
            unreachable: self.unreachable(),
        }
    }
}

/// The feeder's kept alerts, or the empty ones before any.
pub async fn load(store: &Store, id: FeederId) -> Result<Kept, KeptError> {
    let row = store.feeder_alerts(id).await?;
    Ok(row
        .map(|row| decode(id, row.format, &row.state))
        .unwrap_or_default())
}

/// Every feeder's kept alerts.
pub async fn load_all(store: &Store) -> Result<Vec<(FeederId, Kept)>, KeptError> {
    let rows = store.every_feeder_alerts().await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.feeder, decode(row.feeder, row.format, &row.state)))
        .collect())
}

/// Change the feeder's kept alerts with `apply`, written over what it read:
/// `apply` runs again on a fresh read whenever another write came first, so
/// it must not act outside the [`Kept`] it is given. A change that changed
/// nothing writes nothing, so a poll that finds the table as it was costs
/// the store a read alone.
pub async fn change<R>(
    store: &Store,
    id: FeederId,
    now: Timestamp,
    mut apply: impl FnMut(&mut Kept) -> R,
) -> Result<R, KeptError> {
    for _ in 0..TRIES {
        let row = store.feeder_alerts(id).await?;
        let (mut kept, expect) = match row {
            Some(row) => (decode(id, row.format, &row.state), Expect::At(row.version)),
            None => (Kept::default(), Expect::Absent),
        };
        let before = kept.clone();
        let result = apply(&mut kept);
        if kept == before {
            return Ok(result);
        }

        let state = serde_json::to_string(&kept).map_err(KeptError::Encode)?;
        if store
            .put_feeder_alerts(id, expect, FORMAT, &state, now)
            .await?
        {
            return Ok(result);
        }
    }
    Err(KeptError::Contended)
}

fn decode(id: FeederId, format: u32, state: &str) -> Kept {
    if format != FORMAT {
        warn!(feeder = %id, format, "kept alerts of another layout; starting afresh");
        return Kept::default();
    }
    match serde_json::from_str(state) {
        Ok(kept) => kept,
        Err(error) => {
            warn!(feeder = %id, "kept alerts do not read ({error}); starting afresh");
            Kept::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use wellfed_client::alerts::{FeederClock, Mutes};
    use wellfed_model::{Alert, Generation, Hex, OpenAlert};

    use super::*;

    fn table(generation: u32, alerts: &[Alert]) -> ReadTable {
        ReadTable {
            cursor: Cursor {
                epoch: 1,
                boot: 1,
                generation: Generation(generation),
            },
            view: AlertsView {
                epoch: 1,
                generation: Generation(generation),
                more: false,
                settling: false,
                alerts: alerts
                    .iter()
                    .map(|alert| OpenAlert {
                        at: None,
                        alert: *alert,
                    })
                    .collect(),
                unread: 0,
            },
            clock_valid: true,
            at: Timestamp::UNIX_EPOCH,
        }
    }

    #[test]
    fn the_first_table_is_taken_quietly_and_its_alerts_still_clear() {
        let mutes = Mutes::default();
        let context = Context {
            now: Timestamp::UNIX_EPOCH,
            mutes: &mutes,
            own: Hex([0; 4]),
            clock: FeederClock::Known,
        };
        let mut kept = Kept::default();

        kept.take(table(1, &[Alert::FoodLow]), &context);
        assert!(kept.watch.changes().is_empty(), "nothing to deliver");
        assert_eq!(kept.hooked.len(), 1, "noted for its clear");

        kept.take(table(2, &[Alert::FoodLow, Alert::NoClock]), &context);
        assert!(
            matches!(kept.watch.changes().as_slice(), [Change::Post(tracked)] if tracked.notice.kind() == wellfed_client::alerts::NoticeKind::NoClock),
            "a later one is news"
        );

        kept.take(table(3, &[Alert::NoClock]), &context);
        let food = kept.hooked[0].key;
        assert!(kept.watch.changes().contains(&Change::Remove(food)));
    }
}
