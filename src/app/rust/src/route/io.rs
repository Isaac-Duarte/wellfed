//! The router's IO: each request driven on the phone's own session with the
//! feeder, around the rules in [`super::core`], with the pins kept on disk
//! by [`super::pins`]. One lock holds what the router knows; no call holds
//! it across an await, and a write's admission and its pin are written
//! under one hold before it goes, so two feeds asked at once cannot both
//! pass and a write that never got written down never goes.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use jiff::Timestamp;
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;
use wellfed_model::{Answer, Op};

use crate::api::error::{BridgeError, ErrorKind};
use crate::api::feeder::{FeederReply, FeederRequest};
use crate::api::keys::random;
use crate::api::router::{
    NotSentReason, PendingWrite, RetryOutcome, RoutedFeed, RoutedReply, RouterUpdate, Settlement,
    WriteKind,
};
use crate::api::session::FeedRefusal;
use crate::api::store::SetAside;
use crate::link::DirectLink;
use crate::requests;
use crate::route::core::{self, Book, Ended, Held, settled_by_history};
use crate::route::pins::{PinRecord, PinState, PinStore, PinnedWrite};
use crate::route::shape::write_kind;
use crate::session::{Command, Heard, feed_due};

/// The router: cheap to clone, one state behind every clone.
#[derive(Clone)]
pub(crate) struct Router {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<State>,
    closed: watch::Sender<bool>,
}

struct State {
    store: PinStore,
    /// Each feeder's session, by identity (lowercase hex).
    links: HashMap<String, Arc<DirectLink>>,
    book: Book,
    listeners: Vec<mpsc::UnboundedSender<RouterUpdate>>,
}

pub(crate) fn now_ms() -> i64 {
    Timestamp::now().as_millisecond()
}

fn new_key() -> Result<Uuid, BridgeError> {
    Ok(uuid::Builder::from_random_bytes(random()?).into_uuid())
}

impl State {
    fn link(&self, feeder: &str) -> Option<Arc<DirectLink>> {
        self.links.get(feeder).cloned()
    }

    /// A write about to go to `feeder`, as its pin records it: with the
    /// newest event its session knows, so a feed's end is one after it.
    fn pin(&self, key: Uuid, link: &DirectLink, write: PinnedWrite, now: i64) -> PinRecord {
        PinRecord {
            key,
            feeder: link.feeder.clone(),
            write,
            state: PinState::Sending,
            sent_ms: now,
            mark: link.newest(),
            excluded: Vec::new(),
            acknowledged_ms: None,
        }
    }

    fn save(&self) -> Result<(), BridgeError> {
        self.store.save(&self.book)
    }

    /// Save after a change already made in memory: a failure leaves the
    /// pin in memory, where it still holds for this run.
    fn save_or_log(&self) {
        if let Err(error) = self.save() {
            log::warn!("the pending writes could not be saved: {error}");
        }
    }

    fn tell(&mut self, updates: &[RouterUpdate]) {
        if updates.is_empty() {
            return;
        }
        self.listeners.retain(|listener| {
            updates
                .iter()
                .all(|update| listener.send(update.clone()).is_ok())
        });
    }

    /// Take `key` out as `settlement` says, and tell the screens.
    fn settle(&mut self, key: Uuid, settlement: Settlement) -> Option<PendingWrite> {
        let pin = self.book.get(key)?.clone();
        self.book.settle(key);
        self.save_or_log();
        let pending = pin.view();
        self.tell(&[RouterUpdate {
            pending: pending.clone(),
            settlement,
        }]);
        Some(pending)
    }

    /// Settle what the history `feeder`'s session read settles, and drop
    /// the running feeds it gave the ends of. What it settled.
    fn settle_from_history(&mut self, feeder: &str) -> Vec<(Uuid, Settlement)> {
        let Some(link) = self.link(feeder) else {
            return Vec::new();
        };
        let seen = link.seen();
        let ended: Vec<u32> = seen.iter().map(|seen| seen.feed).collect();
        if self.book.ended_running(feeder, &ended) {
            self.save_or_log();
        }

        let settled = settled_by_history(&self.book.pins, feeder, &seen, link.phone());
        for (key, settlement) in &settled {
            self.settle(*key, settlement.clone());
        }
        settled
    }
}

