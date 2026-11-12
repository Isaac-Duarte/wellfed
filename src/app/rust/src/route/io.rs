//! The router's IO: the server's calls and the direct sessions' commands
//! driven around the rules in [`super::core`], with the pins kept on disk
//! by [`super::pins`]. One lock holds what the router knows; no call holds
//! it across an await, and a write's admission and its pin are written
//! under one hold before it goes, so two feeds asked at once cannot both
//! pass and a write that never got written down never goes.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use jiff::Timestamp;
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;
use wellfed_api::{ErrorCode, FeederId, User};
use wellfed_model::{Answer, Op};

use crate::api::error::{BridgeError, ErrorKind};
use crate::api::feeder::{FeederReply, FeederRequest};
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
    Waited, settled_by_command, settled_by_history, settled_unfound,
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
    /// How long after its attempt a server write its journal lacks is
    /// taken as never sent.
    grace_ms: i64,
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
    /// When this run first knew of each pin's last attempt, and of each
    /// lost adoption, by the monotonic clock: a grace from either passes
    /// only when this clock agrees with the wall clock's record.
    tried: Steady<Uuid>,
    adoptions_lost: Steady<String>,
    /// Which server session the state is for, raised whenever the server is
    /// configured, connected, refused or removed: what a read of the server
    /// begun under another session says is never applied to this one.
    session: u64,
    /// A server the app keeps, before its session is handed over: the keys
    /// its kept account may use, so no feeder goes on the phone's own key
    /// that the account would not allow while the app is still reading its
    /// token and asking the server.
    cold: Option<Keys>,
    /// The feeders this phone's own key was claimed on this session, or is
    /// being claimed on: one a claim could not settle leaves it.
    claimed: HashSet<FeederId>,
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

/// A round of claims of the phone's own key, read under one lock.
pub(crate) struct ClaimsDue {
    pub(crate) client: wellfed_server_client::Client,
    /// The phone token's id, which a proof is bound to.
    pub(crate) token: Option<Uuid>,
    pub(crate) session: u64,
    /// The feeders to claim on.
    pub(crate) due: Vec<FeederId>,
}

pub(crate) fn now_ms() -> i64 {
    Timestamp::now().as_millisecond()
}

/// When this run first knew of each key's attempt, by its wall-clock time,
/// on the monotonic clock. Noting an attempt late only makes its wait read
/// shorter, so one this run did not see (from before a restart) counts from
/// when the run first looked at it.
struct Steady<K> {
    seen: HashMap<K, (i64, Instant)>,
}

impl<K: Eq + Hash> Steady<K> {
    fn new() -> Self {
        Self {
            seen: HashMap::new(),
        }
    }

    /// Hold just `attempts` (each key's attempt by the wall clock), noting
    /// from now those this run did not know yet, or knew at another time.
    fn sync(&mut self, attempts: impl IntoIterator<Item = (K, i64)>) {
        let now = Instant::now();
        let mut kept = HashMap::new();
        for (key, at_ms) in attempts {
            let known = self.seen.remove(&key).filter(|(at, _)| *at == at_ms);
            kept.insert(key, known.unwrap_or((at_ms, now)));
        }
        self.seen = kept;
    }

    /// How long since `key`'s attempt at `at_ms`, with the wall clock at
    /// `now_ms`: by the monotonic clock from when this run first knew of it,
    /// or nothing if it does not know of it at that time.
    fn waited(&self, key: &K, at_ms: i64, now_ms: i64) -> Waited {
        let steady_ms = match self.seen.get(key) {
            Some((at, since)) if *at == at_ms => {
                i64::try_from(since.elapsed().as_millis()).unwrap_or(i64::MAX)
            }
            _ => 0,
        };
        Waited {
            wall_ms: now_ms.saturating_sub(at_ms),
            steady_ms,
        }
    }
}

/// A read of a server session the router has since let go.
fn replaced() -> ServerError {
    ServerError::new(
        ServerErrorKind::Closed,
        "the server changed while it was read",
    )
}

