//! The server's per-feeder runtime ([API design]): every task that holds a
//! feeder session lives here, and nothing above it sees a socket.
//!
//! - **Each feeder's task** keeps a session open, reconnecting with
//!   jittered backoff and, when its address fails, finding the feeder over
//!   mDNS. On every connect it first subscribes after the newest event the
//!   store holds, so its own outages lose nothing, then sets the feeder's
//!   clock in the household's [`Zone`] and gives it the zone's POSIX rule,
//!   so it follows DST with the server down, and reads its plans. It
//!   reads them again after an event that may have changed them, and is
//!   the only writer of that cache. It records every event and
//!   sample as the feeder sent it: one store transaction per event, with
//!   its projections and the cursor, and samples in small batches, each
//!   with its number.
//! - **Commands** reach the task on two lanes. A short one is
//!   [`Mirror::ask`]; a long operation holds an [`Operation`] and sends its
//!   steps one at a time. The task pipelines the lanes over one session, so
//!   a short command is answered after at most one step. One that is
//!   sent and not answered in time is [`AskError::Unknown`], never a
//!   failure: the feeder may have run it, and a retry under the same
//!   request id gets its first answer. One that waits too long for its turn
//!   is [`AskError::Expired`], never sent.
//!   An install is a long operation: [`Operation::install`] runs the
//!   update driver the owner's CLI runs, a step at a time on the long lane,
//!   and so are "Restore stock" ([`Operation::restore`]) and reading the
//!   stock backup from bank 1 ([`Operation::read_bank1`]), and so is the
//!   meal call's upload ([`Operation::upload_call`]).
//! - **What a feeder sends** goes out on its own broadcast of
//!   [`MirrorEvent`]s (events, gaps, samples and link changes): an event
//!   or gap after it is stored, a sample as it arrives, ahead of its
//!   batch's write. The service merges the broadcasts into its stream.
//! - **Traces and statistics** are read on the long lane, so never ahead of
//!   a feed or a status read: each feed's trace after its `Fed` event, and
//!   every statistics section at each connect and then periodically
//!   ([`Diagnostics`]). Both are stored beside the history.
//! - **Alerts.** Each feeder's alert table is read over its session
//!   whenever its status shows the table moved, a moment after each event
//!   and every [`AlertReads::Read`]'s `poll`, and kept with what the server makes
//!   of it, by a phone's rules ([`Kept`]). Its reach is judged by a phone's
//!   rule too. [`Mirror::alerts_moved`] ticks when there is something to
//!   deliver.
//! - **The debug log** is tailed over a session of its own, opened for the
//!   first owner who follows it ([`Mirror::follow_log`]) and closed once
//!   the last has left. None of it is stored.
//!
//! The mirror knows feeders by id, identity, key and address. It knows
//! nothing of users or stream ids, and the request ids it sends for
//! a caller are the caller's. Time and randomness come in through [`Clock`]
//! and [`Random`], so tests can fix them.
//!
//! [API design]: ../../../../docs/server/design.md#runtime
#![forbid(unsafe_code)]

mod alerts;
mod call;
mod collect;
mod command;
mod config;
mod connect;
mod desk;
mod error;
mod ingest;
mod install;
mod kept;
mod lanes;
mod log;
mod plans;
mod session;
mod task;
mod time;
mod zone;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::{broadcast, watch};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use wellfed_api::{Connection as Link, FeederId, FeederSample, Gap, HistoryEvent};
use wellfed_model::Skew;
use wellfed_session::noise::Keypair;
use wellfed_store::Store;

pub use alerts::AlertReads;
pub use call::{CallError, CallStart, Clip, StreamError, UploadError, UploadFailed};
pub use command::Operation;
pub use config::{Config, Diagnostics, Discovery, Timing};
pub use connect::Enrolled;
pub use desk::AlertHold;
pub use error::{AskError, ConnectError, LogError, Unreachable};
pub use ingest::{stored_body, stored_event, stored_sample};
pub use install::{BankReadError, PushError, Pushed, ReadError, Restore, Update, UpdateError};
pub use kept::{
    FORMAT as KEPT_FORMAT, Kept, KeptError, ReadTable, change as change_kept, load as load_kept,
    load_all as load_all_kept,
};
pub use time::{Clock, Random, RandomFailed, SystemClock, SystemRandom};
pub use wellfed_client::api::{Invite, InviteText};
pub use wellfed_connect::update::Pace;
pub use zone::{Zone, ZoneError};

