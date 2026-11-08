//! Every request to a feeder, through the owner's server or the phone's own
//! session. With no server configured the router passes everything to the
//! phone's own session, but it still pins every write, so Dart makes every
//! request through it once the app has one, and never around it: a session
//! the router holds refuses [`FeederSession::feed`]. A pins file that does
//! not read is set aside, never deleted, and the router opens on a new one
//! that holds every feed back until the owner has looked at the bowls
//! ([`PathRouter::set_aside`], [`PathRouter::bowls_checked`]). One that
//! cannot be opened at all (already open in another router, or an IO error)
//! fails [`PathRouter::new`], and the app then sends no write at all: it
//! never falls back to unpinned writes.
//!
//! The path is chosen per feeder, server first: a new read or a new write
//! goes direct when the server is unreachable, or reports the feeder offline
//! while the phone's own session reaches it. A viewer, or an account with an
//! expiry, holds no key of its own, so it goes through the server only. A
//! server that refuses this phone's token (revoked, or the account disabled)
//! closes the phone's own keys too, on every feeder: the router never routes
//! around the account, until the owner logs in again or removes the server
//! ([`PathRouter::disconnect_server`]).
//!
//! The app keeps the server with its feeders and its account, and hands it
//! to the router ([`PathRouter::configure_server`]) before anything else
//! asks it, so a cold start goes by the kept account until the server
//! answers. A change the server makes to the account while the app runs
//! ([`RouterUpdate::Account`]) is kept too. One made while the app was
//! closed, with the server down when it starts, is not known on the phone:
//! a demoted owner's kept account still allows its own key, and only the
//! server revoking that phone key on the feeder (as it does for a demotion,
//! an expiry or a deleted account) stops it there.
//! [`PathRouter::path`] and [`RouterUpdate::Path`] say which path a feeder
//! takes now, for the screen's indicator.
//!
//! A write carries one idempotency key from the owner's confirmation to its
//! result, and is pinned on disk before it goes. **A write is sent once, and
//! a write that may have run never switches path.** It is resolved only on
//! its own path, and only by reading: on the server, the journal entry its
//! key names (`GET /commands/{key}`, or the stream's `command` event); on
//! the phone's own session, the feeder's history (a feed by its `Fed`
//! event). [`PathRouter::retry`] reads; it never sends. The server dedups a
//! key only while its journal holds it, and it holds none for a write it
//! never recorded (a lost `feeder_offline` answer, a failure before the
//! insert, a request that never reached it, an app that stopped before it
//! sent), so a resend could feed again after the owner already had. A
//! journal that lacks the key settles it as never sent only when it is the
//! one the write went to (the same token and account), well after its
//! attempt. With its path down it stays uncertain, and
//! [`PathRouter::pending`] lists it.
//!
//! A write the server definitely did not send has run nowhere, and goes on
//! the phone's own session instead: no connection to the server (refused
//! before a byte of the request left), or `feeder_offline`, which the server
//! answers only for a write its feeder task never took, dropping the
//! journal entry.
//!
//! A new feed waits while another feed on the same feeder is sent and not yet
//! answered, or uncertain and not yet put away by the owner
//! ([`PathRouter::acknowledge`]), whichever path either took: the owner looks
//! at the bowl before feeding again, as the feed sheet has always asked, and
//! no feed can double an uncertain one through the other path.

use flutter_rust_bridge::frb;
use serde::{Deserialize, Serialize};

use wellfed_client::history::Delivered;

use crate::api::error::BridgeError;
use crate::api::feeder::{FeederReply, FeederRequest};
use crate::api::keys::ClientKey;
pub use crate::api::model::{FeedOutcome, FeedReason, FoodLevel};
use crate::api::server::{ServerRefusal, ServerSession, ServerUpdate};
use crate::api::server_error::ServerError;
use crate::api::server_views::{KnownServer, ServerAccount, ServerFeeder};
use crate::api::session::{FedBy, FeedRefusal, FeedResult, FeederSession};
use crate::api::store::SetAside;
use crate::frb_generated::StreamSink;
use crate::route::core::UNFOUND_GRACE_MS;
use crate::route::io::Router;

/// The way a request reaches a feeder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutePath {
    /// Through the owner's server.
    Server,
    /// The phone's own session on the LAN.
    Direct,
}

