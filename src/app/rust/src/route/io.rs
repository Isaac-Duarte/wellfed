//! The router's IO: the server's calls and the direct sessions' commands
//! driven around the rules in [`super::core`], with the pins kept on disk
//! by [`super::pins`]. One lock holds what the router knows; no call holds
//! it across an await, and a write's admission and its pin are written
//! under one hold before it goes, so two feeds asked at once cannot both
//! pass and a write that never got written down never goes.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use jiff::Timestamp;
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;
use wellfed_api::{ErrorCode, FeederId, User};
use wellfed_model::{Answer, Op};

use crate::api::error::{BridgeError, ErrorKind};
use crate::api::feeder::{FeederReply, FeederRequest, NotAsked};
use crate::api::keys::random;
use crate::api::router::{
    NotSentReason, PendingWrite, RetryOutcome, RoutePath, RoutedFeed, RoutedReply, RouterUpdate,
    Settlement,
};
use crate::api::server::{
    ServerCommand, ServerCommandOutcome, ServerEvent, ServerHandle, ServerRefusal, ServerUpdate,
};
use crate::api::server_error::{ServerError, ServerErrorKind};
use crate::api::server_views::{
    KnownServer, ServerAccount, ServerConnection, ServerFeeder, ServerRole,
};
use crate::api::session::FeedRefusal;
use crate::api::store::SetAside;
use crate::link::{DirectLink, LinkNews};
use crate::requests;
use crate::route::core::{
    self, Access, Book, Ended, Held, Keys, Reach, RetryPlan, ServerFailure, ServerReach, Standing,
    settled_by_command, settled_by_history,
};
use crate::route::pins::{LostAdoption, PinRecord, PinState, PinStore, PinnedWrite, ServerMark};
use crate::route::server as calls;
use crate::route::shape::{DirectCall, SentWrite, Shape, shape};
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
    server: Option<Server>,
    reach: ServerReach,
    /// Whether the configured server still takes this phone's token. Apart
    /// from `reach`, so a refused token stays refused when that server then
    /// goes down; only a fresh login or removing the server clears it.
    standing: Standing,
    feeders: HashMap<String, Entry>,
    book: Book,
    /// Feeds this phone started through the server, whose end the stream
    /// brings: key to feeder.
    following: HashMap<Uuid, String>,
    /// What the stream said of a write whose call has not returned yet.
    early: HashMap<Uuid, Settlement>,
    listeners: Vec<mpsc::UnboundedSender<RouterUpdate>>,
    /// The path each feeder was last said to take.
    announced: HashMap<String, Option<RoutePath>>,
    /// A server the app keeps, before its session is handed over: the keys
    /// its kept account may use, so no feeder goes on the phone's own key
    /// that the account would not allow while the app is still reading its
    /// token and asking the server.
    cold: Option<Keys>,
}

struct Server {
    handle: ServerHandle,
    keys: Keys,
    /// The account's id, as the server last said.
    user: Option<Uuid>,
    /// The account as the server last said, or as the app kept it while
    /// the server has not answered yet.
    account: Option<ServerAccount>,
}

impl Server {
    /// Whose journal a write sent now goes to.
    fn mark(&self) -> Option<ServerMark> {
        Some(ServerMark {
            token: self.handle.token?,
            user: self.user?,
        })
    }
}

#[derive(Default)]
struct Entry {
    /// The server's id for it, when the server lists it.
    server: Option<FeederId>,
    /// What the server last said of its link: `None` before it said.
    online: Option<bool>,
    direct: Option<Arc<DirectLink>>,
}

pub(crate) fn now_ms() -> i64 {
    Timestamp::now().as_millisecond()
}

fn new_key() -> Result<Uuid, BridgeError> {
    Ok(uuid::Builder::from_random_bytes(random()?).into_uuid())
}

/// Viewers and accounts with an expiry hold no key of their own.
fn keys_of(account: &ServerAccount) -> Keys {
    if account.role == ServerRole::Viewer || account.expires_ms.is_some() {
        Keys::ServerOnly
    } else {
        Keys::Own
    }
}

