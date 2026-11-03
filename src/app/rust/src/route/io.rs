//! The router's IO: the server's calls and the direct sessions' commands
//! driven around the rules in [`super::core`], with the pins kept on disk
//! by [`super::pins`]. One lock holds what the router knows; no call holds
//! it across an await, and a feed's admission and its pin are written under
//! one hold, so two feeds asked at once cannot both pass.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use jiff::Timestamp;
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;
use wellfed_api::FeederId;
use wellfed_model::{Answer, Op};

use crate::api::error::BridgeError;
use crate::api::feeder::{FeederReply, FeederRequest, NotAsked};
use crate::api::keys::random;
use crate::api::router::{
    PendingWrite, RetryOutcome, RoutePath, RoutedFeed, RoutedReply, RouterUpdate, Settlement,
};
use crate::api::server::{
    ServerCommand, ServerCommandOutcome, ServerEvent, ServerHandle, ServerRefusal, ServerUpdate,
};
use crate::api::server_error::{ServerError, ServerErrorKind};
use crate::api::server_views::{ServerAccount, ServerConnection, ServerFeeder, ServerRole};
use crate::link::{DirectLink, LinkNews};
use crate::requests;
use crate::route::core::{
    self, Access, Attempt, Book, Ended, Keys, Reach, ServerReach, settled_by_command,
    settled_by_history, settled_unfound,
};
use crate::route::pins::{self, PinRecord, PinState, PinnedWrite};
use crate::route::server::{self as calls, Answered};
use crate::route::shape::{DirectCall, Shape, shape};
use crate::session::{Command, Heard, feed_due};

/// The router: cheap to clone, one state behind every clone.
#[derive(Clone)]
pub(crate) struct Router {
    inner: Arc<Inner>,
}

struct Inner {
    pins_path: PathBuf,
    state: Mutex<State>,
    closed: watch::Sender<bool>,
}

struct State {
    server: Option<Server>,
    reach: ServerReach,
    feeders: HashMap<String, Entry>,
    book: Book,
    /// Feeds this phone started through the server, whose end the stream
    /// brings: key to feeder.
    following: HashMap<Uuid, String>,
    /// Ends the stream brought for a feed whose call has not returned yet.
    early: HashMap<Uuid, Settlement>,
    listeners: Vec<mpsc::UnboundedSender<RouterUpdate>>,
    /// The path each feeder was last said to take.
    announced: HashMap<String, Option<RoutePath>>,
}

struct Server {
    handle: ServerHandle,
    keys: Keys,
}

#[derive(Default)]
struct Entry {
    /// The server's id for it, when the server lists it.
    server: Option<FeederId>,
    /// What the server last said of its link: `None` before it said.
    online: Option<bool>,
    direct: Option<Arc<DirectLink>>,
}

fn now_ms() -> i64 {
    Timestamp::now().as_millisecond()
}

fn new_key() -> Result<Uuid, BridgeError> {
    Ok(uuid::Builder::from_random_bytes(random()?).into_uuid())
}

impl State {
    fn access(&self, feeder: &str) -> Option<Access> {
        let entry = self.feeders.get(feeder)?;
        let on_server = self.server.is_some() && entry.server.is_some();
        let keys = self.server.as_ref().map_or(Keys::Own, |server| server.keys);
        Access::of(on_server, entry.direct.is_some(), keys)
    }

    fn direct_open(&self, feeder: &str) -> bool {
        self.link(feeder).is_some_and(|link| link.is_open())
    }

    fn link(&self, feeder: &str) -> Option<Arc<DirectLink>> {
        self.feeders.get(feeder)?.direct.clone()
    }

    fn reach(&self, feeder: &str) -> Reach {
        Reach {
            server: self.reach,
            online_on_server: self.feeders.get(feeder).and_then(|entry| entry.online),
            direct_open: self.direct_open(feeder),
        }
    }

    fn route(&self, feeder: &str, now: i64) -> Option<RoutePath> {
        self.access(feeder)
            .map(|access| core::choose(access, self.reach(feeder), now))
    }

    /// The server's client and its id for `feeder`.
    fn server_for(&self, feeder: &str) -> Option<(wellfed_server_client::Client, FeederId)> {
        let server = self.server.as_ref()?;
        let id = self.feeders.get(feeder)?.server?;
        Some((server.handle.client.clone(), id))
    }