/// A device request id: the feeder answers a repeated one from memory.
pub type RequestId = wellfed_proto::api::RequestId;

/// How many of a feeder's updates a slow subscriber may fall behind
/// before it is told it lagged.
const UPDATES: usize = 1024;

/// A feeder to mirror: who it is, the server's key on it and where it was
/// last reached.
pub struct Feeder {
    pub id: FeederId,
    pub identity: [u8; 32],
    /// The server's secret key on this feeder, unsealed.
    pub secret: [u8; 32],
    pub address: Option<String>,
}

impl std::fmt::Debug for Feeder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Feeder")
            .field("id", &self.id)
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

impl Feeder {
    /// The server's public key on the feeder: how its client list shows
    /// the server.
    pub fn public(&self) -> [u8; 32] {
        *Keypair::from_secret(self.secret).public()
    }

    /// How the feeder's history names the server (`Source::Client`): the
    /// first 4 bytes of the server's public key on it.
    pub fn fingerprint(&self) -> [u8; 4] {
        let public = self.public();
        [public[0], public[1], public[2], public[3]]
    }
}

/// One update from a feeder: an event or gap after it is stored, a sample
/// as it arrives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MirrorEvent {
    /// An event the store did not hold before.
    Event(HistoryEvent),
    /// Events the feeder dropped before the server read them.
    Gap(Gap),
    Sample(FeederSample),
    /// The session's state changed.
    Link(Link),
    /// The kept alerts changed: a new table, or the feeder's reach.
    Alerts,
}

/// What every feeder task shares.
pub(crate) struct Shared {
    store: Store,
    config: Config,
    clock: Arc<dyn Clock>,
    random: Arc<dyn Random>,
    /// The household's zone; `None` before there is a household, when the
    /// clock syncs are in UTC.
    zone: watch::Sender<Option<Arc<Zone>>>,
    /// Ticks whenever a feeder's kept alerts have changes to deliver.
    alerts_moved: watch::Sender<u64>,
    /// What each alert reader knows beyond its row.
    desk: desk::Desk,
}

impl Shared {
    fn random<const N: usize>(&self) -> Result<[u8; N], RandomFailed> {
        let mut out = [0; N];
        self.random.fill(&mut out)?;
        Ok(out)
    }
}

/// The running feeder tasks: cheap to clone, one per server.
#[derive(Clone)]
pub struct Mirror {
    shared: Arc<Shared>,
    feeders: Arc<Mutex<HashMap<FeederId, task::Handle>>>,
    /// Held by a start or a stop across the wait for a task to end.
    turns: Arc<tokio::sync::Mutex<()>>,
    cancel: CancellationToken,
    tasks: TaskTracker,
}

impl std::fmt::Debug for Mirror {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mirror").finish_non_exhaustive()
    }
}

