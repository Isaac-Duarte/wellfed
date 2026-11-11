//! The alert webhook ([server docs]): when a feeder's alert opens or
//! clears, POST a small JSON body ([`Delivery`]) to the owner's URL. Home
//! Assistant, or anything else that takes a webhook, needs nothing of its
//! own for it.
//!
//! What opens and clears is the server's kept judgement of each feeder's
//! table ([`wellfed_mirror::Kept`]), made by a phone's own rules: the
//! changes its watch hands over are exactly what a phone would post and
//! take off. The worker delivers them one feeder at a time, in order, and
//! confirms each in the kept alerts only once it is delivered or given up,
//! so a change is not sent again however often the table is read, and a
//! server stopped mid-delivery sends it at its next start.
//!
//! - **At least once, under one id.** A change confirmed is never sent
//!   again. A try that failed is retried after [`BACKOFF`], at most
//!   [`TRIES`] in all, and every try of a change carries the same `id`. A
//!   try whose answer was lost may still have arrived, and a server
//!   stopped between a delivery and its confirmation sends it again at its
//!   next start, so a receiver may see one change twice: one that keeps
//!   the ids it has seen never acts on it twice.
//! - **Pairs by key.** An alert's `opened` and `cleared` carry the same
//!   key. An alert whose kind changes while open (an uncertain meal found
//!   failed) is `opened` again under the same key, with a new id: for each
//!   key, the latest event is the alert's state.
//! - **In order.** A feeder's next change waits while one of its changes
//!   waits to be tried again, so a clear never arrives before its open.
//! - **Off the feeder's path.** The worker is a task of its own, reading
//!   the kept alerts from the store; a slow or dead receiver delays only
//!   the next delivery.
//! - **Only what it opened.** A clear goes out only for an alert whose
//!   opening the receiver took (a 2xx answer), or one open at the first
//!   table the server read ([`wellfed_mirror::Kept::take`]). With no
//!   webhook set, every change is confirmed unsent, so an alert open when a
//!   webhook is first set is not sent until it opens again.
//! - **Not for a removed feeder.** A feeder the owner removed is sent
//!   nothing more, not even the clears of what was open: its kept alerts
//!   are dropped, and a feeder adopted again starts afresh.
//!
//! [server docs]: ../../../../docs/server/README.md#alert-webhook

mod body;
mod target;
mod told;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use wellfed_api::FeederId;
use wellfed_client::alerts::Change;
use wellfed_mirror::{Clock, Kept};
use wellfed_store::Store;

pub use reqwest::Url;

pub use body::{AlertRef, Delivery, FeederRef, Transition, VERSION};
pub use target::{PlainHttp, Target, TargetError};
use told::{Failure, Told, delivery_id, hook, refused, sendable, without_url};

/// Tries of one change before it is given up.
pub const TRIES: u32 = 6;
/// The wait after each failed try: about 20 minutes from the first try to
/// the last, past a receiver's restart or a router's.
pub const BACKOFF: [Duration; 5] = [
    Duration::from_secs(10),
    Duration::from_secs(30),
    Duration::from_secs(90),
    Duration::from_secs(270),
    Duration::from_secs(810),
];
/// The longest one try waits for an answer.
const TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("build the webhook's HTTP client")]
    Client(#[source] reqwest::Error),
}

/// A change still to deliver, as tried so far.
#[derive(Clone, Copy, Debug)]
struct Pending {
    tries: u32,
    next: Instant,
    /// When the server first saw the change.
    time: Timestamp,
}

/// What one try came to.
enum Outcome {
    /// Delivered, given up, or nothing to send: confirm it.
    Done(Told),
    /// Try again at this instant.
    Later(Instant),
    /// The feeder was removed: nothing more goes out for it.
    Gone,
}

/// The worker's state between passes.
pub struct Webhook {
    store: Store,
    target: Option<Target>,
    http: reqwest::Client,
    clock: Arc<dyn Clock>,
    /// Ticks when a feeder's kept alerts have changes to deliver.
    moved: watch::Receiver<u64>,
    /// Changes tried and not yet done, by feeder and delivery id.
    pending: HashMap<(FeederId, String), Pending>,
    /// The changes this pass looked at, so one gone from the kept alerts
    /// meanwhile (ended before its retry, or its feeder removed) is
    /// forgotten.
    live: HashSet<(FeederId, String)>,
    /// The wait after each failed try: [`BACKOFF`], shorter in tests.
    backoff: Vec<Duration>,
}

impl Webhook {
    pub fn new(
        store: Store,
        target: Option<Target>,
        clock: Arc<dyn Clock>,
        moved: watch::Receiver<u64>,
    ) -> Result<Self, Error> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("wellfed-server/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(TIMEOUT)
            // A redirect could carry the body somewhere the owner never
            // named.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(Error::Client)?;
        if let Some(target) = &target {
            info!(host = target.host(), "alert webhook set");
        }
        Ok(Self {
            store,
            target,
            http,
            clock,
            moved,
            pending: HashMap::new(),
            live: HashSet::new(),
            backoff: BACKOFF.to_vec(),
        })
    }

    /// Deliver until `cancel`: a pass at once, then each time the kept
    /// alerts move or a retry falls due.
    pub async fn run(mut self, cancel: CancellationToken) {
        loop {
            let next = self.pass().await;
            tokio::select! {
                biased;
                () = cancel.cancelled() => return,
                changed = self.moved.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                () = sleep_until_some(next), if next.is_some() => {}
            }
        }
    }

