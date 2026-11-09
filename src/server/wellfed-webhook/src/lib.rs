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
//! so a change is sent once however often the table is read, and a server
//! stopped mid-delivery sends it at its next start.
//!
//! - **Once per change.** A change confirmed is never sent again. A try
//!   that failed is retried after [`BACKOFF`], at most [`TRIES`] in all,
//!   and every try of a change carries the same `id`. A try whose answer
//!   was lost may still have arrived, so a receiver that keeps the ids it
//!   has seen never sees a change twice.
//! - **In order.** A feeder's next change waits while one of its changes
//!   waits to be tried again, so a clear never arrives before its open.
//! - **Off the feeder's path.** The worker is a task of its own, reading
//!   the kept alerts from the store; a slow or dead receiver delays only
//!   the next delivery.
//! - **Only what it opened.** A clear goes out only for an alert the
//!   webhook was told opened. With no webhook set, every change is
//!   confirmed unsent, so an alert open when a webhook is first set is not
//!   sent until it opens again.
//!
//! [server docs]: ../../../../docs/server/README.md#alert-webhook

mod body;
mod target;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use reqwest::StatusCode;
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use wellfed_api::FeederId;
use wellfed_client::alerts::{Change, Delivery as Shown, NoticeKey, Tracked};
use wellfed_formats::canonical::{canonical_of, sha256};
use wellfed_mirror::{Clock, Kept};
use wellfed_store::Store;

pub use reqwest::Url;

pub use body::{AlertRef, Delivery, FeederRef, Transition, VERSION};
pub use target::{PlainHttp, Target, TargetError};

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
    #[error("build the webhook's HTTP client: {0}")]
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
    Done,
    /// Try again at this instant.
    Later(Instant),
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
                return Some(Instant::now() + BACKOFF[0]);
            }
        };

        let mut next: Option<Instant> = None;
        for (feeder, kept) in kept {
            for change in kept.watch.changes() {
                match self.deliver(feeder, &kept, &change).await {
                    Outcome::Done => self.confirm(feeder, &change).await,
                    Outcome::Later(at) => {
                        next = Some(next.map_or(at, |next| next.min(at)));
                        break;
                    }
                }
            }
        }
        next
    }

    /// Try `change` once, if it is due and there is anything to send.
    async fn deliver(&mut self, feeder: FeederId, kept: &Kept, change: &Change) -> Outcome {
        let Some(target) = self.target.clone() else {
            return Outcome::Done;
        };
        let (transition, tracked) = match change {
            Change::Post(tracked) => (Transition::Opened, *tracked),
            Change::Remove(key) => match kept.hooked.iter().find(|hooked| hooked.key == *key) {
                Some(hooked) => (Transition::Cleared, *hooked),
                // Never sent as opened: nothing to clear.
                None => return Outcome::Done,
            },
        };
        if tracked.delivery == Shown::Quiet {
            return Outcome::Done;
        }

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
            Ok(Some(row)) => row.name,
            Ok(None) => return Outcome::Done,
            Err(error) => {
                warn!(%feeder, "read the feeder for the webhook: {error}");
                return Outcome::Later(now + BACKOFF[0]);
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
        let retry = match self.post(&target, &body).await {
            Ok(()) => {
                debug!(%feeder, event = transition.name(), key = body.alert.key, "webhook delivered");
                false
            }
            Err(Failure::Final(reason)) => {
                warn!(%feeder, host = target.host(), "webhook refused, not tried again: {reason}");
                false
            }
            Err(Failure::Again(reason)) if tries >= TRIES => {
                warn!(%feeder, host = target.host(), tries, "webhook given up: {reason}");
                false
            }
            Err(Failure::Again(reason)) => {
                info!(%feeder, host = target.host(), tries, "webhook try failed: {reason}");
                true
            }
        };
        if !retry {
            self.pending.remove(&(feeder, id));
            return Outcome::Done;
        }

        let wait = usize::try_from(tries - 1)
            .ok()
            .and_then(|at| BACKOFF.get(at))
            .copied()
            .unwrap_or(BACKOFF[BACKOFF.len() - 1]);
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
        let again = status == StatusCode::REQUEST_TIMEOUT
            || status == StatusCode::TOO_MANY_REQUESTS
            || status.is_server_error();
        let reason = format!("answered {status}");
        Err(if again {
            Failure::Again(reason)
        } else {
            Failure::Final(reason)
        })
    }

    /// Mark `change` done in the feeder's kept alerts, and note what the
    /// webhook was told, so a later clear names it.
    async fn confirm(&self, feeder: FeederId, change: &Change) {
        let sent = self.target.is_some();
        let now = self.clock.now();
        let confirmed = wellfed_mirror::change_kept(&self.store, feeder, now, |kept| {
            kept.watch.confirm(change);
            hook(&mut kept.hooked, change, sent);
        })
        .await;
        if let Err(error) = confirmed {
            warn!(%feeder, "confirm a webhook delivery: {error}");
        }
    }
}

/// Keep `hooked` to the alerts the webhook was told are open.
fn hook(hooked: &mut Vec<Tracked>, change: &Change, sent: bool) {
    match change {
        Change::Post(tracked) => {
            hooked.retain(|each| each.key != tracked.key);
            if sent && tracked.delivery != Shown::Quiet {
                hooked.push(*tracked);
            }
        }
        Change::Remove(key) => hooked.retain(|each| each.key != *key),
    }
}

/// Why a try failed.
enum Failure {
    /// Worth trying again: no answer, or the receiver said so.
    Again(String),
    /// The receiver refused the body: trying again would get the same.
    Final(String),
}

/// A request error's words without its URL, whose path may be a secret.
fn without_url(error: reqwest::Error) -> String {
    let kind = if error.is_timeout() {
        "timed out"
    } else if error.is_connect() {
        "could not connect"
    } else {
        "request failed"
    };
    format!("{kind}: {}", error.without_url())
}

/// The same for every try of one change: the feeder, the transition and
/// the alert, by its key, kind and when the server first saw it, so the
/// same alert opening again later is a new change.
fn delivery_id(feeder: FeederId, transition: Transition, tracked: &Tracked) -> String {
    let seen = tracked.seen.as_millisecond();
    let kind = tracked.notice.kind();
    let key: &NoticeKey = &tracked.key;
    let bytes = canonical_of(&(feeder, transition, key, kind, seen)).unwrap_or_default();
    let mut digest = sha256(&bytes);
    digest.truncate(32);
    digest
}

/// Sleep until `at`; `None` is never selected (its branch is disabled).
async fn sleep_until_some(at: Option<Instant>) {
    if let Some(at) = at {
        tokio::time::sleep_until(at).await;
    }
}

#[cfg(test)]
mod tests;