impl Mirror {
    /// A mirror with no feeders yet. Cancelling `cancel` stops every task
    /// it starts.
    pub fn new(
        store: Store,
        config: Config,
        clock: Arc<dyn Clock>,
        random: Arc<dyn Random>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                store,
                config,
                clock,
                random,
                zone: watch::Sender::new(None),
                alerts_moved: watch::Sender::new(0),
                desk: desk::Desk::default(),
            }),
            feeders: Arc::new(Mutex::new(HashMap::new())),
            turns: Arc::new(tokio::sync::Mutex::new(())),
            cancel,
            tasks: TaskTracker::new(),
        }
    }

    /// Start mirroring `feeder`, and subscribe to its updates from the
    /// first. A feeder already running is stopped first, and its task has
    /// ended before the new one starts, so two sessions never overlap:
    /// starts and stops take turns, so none runs while another waits for
    /// a task to end.
    pub async fn start(&self, feeder: Feeder) -> broadcast::Receiver<MirrorEvent> {
        let _turn = self.turns.lock().await;
        let id = feeder.id;
        let running = self.handles().remove(&id);
        if let Some(old) = running {
            old.stop().await;
            self.shared.desk.forget(id);
        }

        let (updates, first) = broadcast::channel(UPDATES);
        let handle = task::spawn(
            Arc::clone(&self.shared),
            feeder,
            updates,
            self.cancel.child_token(),
            &self.tasks,
        );
        self.handles().insert(id, handle);
        first
    }

    /// Stop mirroring a feeder and wait until its task has ended; whether
    /// it was running. Its last update is `Link::Offline`, then its
    /// broadcast closes.
    pub async fn stop(&self, id: FeederId) -> bool {
        let _turn = self.turns.lock().await;
        let handle = self.handles().remove(&id);
        let Some(handle) = handle else {
            return false;
        };
        handle.stop().await;
        self.shared.desk.forget(id);
        true
    }

    /// Set every feeder's clock in `zone` from now on, and give each the
    /// zone's rule: an online feeder at once, the others when they connect.
    pub fn set_zone(&self, zone: Zone) {
        self.shared.zone.send_replace(Some(Arc::new(zone)));
    }

    /// The household's zone, as the feeders keep it; `None` before there
    /// is a household, or when its stored name is no longer known.
    pub fn zone(&self) -> Option<Arc<Zone>> {
        self.shared.zone.borrow().clone()
    }

    /// The feeder's session state, or `None` when it is not mirrored.
    pub fn link(&self, id: FeederId) -> Option<Link> {
        self.handles().get(&id).map(task::Handle::link)
    }

    /// Wait up to `within` for the feeder's session to be up: whether it
    /// is. A feeder just started connects in the background, and asks
    /// before then are refused offline.
    pub async fn online(&self, id: FeederId, within: Duration) -> bool {
        let Some(mut link) = self.handles().get(&id).map(|handle| handle.link.clone()) else {
            return false;
        };
        let up = link.wait_for(|link| *link == Link::Online);
        matches!(tokio::time::timeout(within, up).await, Ok(Ok(_)))
    }

    /// The epoch the feeder's open session counts its history in, or
    /// `None` while it has none: what the newest event of its status
    /// counts in.
    pub fn epoch(&self, id: FeederId) -> Option<u32> {
        self.handles().get(&id).and_then(task::Handle::epoch)
    }

    /// How the feeder's API revision stands against this server's, from
    /// its last welcome: `None` before one.
    pub fn skew(&self, id: FeederId) -> Option<Skew> {
        self.handles().get(&id).and_then(task::Handle::skew)
    }

    /// Whether the feeder has an open session, welcomed.
    pub fn session_open(&self, id: FeederId) -> bool {
        self.handles().get(&id).is_some_and(task::Handle::is_open)
    }

    /// Ticks whenever a feeder's kept alerts have changes to deliver, for
    /// whatever delivers them.
    pub fn alerts_moved(&self) -> watch::Receiver<u64> {
        self.shared.alerts_moved.subscribe()
    }

    /// When the feeder's alert table was last found unchanged, or read:
    /// kept in memory, since a poll that finds it unchanged writes nothing.
    pub fn alerts_checked(&self, id: FeederId) -> Option<jiff::Timestamp> {
        self.shared.desk.checked(id)
    }

    /// Stop judging the feeder's alerts while it is removed, or start
    /// again if its removal failed: its removal revokes the server's key,
    /// and a reader that judged that would send news of a feeder the owner
    /// is removing that its archive then drops, its clear included.
    pub fn hold_alerts(&self, id: FeederId, hold: AlertHold) {
        self.shared.desk.hold(id, hold);
    }

    /// The store the mirror keeps every feeder in.
    pub fn store(&self) -> &Store {
        &self.shared.store
    }

    /// Follow a feeder's updates from now on.
    pub fn subscribe(&self, id: FeederId) -> Option<broadcast::Receiver<MirrorEvent>> {
        self.handles().get(&id).map(task::Handle::subscribe)
    }

    /// Stop every task and wait for them.
    pub async fn shutdown(&self) {
        self.cancel.cancel();
        self.tasks.close();
        self.tasks.wait().await;
    }

    fn handles(&self) -> MutexGuard<'_, HashMap<FeederId, task::Handle>> {
        // Insertions and removals are whole, so a poisoned map is still
        // consistent.
        self.feeders.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
