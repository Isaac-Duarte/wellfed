//! Every request to a feeder, on the phone's own session with it. The
//! router pins every write before it goes, so Dart makes every request
//! through it once the app has one, and never around it: a
//! [`FeederSession`] has no request of its own. A pins file that does not
//! read is set aside, never deleted, and the router opens on a new one that
//! holds every feed back until the owner has looked at the bowls
//! ([`FeederRouter::set_aside`], [`FeederRouter::bowls_checked`]). One that
//! cannot be opened at all (already open in another router, or an IO error)
//! fails [`FeederRouter::new`], and the app then sends no write at all: it
//! never falls back to unpinned writes.
//!
//! A write carries one idempotency key from the owner's confirmation to its
//! result, and is pinned on disk before it goes. **A write is sent once.** A
//! write that may have run is resolved only by reading: a feed by its `Fed`
//! event in the feeder's history, as the phone's own session reads it.
//! [`FeederRouter::retry`] reads; it never sends. Until then it stays
//! uncertain, and [`FeederRouter::pending`] lists it.
//!
//! A new feed waits while another feed on the same feeder is sent and not yet
//! answered, or uncertain and not yet put away by the owner
//! ([`FeederRouter::acknowledge`]): the owner looks at the bowl before
//! feeding again, as the feed sheet asks.

use flutter_rust_bridge::frb;
use serde::{Deserialize, Serialize};

use wellfed_client::history::Delivered;

use crate::api::error::BridgeError;
use crate::api::feeder::{FeederReply, FeederRequest};
pub use crate::api::model::{FeedOutcome, FeedReason, FoodLevel};
use crate::api::session::{FedBy, FeedRefusal, FeedResult, FeederSession};
use crate::api::store::SetAside;
use crate::frb_generated::StreamSink;
use crate::route::io::Router;

/// What a write asks of a feeder. Stored in `pending.json` (a pin keeps
/// its write's kind), so renaming or removing a variant changes that
/// file's shape: bump its version and keep a reader for every one before
/// it. A new variant an older build cannot read is a new version too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteKind {
    Feed,
    Cancel,
    Settings,
    Lock,
    Desiccant,
    Plans,
    Wifi,
    Play,
    Clock,
    Invite,
    Revoke,
    Reboot,
    Calibration,
    Call,
    Update,
    /// A meal's skip or its undo.
    Skip,
}

/// A write that may have run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingWrite {
    /// Its idempotency key.
    pub key: String,
    /// The feeder's identity, lowercase hex.
    pub feeder: String,
    pub kind: WriteKind,
    /// A feed's portions.
    pub portions: Option<u8>,
    /// When the owner confirmed it, by the phone's clock.
    pub sent_ms: i64,
    /// The owner has put it away: it no longer holds a new feed back.
    pub acknowledged: bool,
}

/// What became of a pending write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Settlement {
    /// The feeder refused the feed; nothing ran.
    Refused { refusal: FeedRefusal },
    /// The feeder reserved the feed under this number and runs it; its end
    /// follows on the session's stream.
    Started { feed: u32 },
    /// The feed ended, as the feeder's history says. An uncertain outcome
    /// delivered between `completed` and `requested` portions.
    Fed {
        feed: u32,
        requested: u8,
        completed: u8,
        outcome: FeedOutcome,
        reason: FeedReason,
    },
    /// It never went out.
    NotSent,
}

/// Why a write went nowhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotSentReason {
    /// The phone has no session with the feeder.
    NoPath,
    /// No idempotency key: the system random source failed.
    Random,
    /// The pin could not be written down, so it did not go.
    Store,
    /// The phone's own session ended before it went.
    Session,
    /// A feed, held back: the record of writes that may have run did not
    /// read and was set aside, and the owner has not yet said the bowls are
    /// checked ([`FeederRouter::bowls_checked`]).
    SetAside,
}