/// What a write asks of a feeder.
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

/// A write that may have run, pinned to the path it went on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingWrite {
    /// Its idempotency key.
    pub key: String,
    /// The feeder's identity, lowercase hex.
    pub feeder: String,
    pub path: RoutePath,
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
    /// It took effect.
    Done,
    /// The feeder refused it; nothing ran.
    Refused { refusal: ServerRefusal },
    /// The feeder reserved the feed under this number and runs it; its end
    /// follows as [`RouterUpdate::FeedEnded`].
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
    /// The feeder restarted with no record of the feed: whether food was
    /// dispensed is not known, and it is never sent again.
    Unrecorded,
    /// The server holds no record of it: it was never sent.
    NotSent,
    /// A meal call write another change to the call overtook: final, and
    /// whether this one ever took effect no longer shows. Read the call to
    /// see it now.
    Superseded,
}

/// Why a write went nowhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotSentReason {
    /// The phone reaches the feeder no way it may use.
    NoPath,
    /// No idempotency key: the system random source failed.
    Random,
    /// The pin could not be written down, so it did not go.
    Store,
    /// The server did not send it: `server_error` says why.
    Server,
    /// The phone's own session ended before it went.
    Session,
    /// A feed, held back: the record of writes that may have run did not
    /// read and was set aside, and the owner has not yet said the bowls are
    /// checked ([`PathRouter::bowls_checked`]).
    SetAside,
}

/// How a feed through the router began.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoutedFeed {
    /// The feeder reserved it. On the server path its end arrives as
    /// [`RouterUpdate::FeedEnded`]; on the direct path, on the session's
    /// stream as before.
    Started {
        key: String,
        path: RoutePath,
        feed: u32,
        portions: u8,
        /// Past this a feeder that stayed on has ended it itself.
        due_seconds: u32,
    },
    /// The feeder refused it: nothing was fed.
    Refused {
        key: String,
        path: RoutePath,
        refusal: FeedRefusal,
    },
    /// It may have run. It is pinned to its path and never sent again;
    /// [`PathRouter::retry`] asks that path what became of it.
    Uncertain { pending: PendingWrite },
    /// Its call ended with no answer, but the server's stream had already
    /// said what became of it: it is no longer pending. A `Started` feed's
    /// end follows as [`RouterUpdate::FeedEnded`].
    Settled {
        pending: PendingWrite,
        settlement: Settlement,
    },
    /// Nothing went out: safe to start over. `error` is the server's
    /// reason when it refused it (a viewer may not feed) or could not be
    /// reached.
    NotSent {
        reason: NotSentReason,
        error: Option<ServerError>,
    },
    /// Another feed on this feeder is unanswered or uncertain: the owner
    /// looks at the bowl and puts it away first.
    Held { pending: PendingWrite },
}

/// How a request through the router ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutedReply {
    /// The path it went on; `None` when it went nowhere.
    pub path: Option<RoutePath>,
    /// The feeder's answer. `Unanswered` is a write that may have run, now
    /// in `pending`; `NotSent` went nowhere, and `server_error` says why
    /// when the server refused it or could not be reached.
    pub reply: FeederReply,
    pub pending: Option<PendingWrite>,
    pub server_error: Option<ServerError>,
    /// Why, when it went nowhere.
    pub not_sent: Option<NotSentReason>,
    /// A write whose call ended unanswered while the server's stream had
    /// already said what became of it: not pending.
    pub settlement: Option<Settlement>,
}

/// What a retry of a pending write found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetryOutcome {
    Settled {
        pending: PendingWrite,
        settlement: Settlement,
    },
    /// Still not known: its own path did not say, or could not be reached.
    /// Nothing was sent.
    Uncertain { pending: PendingWrite },
    /// No pending write has that key.
    Unknown,
}