    fn server_id(&self, feeder: &str) -> Option<Uuid> {
        self.feeders
            .get(feeder)
            .and_then(|entry| entry.server)
            .map(|id| id.0)
    }

    /// The paths that changed since they were last said.
    fn paths(&mut self, now: i64) -> Vec<RouterUpdate> {
        let feeders: Vec<String> = self.feeders.keys().cloned().collect();
        let mut changed = Vec::new();
        for feeder in feeders {
            let path = self.route(&feeder, now);
            if self.announced.get(&feeder) != Some(&path) {
                self.announced.insert(feeder.clone(), path);
                changed.push(RouterUpdate::Path { feeder, path });
            }
        }
        changed
    }

    fn server_answered(&mut self) {
        self.reach = ServerReach::Up;
    }

    fn server_failed(&mut self, now: i64) {
        self.reach = ServerReach::Down { since_ms: now };
    }

    /// The server failed `failure`, as a call found it.
    fn heard_failure(&mut self, failure: core::ServerFailure, now: i64) {
        if core::server_down(failure) {
            self.server_failed(now);
        } else {
            self.server_answered();
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
}

impl Router {
    pub(crate) fn open(pins_path: PathBuf) -> Result<Self, BridgeError> {
        let mut book = Book {
            pins: pins::load(&pins_path)?,
        };
        if book.expire(now_ms()) {
            pins::save(&pins_path, &book.pins)?;
        }
        let state = State {
            server: None,
            reach: ServerReach::Up,
            feeders: HashMap::new(),
            book,
            following: HashMap::new(),
            early: HashMap::new(),
            listeners: Vec::new(),
            announced: HashMap::new(),
        };
        Ok(Self {
            inner: Arc::new(Inner {
                pins_path,
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

    fn persist(&self, state: &State) -> Result<(), BridgeError> {
        pins::save(&self.inner.pins_path, &state.book.pins)
    }

    /// Persist after a change the router has already made in memory: a
    /// failure leaves the pin in memory, where it still holds.
    fn persist_or_log(&self, state: &State) {
        if let Err(error) = self.persist(state) {
            log::warn!("the pending writes could not be saved: {error}");
        }
    }

    fn announce(&self, state: &mut State) {
        let changed = state.paths(now_ms());
        state.tell(&changed);
    }

    pub(crate) fn connect_server(&self, handle: ServerHandle, account: &ServerAccount) {
        let keys = if account.role == ServerRole::Viewer || account.expires_ms.is_some() {
            Keys::ServerOnly
        } else {
            Keys::Own
        };
        let mut state = self.state();
        state.server = Some(Server { handle, keys });
        state.reach = ServerReach::Up;
        self.announce(&mut state);
    }

    pub(crate) fn disconnect_server(&self) {
        let mut state = self.state();
        state.server = None;
        state.following.clear();
        self.announce(&mut state);
    }

    pub(crate) fn remember(&self, feeder: &str, id: &str) -> Result<(), ServerError> {
        let id = crate::server::ids::feeder(id)?;
        let mut state = self.state();
        state.feeders.entry(feeder.to_owned()).or_default().server = Some(id);
        self.announce(&mut state);
        Ok(())
    }

    pub(crate) async fn refresh(&self) -> Result<Vec<ServerFeeder>, ServerError> {
        let client = {
            let state = self.state();
            let Some(server) = state.server.as_ref() else {
                return Err(ServerError::new(
                    ServerErrorKind::Closed,
                    "no server is configured",
                ));
            };
            server.handle.client.clone()
        };

        let listed = client.feeders().await;
        let mut state = self.state();
        let feeders = match listed {
            Ok(feeders) => feeders,
            Err(error) => {
                state.heard_failure(calls::failure(&error), now_ms());
                self.announce(&mut state);
                return Err(error.into());
            }
        };
        state.server_answered();
        for entry in state.feeders.values_mut() {
            entry.server = None;
        }
        for feeder in &feeders {
            let entry = state
                .feeders
                .entry(feeder.identity.to_string())
                .or_default();
            entry.server = Some(feeder.id);
            entry.online = Some(feeder.connection == wellfed_api::Connection::Online);
        }
        self.announce(&mut state);
        Ok(feeders.into_iter().map(ServerFeeder::from).collect())
    }

    pub(crate) fn attach(&self, link: Arc<DirectLink>) {
        let feeder = link.feeder.clone();
        let weak = Arc::downgrade(&self.inner);
        let heard = feeder.clone();
        link.listen(Arc::new(move |news| {
            if let Some(inner) = weak.upgrade() {
                Self { inner }.link_news(&heard, news);
            }
        }));

        let mut state = self.state();
        state.feeders.entry(feeder.clone()).or_default().direct = Some(link);
        self.settle_direct(&mut state, &feeder);
        self.announce(&mut state);
    }

    fn link_news(&self, feeder: &str, news: LinkNews) {
        let mut state = self.state();
        // A session the router no longer holds says nothing of the feeder.
        let current = state
            .feeders
            .get(feeder)
            .and_then(|entry| entry.direct.as_ref());
        if current.is_none() {
            return;
        }
        if let LinkNews::Fed = news {
            self.settle_direct(&mut state, feeder);
        }
        self.announce(&mut state);
    }

    pub(crate) fn path(&self, feeder: &str) -> Option<RoutePath> {
        self.state().route(feeder, now_ms())
    }

    pub(crate) fn pending(&self) -> Vec<PendingWrite> {
        self.state().book.pins.iter().map(PinRecord::view).collect()
    }

    pub(crate) fn acknowledge(&self, key: &str) -> Result<(), BridgeError> {
        let Ok(key) = Uuid::parse_str(key) else {
            return Ok(());
        };
        let mut state = self.state();
        if state.book.acknowledge(key) {
            self.persist(&state)?;
        }
        Ok(())
    }

    /// Take `key` out as `settlement` said, and tell the screens.
    fn settle(&self, state: &mut State, key: Uuid, settlement: Settlement) -> Option<PendingWrite> {
        let pin = state.book.get(key)?.clone();
        state.book.settle(key);
        self.persist_or_log(state);
        if pin.path == RoutePath::Server
            && let Settlement::Started { .. } = settlement
        {
            state.following.insert(key, pin.feeder.clone());
        }
        let pending = pin.view();
        state.tell(&[RouterUpdate::Settled {
            pending: pending.clone(),
            settlement,
        }]);
        Some(pending)
    }

    /// Settle what the history `feeder`'s session read settles.
    fn settle_direct(&self, state: &mut State, feeder: &str) {
        let Some(link) = state.link(feeder) else {
            return;
        };
        let seen = link.seen();
        let settled: Vec<(Uuid, Settlement)> = {
            let others: Vec<&PinRecord> = state.book.pins.iter().collect();
            state
                .book
                .pins
                .iter()
                .filter(|pin| {
                    pin.feeder == feeder
                        && pin.path == RoutePath::Direct
                        && pin.state == PinState::Uncertain
                })
                .filter_map(|pin| {
                    settled_by_history(pin, &others, &seen).map(|settled| (pin.key, settled))
                })
                .collect()
        };
        for (key, settlement) in settled {
            self.settle(state, key, settlement);
        }
    }

    pub(crate) async fn feed(&self, feeder: &str, portions: u8) -> RoutedFeed {
        let Ok(key) = new_key() else {
            return RoutedFeed::NotSent { error: None };
        };
        let now = now_ms();

        let (access, mut path) = {
            let mut state = self.state();
            let Some(access) = state.access(feeder) else {
                return RoutedFeed::NotSent { error: None };
            };
            let path = core::choose(access, state.reach(feeder), now);
            let mark = match path {
                RoutePath::Direct => state.link(feeder).and_then(|link| link.newest()),
                RoutePath::Server => None,
            };
            let pin = PinRecord {
                key,
                feeder: feeder.to_owned(),
                server_feeder: state.server_id(feeder),
                path,
                write: PinnedWrite::Feed { portions },
                state: PinState::Sending,
                sent_ms: now,
                tried_ms: now,
                mark,
                excluded: Vec::new(),
                acknowledged: false,
            };
            if let Err(holding) = state.book.admit(pin) {
                if let Some(holding) = state.book.get(holding) {
                    return RoutedFeed::Held {
                        pending: holding.view(),
                    };
                }
                return RoutedFeed::NotSent { error: None };
            }
            // Written down before it goes, or it does not go.
            if self.persist(&state).is_err() {
                state.book.settle(key);
                return RoutedFeed::NotSent { error: None };
            }
            (access, path)
        };

        let mut fell_back = false;
        loop {
            let tried = match path {
                RoutePath::Server => self.feed_on_server(feeder, portions, key).await,
                RoutePath::Direct => self.feed_direct(feeder, portions).await,
            };

            let mut state = self.state();
            let direct_open = state.direct_open(feeder);
            if core::fallback(path, tried.ended, access, direct_open, fell_back).is_some() {
                let mark = state.link(feeder).and_then(|link| link.newest());
                if state.book.moved(key, mark, now_ms()) && self.persist(&state).is_ok() {
                    path = RoutePath::Direct;
                    fell_back = true;
                    continue;
                }
            }
            return self.fed(&mut state, feeder, key, path, portions, tried);
        }
    }

    /// The end of a feed's last attempt, on `path`.
    fn fed(
        &self,
        state: &mut State,
        feeder: &str,
        key: Uuid,
        path: RoutePath,
        portions: u8,
        tried: FeedTry,
    ) -> RoutedFeed {
        let pin = state.book.get(key).cloned();
        if let Some(pin) = pin {
            let pin = PinRecord {
                tried_ms: now_ms(),
                ..pin
            };
            state.book.ended(pin, tried.ended);
        }
        if let (RoutePath::Direct, Some(feed)) = (path, tried.started) {
            state.book.answered_direct(feeder, feed);
        }
        self.persist_or_log(state);
        let key_text = key.to_string();

        let routed = match (tried.ended, tried.started, tried.refusal) {
            (Ended::Answered, Some(feed), _) => {
                if path == RoutePath::Server {
                    match state.early.remove(&key) {
                        Some(settlement) => state.tell(&[RouterUpdate::FeedEnded {
                            key: key_text.clone(),
                            feeder: feeder.to_owned(),
                            settlement,
                        }]),
                        None => {
                            state.following.insert(key, feeder.to_owned());
                        }
                    }
                }
                RoutedFeed::Started {
                    key: key_text,
                    path,
                    feed,
                    portions,
                    due_seconds: u32::try_from(feed_due(portions).as_secs()).unwrap_or(u32::MAX),
                }
            }
            (Ended::Refused, _, Some(refusal)) => RoutedFeed::Refused {
                key: key_text,
                path,
                refusal,
            },
            (Ended::Uncertain, ..) => match state.book.get(key) {
                Some(pin) => RoutedFeed::Uncertain {
                    pending: pin.view(),
                },
                // Pinned before it went; only a broken invariant loses it.
                None => RoutedFeed::NotSent { error: tried.error },
            },
            _ => RoutedFeed::NotSent { error: tried.error },
        };
        state.early.remove(&key);
        self.announce(state);
        routed
    }

    async fn feed_on_server(&self, feeder: &str, portions: u8, key: Uuid) -> FeedTry {
        let Some((client, id)) = self.state().server_for(feeder) else {
            return FeedTry::unsent(None);
        };
        let sent = client.feed(id, portions, key).await;

        let mut state = self.state();
        match sent {
            Ok(command) => {
                state.server_answered();
                match settled_by_command(command.state, command.outcome.as_ref()) {
                    Some(Settlement::Started { feed }) => FeedTry {
                        ended: Ended::Answered,
                        started: Some(feed),
                        refusal: None,
                        error: None,
                    },
                    // A first attempt is answered once reserved; anything
                    // else is not known to have run or not.
                    _ => FeedTry {
                        ended: Ended::Uncertain,
                        started: None,
                        refusal: None,
                        error: None,
                    },
                }
            }
            Err(error) => {
                let failure = calls::failure(&error);
                state.heard_failure(failure, now_ms());
                FeedTry {
                    ended: core::server_ended(failure, Attempt::First),
                    started: None,
                    refusal: calls::refusal(&error).map(calls::feed_refusal),
                    error: Some(error.into()),
                }
            }
        }
    }

    async fn feed_direct(&self, feeder: &str, portions: u8) -> FeedTry {
        let Some(link) = self.state().link(feeder) else {
            return FeedTry::unsent(None);
        };
        let (reply, heard) = oneshot::channel();
        if link
            .commands
            .send(Command::Feed {
                portions,
                reply: Some(reply),
            })
            .is_err()
        {
            return FeedTry::unsent(None);
        }
        let tried = |ended, started, refusal| FeedTry {
            ended,
            started,
            refusal,
            error: None,
        };
        match heard.await {
            // Dropped unused: the session ended before it went.
            Err(_) | Ok(Heard::Held) => FeedTry::unsent(None),
            Ok(Heard::Unanswered) => tried(Ended::Uncertain, None, None),
            Ok(Heard::Answer(Answer::Feeding { feed })) => tried(Ended::Answered, Some(feed), None),
            Ok(Heard::Answer(Answer::Refused(refusal))) => tried(
                Ended::Refused,
                None,
                Some(calls::feed_refusal(requests::refusal_of(refusal))),
            ),
            // An answer that fits no feed says nothing of whether it ran.
            Ok(Heard::Answer(_)) => tried(Ended::Uncertain, None, None),
        }
    }

    pub(crate) async fn cancel(&self, feeder: &str, feed: u32) -> RoutedReply {
        self.write(
            feeder,
            PinnedWrite::Cancel { feed },
            DirectCall::Op(Op::CancelFeed { feed }),
        )
        .await
    }

    pub(crate) async fn ask(&self, feeder: &str, request: FeederRequest) -> RoutedReply {
        match shape(&request) {
            Shape::NotAsked(reason) => RoutedReply {
                path: None,
                reply: FeederReply::NotAsked { reason },
                pending: None,
                server_error: None,
            },
            Shape::Read => self.read(feeder, request).await,
            Shape::Write(write) => {
                self.write(feeder, write, DirectCall::Request(request))
                    .await
            }
        }
    }

    async fn read(&self, feeder: &str, request: FeederRequest) -> RoutedReply {
        let Some((access, path)) = self.chosen(feeder) else {
            return nowhere(None);
        };
        let mut server_error = None;
        if path == RoutePath::Server {
            let server = self.state().server_for(feeder);
            let read = match &server {
                Some((client, id)) => calls::read(client, *id, &request).await,
                None => None,
            };
            let mut state = self.state();
            match read {
                Some(Ok(reply)) => {
                    state.server_answered();
                    return answered(RoutePath::Server, reply, None);
                }
                Some(Err(error)) => {
                    let failure = calls::failure(&error);
                    state.heard_failure(failure, now_ms());
                    let elsewhere =
                        matches!(core::server_ended(failure, Attempt::First), Ended::Unsent);
                    if !(elsewhere && access == Access::Both && state.direct_open(feeder)) {
                        let reply = calls::refusal(&error)
                            .map_or(FeederReply::NotSent, |refusal| FeederReply::Refused {
                                refusal,
                            });
                        return answered(RoutePath::Server, reply, Some(error.into()));
                    }
                    server_error = Some(error.into());
                }
                // The server serves no such read: the phone's own session,
                // when it may use one.
                None if access == Access::Both => {}
                None => return nowhere(Some(no_route())),
            }
        }

        let Some(link) = self.state().link(feeder) else {
            return nowhere(server_error);
        };
        let reply = direct(&link, DirectCall::Request(request)).await;
        answered(RoutePath::Direct, reply, None)
    }

    async fn write(&self, feeder: &str, write: PinnedWrite, call: DirectCall) -> RoutedReply {
        let Ok(key) = new_key() else {
            return nowhere(None);
        };
        let Some((access, mut path)) = self.chosen(feeder) else {
            return nowhere(None);
        };
        let sent_ms = now_ms();
        let mut call = Some(call);
        let mut fell_back = false;

        loop {
            let link = self.state().link(feeder);
            let (ended, reply, server_error) = match path {
                RoutePath::Server => self.write_on_server(feeder, &write, key).await,
                RoutePath::Direct => match (link, call.take()) {
                    (Some(link), Some(call)) => {
                        let reply = direct(&link, call).await;
                        let ended = match reply {
                            FeederReply::Unanswered => Ended::Uncertain,
                            FeederReply::NotSent
                            | FeederReply::NotAsked {
                                reason: NotAsked::Held,
                            } => Ended::Unsent,
                            _ => Ended::Answered,
                        };
                        (ended, reply, None)
                    }
                    _ => (Ended::Unsent, FeederReply::NotSent, None),
                },
            };

            let mut state = self.state();
            let direct_open = state.direct_open(feeder);
            if core::fallback(path, ended, access, direct_open, fell_back).is_some() {
                path = RoutePath::Direct;
                fell_back = true;
                continue;
            }

            let mut pending = None;
            if ended == Ended::Uncertain {
                let pin = PinRecord {
                    key,
                    feeder: feeder.to_owned(),
                    server_feeder: state.server_id(feeder),
                    path,
                    write,
                    state: PinState::Uncertain,
                    sent_ms,
                    tried_ms: now_ms(),
                    mark: None,
                    excluded: Vec::new(),
                    acknowledged: false,
                };
                pending = Some(pin.view());
                state.book.ended(pin, Ended::Uncertain);
                self.persist_or_log(&state);
            }
            self.announce(&mut state);
            return RoutedReply {
                path: Some(path),
                reply,
                pending,
                server_error,
            };
        }
    }

    async fn write_on_server(
        &self,
        feeder: &str,
        write: &PinnedWrite,
        key: Uuid,
    ) -> (Ended, FeederReply, Option<ServerError>) {
        let Some((client, id)) = self.state().server_for(feeder) else {
            return (Ended::Unsent, FeederReply::NotSent, None);
        };
        let Some(sent) = calls::send(&client, id, write, key).await else {
            // No server route: nothing went out.
            return (Ended::Unsent, FeederReply::NotSent, Some(no_route()));
        };

        let mut state = self.state();
        match sent {
            Ok(Answered::Done { generation }) => {
                state.server_answered();
                (Ended::Answered, FeederReply::Done { generation }, None)
            }
            // Only a feed answers with its entry, and feeds go by `feed`.
            Ok(Answered::Feed(_)) => (Ended::Answered, FeederReply::Unexpected, None),
            Err(error) => {
                let failure = calls::failure(&error);
                state.heard_failure(failure, now_ms());
                let ended = core::server_ended(failure, Attempt::First);
                let reply = match (ended, calls::refusal(&error)) {
                    (Ended::Uncertain, _) => FeederReply::Unanswered,
                    (Ended::Refused, Some(refusal)) => FeederReply::Refused { refusal },
                    _ => FeederReply::NotSent,
                };
                (ended, reply, Some(error.into()))
            }
        }
    }

    /// The access and the path a new request to `feeder` takes now.
    fn chosen(&self, feeder: &str) -> Option<(Access, RoutePath)> {
        let state = self.state();
        let access = state.access(feeder)?;
        Some((access, core::choose(access, state.reach(feeder), now_ms())))
    }

    pub(crate) async fn retry(&self, key: &str) -> RetryOutcome {
        let Ok(key) = Uuid::parse_str(key) else {
            return RetryOutcome::Unknown;
        };
        let (pin, server) = {
            let state = self.state();
            let Some(pin) = state.book.get(key).cloned() else {
                return RetryOutcome::Unknown;
            };
            let server = state
                .server
                .as_ref()
                .map(|server| server.handle.client.clone());
            (pin, server)
        };
        if pin.state == PinState::Sending {
            // Its first attempt is still out.
            return RetryOutcome::Uncertain {
                pending: pin.view(),
            };
        }

        match pin.path {
            RoutePath::Server => self.retry_on_server(pin, server).await,
            RoutePath::Direct => {
                let mut state = self.state();
                self.settle_direct(&mut state, &pin.feeder);
                still(&state, &pin)
            }
        }
    }

    async fn retry_on_server(
        &self,
        pin: PinRecord,
        client: Option<wellfed_server_client::Client>,
    ) -> RetryOutcome {
        let (Some(client), Some(id)) = (client, pin.server_feeder) else {
            return RetryOutcome::Uncertain {
                pending: pin.view(),
            };
        };
        let sent = calls::send(&client, FeederId(id), &pin.write, pin.key).await;

        let mut state = self.state();
        let settlement = match sent {
            None => None,
            Some(Ok(Answered::Feed(command))) => {
                state.server_answered();
                settled_by_command(command.state, command.outcome.as_ref())
            }
            Some(Ok(Answered::Done { .. })) => {
                state.server_answered();
                Some(Settlement::Done)
            }
            Some(Err(error)) => {
                let failure = calls::failure(&error);
                state.heard_failure(failure, now_ms());
                match core::server_ended(failure, Attempt::Retry) {
                    Ended::Refused => Some(Settlement::Refused {
                        refusal: refusal_of(&error),
                    }),
                    Ended::Superseded => Some(Settlement::Superseded),
                    Ended::Answered | Ended::Unsent | Ended::Uncertain => None,
                }
            }
        };
        if let Some(settlement) = settlement
            && let Some(pending) = self.settle(&mut state, pin.key, settlement.clone())
        {
            self.announce(&mut state);
            return RetryOutcome::Settled {
                pending,
                settlement,
            };
        }
        if let Some(held) = state.book.get_mut(pin.key) {
            held.tried_ms = now_ms();
        }
        self.persist_or_log(&state);
        self.announce(&mut state);
        still(&state, &pin)
    }

    pub(crate) async fn resolve(&self) {
        let (asks, client) = {
            let mut state = self.state();
            let feeders: Vec<String> = state.feeders.keys().cloned().collect();
            for feeder in feeders {
                self.settle_direct(&mut state, &feeder);
            }
            let asks: Vec<PinRecord> = state
                .book
                .pins
                .iter()
                .filter(|pin| pin.path == RoutePath::Server && pin.state == PinState::Uncertain)
                .cloned()
                .collect();
            let client = state
                .server
                .as_ref()
                .map(|server| server.handle.client.clone());
            (asks, client)
        };
        let Some(client) = client else {
            return;
        };

        for pin in asks {
            let read = client.command(pin.key).await;
            let mut state = self.state();
            let settlement = match read {
                Ok(command) => {
                    state.server_answered();
                    settled_by_command(command.state, command.outcome.as_ref())
                }
                Err(wellfed_server_client::Error::Problem { problem, .. })
                    if problem.code == wellfed_api::ErrorCode::NotFound =>
                {
                    state.server_answered();
                    settled_unfound(&pin, now_ms())
                }
                Err(error) => {
                    state.heard_failure(calls::failure(&error), now_ms());
                    None
                }
            };
            if let Some(settlement) = settlement {
                self.settle(&mut state, pin.key, settlement);
            }
            self.announce(&mut state);
        }
    }

    pub(crate) async fn follow(
        &self,
        last_event_id: Option<String>,
        mut emit: impl FnMut(ServerUpdate) -> bool,
    ) {
        let handle = self
            .state()
            .server
            .as_ref()
            .map(|server| server.handle.clone());
        let Some(handle) = handle else {
            let error = ServerError::new(ServerErrorKind::Closed, "no server is configured");
            // Nobody listening means nobody to tell.
            let _ = emit(ServerUpdate::Closed { error: Some(error) });
            return;
        };
        handle
            .follow(last_event_id, |update| {
                self.heard(&update);
                emit(update)
            })
            .await;
    }

    /// What the server's stream says, as the router reads it.
    fn heard(&self, update: &ServerUpdate) {
        let mut state = self.state();
        match update {
            ServerUpdate::Opened => {
                state.server_answered();
                self.resolve_later();
            }
            ServerUpdate::Event { event, .. } => match event {
                ServerEvent::Connection { feeder, connection } => {
                    if let Ok(id) = Uuid::parse_str(feeder) {
                        for entry in state.feeders.values_mut() {
                            if entry.server.map(|server| server.0) == Some(id) {
                                entry.online = Some(*connection == ServerConnection::Online);
                            }
                        }
                    }
                }
                ServerEvent::Command { command } => self.commanded(&mut state, command),
                ServerEvent::Resync => self.resolve_later(),
                ServerEvent::History { .. } | ServerEvent::Install { .. } => {}
            },
            ServerUpdate::Closed { error } => {
                let down = error.as_ref().is_some_and(|error| {
                    matches!(
                        error.kind,
                        ServerErrorKind::Unreachable
                            | ServerErrorKind::Interrupted
                            | ServerErrorKind::TokenRevoked
                            | ServerErrorKind::AccountDisabled
                    )
                });
                if down {
                    state.server_failed(now_ms());
                }
            }
        }
        self.announce(&mut state);
    }

    /// A journal entry moved on.
    fn commanded(&self, state: &mut State, command: &ServerCommand) {
        let Ok(key) = Uuid::parse_str(&command.key) else {
            return;
        };
        let settlement = view_settlement(command);

        match state.book.get(key).map(|pin| pin.state) {
            Some(PinState::Uncertain) => {
                if let Some(settlement) = settlement {
                    self.settle(state, key, settlement);
                }
            }
            // Its call is still out, and settles it itself; an end that
            // arrives first is kept for it.
            Some(PinState::Sending) => {
                if let Some(settlement @ (Settlement::Fed { .. } | Settlement::Unrecorded)) =
                    settlement
                {
                    state.early.insert(key, settlement);
                }
            }
            None => {
                if let Some(settlement @ (Settlement::Fed { .. } | Settlement::Unrecorded)) =
                    settlement
                    && let Some(feeder) = state.following.remove(&key)
                {
                    state.tell(&[RouterUpdate::FeedEnded {
                        key: command.key.clone(),
                        feeder,
                        settlement,
                    }]);
                }
            }
        }
    }

    fn resolve_later(&self) {
        let router = self.clone();
        tokio::spawn(async move { router.resolve().await });
    }

    pub(crate) async fn updates(&self, mut emit: impl FnMut(RouterUpdate) -> bool) {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let mut closed = self.inner.closed.subscribe();
        {
            let mut state = self.state();
            state.listeners.push(sender);
            // Every feeder's path, said afresh to this listener.
            let now = now_ms();
            let feeders: Vec<String> = state.feeders.keys().cloned().collect();
            for feeder in feeders {
                let path = state.route(&feeder, now);
                if !emit(RouterUpdate::Path { feeder, path }) {
                    return;
                }
            }
        }
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

/// How one attempt at a feed ended.
struct FeedTry {
    ended: Ended,
    /// The feeder's number for it, once reserved.
    started: Option<u32>,
    refusal: Option<crate::api::session::FeedRefusal>,
    error: Option<ServerError>,
}

impl FeedTry {
    const fn unsent(error: Option<ServerError>) -> Self {
        Self {
            ended: Ended::Unsent,
            started: None,
            refusal: None,
            error,
        }
    }
}

/// Run `call` on the phone's own session behind `link`.
async fn direct(link: &DirectLink, call: DirectCall) -> FeederReply {
    match call {
        DirectCall::Request(request) => requests::run(&link.commands, link.identity, request).await,
        DirectCall::Op(op) => {
            let (reply, heard) = oneshot::channel();
            if link.commands.send(Command::Ask { op, reply }).is_err() {
                return FeederReply::NotSent;
            }
            match heard.await {
                Ok(Heard::Answer(answer)) => requests::reply_of(answer),
                Ok(Heard::Unanswered) => FeederReply::Unanswered,
                Ok(Heard::Held) | Err(_) => FeederReply::NotSent,
            }
        }
    }
}

const fn answered(
    path: RoutePath,
    reply: FeederReply,
    server_error: Option<ServerError>,
) -> RoutedReply {
    RoutedReply {
        path: Some(path),
        reply,
        pending: None,
        server_error,
    }
}

const fn nowhere(server_error: Option<ServerError>) -> RoutedReply {
    RoutedReply {
        path: None,
        reply: FeederReply::NotSent,
        pending: None,
        server_error,
    }
}

fn no_route() -> ServerError {
    ServerError::new(
        ServerErrorKind::Refused,
        "the server has no route for this request, and this phone holds no key of its own",
    )
}

/// The pin as it stands after a retry that settled nothing.
fn still(state: &State, pin: &PinRecord) -> RetryOutcome {
    match state.book.get(pin.key) {
        Some(held) => RetryOutcome::Uncertain {
            pending: held.view(),
        },
        None => RetryOutcome::Unknown,
    }
}

/// The refusal a failed retry replays.
fn refusal_of(error: &wellfed_server_client::Error) -> ServerRefusal {
    match error {
        wellfed_server_client::Error::Problem { problem, .. } => problem
            .refusal
            .map_or(ServerRefusal::Invalid, ServerRefusal::from),
        _ => ServerRefusal::Invalid,
    }
}

/// What a journal entry the stream carried settles; `None` while unknown
/// or in flight.
fn view_settlement(command: &ServerCommand) -> Option<Settlement> {
    use wellfed_api::CommandState;
    if matches!(command.state, CommandState::Sent | CommandState::Unknown) {
        return None;
    }
    let outcome = command.outcome.as_ref()?;
    Some(match outcome {
        ServerCommandOutcome::Done
        | ServerCommandOutcome::Settings
        | ServerCommandOutcome::Plans
        | ServerCommandOutcome::Invited { .. } => Settlement::Done,
        ServerCommandOutcome::Refused { refusal } => Settlement::Refused { refusal: *refusal },
        ServerCommandOutcome::Feeding { feed } => Settlement::Started { feed: *feed },
        ServerCommandOutcome::Fed {
            feed,
            requested,
            completed,
            outcome,
            reason,
        } => Settlement::Fed {
            feed: *feed,
            requested: *requested,
            completed: *completed,
            outcome: *outcome,
            reason: *reason,
        },
        ServerCommandOutcome::Unrecorded => Settlement::Unrecorded,
    })
}
