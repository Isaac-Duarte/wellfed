//! Ingest: what the feeder pushes, stored as it sent it and broadcast. An
//! event is stored (with its projection and the cursor, in one
//! transaction) before it is broadcast, and a store failure ends the
//! session, so the reconnect replays whatever was not stored.
//!
//! Samples are broadcast at once and written in small batches: a batch is
//! written when it holds [`SAMPLE_BATCH`] samples, when its oldest has
//! waited [`Timing::sample_wait`], and when the session ends. The feeder never
//! replays a sample, so the batch is all that a failed write loses; it
//! ends the session like any store failure.
//!
//! Event bodies and sample values are kept in the feeder's own encoding
//! (postcard of the wire types), and [`stored_event`] and [`stored_sample`]
//! are the way back. The store projects the decoded event it is handed and
//! numbers the decoded sample, and rebuilds both from the stored bytes
//! through [`Mirror::refresh_projections`] and [`Mirror::refresh_samples`].
//!
//! [`Timing::sample_wait`]: crate::Timing::sample_wait

use std::sync::Arc;

use jiff::Timestamp;
use tokio::time::Instant;
use tracing::{error, info, warn};
use wellfed_api::{FeederId, FeederSample, Gap, HistoryEvent, SampleValue};
use wellfed_model::{Event, EventBody, Pushed};
use wellfed_proto::api as proto;
use wellfed_store::{EventRow, NewEvent, NewSample, Refresh, SampleRefresh};

use crate::session::{End, Failure};
use crate::task::Task;
use crate::{Mirror, MirrorEvent};

/// Room for an encoded event body (16 bytes on the feeder) or sample.
const RAW: usize = 32;

/// The most samples a batch holds before it is written.
pub(crate) const SAMPLE_BATCH: usize = 32;

/// The samples received and not yet written, and when they must be.
#[derive(Debug, Default)]
pub(crate) struct Batch {
    samples: Vec<NewSample>,
    due: Option<Instant>,
}

impl Batch {
    /// When the batch must be written; `None` while it is empty.
    pub(crate) const fn due(&self) -> Option<Instant> {
        self.due
    }
}

/// A stored event of `epoch` as the feeder sent it, or `None` when its
/// body does not decode. The raw bytes decide, not the stored kind: a row
/// stored unread by an older server is read by any build that knows it.
pub fn stored_event(epoch: u32, row: &EventRow) -> Option<HistoryEvent> {
    let event = Event {
        epoch,
        seq: row.seq,
        at: row.at,
        body: stored_body(&row.body)?,
    };
    Some(HistoryEvent::new(event, row.received))
}

/// A stored body as the event it encodes, or `None` when it does not
/// decode. A body that came unread is stored as the feeder's ring keeps it,
/// trailing zeros trimmed, so it is read padded back to the ring's size.
pub fn stored_body(raw: &[u8]) -> Option<EventBody> {
    let mut padded = [0; proto::EVENT_BODY_MAX];
    padded.get_mut(..raw.len())?.copy_from_slice(raw);
    let body: proto::EventBody = wellfed_proto::decode_padded(&padded)?;
    Some(EventBody::from(body))
}

/// A stored sample's bytes as the value they encode, or `None` when they
/// do not decode.
pub fn stored_sample(raw: &[u8]) -> Option<SampleValue> {
    let metric: proto::Metric = wellfed_proto::decode(raw).ok()?;
    Some(SampleValue::from(metric))
}

impl Mirror {
    /// Rebuild the store's projections from the stored events when the
    /// projection code changed since they were built. Call it before
    /// starting any feeder, so ingest never writes beside a stale build.
    pub async fn refresh_projections(&self) -> wellfed_store::Result<Refresh> {
        let refresh = self
            .shared
            .store
            .refresh_projections(stored_body, proto::REVISION)
            .await?;
        match refresh {
            Refresh::Rebuilt { events, undecoded } => {
                info!(events, "projections rebuilt from the stored events");
                if undecoded > 0 {
                    warn!(
                        undecoded,
                        "stored events that do not decode project nothing"
                    );
                }
            }
            Refresh::Reread { read, unread } => {
                info!(
                    read,
                    unread, "events stored unread were read again by this device API revision"
                );
            }
            Refresh::Current => {}
        }
        Ok(refresh)
    }