impl Router {
    pub(crate) fn open(pins_path: &Path) -> Result<Self, BridgeError> {
        let (store, mut book) = PinStore::open(pins_path)?;
        if book.expire(now_ms()) {
            store.save(&book)?;
        }
        let state = State {
            store,
            links: HashMap::new(),
            book,
            listeners: Vec::new(),
        };
        Ok(Self {
            inner: Arc::new(Inner {
                state: Mutex::new(state),
                closed: watch::Sender::new(false),
            }),
        })
    }

    fn state(&self) -> MutexGuard<'_, State> {
        // Every change below is whole before the lock is let go, so a
        // panic elsewhere leaves a state still worth reading.
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn attach(&self, link: Arc<DirectLink>) {
        let feeder = link.feeder.clone();
        let weak = Arc::downgrade(&self.inner);
        let heard = feeder.clone();
        let own = Arc::downgrade(&link);
        link.listen(Arc::new(move || {
            if let Some(inner) = weak.upgrade() {
                Self { inner }.fed(&heard, &own);
            }
        }));

        let mut state = self.state();
        state.links.insert(feeder.clone(), link);
        state.settle_from_history(&feeder);
    }

    /// `from`, a session attached for `feeder`, read one of this phone's
    /// feed ends.
    fn fed(&self, feeder: &str, from: &Weak<DirectLink>) {
        let mut state = self.state();
        // A session another attach replaced says nothing of the feeder.
        let current = state.links.get(feeder);
        if current.is_some_and(|current| std::ptr::eq(Arc::as_ptr(current), from.as_ptr())) {
            state.settle_from_history(feeder);
        }
    }

    pub(crate) fn pending(&self) -> Vec<PendingWrite> {
        self.state().book.pins.iter().map(PinRecord::view).collect()
    }

    /// The pins file that did not read at the open, while feeds still wait
    /// for the owner to check the bowls.
    pub(crate) fn set_aside(&self) -> Option<SetAside> {
        self.state().book.set_aside.clone()
    }