impl State {
    fn access(&self, feeder: &str) -> Option<Access> {
        let entry = self.feeders.get(feeder)?;
        let on_server = self.server.is_some() && entry.server.is_some();
        let keys = match &self.server {
            Some(server) => server.keys,
            None => self.cold.unwrap_or(Keys::Own),
        }
        .under(self.standing);
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

    fn client(&self) -> Option<wellfed_server_client::Client> {
        self.server
            .as_ref()
            .map(|server| server.handle.client.clone())
    }

    /// The server's client and its id for `feeder`.
    fn server_for(&self, feeder: &str) -> Option<(wellfed_server_client::Client, FeederId)> {
        let client = self.client()?;
        let id = self.feeders.get(feeder)?.server?;
        Some((client, id))
    }

    fn server_id(&self, feeder: &str) -> Option<Uuid> {
        self.feeders
            .get(feeder)
            .and_then(|entry| entry.server)
            .map(|id| id.0)
    }

    /// A write about to go on `path`, as its pin records it.
    fn pin(
        &self,
        key: Uuid,
        feeder: &str,
        path: RoutePath,
        write: PinnedWrite,
        now: i64,
    ) -> PinRecord {
        let (server, mark) = match path {
            RoutePath::Server => (self.server.as_ref().and_then(Server::mark), None),
            RoutePath::Direct => (None, self.link(feeder).and_then(|link| link.newest())),
        };
        PinRecord {
            key,
            feeder: feeder.to_owned(),
            server_feeder: self.server_id(feeder),
            server,
            path,
            write,
            state: PinState::Sending,
            sent_ms: now,
            tried_ms: now,
            mark,
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

    /// The account as the server says it is now: a demotion or a new
    /// expiry takes the phone's own keys away from here on. The app is told
    /// of a change, and keeps it for a cold start with the server down.
    fn account_is(&mut self, user: User) {
        let account = ServerAccount::from(user);
        let Some(server) = self.server.as_mut() else {
            return;
        };
        server.keys = keys_of(&account);
        server.user = Uuid::parse_str(&account.id).ok();
        if server.account.as_ref() != Some(&account) {
            server.account = Some(account.clone());
            self.tell(&[RouterUpdate::Account { account }]);
        }
    }

    /// What a failed call says of the server.
    fn heard_failure(&mut self, failure: ServerFailure, now: i64) {
        self.reach = core::reach_after(failure, now);
        self.standing = core::standing_after(self.standing, failure);
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

    fn announce(&mut self) {
        let changed = self.paths(now_ms());
        self.tell(&changed);
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

    /// Take `key` out as `settlement` says, without telling anyone: the
    /// caller hands it back itself. A server feed that started is followed
    /// to its end.
    fn settle_quiet(&mut self, key: Uuid, settlement: &Settlement) -> Option<PendingWrite> {
        let pin = self.book.get(key)?.clone();
        self.book.settle(key);
        self.early.remove(&key);
        self.save_or_log();
        if pin.path == RoutePath::Server
            && let Settlement::Started { .. } = settlement
        {
            self.following.insert(key, pin.feeder.clone());
        }
        Some(pin.view())
    }

    /// Take `key` out as `settlement` says, and tell the screens.
    fn settle(&mut self, key: Uuid, settlement: Settlement) -> Option<PendingWrite> {
        let pending = self.settle_quiet(key, &settlement)?;
        self.tell(&[RouterUpdate::Settled {
            pending: pending.clone(),
            settlement,
        }]);
        Some(pending)
    }

    /// Settle what the history `feeder`'s session read settles, and drop
    /// the running feeds it gave the ends of. What it settled.
    fn settle_direct(&mut self, feeder: &str) -> Vec<(Uuid, Settlement)> {
        let Some(link) = self.link(feeder) else {
            return Vec::new();
        };
        let seen = link.seen();
        let ended: Vec<u32> = seen.iter().map(|seen| seen.feed).collect();
        if self.book.ended_direct(feeder, &ended) {
            self.save_or_log();
        }

        let settled: Vec<(Uuid, Settlement)> = {
            let others: Vec<&PinRecord> = self.book.pins.iter().collect();
            self.book
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
        for (key, settlement) in &settled {
            self.settle(*key, settlement.clone());
        }
        settled
    }

    /// Keep what the stream said of a write whose call is still out; an
    /// end is never replaced by its start.
    fn keep_early(&mut self, key: Uuid, settlement: Settlement) {
        let ended = matches!(
            self.early.get(&key),
            Some(Settlement::Fed { .. } | Settlement::Unrecorded)
        );
        if !(ended && matches!(settlement, Settlement::Started { .. })) {
            self.early.insert(key, settlement);
        }
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
            server: None,
            reach: ServerReach::Up,
            standing: Standing::Taken,
            feeders: HashMap::new(),
            book,
            following: HashMap::new(),
            early: HashMap::new(),
            listeners: Vec::new(),
            announced: HashMap::new(),
            cold: None,
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

    pub(crate) fn connect_server(&self, handle: ServerHandle, account: &ServerAccount) {
        let mut state = self.state();
        state.server = Some(Server {
            handle,
            keys: keys_of(account),
            user: Uuid::parse_str(&account.id).ok(),
            account: Some(account.clone()),
        });
        state.reach = ServerReach::Up;
        state.standing = Standing::Taken;
        state.announce();
    }

    /// The server the app keeps, before anything else asks the router: its
    /// feeders' ids, and the keys its kept account may use (none of its own
    /// with no account kept) until the session is handed over. It sends
    /// nothing.
    pub(crate) fn configure(&self, kept: &KnownServer) {
        let mut state = self.state();
        for listed in &kept.feeders {
            // A kept id that does not parse: the next list replaces it.
            if let Ok(id) = crate::server::ids::feeder(&listed.id) {
                state
                    .feeders
                    .entry(listed.identity.clone())
                    .or_default()
                    .server = Some(id);
            }
        }
        state.cold = Some(kept.account.as_ref().map_or(Keys::ServerOnly, keys_of));
        state.announce();
    }

    /// A server whose phone token secret is gone: the app is configured
    /// with it, so nothing goes on the phone's own keys, as with a refused
    /// token, until a login or removing the server.
    pub(crate) fn refuse_server(&self) {
        let mut state = self.state();
        state.server = None;
        state.standing = Standing::Refused;
        state.following.clear();
        state.announce();
    }

    /// Route through `handle`, a session the app kept, at a cold start:
    /// what the server says of the account now, and with the server not
    /// answering, `cached`, the account as the last login or list read it.
    /// With neither, the account's standing is unknown, so it holds no
    /// key of its own until the server says: a viewer or an account with an
    /// expiry must never fall to the phone's own keys because its server was
    /// down when the app started. A refused token stays refused.
    pub(crate) async fn resume(
        &self,
        handle: ServerHandle,
        cached: Option<ServerAccount>,
    ) -> Result<ServerAccount, ServerError> {
        let read = handle.client.session().await;

        let error = match read {
            Ok(user) => {
                let account = ServerAccount::from(user);
                self.connect_server(handle, &account);
                return Ok(account);
            }
            Err(error) => error,
        };
        let failure = calls::failure(&error);
        let mut state = self.state();
        let (keys, user) = match (&cached, failure) {
            (_, ServerFailure::Unauthorized) | (None, _) => (Keys::ServerOnly, None),
            (Some(account), _) => (keys_of(account), Uuid::parse_str(&account.id).ok()),
        };
        state.server = Some(Server {
            handle,
            keys,
            user,
            account: cached,
        });
        state.heard_failure(failure, now_ms());
        state.announce();
        Err(error.into())
    }

    pub(crate) fn disconnect_server(&self) {
        let mut state = self.state();
        state.server = None;
        state.cold = None;
        state.reach = ServerReach::Up;
        state.standing = Standing::Taken;
        state.following.clear();
        state.announce();
    }

    pub(crate) async fn refresh(&self) -> Result<Vec<ServerFeeder>, ServerError> {
        let Some(client) = self.state().client() else {
            return Err(ServerError::new(
                ServerErrorKind::Closed,
                "no server is configured",
            ));
        };

        let (account, listed) = tokio::join!(client.session(), client.feeders());
        let mut state = self.state();
        let now = now_ms();
        let read = account.and_then(|user| listed.map(|feeders| (user, feeders)));
        let (user, feeders) = match read {
            Ok(read) => read,
            Err(error) => {
                state.heard_failure(calls::failure(&error), now);
                state.announce();
                return Err(error.into());
            }
        };
        state.reach = ServerReach::Up;
        state.account_is(user);
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
        state.announce();
        Ok(feeders.into_iter().map(ServerFeeder::from).collect())
    }

    /// When `feeder`'s adoption last lost its answer, while that is not
    /// settled.
    pub(crate) fn adoption_lost_at(&self, feeder: &str) -> Option<i64> {
        let state = self.state();
        state
            .book
            .adoptions
            .iter()
            .find(|lost| lost.feeder == feeder)
            .map(|lost| lost.at_ms)
    }

    /// `feeder`'s adoption lost its answer at `at`; `None` once settled.
    pub(crate) fn adoption_lost(&self, feeder: &str, at: Option<i64>) {
        let mut state = self.state();
        let before = state.book.adoptions.clone();
        state.book.adoptions.retain(|lost| lost.feeder != feeder);
        if let Some(at_ms) = at {
            state.book.adoptions.push(LostAdoption {
                feeder: feeder.to_owned(),
                at_ms,
            });
        }
        if state.book.adoptions != before {
            // Kept for a restart; one that fails to save still holds this run.
            state.save_or_log();
        }
    }

    /// How long a lost adoption waits before the server's list may permit
    /// another invite. This never settles a missing command journal entry.
    pub(crate) fn grace_ms(&self) -> i64 {
        core::ADOPTION_GRACE_MS
    }

    /// The server's client, when one is configured.
    pub(crate) fn server_client(&self) -> Option<wellfed_server_client::Client> {
        self.state().client()
    }

    /// The server's id for `feeder`, while a server is configured and its
    /// last list named it.
    pub(crate) fn listed(&self, feeder: &str) -> Option<FeederId> {
        let state = self.state();
        state.server.as_ref()?;
        state.feeders.get(feeder)?.server
    }

    /// Read the account again, so a demotion or a new expiry routes from
    /// now on.
    async fn reread_account(&self) {
        let Some(client) = self.state().client() else {
            return;
        };
        let read = client.session().await;
        let mut state = self.state();
        match read {
            Ok(user) => {
                state.reach = ServerReach::Up;
                state.account_is(user);
            }
            Err(error) => state.heard_failure(calls::failure(&error), now_ms()),
        }
        state.announce();
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
        state.settle_direct(&feeder);
        state.announce();
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
            state.settle_direct(feeder);
        }
        state.announce();
    }

    pub(crate) fn path(&self, feeder: &str) -> Option<RoutePath> {
        self.state().route(feeder, now_ms())
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
            return not_sent(NotSentReason::Random, None);
        };
        let now = now_ms();

        let (access, mut path) = {
            let mut state = self.state();
            let Some(access) = state.access(feeder) else {
                return not_sent(NotSentReason::NoPath, None);
            };
            let path = core::choose(access, state.reach(feeder), now);
            let pin = state.pin(key, feeder, path, PinnedWrite::Feed { portions }, now);
            match state.book.admit(pin, now) {
                Ok(()) => {}
                Err(Held::SetAside) => return not_sent(NotSentReason::SetAside, None),
                Err(Held::By(holding)) => {
                    return match state.book.get(holding) {
                        Some(holding) => RoutedFeed::Held {
                            pending: holding.view(),
                        },
                        None => not_sent(NotSentReason::NoPath, None),
                    };
                }
            }
            // Written down before it goes, or it does not go.
            if state.save().is_err() {
                state.book.settle(key);
                return not_sent(NotSentReason::Store, None);
            }
            (access, path)
        };

        let mut fell_back = false;
        loop {
            let mut tried = match path {
                RoutePath::Server => self.feed_on_server(feeder, portions, key).await,
                RoutePath::Direct => self.feed_direct(feeder, portions).await,
            };

            let mut state = self.state();
            let direct_open = state.direct_open(feeder);
            if core::fallback(path, tried.ended(), access, direct_open, fell_back).is_some() {
                let mark = state.link(feeder).and_then(|link| link.newest());
                if state.book.moved(key, mark, now_ms()) {
                    if state.save().is_ok() {
                        path = RoutePath::Direct;
                        fell_back = true;
                        continue;
                    }
                    tried = FeedTry::Unsent(NotSentReason::Store, None);
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
        let now = now_ms();
        let held = state.book.get(key).map(PinRecord::view);
        state.book.ended(key, tried.ended(), now);
        if let (RoutePath::Direct, FeedTry::Started(feed)) = (path, &tried) {
            // Its end may come late, after a gap in what the session reads:
            // until then it is no other feed's end.
            state
                .book
                .started_direct(feeder, *feed, now.saturating_add(core::KEEP_MS));
        }
        state.save_or_log();
        let early = state.early.remove(&key);
        let key_text = key.to_string();

        let routed = match tried {
            FeedTry::Started(feed) => {
                if path == RoutePath::Server {
                    match early {
                        Some(settlement @ (Settlement::Fed { .. } | Settlement::Unrecorded)) => {
                            state.tell(&[RouterUpdate::FeedEnded {
                                key: key_text.clone(),
                                feeder: feeder.to_owned(),
                                settlement,
                            }]);
                        }
                        _ => {
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
            FeedTry::Refused(refusal) => RoutedFeed::Refused {
                key: key_text,
                path,
                refusal,
            },
            FeedTry::Uncertain => match early {
                // The stream told its end while the call was out.
                Some(settlement) => match state.settle_quiet(key, &settlement) {
                    Some(pending) => RoutedFeed::Settled {
                        pending,
                        settlement,
                    },
                    None => RoutedFeed::NotSent {
                        reason: NotSentReason::Store,
                        error: None,
                    },
                },
                None => {
                    if path == RoutePath::Server {
                        self.resolve_later();
                    }
                    match state.book.get(key) {
                        Some(pin) => RoutedFeed::Uncertain {
                            pending: pin.view(),
                        },
                        // Pinned before it went; only a broken invariant
                        // loses it, and then nothing more is known.
                        None => RoutedFeed::NotSent {
                            reason: NotSentReason::Store,
                            error: None,
                        },
                    }
                }
            },
            FeedTry::Unsent(reason, error) => RoutedFeed::NotSent { reason, error },
            FeedTry::Rejected(error) => RoutedFeed::NotSent {
                reason: NotSentReason::Server,
                error,
            },
        };
        // A screen made again while the call was out took the pin up from
        // `pending` as uncertain, and only hears its end here; the caller's
        // own screen has the call's answer, and a feed still sending there
        // is no key's yet.
        if let Some(pending) = held
            && let Some(settlement) = feed_settlement(&routed)
        {
            state.tell(&[RouterUpdate::Settled {
                pending,
                settlement,
            }]);
        }
        state.announce();
        routed
    }

    async fn feed_on_server(&self, feeder: &str, portions: u8, key: Uuid) -> FeedTry {
        let Some((client, id)) = self.state().server_for(feeder) else {
            return FeedTry::Unsent(NotSentReason::NoPath, None);
        };
        let sent = client.feed(id, portions, key).await;

        let mut state = self.state();
        match sent {
            Ok(command) => {
                state.reach = ServerReach::Up;
                match settled_by_command(command.state, command.outcome.as_ref()) {
                    Some(Settlement::Started { feed }) => FeedTry::Started(feed),
                    Some(Settlement::Refused { .. }) => FeedTry::Refused(FeedRefusal::Other),
                    // A first attempt is answered once reserved; anything
                    // else is not known to have run or not.
                    _ => FeedTry::Uncertain,
                }
            }
            Err(error) => {
                let failure = calls::failure(&error);
                state.heard_failure(failure, now_ms());
                let refusal = calls::refusal(&error).map(calls::feed_refusal);
                let error = Some(error.into());
                match core::server_ended(failure) {
                    Ended::Unsent => FeedTry::Unsent(NotSentReason::Server, error),
                    // A refusal of the feeder's own is the feed sheet's; the server's
                    // (a viewer may not feed) is in `error`.
                    Ended::Refused => match refusal {
                        Some(refusal) => FeedTry::Refused(refusal),
                        None => FeedTry::Rejected(error),
                    },
                    Ended::Answered | Ended::Uncertain => FeedTry::Uncertain,
                }
            }
        }
    }

    async fn feed_direct(&self, feeder: &str, portions: u8) -> FeedTry {
        let Some(link) = self.state().link(feeder) else {
            return FeedTry::Unsent(NotSentReason::NoPath, None);
        };
        let (reply, heard) = oneshot::channel();
        let command = Command::Feed {
            portions,
            reply: Some(reply),
        };
        if link.commands.send(command).is_err() {
            return FeedTry::Unsent(NotSentReason::Session, None);
        }
        match heard.await {
            // Dropped without a word: it may have gone.
            Err(_) | Ok(Heard::Unanswered) => FeedTry::Uncertain,
            Ok(Heard::NotSent | Heard::Held) => FeedTry::Unsent(NotSentReason::Session, None),
            Ok(Heard::Answer(Answer::Feeding { feed })) => FeedTry::Started(feed),
            Ok(Heard::Answer(Answer::Refused(refusal))) => {
                FeedTry::Refused(calls::feed_refusal(requests::refusal_of(refusal)))
            }
            // An answer that fits no feed says nothing of whether it ran.
            Ok(Heard::Answer(_)) => FeedTry::Uncertain,
        }
    }

    pub(crate) async fn cancel(&self, feeder: &str, feed: u32) -> RoutedReply {
        self.write(
            feeder,
            SentWrite::Cancel { feed },
            None,
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
                not_sent: None,
                settlement: None,
            },
            Shape::Read => self.read(feeder, request).await,
            Shape::Write { write, stream } => {
                self.write(feeder, write, stream, DirectCall::Request(request))
                    .await
            }
        }
    }

    async fn read(&self, feeder: &str, request: FeederRequest) -> RoutedReply {
        let Some((access, path)) = self.chosen(feeder) else {
            return nowhere(NotSentReason::NoPath, None);
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
                    state.reach = ServerReach::Up;
                    return answered(RoutePath::Server, reply, None);
                }
                Some(Err(error)) => {
                    let failure = calls::failure(&error);
                    state.heard_failure(failure, now_ms());
                    let elsewhere = core::server_ended(failure) == Ended::Unsent;
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
                None => return nowhere(NotSentReason::Server, Some(no_route())),
            }
        }

        let Some(link) = self.state().link(feeder) else {
            return nowhere(NotSentReason::NoPath, server_error);
        };
        let reply = direct(&link, DirectCall::Request(request)).await;
        answered(RoutePath::Direct, reply, None)
    }

    async fn write(
        &self,
        feeder: &str,
        write: SentWrite,
        stream: Option<Vec<u8>>,
        call: DirectCall,
    ) -> RoutedReply {
        let Ok(key) = new_key() else {
            return nowhere(NotSentReason::Random, None);
        };
        let now = now_ms();

        let (access, mut path) = {
            let mut state = self.state();
            let Some(access) = state.access(feeder) else {
                return nowhere(NotSentReason::NoPath, None);
            };
            let path = core::choose(access, state.reach(feeder), now);
            let pin = state.pin(key, feeder, path, write.pinned(), now);
            // Only a feed is ever held back, so this admits it; written
            // down before it goes, or it does not go.
            if state.book.admit(pin, now).is_err() || state.save().is_err() {
                state.book.settle(key);
                return nowhere(NotSentReason::Store, None);
            }
            (access, path)
        };

        let mut call = Some(call);
        let mut stream = stream;
        let mut fell_back = false;
        loop {
            let mut tried = match path {
                RoutePath::Server => {
                    self.write_on_server(feeder, &write, stream.take(), key)
                        .await
                }
                RoutePath::Direct => {
                    let link = self.state().link(feeder);
                    match (link, call.take()) {
                        (Some(link), Some(call)) => direct_write(&link, call).await,
                        _ => WriteTry::unsent(NotSentReason::NoPath, None),
                    }
                }
            };

            let mut state = self.state();
            let direct_open = state.direct_open(feeder);
            if core::fallback(path, tried.ended, access, direct_open, fell_back).is_some()
                && state.book.moved(key, None, now_ms())
            {
                if state.save().is_ok() {
                    path = RoutePath::Direct;
                    fell_back = true;
                    continue;
                }
                tried.not_sent = Some(NotSentReason::Store);
            }
            return self.wrote(&mut state, key, path, tried);
        }
    }

    /// The end of a write's last attempt, on `path`.
    fn wrote(&self, state: &mut State, key: Uuid, path: RoutePath, tried: WriteTry) -> RoutedReply {
        state.book.ended(key, tried.ended, now_ms());
        state.save_or_log();
        let early = state.early.remove(&key);

        let mut pending = None;
        let mut settlement = None;
        if tried.ended == Ended::Uncertain {
            match early {
                // The stream told its end while the call was out.
                Some(early) => {
                    state.settle_quiet(key, &early);
                    settlement = Some(early);
                }
                None => {
                    pending = state.book.get(key).map(PinRecord::view);
                    if path == RoutePath::Server {
                        self.resolve_later();
                    }
                }
            }
        }
        state.announce();
        RoutedReply {
            path: Some(path),
            reply: tried.reply,
            pending,
            server_error: tried.error,
            not_sent: tried.not_sent,
            settlement,
        }
    }

    async fn write_on_server(
        &self,
        feeder: &str,
        write: &SentWrite,
        stream: Option<Vec<u8>>,
        key: Uuid,
    ) -> WriteTry {
        let Some((client, id)) = self.state().server_for(feeder) else {
            return WriteTry::unsent(NotSentReason::NoPath, None);
        };
        let Some(sent) = calls::send(&client, id, write, stream, key).await else {
            // No server route: nothing went out.
            return WriteTry::unsent(NotSentReason::Server, Some(no_route()));
        };

        let mut state = self.state();
        match sent {
            Ok(reply) => {
                state.reach = ServerReach::Up;
                WriteTry {
                    ended: Ended::Answered,
                    reply,
                    error: None,
                    not_sent: None,
                }
            }
            Err(error) => {
                let failure = calls::failure(&error);
                state.heard_failure(failure, now_ms());
                let ended = core::server_ended(failure);
                let reply = match (ended, calls::refusal(&error)) {
                    (Ended::Uncertain, _) => FeederReply::Unanswered,
                    (Ended::Refused, Some(refusal)) => FeederReply::Refused { refusal },
                    _ => FeederReply::NotSent,
                };
                let not_sent =
                    matches!(reply, FeederReply::NotSent).then_some(NotSentReason::Server);
                WriteTry {
                    ended,
                    reply,
                    error: Some(error.into()),
                    not_sent,
                }
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
        let Some(pin) = self.state().book.get(key).cloned() else {
            return RetryOutcome::Unknown;
        };
        if pin.state == PinState::Sending {
            // Its one attempt is still out.
            return RetryOutcome::Uncertain {
                pending: pin.view(),
            };
        }

        let settlement = match core::retry_plan(&pin) {
            RetryPlan::ReadJournal => self.read_journal(&pin).await,
            RetryPlan::ReadHistory => {
                let settled = self.state().settle_direct(&pin.feeder);
                let mine = settled.into_iter().find(|(settled, _)| *settled == key);
                if let Some((_, settlement)) = mine {
                    return RetryOutcome::Settled {
                        pending: pin.view(),
                        settlement,
                    };
                }
                None
            }
        };

        let mut state = self.state();
        if let Some(settlement) = settlement
            && let Some(pending) = state.settle(key, settlement.clone())
        {
            state.announce();
            return RetryOutcome::Settled {
                pending,
                settlement,
            };
        }
        state.announce();
        match state.book.get(key) {
            Some(held) => RetryOutcome::Uncertain {
                pending: held.view(),
            },
            None => RetryOutcome::Unknown,
        }
    }

    /// What the server's journal says of `pin` now: only ever a read.
    async fn read_journal(&self, pin: &PinRecord) -> Option<Settlement> {
        let client = self.state().client()?;
        let read = client.command(pin.key).await;

        let mut state = self.state();
        match read {
            Ok(command) => {
                state.reach = ServerReach::Up;
                settled_by_command(command.state, command.outcome.as_ref())
            }
            Err(wellfed_server_client::Error::Problem { problem, .. })
                if problem.code == ErrorCode::NotFound =>
            {
                state.reach = ServerReach::Up;
                // A restored backup can keep the same token and account
                // while losing a write that already dispensed food. Even
                // an old 404 cannot prove the request was never sent.
                None
            }
            Err(error) => {
                state.heard_failure(calls::failure(&error), now_ms());
                None
            }
        }
    }

    pub(crate) async fn resolve(&self) {
        self.reread_account().await;
        let asks: Vec<PinRecord> = {
            let mut state = self.state();
            let feeders: Vec<String> = state.feeders.keys().cloned().collect();
            for feeder in feeders {
                state.settle_direct(&feeder);
            }
            state
                .book
                .pins
                .iter()
                .filter(|pin| pin.path == RoutePath::Server && pin.state == PinState::Uncertain)
                .cloned()
                .collect()
        };

        for pin in asks {
            let settlement = self.read_journal(&pin).await;
            let mut state = self.state();
            if let Some(settlement) = settlement {
                state.settle(pin.key, settlement);
            }
            state.announce();
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
                state.reach = ServerReach::Up;
                // Reads the account again too.
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
                ServerEvent::Command { command } => commanded(&mut state, command),
                ServerEvent::Resync => self.resolve_later(),
                ServerEvent::History { .. } | ServerEvent::Install { .. } => {}
            },
            ServerUpdate::Closed { error } => {
                if let Some(error) = error {
                    match error.kind {
                        // No server access for this phone, which is not a
                        // server that is down.
                        ServerErrorKind::TokenRevoked | ServerErrorKind::AccountDisabled => {
                            state.reach = ServerReach::Refused;
                            state.standing = Standing::Refused;
                        }
                        ServerErrorKind::Unreachable | ServerErrorKind::Interrupted => {
                            state.reach = ServerReach::Down { since_ms: now_ms() };
                        }
                        _ => {}
                    }
                }
            }
        }
        state.announce();
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

/// A journal entry moved on.
fn commanded(state: &mut State, command: &ServerCommand) {
    let Ok(key) = Uuid::parse_str(&command.key) else {
        return;
    };
    let settlement = view_settlement(command);

    match state.book.get(key).map(|pin| pin.state) {
        Some(PinState::Uncertain) => {
            if let Some(settlement) = settlement {
                state.settle(key, settlement);
            }
        }
        // Its call is still out and settles it; what arrives first is kept
        // for it, so a call that then ends uncertain is settled at once.
        Some(PinState::Sending) => {
            if let Some(settlement) = settlement {
                state.keep_early(key, settlement);
            }
        }
        None => {
            if let Some(settlement @ (Settlement::Fed { .. } | Settlement::Unrecorded)) = settlement
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

/// How one attempt at a feed ended.
enum FeedTry {
    /// The feeder reserved it under this number.
    Started(u32),
    Refused(FeedRefusal),
    /// The server refused it itself (a viewer may not feed, a token it no
    /// longer takes): nothing ran, and it goes nowhere else.
    Rejected(Option<ServerError>),
    Unsent(NotSentReason, Option<ServerError>),
    Uncertain,
}

impl FeedTry {
    const fn ended(&self) -> Ended {
        match self {
            Self::Started(_) => Ended::Answered,
            Self::Refused(_) | Self::Rejected(_) => Ended::Refused,
            Self::Unsent(..) => Ended::Unsent,
            Self::Uncertain => Ended::Uncertain,
        }
    }
}

/// How one attempt at a write other than a feed ended.
struct WriteTry {
    ended: Ended,
    reply: FeederReply,
    error: Option<ServerError>,
    not_sent: Option<NotSentReason>,
}

impl WriteTry {
    const fn unsent(reason: NotSentReason, error: Option<ServerError>) -> Self {
        Self {
            ended: Ended::Unsent,
            reply: FeederReply::NotSent,
            error,
            not_sent: Some(reason),
        }
    }
}

const fn not_sent(reason: NotSentReason, error: Option<ServerError>) -> RoutedFeed {
    RoutedFeed::NotSent { reason, error }
}

/// Run a write on the phone's own session behind `link`.
async fn direct_write(link: &DirectLink, call: DirectCall) -> WriteTry {
    let reply = direct(link, call).await;
    let (ended, not_sent) = match reply {
        FeederReply::Unanswered | FeederReply::Unreadable => (Ended::Uncertain, None),
        FeederReply::NotSent
        | FeederReply::NotAsked {
            reason: NotAsked::Held,
        } => (Ended::Unsent, Some(NotSentReason::Session)),
        _ => (Ended::Answered, None),
    };
    WriteTry {
        ended,
        reply,
        error: None,
        not_sent,
    }
}

/// Run `call` on the phone's own session behind `link`.
async fn direct(link: &DirectLink, call: DirectCall) -> FeederReply {
    match call {
        DirectCall::Request(request) => requests::run(&link.commands, link.identity, request).await,
        DirectCall::Op(op) => match requests::ask(&link.commands, op).await {
            Ok(answer) => requests::reply_of(answer),
            Err(lost) => lost.into(),
        },
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
        not_sent: None,
        settlement: None,
    }
}

const fn nowhere(reason: NotSentReason, server_error: Option<ServerError>) -> RoutedReply {
    RoutedReply {
        path: None,
        reply: FeederReply::NotSent,
        pending: None,
        server_error,
        not_sent: Some(reason),
        settlement: None,
    }
}

fn no_route() -> ServerError {
    ServerError::new(
        ServerErrorKind::Refused,
        "the server has no route for this request, and this phone holds no key of its own",
    )
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
        | ServerCommandOutcome::Calibration
        | ServerCommandOutcome::Call
        | ServerCommandOutcome::Invited { .. } => Settlement::Done,
        ServerCommandOutcome::Superseded => Settlement::Superseded,
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

/// What a feed call's answer settles of its pin, for a screen that holds
/// only the pin; `None` while it may still have run. A refusal the device
/// API names no more closely than "other" is left to the bowl.
fn feed_settlement(routed: &RoutedFeed) -> Option<Settlement> {
    let refusal = |refusal: FeedRefusal| match refusal {
        FeedRefusal::Forbidden => Some(ServerRefusal::Forbidden),
        FeedRefusal::Busy => Some(ServerRefusal::Busy),
        FeedRefusal::Fault => Some(ServerRefusal::Fault),
        FeedRefusal::Portions => Some(ServerRefusal::Portions),
        FeedRefusal::NotFound => Some(ServerRefusal::NotFound),
        FeedRefusal::Unavailable => Some(ServerRefusal::Unavailable),
        FeedRefusal::Other => None,
    };
    match routed {
        RoutedFeed::Started { feed, .. } => Some(Settlement::Started { feed: *feed }),
        RoutedFeed::Refused { refusal: why, .. } => {
            refusal(*why).map(|refusal| Settlement::Refused { refusal })
        }
        RoutedFeed::NotSent { .. } => Some(Settlement::NotSent),
        RoutedFeed::Settled { settlement, .. } => Some(settlement.clone()),
        RoutedFeed::Uncertain { .. } | RoutedFeed::Held { .. } => None,
    }
}

#[cfg(test)]
#[path = "io_tests.rs"]
mod tests;