    /// Number the stored samples again when what a number is changed
    /// since they were numbered, then bring the rollups up to date. Call
    /// it before starting any feeder, like [`Self::refresh_projections`].
    pub async fn refresh_samples(&self) -> wellfed_store::Result<SampleRefresh> {
        let store = &self.shared.store;
        let refresh = store.refresh_samples(stored_sample).await?;
        if let SampleRefresh::Decoded { samples, undecoded } = refresh
            && samples > 0
        {
            info!(samples, "stored samples numbered again");
            if undecoded > 0 {
                warn!(
                    undecoded,
                    "stored samples that do not decode have no number"
                );
            }
            store.roll_up_samples().await?;
        }
        Ok(refresh)
    }
}

/// Store `event` with its projections and the cursor; whether it is new
/// and stored readable, so it is broadcast.
///
/// A store error ends the session, so the reconnect replays what was not
/// stored. An event refused for what it carries would be refused again at
/// every reconnect, which subscribes after the same cursor, and nothing
/// after it would ever be stored: so one that does not encode, or whose
/// body the store refuses, is kept unread, as a newer build's event is,
/// and the cursor moves past it. One refused even so is left out, and the
/// session goes on.
async fn record_event(
    store: &wellfed_store::Store,
    id: FeederId,
    event: &Event,
    received: Timestamp,
) -> Result<bool, End> {
    let unread = |raw| NewEvent {
        epoch: event.epoch,
        seq: event.seq,
        at: event.at,
        received,
        body: None,
        raw,
    };
    let mut raw = [0u8; RAW];
    let encoded = proto::EventBody::try_from(event.body)
        .ok()
        .and_then(|body| wellfed_proto::encode(&body, &mut raw).ok());
    let refused = match encoded {
        Some(len) => {
            let readable = NewEvent {
                body: Some(event.body),
                ..unread(&raw[..len])
            };
            match store.record_event(id, readable).await {
                Ok(new) => return Ok(new),
                Err(error) if error.refuses_value() => error,
                Err(error) => return Err(error.into()),
            }
        }
        None => {
            error!(seq = event.seq, body = ?event.body, "an event with no wire form is kept unread");
            match store.record_event(id, unread(&[])).await {
                Ok(_) => return Ok(false),
                Err(error) if error.refuses_value() => error,
                Err(error) => return Err(error.into()),
            }
        }
    };
    error!(
        seq = event.seq,
        "the store refused an event ({refused}); it is kept unread"
    );
    match store.record_event(id, unread(&raw[..0])).await {
        Ok(_) => Ok(false),
        Err(error) if error.refuses_value() => {
            error!(
                seq = event.seq,
                "the store refused an event even unread ({error}); it is left out"
            );
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

impl Task {
    /// Store what the feeder pushed, or batch it if a sample, then
    /// broadcast it.
    pub(crate) async fn apply(&mut self, pushed: Pushed) -> Result<(), End> {
        let shared = Arc::clone(&self.shared);
        let store = &shared.store;
        let received = self.shared.clock.now();
        let mut raw = [0u8; RAW];
        let update = match pushed {
            Pushed::Event(event) => {
                if !record_event(store, self.id, &event, received).await? {
                    return Ok(());
                }
                // The session loop reads them again after this step: an
                // event may arrive inside another request's wait, where a
                // request of the task's own cannot go out.
                if self.changes_plans(&event.body) {
                    self.plans_stale = true;
                }
                MirrorEvent::Event(HistoryEvent::new(event, received))
            }
            // A newer build's event, which this server cannot read: kept
            // as the ring holds it, so a later server reads it, and the
            // cursor moves past it. Nothing is broadcast: no client of this
            // server's build could show it.
            Pushed::Unknown(event) => {
                let used = event
                    .raw
                    .0
                    .iter()
                    .rposition(|&b| b != 0)
                    .map_or(0, |last| last + 1);
                store
                    .record_event(
                        self.id,
                        NewEvent {
                            epoch: event.epoch,
                            seq: event.seq,
                            at: event.at,
                            received,
                            body: None,
                            raw: event.raw.0.get(..used).unwrap_or_default(),
                        },
                    )
                    .await?;
                return Ok(());
            }
            Pushed::Gap(gap) => {
                let (epoch, from, to) = (gap.epoch, gap.from, gap.to);
                warn!(from, to, "events lost before the server read them");
                store.record_gap(self.id, epoch, from, to, received).await?;
                MirrorEvent::Gap(Gap {
                    epoch,
                    from,
                    to,
                    received,
                })
            }
            Pushed::Sample(sample) => {
                // A sample from the wire always has a wire form.
                let metric = proto::Metric::try_from(sample.value)
                    .map_err(|_| Failure::Encode)
                    .map_err(End::Failed)?;
                let len = wellfed_proto::encode(&metric, &mut raw)
                    .map_err(|_| Failure::Encode)
                    .map_err(End::Failed)?;
                let batch = &mut self.batch;
                batch.samples.push(NewSample {
                    received,
                    uptime_ms: sample.uptime_ms,
                    value: sample.value,
                    raw: raw[..len].to_vec(),
                });
                let wait = shared.config.timing.sample_wait;
                batch.due.get_or_insert_with(|| Instant::now() + wait);
                if batch.samples.len() >= SAMPLE_BATCH {
                    self.write_samples().await?;
                }
                MirrorEvent::Sample(FeederSample::new(sample, received))
            }
            // This session never asks for the log: a tail has a session
            // of its own (`log.rs`).
            Pushed::Log(_) => return Ok(()),
        };
        // No subscriber is fine: the store holds it.
        let _ = self.updates.send(update);
        Ok(())
    }

    /// Write the batch of samples, emptying it whether or not the write
    /// succeeds: the feeder will not send them again.
    pub(crate) async fn write_samples(&mut self) -> Result<(), End> {
        let batch = std::mem::take(&mut self.batch);
        if batch.samples.is_empty() {
            return Ok(());
        }
        self.shared
            .store
            .record_samples(self.id, &batch.samples)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;
    use wellfed_api::{EventKind, Model};
    use wellfed_model::{ClientRole, CoreEvent, EnrollPath, Hex};
    use wellfed_server_testkit::fixed::FixedClock;
    use wellfed_server_testkit::{TempStore, home, local};
    use wellfed_store::{NewFeeder, SealedKey, Store, Synced};

    use super::*;

    const FEEDER: FeederId = FeederId(Uuid::from_u128(7));

    async fn adopted(store: &Store) {
        store.create_home(&home(), FixedClock.now()).await.unwrap();
        store
            .insert_feeder(NewFeeder {
                id: FEEDER,
                home: home().id,
                model: Model::Plaf103,
                identity: [1; 32],
                name: "kitchen",
                role: wellfed_api::ClientRole::Server,
                address: None,
                key: SealedKey {
                    nonce: [0; 12],
                    sealed: Vec::new(),
                },
                created: FixedClock.now(),
            })
            .await
            .unwrap();
    }

    fn event(seq: u64, body: EventBody) -> Event {
        Event {
            epoch: 3,
            seq,
            at: None,
            body,
        }
    }

    /// An event that would be refused at every reconnect never ends the
    /// session: one with no wire form (a role this build cannot name) is
    /// kept unread and the cursor moves past it, so the next event is
    /// stored readable; one whose number the database cannot hold is left
    /// out.
    #[test]
    fn an_event_the_store_would_always_refuse_is_kept_unread_or_left_out() {
        local(async {
            let temp = TempStore::new();
            let store = temp.open().await;
            adopted(&store).await;
            let received = FixedClock.now();
            let unnamed = EventBody::Core(CoreEvent::Enrolled {
                client: Hex([9; 4]),
                role: ClientRole::Unknown,
                path: EnrollPath::Invite,
            });

            let kept = record_event(&store, FEEDER, &event(1, unnamed), received).await;
            assert!(matches!(kept, Ok(false)), "kept, but not broadcast");
            let next = EventBody::Core(CoreEvent::Restarting);
            let stored = record_event(&store, FEEDER, &event(2, next), received).await;
            assert!(matches!(stored, Ok(true)));
            let rows = store.events(FEEDER, 3, None, 10, None).await.unwrap();
            let kinds: Vec<_> = rows.iter().map(|row| (row.seq, row.kind)).collect();
            assert_eq!(kinds, [(1, None), (2, Some(EventKind::Restarting))]);

            let past = event(u64::MAX, EventBody::Core(CoreEvent::Restarting));
            let left = record_event(&store, FEEDER, &past, received).await;
            assert!(matches!(left, Ok(false)), "left out, not a failure");
            let row = store.feeder(FEEDER).await.unwrap().unwrap();
            assert_eq!(row.synced, Some(Synced { epoch: 3, seq: 2 }));
            store.close().await;
        });
    }

    /// A stored body longer than the ring's 16 bytes is no body the feeder
    /// sent: it reads as undecodable rather than panicking or truncating.
    #[test]
    fn a_stored_body_past_the_ring_s_size_does_not_decode() {
        let mut raw = vec![0; proto::EVENT_BODY_MAX + 1];
        let len = wellfed_proto::encode(&proto::EventBody::Restarting, &mut raw).unwrap();
        assert!(stored_body(&raw[..len]).is_some(), "the short form reads");

        assert_eq!(stored_body(&raw), None);
    }
}