/// How a feed through the router began.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoutedFeed {
    /// The feeder reserved it. Its end arrives on the session's stream.
    Started {
        key: String,
        feed: u32,
        portions: u8,
        /// Past this a feeder that stayed on has ended it itself.
        due_seconds: u32,
    },
    /// The feeder refused it: nothing was fed.
    Refused { key: String, refusal: FeedRefusal },
    /// It may have run. It stays pinned and is never sent again;
    /// [`FeederRouter::retry`] reads the feeder's history for it.
    Uncertain { pending: PendingWrite },
    /// Nothing went out: safe to start over.
    NotSent { reason: NotSentReason },
    /// Another feed on this feeder is unanswered or uncertain: the owner
    /// looks at the bowl and puts it away first.
    Held { pending: PendingWrite },
}

/// How a request through the router ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutedReply {
    /// The feeder's answer. `Unanswered` is a write that may have run, now
    /// in `pending`; `NotSent` went nowhere, and says why.
    pub reply: FeederReply,
    pub pending: Option<PendingWrite>,
}

/// What a retry of a pending write found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetryOutcome {
    Settled {
        pending: PendingWrite,
        settlement: Settlement,
    },
    /// Still not known: the history the session read does not say. Nothing
    /// was sent.
    Uncertain { pending: PendingWrite },
    /// No pending write has that key.
    Unknown,
}

/// What the router tells the screens: a pending write settled, and is no
/// longer pending.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouterUpdate {
    pub pending: PendingWrite,
    pub settlement: Settlement,
}

/// A feed's end as a settlement gives it, as the feed sheet shows one, for
/// an uncertain feed the feeder's history settled. When the feeder recorded
/// it and what its beam read are the history row's to say, and its
/// requester is left [`FedBy::Unknown`]. `None` for any settlement but
/// [`Settlement::Fed`].
#[frb(sync)]
pub fn settled_feed(settlement: Settlement) -> Option<FeedResult> {
    let Settlement::Fed {
        feed,
        requested,
        completed,
        outcome,
        reason,
    } = settlement
    else {
        return None;
    };
    let delivered = Delivered::of(outcome, completed, requested);
    let (confirmed, at_most) = delivered.bounds();
    Some(FeedResult {
        feed,
        requested,
        confirmed,
        at_most,
        outcome,
        reason,
        uncertain: outcome == FeedOutcome::Uncertain,
        words: delivered.describe(requested, outcome),
        at_ms: None,
        by: FedBy::Unknown,
        hopper: FoodLevel::Unread,
    })
}

/// How long a feed of `portions` may take, in seconds: past this a feeder
/// that stayed on has ended it itself, so one whose end never arrived is
/// shown as uncertain, not as running.
#[frb(sync)]
pub fn feed_due_seconds(portions: u8) -> u32 {
    u32::try_from(crate::session::feed_due(portions).as_secs()).unwrap_or(u32::MAX)
}

#[frb(opaque)]
pub struct FeederRouter {
    router: Router,
}

impl FeederRouter {
    /// The one router keeping its pins at `pins_path` (`pending.json`
    /// beside the app's file); [`ErrorKind::InUse`] while another holds
    /// them.
    ///
    /// [`ErrorKind::InUse`]: crate::api::error::ErrorKind::InUse
    #[frb(sync)]
    pub fn new(pins_path: String) -> Result<Self, BridgeError> {
        Ok(Self {
            router: Router::open(std::path::Path::new(&pins_path))?,
        })
    }

    /// Reach `session`'s feeder through it. Call it before running the
    /// session, so the router hears its history from the start; a later
    /// session replaces it.
    #[frb(sync)]
    pub fn attach(&self, session: &FeederSession) {
        self.router.attach(session.direct());
    }

    /// Feed `portions` (1-48) on `feeder` once, as the owner confirmed it.
    pub async fn feed(&self, feeder: String, portions: u8) -> RoutedFeed {
        self.router.feed(&feeder, portions).await
    }

    /// Stop the feed numbered `feed` if it still runs.
    pub async fn cancel_feed(&self, feeder: String, feed: u32) -> RoutedReply {
        self.router.cancel(&feeder, feed).await
    }

    /// Run one request.
    pub async fn ask(&self, feeder: String, request: FeederRequest) -> RoutedReply {
        self.router.ask(&feeder, request).await
    }

    /// The writes that may have run, oldest first.
    #[frb(sync)]
    pub fn pending(&self) -> Vec<PendingWrite> {
        self.router.pending()
    }