/// What the router tells the screens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouterUpdate {
    /// The path a new request to `feeder` takes now; `None` when the
    /// phone reaches it no way at all.
    Path {
        feeder: String,
        path: Option<RoutePath>,
    },
    /// A pending write settled; it is no longer pending.
    Settled {
        pending: PendingWrite,
        settlement: Settlement,
    },
    /// The server read the account anew and it changed (a new role or
    /// expiry): the app keeps it, so a cold start with the server down goes
    /// by it.
    Account { account: ServerAccount },
    /// A feed this phone sent through the server ended.
    FeedEnded {
        key: String,
        feeder: String,
        settlement: Settlement,
    },
}
/// How adopting a feeder this phone paired before the server ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Adoption {
    /// The server holds it, and this phone's key on it is the account's
    /// phone key: deleting the account on the server revokes it.
    Adopted { feeder: ServerFeeder },
    /// The server holds it, and this phone's key is not the account's yet:
    /// `error` says why, and [`PathRouter::claim_phone`] tries again.
    Unclaimed {
        feeder: ServerFeeder,
        error: ServerError,
    },
    /// The adoption's answer was lost, and the server did not list the
    /// feeder when read: it may still be adopting it. Nothing more is sent
    /// for it; [`PathRouter::check_adoption`] reads the list again, and a
    /// new adoption only reads too until the router's grace has passed.
    Unsettled { error: ServerError },
    /// The feeder made no invite: `reply` says how that request ended.
    NoInvite { reply: RoutedReply },
    /// The server did not adopt it; `error` says why.
    Failed { error: ServerError },
}

/// A feed's end as a settlement gives it, as the feed sheet shows one, for
/// a feed whose `Fed` event this phone has not read: a server feed whose
/// stream was down when it ended. When the feeder recorded it, and what its
/// beam read, are the history's to say, and its requester is left
/// [`FedBy::Unknown`], since the server's journal does not name the key
/// the feeder heard. `None` for any settlement but [`Settlement::Fed`].
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
pub struct PathRouter {
    router: Router,
}

impl PathRouter {
    /// The one router keeping its pins at `pins_path` (`pending.json`
    /// beside the app's file); [`ErrorKind::InUse`] while another holds
    /// them. With no server configured it passes every request to the
    /// phone's own sessions.
    ///
    /// [`ErrorKind::InUse`]: crate::api::error::ErrorKind::InUse
    #[frb(sync)]
    pub fn new(pins_path: String) -> Result<Self, BridgeError> {
        Self::open_with(pins_path, UNFOUND_GRACE_MS)
    }

    /// [`Self::new`], taking a server write its journal lacks as never sent
    /// `grace_ms` after its attempt, for Rust tests.
    #[frb(ignore)]
    pub fn open_with(pins_path: String, grace_ms: i64) -> Result<Self, BridgeError> {
        Ok(Self {
            router: Router::open(std::path::Path::new(&pins_path), grace_ms)?,
        })
    }

    /// Route through `session` from now on, as `account`: a viewer or an
    /// account with an expiry goes through the server only.
    #[frb(sync)]
    pub fn connect_server(&self, session: &ServerSession, account: ServerAccount) {
        self.router.connect_server(session.handle(), &account);
    }

    /// The server the app keeps, handed over before anything else asks the
    /// router (before the token is read, a session attached or a feed
    /// made): its feeders' ids, and the keys its kept account may use. With
    /// no account kept the phone uses none of its own until the server
    /// says. [`Self::resume_server`] then refines it.
    #[frb(sync)]
    pub fn configure_server(&self, kept: KnownServer) {
        self.router.configure(&kept);
    }

    /// At a cold start, route through `session`, a kept login: by the
    /// account the server reads now, or with the server not answering, by
    /// `cached`, the account as last read. With no cached account the
    /// account counts as one with no key of its own until the server says,
    /// and a token the server refuses closes the phone's own keys, as
    /// [`Self::connect_server`] after a fresh login opens them. The error
    /// is what the server, or the way to it, said.
    pub async fn resume_server(
        &self,
        session: &ServerSession,
        cached: Option<ServerAccount>,
    ) -> Result<ServerAccount, ServerError> {
        self.router.resume(session.handle(), cached).await
    }

    /// The app is configured with a server whose token it no longer holds
    /// (its secret is gone from secure storage): no feeder goes on the
    /// phone's own keys until a login or [`Self::disconnect_server`].
    #[frb(sync)]
    pub fn refuse_server(&self) {
        self.router.refuse_server();
    }

    /// No server any more (logged out): every feeder goes direct. A write
    /// pinned to the server stays pinned and uncertain.
    #[frb(sync)]
    pub fn disconnect_server(&self) {
        self.router.disconnect_server();
    }