    /// One pass over every feeder's changes; when the next retry is due.
    async fn pass(&mut self) -> Option<Instant> {
        let kept = match wellfed_mirror::load_all_kept(&self.store).await {
            Ok(kept) => kept,
            Err(error) => {
                warn!("read the kept alerts: {error}");
                return Some(Instant::now() + self.wait(0));
            }
        };

        // Every change still in the kept alerts keeps its tries, the ones
        // behind one waiting to be tried again included: only a change gone
        // from them is forgotten.
        self.live.clear();
        for (feeder, kept) in &kept {
            for change in kept.watch.changes() {
                if let Some((transition, tracked)) = sendable(kept, &change) {
                    let id = delivery_id(*feeder, transition, &tracked);
                    self.live.insert((*feeder, id));
                }
            }
        }

        let mut next: Option<Instant> = None;
        for (feeder, kept) in kept {
            for change in kept.watch.changes() {
                match self.deliver(feeder, &kept, &change).await {
                    Outcome::Done(told) => self.confirm(feeder, &change, told).await,
                    Outcome::Later(at) => {
                        next = Some(next.map_or(at, |next| next.min(at)));
                        break;
                    }
                    Outcome::Gone => {
                        self.forget(feeder).await;
                        break;
                    }
                }
            }
        }
        let live = &self.live;
        self.pending.retain(|key, _| live.contains(key));
        next
    }

    /// Try `change` once, if it is due and there is anything to send.
    async fn deliver(&mut self, feeder: FeederId, kept: &Kept, change: &Change) -> Outcome {
        let Some(target) = self.target.clone() else {
            return Outcome::Done(Told::Not);
        };
        let Some((transition, tracked)) = sendable(kept, change) else {
            return Outcome::Done(Told::Not);
        };

        let id = delivery_id(feeder, transition, &tracked);
        let now = Instant::now();
        let pending = *self
            .pending
            .entry((feeder, id.clone()))
            .or_insert_with(|| Pending {
                tries: 0,
                next: now,
                time: match transition {
                    Transition::Opened => tracked.seen,
                    Transition::Cleared => self.clock.now(),
                },
            });
        if pending.next > now {
            return Outcome::Later(pending.next);
        }

        let name = match self.store.feeder(feeder).await {
            Ok(Some(row)) if row.archived.is_none() => row.name,
            Ok(_) => return Outcome::Gone,
            Err(error) => {
                warn!(%feeder, "read the feeder for the webhook: {error}");
                return Outcome::Later(now + self.wait(0));
            }
        };
        let body = Delivery {
            version: VERSION,
            id: id.clone(),
            event: transition,
            time: pending.time,
            feeder: FeederRef { id: feeder, name },
            alert: AlertRef::new(&tracked.key, &tracked.notice),
        };

        let tries = pending.tries.saturating_add(1);
        let done = match self.post(&target, &body).await {
            Ok(()) => {
                debug!(%feeder, event = transition.name(), key = body.alert.key, "webhook delivered");
                Some(Told::Took)
            }
            Err(Failure::Final(reason)) => {
                warn!(%feeder, host = target.host(), "webhook refused, not tried again: {reason}");
                Some(Told::Not)
            }
            Err(Failure::Again(reason)) if tries >= TRIES => {
                warn!(%feeder, host = target.host(), tries, "webhook given up: {reason}");
                Some(Told::Not)
            }
            Err(Failure::Again(reason)) => {
                info!(%feeder, host = target.host(), tries, "webhook try failed: {reason}");
                None
            }
        };
        if let Some(told) = done {
            self.pending.remove(&(feeder, id));
            return Outcome::Done(told);
        }

        let wait = self.wait(usize::try_from(tries - 1).unwrap_or(usize::MAX));
        let next = Instant::now() + wait;
        self.pending.insert(
            (feeder, id),
            Pending {
                tries,
                next,
                ..pending
            },
        );
        Outcome::Later(next)
    }

    async fn post(&self, target: &Target, body: &Delivery) -> Result<(), Failure> {
        let response = self
            .http
            .post(target.url().clone())
            .json(body)
            .send()
            .await
            // The request never reached the receiver, or its answer was
            // lost: the same id goes again.
            .map_err(|error| Failure::Again(without_url(error)))?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        Err(refused(status))
    }

    /// The wait after the `after`th failed try.
    fn wait(&self, after: usize) -> Duration {
        self.backoff
            .get(after)
            .or(self.backoff.last())
            .copied()
            .unwrap_or_default()
    }

    /// Mark `change` done in the feeder's kept alerts, and note whether
    /// the receiver took it, so a later clear goes only where an open did.
    async fn confirm(&self, feeder: FeederId, change: &Change, told: Told) {
        let now = self.clock.now();
        let confirmed = wellfed_mirror::change_kept(&self.store, feeder, now, |kept| {
            kept.watch.confirm(change);
            hook(&mut kept.hooked, change, told);
        })
        .await;
        if let Err(error) = confirmed {
            warn!(%feeder, "confirm a webhook delivery: {error}");
        }
    }

    /// Drop a removed feeder's kept alerts, sending nothing: the owner
    /// removed it, and a clear of each alert would be news of a feeder
    /// they no longer hold. The store drops them as it archives the feeder;
    /// this catches a reader's last write after that.
    async fn forget(&self, feeder: FeederId) {
        if let Err(error) = self.store.delete_feeder_alerts(feeder).await {
            warn!(%feeder, "drop a removed feeder's alerts: {error}");
        }
    }
}

/// Sleep until `at`; `None` is never selected (its branch is disabled).
async fn sleep_until_some(at: Option<Instant>) {
    if let Some(at) = at {
        tokio::time::sleep_until(at).await;
    }
}

#[cfg(test)]
mod tests;
