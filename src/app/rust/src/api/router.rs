//! Every request to a feeder, through the owner's server or the phone's own
//! session, as Dart makes it once a server is configured. With none, the
//! router passes everything to the phone's own session as before.
//!
//! The path is chosen per feeder, server first: a new read or a new write
//! goes direct when the server is unreachable, or reports the feeder offline
//! while the phone's own session reaches it. A viewer, or an account with an
//! expiry, holds no key of its own, so it goes through the server only.
//! [`PathRouter::path`] and [`RouterUpdate::Path`] say which path a feeder
//! takes now, for the screen's indicator.
//!
//! A write carries one idempotency key from the owner's confirmation to its
//! result, on whichever path it first went. **A write that may have run never
//! switches path.** It is pinned to its path, on disk, and resolved there
//! only: on the server by a retry under its key or the journal entry the key
//! names (`GET /commands/{key}`, or the stream's `command` event); on the
//! phone's own session from the feeder's history (a feed by its `Fed`
//! event). Nothing here sends it again on its own, and a retry never sends it
//! on the other path, even with its own path down: it then stays uncertain,
//! and [`PathRouter::pending`] lists it. A write the server definitely did
//! not send has run nowhere, and goes on the phone's own session instead:
//! no connection to the server (refused before a byte of the request left),
//! or `feeder_offline` on a first attempt, which the server answers only for
//! a write its feeder task never took, dropping the journal entry. Since the
//! server keeps no entry, the router never sends that key to the server
//! again: the write is the direct path's from then on.
//!
//! A new feed waits while another feed on the same feeder is sent and not yet
//! answered, or uncertain and not yet put away by the owner
//! ([`PathRouter::acknowledge`]), whichever path either took: the owner looks
//! at the bowl before feeding again, as the feed sheet has always asked, and
//! no feed can double an uncertain one through the other path.

use flutter_rust_bridge::frb;
use serde::{Deserialize, Serialize};

use crate::api::error::BridgeError;
use crate::api::feeder::{FeederReply, FeederRequest};
pub use crate::api::model::{FeedOutcome, FeedReason};
use crate::api::server::{ServerRefusal, ServerSession, ServerUpdate};
use crate::api::server_error::ServerError;
use crate::api::server_views::{ServerAccount, ServerFeeder};
use crate::api::session::{FeedRefusal, FeederSession};
use crate::frb_generated::StreamSink;
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
    /// A settings or plans write whose retry found the store moved on,
    /// which the write itself may have done: read it to see.
    Superseded,
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
    /// Nothing went out: safe to start over. `error` is the server's
    /// reason when it refused it (a viewer may not feed) or could not be
    /// reached.
    NotSent { error: Option<ServerError> },
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
}

/// What a retry of a pending write found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetryOutcome {
    Settled {
        pending: PendingWrite,
        settlement: Settlement,
    },
    /// Still not known: its own path did not say, or could not be reached.
    /// It was not sent on the other path.
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
    /// A feed this phone sent through the server ended.
    FeedEnded {
        key: String,
        feeder: String,
        settlement: Settlement,
    },
}
#[frb(opaque)]
pub struct PathRouter {
    router: Router,
}

impl PathRouter {
    /// A router keeping its pins at `pins_path` (`pending.json` beside the
    /// app's file). With no server configured it passes every request to
    /// the phone's own sessions.
    #[frb(sync)]
    pub fn new(pins_path: String) -> Result<Self, BridgeError> {
        Ok(Self {
            router: Router::open(pins_path.into())?,
        })
    }

    /// Route through `session` from now on, as `account`: a viewer or an
    /// account with an expiry goes through the server only.
    #[frb(sync)]
    pub fn connect_server(&self, session: &ServerSession, account: ServerAccount) {
        self.router.connect_server(session.handle(), &account);
    }

    /// No server any more (logged out): every feeder goes direct. A write
    /// pinned to the server stays pinned and uncertain.
    #[frb(sync)]
    pub fn disconnect_server(&self) {
        self.router.disconnect_server();
    }

    /// The server's id for `feeder` (identity hex), as its last feeder list
    /// gave it, for a cold start before [`Self::refresh`] answers.
    #[frb(sync)]
    pub fn remember_server_feeder(&self, feeder: String, id: String) -> Result<(), ServerError> {
        self.router.remember(&feeder, &id)
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

    /// Ask a pending write's own path what became of it: the server again
    /// under its key (which sends a feed nothing new), or the feeder's
    /// history as the phone's own session read it. Never the other path.
    pub async fn retry(&self, key: String) -> RetryOutcome {
        self.router.retry(&key).await
    }

    /// The owner looked at the bowl and put `key` away: a new feed may go.
    /// It stays pinned to its path until it settles, or for a week; one
    /// nothing can settle (a direct write other than a feed) goes now.
    #[frb(sync)]
    pub fn acknowledge(&self, key: String) -> Result<(), BridgeError> {
        self.router.acknowledge(&key)
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