    /// Ask what became of a pending write, only by reading the feeder's
    /// history as the phone's own session read it. Nothing is sent.
    #[frb(sync)]
    pub fn retry(&self, key: String) -> RetryOutcome {
        self.router.retry(&key)
    }

    /// The owner looked at the bowl and put `key` away: a new feed may go.
    /// A feed stays pinned until the history settles it, or for a week from
    /// now; one nothing can settle (a write other than a feed, or a feed
    /// sent before its session knew any event) goes now. A key that is no
    /// idempotency key is refused.
    #[frb(sync)]
    pub fn acknowledge(&self, key: String) -> Result<(), BridgeError> {
        self.router.acknowledge(&key)
    }

    /// The record of writes that may have run, when it did not read at the
    /// open and was set aside: every feed waits, as
    /// [`NotSentReason::SetAside`], until [`Self::bowls_checked`].
    #[frb(sync)]
    pub fn set_aside(&self) -> Option<SetAside> {
        self.router.set_aside()
    }

    /// The owner looked at every feeder's bowl after the record of writes
    /// was set aside: feeds go again.
    #[frb(sync)]
    pub fn bowls_checked(&self) -> Result<(), BridgeError> {
        self.router.bowls_checked()
    }

    /// Hear the router's updates until [`Self::close`].
    pub async fn updates(&self, updates: StreamSink<RouterUpdate>) {
        self.router
            .updates(|update| updates.add(update).is_ok())
            .await;
    }

    /// [`Self::updates`] into `emit`, for Rust tests.
    #[frb(ignore)]
    pub async fn updates_with(&self, emit: impl FnMut(RouterUpdate) -> bool) {
        self.router.updates(emit).await;
    }

    /// End [`Self::updates`].
    #[frb(sync)]
    pub fn close(&self) {
        self.router.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fed(completed: u8, outcome: FeedOutcome, reason: FeedReason) -> Settlement {
        Settlement::Fed {
            feed: 7,
            requested: 3,
            completed,
            outcome,
            reason,
        }
    }

    /// A counted end is one number; an uncertain one, or one a newer
    /// feeder named, is the range from what it confirmed to what it asked,
    /// never a count. The reason goes through, and the journal names no
    /// requester.
    #[test]
    fn a_settled_feed_reads_as_the_history_would_show_it() {
        let cases = [
            (
                fed(2, FeedOutcome::Failed, FeedReason::Jammed),
                (2, 2),
                false,
            ),
            (fed(3, FeedOutcome::Done, FeedReason::Done), (3, 3), false),
            (
                fed(1, FeedOutcome::Uncertain, FeedReason::Interrupted),
                (1, 3),
                true,
            ),
            (
                fed(0, FeedOutcome::Unknown, FeedReason::Unknown),
                (0, 3),
                false,
            ),
        ];
        for (settlement, (confirmed, at_most), uncertain) in cases {
            let Settlement::Fed { reason, .. } = settlement else {
                unreachable!("every case is a feed's end");
            };
            let result = settled_feed(settlement.clone()).expect("a feed's end");
            assert_eq!(
                (result.confirmed, result.at_most),
                (confirmed, at_most),
                "{settlement:?}"
            );
            assert_eq!(result.uncertain, uncertain, "{settlement:?}");
            assert_eq!(result.reason, reason);
            assert_eq!((result.feed, result.requested), (7, 3));
            assert_eq!(result.by, FedBy::Unknown);
            assert_eq!(result.at_ms, None);
            assert_eq!(result.hopper, FoodLevel::Unread);
        }
        let uncertain = settled_feed(fed(1, FeedOutcome::Uncertain, FeedReason::Interrupted));
        assert_eq!(
            uncertain.map(|result| result.words),
            Some("1 to 3 of 3 portions, uncertain".to_owned())
        );
    }

    /// Only a feed's end is a feed result.
    #[test]
    fn only_a_feed_s_end_is_a_feed_result() {
        for settlement in [
            Settlement::Started { feed: 7 },
            Settlement::NotSent,
            Settlement::Refused {
                refusal: FeedRefusal::Busy,
            },
        ] {
            assert!(settled_feed(settlement).is_none());
        }
    }
}