    /// Read the server's feeders, and route by what it says of each.
    pub async fn refresh(&self) -> Result<Vec<ServerFeeder>, ServerError> {
        self.router.refresh().await
    }

    /// Reach `session`'s feeder through it, when the path is direct. Call
    /// it before running the session, so the router hears its history from
    /// the start; a later session replaces it.
    #[frb(sync)]
    pub fn attach(&self, session: &FeederSession) {
        self.router.attach(session.direct());
    }

    /// Adopt `feeder`, which this phone paired before the server, as
    /// `name`: an owner's invite made with `key` (this phone's own) goes
    /// to the server, which records `key` as the account's phone key.
    /// `address` is where the phone last found it (`ip:port`); without one
    /// the server looks for it on mDNS. A feeder the server already lists
    /// is only claimed.
    pub async fn adopt(
        &self,
        feeder: String,
        name: String,
        address: Option<String>,
        key: &ClientKey,
    ) -> Adoption {
        let phone = *key.keypair().public();
        self.router.adopt(&feeder, name, address, phone).await
    }

    /// Read the server's list for `feeder` after an [`Adoption::Unsettled`]
    /// one: adopted, its key is claimed; still unlisted while the server may
    /// be adopting it, still unsettled; unlisted past the grace, `Failed`
    /// with `NotFound`, and a new adoption makes a new invite. It sends
    /// nothing but the claim.
    pub async fn check_adoption(&self, feeder: String, key: &ClientKey) -> Adoption {
        let phone = *key.keypair().public();
        self.router.check_adoption(&feeder, phone).await
    }

    /// Record `key`, this phone's own key on `feeder`, as the account's
    /// phone key, for an adoption that left it [`Adoption::Unclaimed`].
    pub async fn claim_phone(&self, feeder: String, key: &ClientKey) -> Adoption {
        let phone = *key.keypair().public();
        self.router.claim_phone(&feeder, phone).await
    }

    /// The path a new request to `feeder` takes now.
    #[frb(sync)]
    pub fn path(&self, feeder: String) -> Option<RoutePath> {
        self.router.path(&feeder)
    }

    /// Feed `portions` (1-48) on `feeder` once, as the owner confirmed it.
    pub async fn feed(&self, feeder: String, portions: u8) -> RoutedFeed {
        self.router.feed(&feeder, portions).await
    }

    /// Stop the feed numbered `feed` if it still runs.
    pub async fn cancel_feed(&self, feeder: String, feed: u32) -> RoutedReply {
        self.router.cancel(&feeder, feed).await
    }

    /// Run one request. One the server has no route for goes on the
    /// phone's own session, when it may use one.
    pub async fn ask(&self, feeder: String, request: FeederRequest) -> RoutedReply {
        self.router.ask(&feeder, request).await
    }

    /// The writes that may have run, oldest first.
    #[frb(sync)]
    pub fn pending(&self) -> Vec<PendingWrite> {
        self.router.pending()
    }

    /// Ask a pending write's own path what became of it, only by reading:
    /// the server's journal entry for its key, or the feeder's history as
    /// the phone's own session read it. Nothing is sent, on either path.
    pub async fn retry(&self, key: String) -> RetryOutcome {
        self.router.retry(&key).await
    }

    /// The owner looked at the bowl and put `key` away: a new feed may go.
    /// It stays pinned to its path until it settles, or for a week from
    /// now; one nothing can settle (a direct write other than a feed) goes
    /// now. A key that is no idempotency key is refused.
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

    /// Read the server's journal for every write pinned to it, and settle
    /// those it answers for.
    pub async fn resolve(&self) {
        self.router.resolve().await;
    }

    /// Follow the server's live stream through the router, which settles
    /// pinned writes and routes by what it hears: as
    /// [`ServerSession::follow`].
    pub async fn follow(&self, last_event_id: Option<String>, updates: StreamSink<ServerUpdate>) {
        self.router
            .follow(last_event_id, |update| updates.add(update).is_ok())
            .await;
    }

    /// [`Self::follow`] into `emit`, for Rust tests.
    #[frb(ignore)]
    pub async fn follow_with(
        &self,
        last_event_id: Option<String>,
        emit: impl FnMut(ServerUpdate) -> bool,
    ) {
        self.router.follow(last_event_id, emit).await;
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