    pub(crate) fn bowls_checked(&self) -> Result<(), BridgeError> {
        let mut state = self.state();
        let held = state.book.set_aside.clone();
        if state.book.bowls_checked()
            && let Err(error) = state.save()
        {
            // Not written down: the hold stays, so a restart still has it.
            state.book.set_aside = held;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn acknowledge(&self, key: &str) -> Result<(), BridgeError> {
        let key = Uuid::parse_str(key)
            .map_err(|_| BridgeError::new(ErrorKind::Key, "not an idempotency key"))?;
        let mut state = self.state();
        if state.book.acknowledge(key, now_ms()) {
            state.save()?;
        }
        Ok(())
    }

    pub(crate) async fn feed(&self, feeder: &str, portions: u8) -> RoutedFeed {
        let Ok(key) = new_key() else {
            return RoutedFeed::NotSent {
                reason: NotSentReason::Random,
            };
        };
        let now = now_ms();

        let link = {
            let mut state = self.state();
            let Some(link) = state.link(feeder) else {
                return RoutedFeed::NotSent {
                    reason: NotSentReason::NoPath,
                };
            };
            let pin = state.pin(key, &link, PinnedWrite::Feed { portions }, now);
            match state.book.admit(pin, now) {
                Ok(()) => {}
                Err(Held::SetAside) => {
                    return RoutedFeed::NotSent {
                        reason: NotSentReason::SetAside,
                    };
                }
                Err(Held::By(holding)) => {
                    return match state.book.get(holding) {
                        Some(holding) => RoutedFeed::Held {
                            pending: holding.view(),
                        },
                        None => RoutedFeed::NotSent {
                            reason: NotSentReason::NoPath,
                        },
                    };
                }
            }
            // Written down before it goes, or it does not go.
            if state.save().is_err() {
                state.book.settle(key);
                return RoutedFeed::NotSent {
                    reason: NotSentReason::Store,
                };
            }
            link
        };

        let tried = feed_on(&link, portions).await;

        let mut state = self.state();
        Self::feed_ended(&mut state, feeder, key, portions, tried)
    }

    /// The end of a feed's one attempt.
    fn feed_ended(
        state: &mut State,
        feeder: &str,
        key: Uuid,
        portions: u8,
        tried: FeedTry,
    ) -> RoutedFeed {
        let now = now_ms();
        let held = state.book.get(key).map(PinRecord::view);
        state.book.ended(key, tried.ended());
        if let FeedTry::Started(feed) = tried {
            // Its end may come late, after a gap in what the session reads:
            // until then it is no other feed's end.
            state
                .book
                .started(feeder, feed, now.saturating_add(core::KEEP_MS));
        }
        state.save_or_log();
        let key_text = key.to_string();

        let routed = match tried {
            FeedTry::Started(feed) => RoutedFeed::Started {
                key: key_text,
                feed,
                portions,
                due_seconds: u32::try_from(feed_due(portions).as_secs()).unwrap_or(u32::MAX),
            },
            FeedTry::Refused(refusal) => RoutedFeed::Refused {
                key: key_text,
                refusal,
            },
            FeedTry::Uncertain => match state.book.get(key) {
                Some(pin) => RoutedFeed::Uncertain {
                    pending: pin.view(),
                },
                // Pinned before it went; only a broken invariant loses it,
                // and then nothing more is known.
                None => RoutedFeed::NotSent {
                    reason: NotSentReason::Store,
                },
            },
            FeedTry::Unsent => RoutedFeed::NotSent {
                reason: NotSentReason::Session,
            },
        };
        // A screen made again while the call was out took the pin up from
        // `pending` as uncertain, and only hears its end here; the caller's
        // own screen has the call's answer, and a feed still sending there
        // is no key's yet.
        if let Some(pending) = held
            && let Some(settlement) = feed_settlement(&routed)
        {
            state.tell(&[RouterUpdate {
                pending,
                settlement,
            }]);
        }
        routed
    }

    pub(crate) async fn cancel(&self, feeder: &str, feed: u32) -> RoutedReply {
        self.write(feeder, WriteKind::Cancel, Call::Op(Op::CancelFeed { feed }))
            .await
    }

    pub(crate) async fn ask(&self, feeder: &str, request: FeederRequest) -> RoutedReply {
        match write_kind(&request) {
            None => {
                let Some(link) = self.state().link(feeder) else {
                    return nowhere(NotSentReason::NoPath);
                };
                RoutedReply {
                    reply: call(&link, Call::Request(request)).await,
                    pending: None,
                }
            }
            Some(kind) => self.write(feeder, kind, Call::Request(request)).await,
        }
    }

    async fn write(&self, feeder: &str, kind: WriteKind, request: Call) -> RoutedReply {
        let Ok(key) = new_key() else {
            return nowhere(NotSentReason::Random);
        };
        let now = now_ms();

        let link = {
            let mut state = self.state();
            let Some(link) = state.link(feeder) else {
                return nowhere(NotSentReason::NoPath);
            };
            let pin = state.pin(key, &link, PinnedWrite::Other { kind }, now);
            // Only a feed is ever held back, so this admits it; written
            // down before it goes, or it does not go.
            if state.book.admit(pin, now).is_err() || state.save().is_err() {
                state.book.settle(key);
                return nowhere(NotSentReason::Store);
            }
            link
        };

        let reply = call(&link, request).await;

        let mut state = self.state();
        state.book.ended(key, core::direct_ended(&reply));
        state.save_or_log();
        let pending = state.book.get(key).map(PinRecord::view);
        RoutedReply { reply, pending }
    }

    pub(crate) fn retry(&self, key: &str) -> RetryOutcome {
        let Ok(key) = Uuid::parse_str(key) else {
            return RetryOutcome::Unknown;
        };
        let mut state = self.state();
        let Some(pin) = state.book.get(key).cloned() else {
            return RetryOutcome::Unknown;
        };
        if pin.state == PinState::Sending {
            // Its one attempt is still out.
            return RetryOutcome::Uncertain {
                pending: pin.view(),
            };
        }

        let settled = state.settle_from_history(&pin.feeder);
        match settled.into_iter().find(|(settled, _)| *settled == key) {
            Some((_, settlement)) => RetryOutcome::Settled {
                pending: pin.view(),
                settlement,
            },
            None => RetryOutcome::Uncertain {
                pending: pin.view(),
            },
        }
    }

    pub(crate) async fn updates(&self, mut emit: impl FnMut(RouterUpdate) -> bool) {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let mut closed = self.inner.closed.subscribe();
        self.state().listeners.push(sender);
        if *closed.borrow() {
            return;
        }
        loop {
            let update = tokio::select! {
                update = receiver.recv() => update,
                _ = closed.wait_for(|closed| *closed) => return,
            };
            let Some(update) = update else { return };
            if !emit(update) {
                return;
            }
        }
    }

    pub(crate) fn close(&self) {
        self.inner.closed.send_replace(true);
    }
}

/// How a feed's one attempt ended.
#[derive(Clone, Copy)]
enum FeedTry {
    /// The feeder reserved it under this number.
    Started(u32),
    Refused(FeedRefusal),
    Unsent,
    Uncertain,
}

impl FeedTry {
    const fn ended(&self) -> Ended {
        match self {
            Self::Started(_) => Ended::Answered,
            Self::Refused(_) => Ended::Refused,
            Self::Unsent => Ended::Unsent,
            Self::Uncertain => Ended::Uncertain,
        }
    }
}

/// Feed `portions` on the session behind `link`, once.
async fn feed_on(link: &DirectLink, portions: u8) -> FeedTry {
    let (reply, heard) = oneshot::channel();
    let command = Command::Feed {
        portions,
        reply: Some(reply),
    };
    if link.commands.send(command).is_err() {
        return FeedTry::Unsent;
    }
    match heard.await {
        // Dropped without a word: it may have gone.
        Err(_) | Ok(Heard::Unanswered) => FeedTry::Uncertain,
        Ok(Heard::NotSent | Heard::Held) => FeedTry::Unsent,
        Ok(Heard::Answer(Answer::Feeding { feed })) => FeedTry::Started(feed),
        Ok(Heard::Answer(Answer::Refused(refusal))) => FeedTry::Refused(refusal.into()),
        // An answer that fits no feed says nothing of whether it ran.
        Ok(Heard::Answer(_)) => FeedTry::Uncertain,
    }
}

/// How a request goes on the session.
enum Call {
    /// Through the request's own ops.
    Request(FeederRequest),
    /// One op.
    Op(Op),
}

/// Run `request` on the session behind `link`.
async fn call(link: &DirectLink, request: Call) -> FeederReply {
    match request {
        Call::Request(request) => requests::run(&link.commands, link.identity, request).await,
        Call::Op(op) => match requests::ask(&link.commands, op).await {
            Ok(answer) => requests::reply_of(answer),
            Err(lost) => lost.into(),
        },
    }
}

pub(crate) const fn nowhere(reason: NotSentReason) -> RoutedReply {
    RoutedReply {
        reply: FeederReply::NotSent { reason },
        pending: None,
    }
}

/// What a feed call's answer settles of its pin, for a screen that holds
/// only the pin; `None` while it may still have run.
fn feed_settlement(routed: &RoutedFeed) -> Option<Settlement> {
    match routed {
        RoutedFeed::Started { feed, .. } => Some(Settlement::Started { feed: *feed }),
        RoutedFeed::Refused { refusal, .. } => Some(Settlement::Refused { refusal: *refusal }),
        RoutedFeed::NotSent { .. } => Some(Settlement::NotSent),
        RoutedFeed::Uncertain { .. } | RoutedFeed::Held { .. } => None,
    }
}