fn new_key() -> Result<Uuid, BridgeError> {
    Ok(uuid::Builder::from_random_bytes(random()?).into_uuid())
}

/// Viewers, accounts with an expiry and roles a newer server added (which
/// grant nothing here) hold no key of their own.
fn keys_of(account: &ServerAccount) -> Keys {
    let least = matches!(account.role, ServerRole::Viewer | ServerRole::Unknown);
    if least || account.expires_ms.is_some() {
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

    /// Whether a write whose attempt on `path` ended `ended` goes once more
    /// on the phone's own session, by what the phone may use now: while the
    /// server call was out, the server may have refused the token or
    /// narrowed the account, or the owner removed it, and an access read
    /// before the call would still allow the phone's own key.
    fn falls_back(&self, feeder: &str, path: RoutePath, ended: Ended, fell_back: bool) -> bool {
        self.access(feeder).is_some_and(|access| {
            core::fallback(path, ended, access, self.direct_open(feeder), fell_back).is_some()
        })
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

    /// The server's client, and the session it belongs to.
    fn session_client(&self) -> Option<(wellfed_server_client::Client, u64)> {
        Some((self.client()?, self.session))
    }

    /// Another server session from here on: its token claims the phone's
    /// own key anew.
    fn new_session(&mut self) {
        self.session = self.session.wrapping_add(1);
        self.claimed.clear();
    }

    /// Route through `handle`, under `account`.
    fn connect(&mut self, handle: ServerHandle, account: &ServerAccount) {
        self.new_session();
        self.server = Some(Server {
            handle,
            keys: keys_of(account),
            user: Uuid::parse_str(&account.id).ok(),
            account: Some(account.clone()),
        });
        self.reach = ServerReach::Up;
        self.standing = Standing::Taken;
        self.announce();
    }

    /// The server's client and its id for `feeder`, and the session the
    /// call goes under.
    fn server_for(&self, feeder: &str) -> Option<(wellfed_server_client::Client, FeederId, u64)> {
        let client = self.client()?;
        let id = self.feeders.get(feeder)?.server?;
        Some((client, id, self.session))
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

    /// Note every pin's last attempt and every lost adoption this run did
    /// not know yet, from now, and forget those settled.
    fn note_attempts(&mut self) {
        self.tried
            .sync(self.book.pins.iter().map(|pin| (pin.key, pin.tried_ms)));
        self.adoptions_lost.sync(
            self.book
                .adoptions
                .iter()
                .map(|lost| (lost.feeder.clone(), lost.at_ms)),
        );
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

    /// What a failed call made under `session` says of the server: nothing
    /// once that session was replaced, so a late refusal of an old token
    /// never closes the phone's own keys under a fresh login.
    fn heard_failure(&mut self, session: u64, failure: ServerFailure, now: i64) {
        if session != self.session {
            return;
        }
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

        let settled = settled_by_history(&self.book.pins, feeder, &seen, link.phone());
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
    pub(crate) fn open(pins_path: &Path, grace_ms: i64) -> Result<Self, BridgeError> {
        let (store, mut book) = PinStore::open(pins_path)?;
        if book.expire(now_ms()) {
            store.save(&book)?;
        }
        let mut state = State {
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
            tried: Steady::new(),
            adoptions_lost: Steady::new(),
            session: 0,
            claimed: HashSet::new(),
        };
        // What the last run left counts from this run's start, never from
        // the wall clock's record alone.
        state.note_attempts();
        Ok(Self {
            inner: Arc::new(Inner {
                state: Mutex::new(state),
                closed: watch::Sender::new(false),
                grace_ms,
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
        self.state().connect(handle, account);
    }

    /// The server the app keeps, before anything else asks the router: its
    /// feeders' ids, and the keys its kept account may use (none of its own
    /// with no account kept) until the session is handed over. It sends
    /// nothing.
    pub(crate) fn configure(&self, kept: &KnownServer) {
        let mut state = self.state();
        state.new_session();
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
        state.new_session();
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
        let session = self.state().session;
        let read = handle.client.session().await;

        let mut state = self.state();
        // Another server, or none, since the read began: this one was let
        // go, and what it says is not applied.
        if state.session != session {
            return Err(replaced());
        }
        let error = match read {
            Ok(user) => {
                let account = ServerAccount::from(user);
                state.connect(handle, &account);
                return Ok(account);
            }
            Err(error) => error,
        };
        let failure = error.failure();
        state.heard_failure(session, failure, now_ms());
        state.new_session();
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
        state.announce();
        Err(error.into())
    }

    pub(crate) fn disconnect_server(&self) {
        let mut state = self.state();
        state.new_session();
        state.server = None;
        state.cold = None;
        state.reach = ServerReach::Up;
        state.standing = Standing::Taken;
        state.following.clear();
        state.announce();
    }

    pub(crate) async fn refresh(&self) -> Result<Vec<ServerFeeder>, ServerError> {
        let Some((client, session)) = self.state().session_client() else {
            return Err(ServerError::new(
                ServerErrorKind::Closed,
                "no server is configured",
            ));
        };

        let (account, listed) = tokio::join!(client.session(), client.feeders());
        let mut state = self.state();
        if state.session != session {
            return Err(replaced());
        }
        let now = now_ms();
        let read = account.and_then(|user| listed.map(|feeders| (user, feeders)));
        let (user, feeders) = match read {
            Ok(read) => read,
            Err(error) => {
                state.heard_failure(session, error.failure(), now);
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

    /// How long since `feeder`'s adoption lost its answer at `at_ms`, by
    /// both clocks.
    pub(crate) fn adoption_waited(&self, feeder: &str, at_ms: i64) -> Waited {
        let mut state = self.state();
        state.note_attempts();
        state
            .adoptions_lost
            .waited(&feeder.to_owned(), at_ms, now_ms())
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
        state.note_attempts();
        if state.book.adoptions != before {
            // Kept for a restart; one that fails to save still holds this run.
            state.save_or_log();
        }
    }

    /// `feeder`'s adoption is about to go: kept on disk as one whose answer
    /// was lost until it settles, so an app that stops while it is out
    /// never makes a second owner invite at its next start. Refused, with
    /// nothing kept, if it cannot be written down.
    pub(crate) fn guard_adoption(&self, feeder: &str) -> Result<(), BridgeError> {
        let mut state = self.state();
        let before = state.book.adoptions.clone();
        state.book.adoptions.retain(|lost| lost.feeder != feeder);
        state.book.adoptions.push(LostAdoption {
            feeder: feeder.to_owned(),
            at_ms: now_ms(),
        });
        let saved = state.save();
        if saved.is_err() {
            state.book.adoptions = before;
        }
        state.note_attempts();
        saved
    }

    /// How long after a lost attempt the server's silence counts as never
    /// sent.
    pub(crate) fn grace_ms(&self) -> i64 {
        self.inner.grace_ms
    }

    /// The server's client, when one is configured.
    pub(crate) fn server_client(&self) -> Option<wellfed_server_client::Client> {
        self.state().client()
    }

    /// The server's client and the phone token's id, when one is
    /// configured.
    pub(crate) fn server_login(&self) -> Option<(wellfed_server_client::Client, Option<Uuid>)> {
        let state = self.state();
        let server = state.server.as_ref()?;
        Some((server.handle.client.clone(), server.handle.token))
    }

    /// The server's id for `feeder`, while a server is configured and its
    /// last list named it.
    pub(crate) fn listed(&self, feeder: &str) -> Option<FeederId> {
        let state = self.state();
        state.server.as_ref()?;
        state.feeders.get(feeder)?.server
    }

    /// The server's client and session, and the feeders its last list
    /// named that this phone's own key is not claimed on yet this session,
    /// marked as claimed from now so a second round sends none of them.
    /// `None` with no server, or with an account that holds no key of its
    /// own.
    pub(crate) fn claims_due(&self) -> Option<ClaimsDue> {
        let mut guard = self.state();
        let state = &mut *guard;
        let server = state.server.as_ref()?;
        if server.keys.under(state.standing) != Keys::Own {
            return None;
        }

        let client = server.handle.client.clone();
        let token = server.handle.token;
        let claimed = &mut state.claimed;
        let due = state
            .feeders
            .values()
            .filter_map(|entry| entry.server)
            .filter(|id| claimed.insert(*id))
            .collect();
        Some(ClaimsDue {
            client,
            token,
            session: state.session,
            due,
        })
    }

    /// Claim the phone's own key on `id` again in the next round, while
    /// `session` lasts.
    pub(crate) fn claim_again(&self, session: u64, id: FeederId) {
        let mut state = self.state();
        if state.session == session {
            state.claimed.remove(&id);
        }
    }

    /// Read the account again, so a demotion or a new expiry routes from
    /// now on.
    async fn reread_account(&self) {
        let Some((client, session)) = self.state().session_client() else {
            return;
        };
        let read = client.session().await;
        let mut state = self.state();
        if state.session != session {
            return;
        }
        match read {
            Ok(user) => {
                state.reach = ServerReach::Up;
                state.account_is(user);
            }
            Err(error) => state.heard_failure(session, error.failure(), now_ms()),
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

        let mut path = {
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
            path
        };

        let mut fell_back = false;
        loop {
            let mut tried = match path {
                RoutePath::Server => self.feed_on_server(feeder, portions, key).await,
                RoutePath::Direct => self.feed_direct(feeder, portions).await,
            };

            let mut state = self.state();
            if state.falls_back(feeder, path, tried.ended(), fell_back) {
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
        state.note_attempts();
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
        let Some((client, id, session)) = self.state().server_for(feeder) else {
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
                let failure = error.failure();
                state.heard_failure(session, failure, now_ms());
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
                Some((client, id, _)) => calls::read(client, *id, &request).await,
                None => None,
            };
            let session = server.map(|(_, _, session)| session);
            let mut state = self.state();
            match read {
                Some(Ok(reply)) => {
                    state.reach = ServerReach::Up;
                    return answered(RoutePath::Server, reply);
                }
                Some(Err(error)) => {
                    let failure = error.failure();
                    if let Some(session) = session {
                        state.heard_failure(session, failure, now_ms());
                    }
                    let elsewhere = core::server_ended(failure) == Ended::Unsent;
                    // What the phone may use now, not before the call: the
                    // server may have narrowed the account meanwhile.
                    let own = state.access(feeder) == Some(Access::Both);
                    if !(elsewhere && own && state.direct_open(feeder)) {
                        let reply = match calls::refusal(&error) {
                            Some(refusal) => FeederReply::Refused { refusal },
                            None => FeederReply::NotSent {
                                reason: NotSentReason::Server,
                                server: Some(error.into()),
                            },
                        };
                        return answered(RoutePath::Server, reply);
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
        answered(RoutePath::Direct, reply)
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

        let mut path = {
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
            path
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
            if state.falls_back(feeder, path, tried.ended, fell_back)
                && state.book.moved(key, None, now_ms())
            {
                if state.save().is_ok() {
                    path = RoutePath::Direct;
                    fell_back = true;
                    continue;
                }
                let server = match tried.reply {
                    FeederReply::NotSent { server, .. } => server,
                    _ => None,
                };
                tried.reply = FeederReply::NotSent {
                    reason: NotSentReason::Store,
                    server,
                };
            }
            return self.wrote(&mut state, key, path, tried);
        }
    }

    /// The end of a write's last attempt, on `path`.
    fn wrote(&self, state: &mut State, key: Uuid, path: RoutePath, tried: WriteTry) -> RoutedReply {
        state.book.ended(key, tried.ended, now_ms());
        state.note_attempts();
        state.save_or_log();
        let early = state.early.remove(&key);

        let mut pending = None;
        let mut reply = tried.reply;
        if tried.ended == Ended::Uncertain {
            match early {
                // The stream told its end while the call was out.
                Some(early) => {
                    state.settle_quiet(key, &early);
                    reply = core::settled_reply(&early);
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
            reply,
            pending,
        }
    }

    async fn write_on_server(
        &self,
        feeder: &str,
        write: &SentWrite,
        stream: Option<Vec<u8>>,
        key: Uuid,
    ) -> WriteTry {
        let Some((client, id, session)) = self.state().server_for(feeder) else {
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
                }
            }
            Err(error) => {
                let failure = error.failure();
                state.heard_failure(session, failure, now_ms());
                let ended = core::server_ended(failure);
                let reply = match (ended, calls::refusal(&error)) {
                    (Ended::Uncertain, _) => FeederReply::Unanswered,
                    (Ended::Refused, Some(refusal)) => FeederReply::Refused { refusal },
                    _ => FeederReply::NotSent {
                        reason: NotSentReason::Server,
                        server: Some(error.into()),
                    },
                };
                WriteTry { ended, reply }
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
        let (client, mark, session) = {
            let state = self.state();
            let server = state.server.as_ref()?;
            (server.handle.client.clone(), server.mark(), state.session)
        };
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
                state.note_attempts();
                let since = state.tried.waited(&pin.key, pin.tried_ms, now_ms());
                settled_unfound(pin, mark, since, self.inner.grace_ms)
            }
            Err(error) => {
                state.heard_failure(session, error.failure(), now_ms());
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
                        ServerErrorKind::Unreachable
                        | ServerErrorKind::Interrupted
                        | ServerErrorKind::ServerFailed => {
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
}

impl WriteTry {
    const fn unsent(reason: NotSentReason, server: Option<ServerError>) -> Self {
        Self {
            ended: Ended::Unsent,
            reply: FeederReply::NotSent { reason, server },
        }
    }
}

const fn not_sent(reason: NotSentReason, error: Option<ServerError>) -> RoutedFeed {
    RoutedFeed::NotSent { reason, error }
}

/// Run a write on the phone's own session behind `link`.
async fn direct_write(link: &DirectLink, call: DirectCall) -> WriteTry {
    let reply = direct(link, call).await;
    WriteTry {
        ended: core::direct_ended(&reply),
        reply,
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

const fn answered(path: RoutePath, reply: FeederReply) -> RoutedReply {
    RoutedReply {
        path: Some(path),
        reply,
        pending: None,
    }
}

pub(crate) const fn nowhere(reason: NotSentReason, server: Option<ServerError>) -> RoutedReply {
    RoutedReply {
        path: None,
        reply: FeederReply::NotSent { reason, server },
        pending: None,
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
mod tests {
    use super::*;

    /// A step of the wall clock inside a run moves only the wall clock's
    /// side of the wait: the monotonic side counts from when the run noted
    /// the attempt, and a new attempt (another wall-clock time) or one the
    /// run never noted counts from nothing.
    #[test]
    fn the_monotonic_wait_counts_only_from_what_this_run_noted() {
        let key = Uuid::from_u128(1);
        let mut steady = Steady::new();
        steady.sync([(key, 1_000)]);
        std::thread::sleep(std::time::Duration::from_millis(20));

        // The wall clock stepped ten minutes on.
        let waited = steady.waited(&key, 1_000, 1_000 + 600_000);
        assert_eq!(waited.wall_ms, 600_000);
        assert!((20..600_000).contains(&waited.steady_ms), "{waited:?}");
        assert!(!waited.passed(600_000));

        assert_eq!(steady.waited(&key, 2_000, 602_000).steady_ms, 0);
        assert_eq!(
            steady.waited(&Uuid::from_u128(2), 1_000, 601_000).steady_ms,
            0
        );

        // Noted again at the same time, it keeps its start; at a new one,
        // or gone from the book, it starts over or is forgotten.
        steady.sync([(key, 1_000)]);
        assert!(steady.waited(&key, 1_000, 1_000).steady_ms >= 20);
        steady.sync([(key, 5_000)]);
        assert!(steady.waited(&key, 5_000, 5_000).steady_ms < 20);
        steady.sync([]);
        assert!(steady.seen.is_empty());
    }
}
