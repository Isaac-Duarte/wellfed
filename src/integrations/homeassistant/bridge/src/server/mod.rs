//! One feeder's session through the owner's server, for a Home Assistant
//! set up on the server route: the server's `/api/v1` under a token of the
//! owner's account, through `wellfed-server-client`, in place of a device
//! API session of its own. It hands out the same [`Update`]s as the direct
//! session ([`crate::feeder`]), so the integration's entities do not know
//! which route they are on.
//!
//! - **On connect** it reads the server's API revision, the token's account
//!   and the feeder, and opens the event stream before reading the facts
//!   and the history, so nothing that happens meanwhile is missed; what the
//!   stream repeats of the history is dropped by the event cursor. A first
//!   connect replays the newest 2,000 events only; a later one replays from
//!   its cursor, and a history that started over is a gap.
//! - **The feeder's connection** is the server's: while the server cannot
//!   reach it the session reads as disconnected and every write is refused
//!   unsent, as the direct session's are between sessions.
//! - **Writes** go out once, under a fresh idempotency key, and are never
//!   sent again here. A feed whose answer never came is tracked by its key
//!   and settled from the server's journal: by its `Fed` event once the
//!   journal names its number, as refused, or as unknown when the journal
//!   says the feeder restarted with no record of it. A journal that holds
//!   no entry under its key, asked under the token it went under and
//!   10 minutes after it went by both clocks, settles it as never
//!   sent: the server records a write before it sends it, and retires a key
//!   a read found no write under, so it can never run after. One unsettled
//!   after 15 minutes is reported still unknown and never resent. While one
//!   is unsettled and not released by the owner, no feed goes out
//!   ([`NotSent::Held`]), as the app holds on every path: the server's
//!   journal dedupes a retry under one key, not a new feed under a new
//!   one. Lost feeds the direct route left hold here too, unsettled, until
//!   the owner releases them; a released feed stays open
//!   to the journal for a week from its release. A write that waited
//!   longer than 5 seconds for the session, or arrived while it connects,
//!   is refused unsent.
//! - **The event stream** is read by a task of its own, so a slow write or
//!   a long connect never leaves it unread past its read timeout. An event
//!   that changes the settings or the plans has them read again at once.
//!   When the stream falls behind and says to resync, the session reads
//!   the feeder anew, replays the history it missed and asks the journal
//!   about each unsettled feed, as a new session would.
//! - **Ends.** A token the server no longer takes stops the session until a
//!   new login. A feeder the server no longer holds (removed, another
//!   feeder under its id, or its key dropped by the feeder) is reported
//!   once, and the session keeps checking every [`Clocks::recheck`], so it
//!   comes back by itself once the owner adopts the feeder again.

mod account;

use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use jiff::tz::TimeZone;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;
use wellfed_client::history::Feed;
use wellfed_client::today::Facts;
use wellfed_model::{self as model, CalibrationView, ClientRole, DeviceKind, Op, Settings};
use wellfed_server_client::api::{
    self as server, CommandOutcome, Connection, ErrorCode, EventQuery, FeederDetail, FeederId,
    HistoryEvent, PlansChange, ServerSkew, StreamEvent, StreamQuery,
};
use wellfed_server_client::{
    Auth, Client, Failure, Lock, PlainHttp, Settled, Timeouts, Updates, Url,
};

pub use account::{
    Login, Refused, ServerFeeder, address, client_listed, feeder, feeders, login, revoke_client,
    revoke_token,
};

use crate::Error;
use crate::alerts::Tracker;
use crate::api::{
    self, Capabilities, NotSent, Pending, RefusalKind, Role, Skew, State, Update, Written,
};
use crate::feeder::{
    self as direct, Carried, Command, SETTLE, TURN, now_ms, refusing, sent_at, stale,
};
use crate::hold::{Holds, Releases};
use crate::writes::{self, Kind, Write};

/// The newest events a first connect replays, of a history the server may
/// have kept for years: far more than a day holds.
const BACKLOG: u64 = 2_000;
/// Events read per page of the history.
const PAGE: u32 = 500;

/// Where the session finds its feeder: the owner's server, the token it
/// signs in with, and the feeder by its id there and its identity key.
#[derive(Clone)]
pub struct Target {
    pub url: Url,
    pub token: String,
    /// The token's id: a feed sent under it records it, so only this
    /// token's journal is trusted to lack it.
    pub token_id: String,
    pub feeder: FeederId,
    /// The feeder's identity: a feeder under the same id with another one
    /// is another feeder, and is never served as this one.
    pub identity: [u8; 32],
}

impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token is a secret.
        f.debug_struct("Target")
            .field("url", &self.url.as_str())
            .field("feeder", &self.feeder)
            .finish_non_exhaustive()
    }
}

/// How long the session waits on the server, and between sessions. Home
/// Assistant runs on [`Clocks::DEFAULT`]; a test shortens them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clocks {
    pub timeouts: Timeouts,
    /// How long a lost session waits before opening another.
    pub retry: Duration,
    /// How long a session that found its feeder gone waits before looking
    /// again.
    pub recheck: Duration,
}

impl Clocks {
    pub const DEFAULT: Self = Self {
        timeouts: Timeouts {
            // Three of the server's stream keep-alives.
            read: Some(Duration::from_secs(45)),
            // Longer than the server waits on its feeder for an answer.
            request: Some(Duration::from_secs(30)),
        },
        retry: Duration::from_secs(10),
        recheck: Duration::from_secs(5 * 60),
    };
}

/// The client `target` is reached through, on `clocks`.
pub(crate) fn client(target: &Target, clocks: Clocks) -> Result<Client, Error> {
    // An entry's address went through the config flow, which asks the
    // owner before any plain `http://` beyond this host.
    let client = Client::with_timeouts(&target.url, clocks.timeouts, PlainHttp::Allowed)?;
    Ok(client.with_auth(Some(Auth::Token(target.token.clone()))))
}

/// How long after it went out a feed the journal does not hold is taken as
/// never sent, by both the wall clock and the monotonic one: the server
/// records a write before it sends it, and the wait covers a request still
/// on its way to a server slow to take it, well past the 30 s this client
/// waits on an answer. The app waits as long.
pub(crate) const UNFOUND_GRACE_MS: i64 = 10 * 60_000;

/// A feed that went out and was never answered, by the key it went under.
#[derive(Clone, Debug)]
struct Uncertain {
    key: Uuid,
    pending: Pending,
    /// Its number, once the server's journal names it.
    feed: Option<u32>,
    /// When it went out, for the report: reported still unknown
    /// [`SETTLE`] after. It holds feeds back until settled or released.
    sent: Instant,
    /// When this run first knew of it: the monotonic half of the grace a
    /// journal's lack of it waits out, which never counts from before.
    known: Instant,
    /// The journal held no entry under its key before the grace passed:
    /// asked again at each turn until the grace has.
    unfound: bool,
}

impl Uncertain {
    fn new(key: Uuid, pending: Pending, feed: Option<u32>) -> Self {
        Self {
            key,
            sent: sent_at(pending.sent_ms),
            known: Instant::now(),
            unfound: false,
            pending,
            feed,
        }
    }

    /// Whether the journal's lack of it, asked now under `token_id`, says
    /// it was never sent: the same token it went under, and the grace past
    /// by both clocks.
    fn never_sent(&self, token_id: &str) -> bool {
        let grace = Duration::from_millis(UNFOUND_GRACE_MS.unsigned_abs());
        self.pending.token_id.as_deref() == Some(token_id)
            && now_ms().saturating_sub(self.pending.sent_ms) >= UNFOUND_GRACE_MS
            && self.known.elapsed() >= grace
    }
}

/// Where a history was read to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cursor {
    epoch: u32,
    seq: u64,
}

/// What outlives one session: where the history was read to, what it
/// said, and the feeds still to settle.
#[derive(Debug, Default)]
struct Memory {
    cursor: Option<Cursor>,
    facts: Facts,
    calibration: Option<CalibrationView>,
    uncertain: Vec<Uncertain>,
    /// The alerts the server served so far.
    alerts: Tracker,
    /// What holds feeds back besides this route's own lost feeds.
    holds: Holds,
}

impl Memory {
    /// Whether a new feed waits: on a lost feed of this route's that is
    /// neither settled nor released, or on what the holds keep.
    fn holds_feeds(&self) -> bool {
        self.holds.holding()
            || self
                .uncertain
                .iter()
                .any(|uncertain| uncertain.pending.released_ms.is_none())
    }

    /// Apply what the owner released since the last call: the updates that
    /// confirm it.
    fn take_releases(&mut self) -> Vec<Update> {
        let (ids, mut updates) = self.holds.take();
        if ids.is_empty() {
            return updates;
        }

        let at = now_ms();
        for uncertain in &mut self.uncertain {
            let pending = &mut uncertain.pending;
            if pending.released_ms.is_none() && ids.contains(&pending.id) {
                pending.released_ms = Some(at);
                updates.push(Update::released(pending.clone()));
            }
        }
        updates
    }
}

enum Ended {
    /// The handle is gone.
    Dropped,
    /// The server no longer takes the token.
    TokenRefused,
    /// The server no longer holds the feeder, for the reason given.
    LostFeeder(String),
    Lost(String),
}

/// The session's own failure, or the handle gone.
type Step = Result<(), Ended>;

/// One update off the event stream, as its reader hands it on.
type Streamed = Result<wellfed_server_client::Update, wellfed_server_client::Error>;

/// The event stream, read by a task of its own: the session may spend
/// longer than the stream's read timeout on a write or a connect, and a
/// stream left unpolled that long would fail even with its data waiting.
/// The task ends with the stream or when this is dropped.
struct Stream {
    updates: mpsc::UnboundedReceiver<Streamed>,
    reader: tokio::task::JoinHandle<()>,
}

impl Stream {
    fn read(mut stream: Updates) -> Self {
        let (sender, updates) = mpsc::unbounded_channel();
        let reader = tokio::spawn(async move {
            while let Some(update) = stream.next().await {
                if sender.send(update).is_err() {
                    return;
                }
            }
        });
        Self { updates, reader }
    }

    /// The next update, or `None` once the stream has ended.
    async fn next(&mut self) -> Option<Streamed> {
        self.updates.recv().await
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

/// Connect, serve until the session ends, and go again, until the handle is
/// dropped or the server refuses the token.
pub(crate) async fn run(
    client: Client,
    target: Target,
    clocks: Clocks,
    carried: Carried,
    releases: Releases,
    mut commands: mpsc::UnboundedReceiver<Command>,
    updates: std_mpsc::Sender<Update>,
) {
    let mut memory = Memory::default();
    let mut elsewhere = Vec::new();
    for pending in carried.pending {
        // A feed sent through a server names its command by its key; one
        // the direct route sent names nothing here, and only holds.
        let key = Uuid::parse_str(&pending.id)
            .ok()
            .filter(|_| pending.token_id.is_some());
        match key {
            Some(key) => memory.uncertain.push(Uncertain::new(key, pending, None)),
            None => elsewhere.push(pending),
        }
    }
    memory.holds = Holds::new(releases, elsewhere, carried.unread);

    // Each lost feed that still holds feeds back is announced again, so the
    // integration's repairs say so.
    let mut again: Vec<Update> = memory
        .uncertain
        .iter()
        .filter_map(|uncertain| direct::announced(&uncertain.pending))
        .collect();
    again.extend(memory.holds.announce());
    for update in again {
        if updates.send(update).is_err() {
            return;
        }
    }
    let mut lost_feeder = false;

    loop {
        // Releases made while no session was open.
        for update in memory.take_releases() {
            if updates.send(update).is_err() {
                return;
            }
        }

        let session = Session::new(&client, &target, &mut memory, &updates);
        let (update, wait) = match session.serve(&mut commands).await {
            Ended::Dropped => return,
            Ended::TokenRefused => {
                direct::stopped(Update::token_refused(), &mut commands, &updates).await;
                return;
            }
            // Said once, however long the feeder stays gone.
            Ended::LostFeeder(reason) => {
                let update = (!lost_feeder).then(|| Update::server_lost_feeder(reason));
                lost_feeder = true;
                (update, clocks.recheck)
            }
            Ended::Lost(reason) => (Some(Update::disconnected(reason)), clocks.retry),
        };
        if let Some(update) = update
            && updates.send(update).is_err()
        {
            return;
        }
        if !direct::pause(wait, &mut commands).await {
            return;
        }
    }
}

struct Session<'a> {
    client: &'a Client,
    target: &'a Target,
    memory: &'a mut Memory,
    updates: &'a std_mpsc::Sender<Update>,
    /// What the token's account may do, as the writes' gate reads it.
    role: ClientRole,
    /// The device's kind; `None` for one a newer server added, which this
    /// integration serves nothing of and names as needing an update.
    kind: Option<DeviceKind>,
    /// The home's zone: the feeder's, which the server keeps on it.
    zone: TimeZone,
    /// Whether the server reaches the feeder now.
    online: bool,
    /// Whether the feeder runs a build from before API revisions, which the
    /// server sends nothing but an update.
    legacy: bool,
    /// How the server's API revision stands against this integration's.
    server: ServerSkew,
    /// The token's account's role.
    account: server::Role,
}

impl<'a> Session<'a> {
    fn new(
        client: &'a Client,
        target: &'a Target,
        memory: &'a mut Memory,
        updates: &'a std_mpsc::Sender<Update>,
    ) -> Self {
        Self {
            client,
            target,
            memory,
            updates,
            role: ClientRole::Unknown,
            kind: None,
            zone: TimeZone::system(),
            online: false,
            legacy: false,
            server: ServerSkew::Same,
            account: server::Role::Unknown,
        }
    }

    async fn serve(mut self, commands: &mut mpsc::UnboundedReceiver<Command>) -> Ended {
        let stream = match refusing(self.open(), commands).await {
            None => return Ended::Dropped,
            Some(Ok(stream)) => stream,
            Some(Err(ended)) => return ended,
        };
        self.follow(stream, commands).await
    }

    /// Check the server, the token and the feeder, open the stream, then
    /// read the facts and the history.
    async fn open(&mut self) -> Result<Stream, Ended> {
        self.server = self.client.skew().await.map_err(|error| ended(&error))?;
        self.account = self
            .client
            .session()
            .await
            .map_err(|error| ended(&error))?
            .role;
        let stream = self
            .client
            .stream(&StreamQuery::default(), None)
            .await
            .map_err(|error| ended(&error))?;
        let stream = Stream::read(stream);
        let detail = self.detail().await?;

        // The account's role as the feeder's, for the write gate: what the
        // person may do, not the server's key it goes out under
        // (`writes::gate`).
        self.role = match self.account {
            server::Role::Owner => ClientRole::Owner,
            server::Role::Member => ClientRole::Member,
            // A viewer changes nothing, and a role a newer server added
            // grants nothing here.
            server::Role::Viewer | server::Role::Unknown => ClientRole::Unknown,
        };
        if let Ok(home) = self.client.home().await {
            self.zone = TimeZone::get(&home.zone).unwrap_or_else(|_| TimeZone::system());
        }

        self.took(&detail);
        if detail.feeder.connection == Connection::Online {
            self.reached(&detail).await?;
        } else {
            self.send(Update::disconnected(OFFLINE))?;
        }
        self.backlog(&detail).await?;
        self.recheck().await?;
        // Before the state: once it is out, the session takes writes, and
        // one sent while this read waited would be refused unsent.
        self.read_alerts().await?;
        self.state()?;
        Ok(stream)
    }

    async fn follow(
        &mut self,
        mut stream: Stream,
        commands: &mut mpsc::UnboundedReceiver<Command>,
    ) -> Ended {
        let mut turn = tokio::time::interval_at(tokio::time::Instant::now() + TURN, TURN);
        // Releases made while this session connected.
        if let Err(ended) = self.release() {
            return ended;
        }
        loop {
            let step = tokio::select! {
                command = commands.recv() => match command {
                    None => return Ended::Dropped,
                    Some(Command::Write { write, reply, queued }) => match stale(queued, reply) {
                        Some(reply) => self.write(write, reply).await,
                        None => Ok(()),
                    },
                    Some(Command::Refresh) => self.refresh().await,
                    Some(Command::Release) => self.release(),
                },
                update = stream.next() => match update {
                    None => Err(Ended::Lost("the server ended the event stream".into())),
                    Some(Err(error)) => Err(ended(&error)),
                    Some(Ok(update)) => self.streamed(update.event).await,
                },
                _ = turn.tick() => self.refresh().await,
            };
            if let Err(ended) = step.and_then(|()| self.expire()) {
                return ended;
            }
        }
    }

    fn send(&self, update: Update) -> Step {
        self.updates.send(update).map_err(|_| Ended::Dropped)
    }

    /// Apply what the owner released: new feeds go again once nothing
    /// holds them back.
    fn release(&mut self) -> Step {
        for update in self.memory.take_releases() {
            self.send(update)?;
        }
        Ok(())
    }

    /// The state as it now stands.
    fn state(&self) -> Step {
        let state = State::of(
            &self.memory.facts,
            self.memory.calibration.as_ref(),
            &self.zone,
        );
        match state {
            Some(state) => self.send(Update::state(state)),
            None => Ok(()),
        }
    }

    /// The feeder as the server holds it, if it is still the feeder this
    /// session serves.
    async fn detail(&self) -> Result<FeederDetail, Ended> {
        let detail = self
            .client
            .feeder(self.target.feeder)
            .await
            .map_err(|error| ended(&error))?;

        if detail.feeder.identity.0 != self.target.identity {
            return Err(Ended::LostFeeder(
                "the server holds another feeder under this id".into(),
            ));
        }
        if detail.feeder.archived.is_some() {
            return Err(Ended::LostFeeder(
                "the feeder was removed from the server".into(),
            ));
        }
        if detail.feeder.connection == Connection::Refused {
            return Err(Ended::LostFeeder(
                "the feeder no longer knows the server's key".into(),
            ));
        }
        Ok(detail)
    }

    /// Keep what the detail says of the feeder.
    fn took(&mut self, detail: &FeederDetail) {
        self.kind = detail.feeder.kind.known();
        self.legacy = detail.feeder.skew == Some(model::Skew::Legacy);
        if let Some(snapshot) = &detail.status {
            self.memory.facts.status = Some(snapshot.status.clone());
        }
    }

    /// The server reaches the feeder: announce it as a session that
    /// opened, as the direct session does at each connect, and read the
    /// facts.
    async fn reached(&mut self, detail: &FeederDetail) -> Step {
        self.online = true;
        let build = detail
            .status
            .as_ref()
            .map(|snapshot| snapshot.status.build.clone())
            .unwrap_or_default();
        self.send(Update::connected_as(
            build,
            Role::from(self.account),
            self.kind.map_or(Capabilities::NONE, Capabilities::of),
            match self.kind {
                Some(_) => skew(self.server, detail.feeder.skew),
                None => Skew::NewerBuild,
            },
        ))?;
        self.read_facts().await
    }

    /// Read the settings, the plans and the calibration, each the feeder's
    /// kind serves. One the server could not read (the feeder went offline
    /// meanwhile, or a member reading the calibration) is read again at the
    /// next turn.
    async fn read_facts(&mut self) -> Step {
        self.read_settings().await?;
        self.read_plans().await?;
        let (client, id) = (self.client, self.target.feeder);
        if self.serves(&Op::GetCalibration)
            && let Some(view) = read(client.calibration(id).await)?
        {
            self.memory.calibration = Some(view);
        }
        Ok(())
    }

    /// Read the settings, if the feeder's kind serves them; ones the
    /// server could not read stay as they were.
    async fn read_settings(&mut self) -> Step {
        let (client, id) = (self.client, self.target.feeder);
        if self.serves(&Op::GetSettings)
            && let Some(Settings::Feeder(settings)) = read(client.settings(id).await)?
        {
            self.memory.facts.settings = Some(settings);
        }
        Ok(())
    }

    /// Read the plans, as [`Self::read_settings`] reads the settings.
    async fn read_plans(&mut self) -> Step {
        let (client, id) = (self.client, self.target.feeder);
        if self.serves(&Op::GetPlans)
            && let Some(plans) = read(client.plans(id).await)?
        {
            self.memory.facts.plans = Some(plans);
        }
        Ok(())
    }

    /// Whether the feeder's kind serves `op`, by the op's own kind.
    fn serves(&self, op: &Op) -> bool {
        op.kind().is_none_or(|kind| Some(kind) == self.kind)
    }

    /// Replay the history from the cursor, or the newest of it.
    async fn backlog(&mut self, detail: &FeederDetail) -> Step {
        let Some(epoch) = detail.feeder.epoch else {
            return Ok(());
        };
        let newest = detail
            .feeder
            .synced
            .map(|synced| synced.saturating_sub(BACKLOG));
        let mut after = match self.memory.cursor {
            Some(held) if held.epoch == epoch => Some(held.seq),
            Some(_) => {
                self.memory.facts.feeds.clear();
                self.memory.cursor = None;
                self.send(Update::gap(None))?;
                newest
            }
            None => newest,
        };

        loop {
            let query = EventQuery {
                epoch: Some(epoch),
                after,
                limit: Some(PAGE),
                kind: None,
            };
            let page = self
                .client
                .events(self.target.feeder, &query)
                .await
                .map_err(|error| ended(&error))?;
            for gap in &page.gaps {
                self.gap(gap)?;
            }
            for event in page.events {
                self.event(&event, true)?;
            }
            match page.next {
                Some(next) => after = Some(next),
                None => return Ok(()),
            }
        }
    }

    /// Ask the server's journal about each feed still unsettled: the stream
    /// tells only of changes while it is open.
    async fn recheck(&mut self) -> Step {
        let keys: Vec<Uuid> = self.memory.uncertain.iter().map(|feed| feed.key).collect();
        self.ask_journal(keys).await
    }

    /// Ask the journal again about each feed it lacked before the grace
    /// passed, once the grace has.
    async fn recheck_unfound(&mut self) -> Step {
        let token_id = &self.target.token_id;
        let keys: Vec<Uuid> = self
            .memory
            .uncertain
            .iter()
            .filter(|feed| feed.unfound && feed.never_sent(token_id))
            .map(|feed| feed.key)
            .collect();
        self.ask_journal(keys).await
    }

    async fn ask_journal(&mut self, keys: Vec<Uuid>) -> Step {
        for key in keys {
            match self.client.command(key).await {
                Ok(command) => self.journalled(&command)?,
                Err(error) if error.code() == Some(ErrorCode::NotFound) => self.unfound(key)?,
                // Unauthorized ends the session; anything else is asked
                // again at the next connect or resync.
                Err(error) if error.failure() == Failure::Unauthorized => {
                    return Err(Ended::TokenRefused);
                }
                Err(_) => {}
            }
        }
        Ok(())
    }

    /// The journal holds no entry under `key`. The server records a write
    /// before it sends it and retires a key a read found no write under, so
    /// past the grace, under the token the feed went under, it was never
    /// sent and never will be. Sooner, or under another token, the read
    /// says nothing: a request may still be on its way, and another
    /// token's account may not see the entry.
    fn unfound(&mut self, key: Uuid) -> Step {
        let token_id = &self.target.token_id;
        let Some(index) = self
            .memory
            .uncertain
            .iter()
            .position(|uncertain| uncertain.key == key)
        else {
            return Ok(());
        };
        if !self.memory.uncertain[index].never_sent(token_id) {
            self.memory.uncertain[index].unfound = true;
            return Ok(());
        }
        let uncertain = self.memory.uncertain.remove(index);
        self.send(Update::settled_unfed(uncertain.pending))
    }

    async fn streamed(&mut self, event: StreamEvent) -> Step {
        let ours = self.target.feeder;
        match event {
            StreamEvent::Event { feeder, event } if feeder == ours => {
                if self.event(&event, false)? {
                    self.news(&event.body).await?;
                }
                self.state()
            }
            StreamEvent::Gap { feeder, gap } if feeder == ours => self.gap(&gap),
            StreamEvent::Connection { feeder, connection } if feeder == ours => {
                self.connection(connection).await
            }
            StreamEvent::Command { command } if command.feeder == ours => {
                self.journalled(&command)?;
                self.state()
            }
            StreamEvent::Alerts { feeder } if feeder == ours => self.read_alerts().await,
            // The stream fell behind and dropped what it held: read what it
            // would have said.
            StreamEvent::Resync => self.resync().await,
            _ => Ok(()),
        }
    }

    /// The feeder's alerts as the server last read them, and whether it
    /// reaches the feeder. A server that can't serve them (one from before
    /// the route, or a passing failure) is asked again at the next turn.
    async fn read_alerts(&mut self) -> Step {
        match self.client.feeder_alerts(self.target.feeder).await {
            Ok(alerts) => match self.memory.alerts.relayed(&alerts) {
                Some(update) => self.send(update),
                None => Ok(()),
            },
            Err(error) if error.failure() == Failure::Unauthorized => Err(Ended::TokenRefused),
            Err(_) => Ok(()),
        }
    }

    /// One event of the history: `replayed` when read from the history
    /// rather than as it happened. Whether it was new: the stream repeats
    /// what a backlog read.
    fn event(&mut self, event: &HistoryEvent, replayed: bool) -> Result<bool, Ended> {
        match self.memory.cursor {
            // The stream repeats what the backlog read.
            Some(held) if held.epoch == event.epoch && event.seq <= held.seq => return Ok(false),
            // A history that started over while the stream was open.
            Some(held) if held.epoch != event.epoch => {
                self.memory.facts.feeds.clear();
                self.send(Update::gap(None))?;
            }
            _ => {}
        }
        self.memory.cursor = Some(Cursor {
            epoch: event.epoch,
            seq: event.seq,
        });

        let event = model::Event {
            epoch: event.epoch,
            seq: event.seq,
            at: event.at,
            body: event.body,
        };
        if let Some(feed) = Feed::from_event(&event) {
            direct::fed(&mut self.memory.facts, feed);
            self.settle(&feed)?;
        }
        self.send(Update::event(&event, replayed))?;
        Ok(true)
    }

    /// An event as it happened: one that changes the settings or the plans
    /// has them read again now, so the state never shows them unread and a
    /// write is never refused for want of them. A store this session already
    /// holds at the event's generation (its own write, answered with the
    /// store) is not read again.
    async fn news(&mut self, body: &model::EventBody) -> Step {
        use model::{EventBody, FeederEvent};
        let facts = &self.memory.facts;
        match body {
            EventBody::Feeder(FeederEvent::Settings { generation, .. }) => {
                let held = facts.settings.as_ref().and_then(|held| held.generation);
                if held != Some(*generation) {
                    self.read_settings().await?;
                }
            }
            EventBody::Feeder(FeederEvent::Plans { generation, .. }) => {
                let held = facts.plans.as_ref().map(|held| held.generation);
                if held != Some(*generation) {
                    self.read_plans().await?;
                }
            }
            body if body.moves_plans() => self.read_plans().await?,
            _ => {}
        }
        Ok(())
    }

    fn gap(&mut self, gap: &server::Gap) -> Step {
        if self
            .memory
            .cursor
            .is_none_or(|held| held.epoch == gap.epoch && held.seq < gap.to)
        {
            self.memory.cursor = Some(Cursor {
                epoch: gap.epoch,
                seq: gap.to,
            });
        }
        let gap = model::Gap {
            epoch: gap.epoch,
            from: gap.from,
            to: gap.to,
        };
        self.send(Update::gap(Some(&gap)))
    }

    async fn connection(&mut self, connection: Connection) -> Step {
        match connection {
            Connection::Refused => Err(Ended::LostFeeder(
                "the feeder no longer knows the server's key".into(),
            )),
            // Announced as a new session, with what changed meanwhile.
            Connection::Online if !self.online => self.refresh().await,
            Connection::Online => Ok(()),
            Connection::Offline | Connection::Connecting | Connection::Unknown => {
                if self.online {
                    self.online = false;
                    self.send(Update::disconnected(OFFLINE))?;
                }
                Ok(())
            }
        }
    }

    /// Read the feeder anew: at each turn, and on the integration's asking.
    async fn refresh(&mut self) -> Step {
        self.recheck_unfound().await?;
        let detail = self.detail().await?;
        self.took(&detail);
        let online = detail.feeder.connection == Connection::Online;
        if online && !self.online {
            self.reached(&detail).await?;
        } else if !online && self.online {
            self.online = false;
            self.send(Update::disconnected(OFFLINE))?;
        } else if online {
            let facts = &self.memory.facts;
            if facts.settings.is_none()
                || facts.plans.is_none()
                || self.memory.calibration.is_none()
            {
                self.read_facts().await?;
            }
        }
        self.state()?;
        self.read_alerts().await
    }

    /// The stream fell behind the feeder and dropped what it held: read the
    /// feeder and its facts anew, replay the history from the cursor, flagged
    /// as replayed so nothing fires for it, and ask the journal about each
    /// feed still unsettled, as a new session would, without ending this one.
    async fn resync(&mut self) -> Step {
        let detail = self.detail().await?;
        self.took(&detail);
        let online = detail.feeder.connection == Connection::Online;
        if online && !self.online {
            self.reached(&detail).await?;
        } else if online {
            self.read_facts().await?;
        } else if self.online {
            self.online = false;
            self.send(Update::disconnected(OFFLINE))?;
        }
        self.backlog(&detail).await?;
        self.recheck().await?;
        self.state()?;
        self.read_alerts().await
    }

    async fn write(&mut self, write: Write, reply: oneshot::Sender<Written>) -> Step {
        let (written, step) = match self.prepare(&write) {
            Ok((op, kind)) => self.call(op, kind).await,
            Err(why) => (Written::not_sent(why), Ok(())),
        };
        let _ = reply.send(written);
        step
    }

    /// The op `write` becomes, unless it is never sent.
    fn prepare(&self, write: &Write) -> Result<(Op, Kind), NotSent> {
        wellfed_formats::require(self.online, || NotSent::Disconnected)?;
        wellfed_formats::require(!self.legacy, || NotSent::Legacy)?;

        let facts = &self.memory.facts;
        let read = writes::Read {
            // A view with no generation (read-only while the feeder
            // repairs its settings) gives a write nothing to name.
            settings: facts
                .settings
                .as_ref()
                .and_then(|settings| settings.generation),
            plans: facts.plans.as_ref(),
            calibration: self.memory.calibration.as_ref(),
        };
        let (op, kind) = writes::build(write, &read)?;
        let held = matches!(kind, Kind::Feed { .. }) && self.memory.holds_feeds();
        wellfed_formats::require(!held, || NotSent::Held)?;
        writes::gate(self.role, &op)?;
        writes::served(&op, self.kind.ok_or(NotSent::Unsupported)?)?;
        Ok((op, kind))
    }

    /// Send `op` once, under a fresh key: how it ended, and whether the
    /// session goes on.
    async fn call(&mut self, op: Op, kind: Kind) -> (Written, Step) {
        let (client, id) = (self.client, self.target.feeder);
        let key = Uuid::new_v4();

        let called = match op {
            Op::Feed { portions } => match client.feed(id, portions, key).await {
                Ok(command) => Ok(self.feeding(&command, portions)),
                Err(error) => Err(error),
            },
            Op::CancelFeed { feed } => client.cancel_feed(id, feed, key).await.map(done),
            Op::SetLock { locked } => {
                let lock = if locked { Lock::Locked } else { Lock::Unlocked };
                client.set_lock(id, lock, key).await.map(done)
            }
            Op::ResetDesiccant => client.reset_desiccant(id, key).await.map(done),
            Op::Play { tune } => client.play(id, tune.into(), key).await.map(done),
            Op::PutSettings { base, changes } => {
                match client.put_settings(id, base, &changes, key).await {
                    Ok(settings) => {
                        self.memory.facts.settings = settings.feeder().copied();
                        Ok(Written::done())
                    }
                    Err(error) => Err(error),
                }
            }
            Op::PutPlans { base, plans } => {
                match client
                    .put_plans(id, base, &PlansChange { plans }, key)
                    .await
                {
                    Ok(plans) => {
                        self.memory.facts.plans = Some(plans);
                        Ok(Written::done())
                    }
                    Err(error) => Err(error),
                }
            }
            Op::PutCalibration { base, calibration } => {
                match client.put_calibration(id, base, &calibration, key).await {
                    Ok(view) => {
                        self.memory.calibration = Some(view);
                        Ok(Written::done())
                    }
                    Err(error) => Err(error),
                }
            }
            // Every write Home Assistant builds has its route; another
            // never leaves.
            _ => return (Written::not_sent(NotSent::Unsupported), Ok(())),
        };

        match called {
            Ok(written) => (written, self.state()),
            Err(error) => self.failed(&error, kind, key),
        }
    }

    /// How a feed the server answered ended.
    fn feeding(&mut self, command: &server::Command, portions: u8) -> Written {
        match command.outcome {
            Some(CommandOutcome::Feeding { feed } | CommandOutcome::Fed { feed, .. }) => {
                Written::feeding(feed)
            }
            Some(CommandOutcome::Refused { refusal }) => Written::refused_for(refusal.into()),
            // Taken, with no answer yet: the journal settles it.
            _ => self.unanswered(command.key, portions, command.feed),
        }
    }

    /// A feed under `key` went out and its answer never came: uncertain,
    /// and kept until the journal settles it.
    fn unanswered(&mut self, key: Uuid, portions: u8, feed: Option<u32>) -> Written {
        let pending = Pending {
            id: key.to_string(),
            portions,
            sent_ms: now_ms(),
            after: None,
            reported: false,
            released_ms: None,
            token_id: Some(self.target.token_id.clone()),
        };
        // Announced on the updates before anything can settle it, so the
        // integration keeps it first; a handle gone has no one to tell.
        let _ = self.updates.send(Update::pending(pending.clone()));
        self.memory
            .uncertain
            .push(Uncertain::new(key, pending.clone(), feed));
        Written::uncertain_feed(pending)
    }

    /// How a write of `kind` under `key` that failed with `error` ended,
    /// and whether the session goes on.
    fn failed(
        &mut self,
        error: &wellfed_server_client::Error,
        kind: Kind,
        key: Uuid,
    ) -> (Written, Step) {
        match error.failure() {
            // Nothing left this host: the server is gone for now.
            Failure::Connect => (
                Written::not_sent(NotSent::Disconnected),
                Err(Ended::Lost(error.to_string())),
            ),
            // The server's feeder task never took it.
            Failure::FeederOffline => (Written::not_sent(NotSent::Disconnected), Ok(())),
            // It may have run: never sent again.
            Failure::Interrupted | Failure::OutcomeUnknown => {
                let written = match kind {
                    Kind::Feed { portions } => self.unanswered(key, portions, None),
                    _ => Written::uncertain(),
                };
                (written, Ok(()))
            }
            Failure::FeederRefused => {
                let refusal = refusal_of(error).unwrap_or(RefusalKind::NewerBuild);
                (Written::refused_for(refusal), Ok(()))
            }
            Failure::Stale => {
                self.stale(error);
                (Written::refused_for(RefusalKind::Stale), self.state())
            }
            // A key drawn fresh for this write names no other.
            Failure::KeyReused => (Written::not_sent(NotSent::Invalid), Ok(())),
            Failure::Unauthorized => (
                Written::not_sent(NotSent::Stopped),
                Err(Ended::TokenRefused),
            ),
            Failure::Rejected => rejected(error),
        }
    }

    /// Keep the store a stale write's refusal carries, as it now stands.
    fn stale(&mut self, error: &wellfed_server_client::Error) {
        use wellfed_server_client::Error as E;
        match error {
            E::Stale(settings) => {
                self.memory.facts.settings = settings.feeder().copied();
            }
            E::StalePlans(plans) => self.memory.facts.plans = Some(**plans),
            E::StaleCalibration(view) => self.memory.calibration = Some(**view),
            _ => {}
        }
    }

    /// What the server's journal says of a feed this session never had an
    /// answer for.
    fn journalled(&mut self, command: &server::Command) -> Step {
        let Some(index) = self
            .memory
            .uncertain
            .iter()
            .position(|uncertain| uncertain.key == command.key)
        else {
            return Ok(());
        };

        match wellfed_server_client::settled(command.state, command.outcome.as_ref()) {
            Some(Settled::Refused(_)) => {
                let uncertain = self.memory.uncertain.remove(index);
                self.send(Update::settled_unfed(uncertain.pending))
            }
            // Whether it ran is not known; a `Fed` the server finds later
            // replaces the journal's answer, so it is still waited on.
            Some(Settled::Unrecorded) => self.report(|uncertain| uncertain.key == command.key),
            // The journal has the feed's end: settled now, whether or not
            // its event has come.
            Some(Settled::Fed {
                feed,
                requested,
                completed,
                outcome,
                reason,
            }) => {
                let uncertain = self.memory.uncertain.remove(index);
                let fed = api::Feed::journalled(
                    feed,
                    requested,
                    completed,
                    outcome,
                    reason,
                    command.updated,
                );
                self.send(Update::settled(fed, uncertain.pending))
            }
            // Started, or not yet answered: its `Fed` settles it once the
            // journal names its number.
            Some(Settled::Started { .. } | Settled::Done | Settled::Superseded) | None => {
                let feed = command
                    .feed
                    .or_else(|| command.outcome.as_ref().and_then(CommandOutcome::feed));
                let Some(feed) = feed else {
                    return Ok(());
                };
                self.memory.uncertain[index].feed = Some(feed);
                // Its `Fed` event may be in already.
                let fed = self
                    .memory
                    .facts
                    .feeds
                    .iter()
                    .rev()
                    .find(|fed| fed.feed == feed)
                    .copied();
                match fed {
                    Some(fed) => self.settle(&fed),
                    None => Ok(()),
                }
            }
        }
    }

    /// The `Fed` event of a feed this session never had an answer for
    /// settles it.
    fn settle(&mut self, feed: &Feed) -> Step {
        let found = self
            .memory
            .uncertain
            .iter()
            .position(|uncertain| uncertain.feed == Some(feed.feed));
        let Some(found) = found else {
            return Ok(());
        };
        let uncertain = self.memory.uncertain.remove(found);
        self.send(Update::settled(api::Feed::from(feed), uncertain.pending))
    }

    /// Uncertain feeds past [`SETTLE`] the journal has not settled are
    /// still unknown, and still held; those released a week ago are
    /// forgotten. One still holding is never forgotten.
    fn expire(&mut self) -> Step {
        let (gone, open) = std::mem::take(&mut self.memory.uncertain)
            .into_iter()
            .partition(|uncertain| uncertain.pending.released_ms.is_some_and(direct::forgotten));
        self.memory.uncertain = open;
        for uncertain in gone {
            self.send(Update::forgotten(uncertain.pending))?;
        }
        for update in self.memory.holds.past_keep() {
            self.send(update)?;
        }

        let now = Instant::now();
        self.report(|uncertain| now.duration_since(uncertain.sent) >= SETTLE)
    }

    /// Report each uncertain feed `due` picks, not yet reported, as still
    /// unknown; it stays open to the journal.
    fn report(&mut self, due: impl Fn(&Uncertain) -> bool) -> Step {
        let mut reported = Vec::new();
        for uncertain in &mut self.memory.uncertain {
            if !uncertain.pending.reported && due(uncertain) {
                uncertain.pending.reported = true;
                reported.push(uncertain.pending.clone());
            }
        }
        for pending in reported {
            self.send(Update::still_unknown(pending))?;
        }
        Ok(())
    }
}

/// The disconnected update's reason while the server cannot reach the
/// feeder.
const OFFLINE: &str = "the server cannot reach the feeder";

fn done((): ()) -> Written {
    Written::done()
}

/// What the integration names to update: the feeder, the server or itself.
/// A feeder from before API revisions comes first, since nothing but an
/// update reaches it; then a server older than either side, since what it
/// lacks is lost on the way whichever side has it.
const fn skew(server: ServerSkew, feeder: Option<model::Skew>) -> Skew {
    match (server, feeder) {
        (_, Some(model::Skew::Legacy)) => Skew::Legacy,
        (ServerSkew::ServerBehind, _) | (_, Some(model::Skew::FeederAhead)) => Skew::ServerBehind,
        (_, Some(model::Skew::FeederBehind)) => Skew::FeederBehind,
        (ServerSkew::ServerAhead, _) => Skew::ServerAhead,
        (_, Some(model::Skew::Unknown)) => Skew::NewerBuild,
        (ServerSkew::Same, Some(model::Skew::Same) | None) => Skew::Same,
    }
}

/// How a call that ended the session with `error` ends it.
fn ended(error: &wellfed_server_client::Error) -> Ended {
    if error.failure() == Failure::Unauthorized {
        return Ended::TokenRefused;
    }
    match error.code() {
        Some(ErrorCode::NotFound) => {
            Ended::LostFeeder("the server no longer holds the feeder".into())
        }
        Some(ErrorCode::FeederArchived) => {
            Ended::LostFeeder("the feeder was removed from the server".into())
        }
        _ => Ended::Lost(error.to_string()),
    }
}

/// A read's value, or `None` when the server could not read it now; a
/// refused token or a server gone ends the session.
fn read<T>(read: Result<T, wellfed_server_client::Error>) -> Result<Option<T>, Ended> {
    match read {
        Ok(value) => Ok(Some(value)),
        Err(error) => match error.failure() {
            Failure::Unauthorized => Err(Ended::TokenRefused),
            Failure::Connect => Err(Ended::Lost(error.to_string())),
            _ => Ok(None),
        },
    }
}

/// The feeder's refusal a failed write carries.
fn refusal_of(error: &wellfed_server_client::Error) -> Option<RefusalKind> {
    match error {
        wellfed_server_client::Error::Problem { problem, .. } => problem.refusal.map(Into::into),
        _ => None,
    }
}

/// A write the server refused before sending anything.
fn rejected(error: &wellfed_server_client::Error) -> (Written, Step) {
    let not_sent = match error.code() {
        Some(ErrorCode::FeederLegacy) => NotSent::Legacy,
        Some(ErrorCode::Forbidden) => NotSent::Forbidden,
        Some(ErrorCode::OtherKind | ErrorCode::OtherModel) => NotSent::Unsupported,
        Some(ErrorCode::NotFound | ErrorCode::FeederArchived) => {
            return (Written::not_sent(NotSent::Stopped), Err(ended(error)));
        }
        _ => NotSent::Invalid,
    };
    (Written::not_sent(not_sent), Ok(()))
}

#[cfg(test)]
mod tests {
    use wellfed_server_client::api::{CommandKind, CommandState, FeedOutcome, FeedReason, UserId};

    use crate::api::UpdateKind;

    use super::*;

    /// A session that reaches no server: the journal's answers are handed
    /// to it as the stream would.
    fn offline(memory: &mut Memory, test: impl FnOnce(&mut Session<'_>)) -> Vec<Update> {
        let target = Target {
            url: Url::parse("http://127.0.0.1:9").unwrap(),
            token: "token".into(),
            token_id: TOKEN_ID.into(),
            feeder: FeederId(Uuid::from_u128(1)),
            identity: [0; 32],
        };
        let client =
            Client::with_timeouts(&target.url, Clocks::DEFAULT.timeouts, PlainHttp::Allowed)
                .unwrap();
        let (updates, heard) = std_mpsc::channel();
        let mut session = Session::new(&client, &target, memory, &updates);
        test(&mut session);
        drop(session);
        drop(updates);
        heard.into_iter().collect()
    }

    const TOKEN_ID: &str = "the-token";

    fn lost(key: Uuid, sent_ms: i64) -> Uncertain {
        let pending = Pending {
            id: key.to_string(),
            portions: 2,
            sent_ms,
            after: None,
            reported: false,
            released_ms: None,
            token_id: Some(TOKEN_ID.into()),
        };
        Uncertain::new(key, pending, None)
    }

    fn journal(key: Uuid, state: CommandState, outcome: CommandOutcome) -> server::Command {
        server::Command {
            key,
            user: UserId(Uuid::from_u128(2)),
            token: None,
            feeder: FeederId(Uuid::from_u128(1)),
            kind: CommandKind::Feed,
            state,
            outcome: Some(outcome),
            feed: None,
            boot_count: None,
            sent: jiff::Timestamp::UNIX_EPOCH,
            updated: jiff::Timestamp::UNIX_EPOCH,
        }
    }

    fn kinds(updates: &[Update]) -> Vec<UpdateKind> {
        updates.iter().map(|update| update.kind).collect()
    }

    /// The journal says the feeder restarted with no record of the feed:
    /// still unknown, and still waited on, so the `Fed` the server finds
    /// later settles it.
    #[test]
    fn a_feed_the_journal_found_unrecorded_is_settled_by_a_later_fed() {
        let key = Uuid::from_u128(7);
        let mut memory = Memory::default();
        memory.uncertain.push(lost(key, now_ms()));

        let unrecorded = journal(key, CommandState::Resolved, CommandOutcome::Unrecorded);
        let heard = offline(&mut memory, |session| {
            assert!(session.journalled(&unrecorded).is_ok());
        });
        assert_eq!(kinds(&heard), [UpdateKind::StillUnknown]);
        assert_eq!(memory.uncertain.len(), 1, "still waited on");
        assert!(memory.uncertain[0].pending.reported);

        let fed = CommandOutcome::Fed {
            feed: 9,
            requested: 2,
            completed: 2,
            outcome: FeedOutcome::Done,
            reason: FeedReason::Done,
        };
        let fed = journal(key, CommandState::Resolved, fed);
        let heard = offline(&mut memory, |session| {
            assert!(session.journalled(&fed).is_ok());
        });
        assert_eq!(kinds(&heard), [UpdateKind::Settled]);
        assert_eq!(heard[0].feed.as_ref().map(|fed| fed.feed), Some(9));
        assert!(memory.uncertain.is_empty());
    }

    /// A feed the journal has not settled is reported still unknown once
    /// its settle window has passed, and holds feeds back however old it
    /// is; only one the owner released a week ago is forgotten.
    #[test]
    fn a_lost_feed_is_reported_and_forgotten_only_a_week_after_its_release() {
        let mut memory = Memory::default();
        let fresh = Uuid::from_u128(1);
        let settle = i64::try_from(SETTLE.as_millis()).unwrap();
        let reported = Uuid::from_u128(2);
        let old = Uuid::from_u128(3);
        let released = Uuid::from_u128(4);
        memory.uncertain.push(lost(fresh, now_ms()));
        memory
            .uncertain
            .push(lost(reported, now_ms() - settle - 1000));
        memory
            .uncertain
            .push(lost(old, now_ms() - direct::KEEP_MS - 1000));
        let mut gone = lost(released, now_ms() - direct::KEEP_MS * 2);
        gone.pending.reported = true;
        gone.pending.released_ms = Some(now_ms() - direct::KEEP_MS - 1000);
        memory.uncertain.push(gone);

        let heard = offline(&mut memory, |session| assert!(session.expire().is_ok()));
        let what: Vec<(UpdateKind, String)> = heard
            .iter()
            .map(|update| (update.kind, update.pending.as_ref().unwrap().id.clone()))
            .collect();
        assert_eq!(
            what,
            [
                (UpdateKind::Forgotten, released.to_string()),
                (UpdateKind::StillUnknown, reported.to_string()),
                (UpdateKind::StillUnknown, old.to_string()),
            ]
        );
        let open: Vec<Uuid> = memory
            .uncertain
            .iter()
            .map(|uncertain| uncertain.key)
            .collect();
        assert_eq!(open, [fresh, reported, old]);
        assert!(memory.holds_feeds());
    }

    /// A journal that lacks a feed's key says it was never sent only under
    /// the token it went under, and only once the grace has passed by both
    /// clocks; sooner it is asked again later.
    #[test]
    fn a_feed_the_journal_lacks_is_never_sent_only_past_the_grace_under_its_token() {
        let grace = Duration::from_millis(UNFOUND_GRACE_MS.unsigned_abs());
        let long_ago = now_ms() - UNFOUND_GRACE_MS - 1000;
        let (recent, unknown_since, other, past) = (
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            Uuid::from_u128(4),
        );
        let mut memory = Memory::default();
        memory.uncertain.push(lost(recent, now_ms()));
        // Old by the wall clock, but this run has known it only now.
        memory.uncertain.push(lost(unknown_since, long_ago));
        let mut another = lost(other, long_ago);
        another.pending.token_id = Some("another-token".into());
        another.known = Instant::now() - grace;
        memory.uncertain.push(another);
        let mut settled = lost(past, long_ago);
        settled.known = Instant::now() - grace;
        memory.uncertain.push(settled);

        let heard = offline(&mut memory, |session| {
            for key in [recent, unknown_since, other, past] {
                assert!(session.unfound(key).is_ok());
            }
        });
        assert_eq!(kinds(&heard), [UpdateKind::SettledUnfed]);
        assert_eq!(heard[0].pending.as_ref().unwrap().id, past.to_string());
        let open: Vec<(Uuid, bool)> = memory
            .uncertain
            .iter()
            .map(|uncertain| (uncertain.key, uncertain.unfound))
            .collect();
        assert_eq!(
            open,
            [(recent, true), (unknown_since, true), (other, true)],
            "asked again later"
        );
    }

    /// Lost feeds hold the next feed back until each settles or the owner
    /// releases it, the direct route's ones carried here among them.
    #[test]
    fn a_lost_feed_from_either_route_holds_until_released() {
        let shared = Releases::default();
        let direct = Pending {
            token_id: None,
            ..lost(Uuid::from_u128(2), now_ms()).pending
        };
        let mut memory = Memory {
            holds: Holds::new(std::sync::Arc::clone(&shared), vec![direct], false),
            ..Memory::default()
        };
        memory.uncertain.push(lost(Uuid::from_u128(1), now_ms()));
        assert!(memory.holds_feeds());

        crate::hold::releases(&shared)
            .push(crate::hold::Release::Feed(Uuid::from_u128(1).to_string()));
        assert_eq!(memory.take_releases().len(), 1);
        assert!(memory.holds_feeds(), "the direct route's still holds");

        crate::hold::releases(&shared)
            .push(crate::hold::Release::Feed(Uuid::from_u128(2).to_string()));
        assert_eq!(memory.take_releases().len(), 1);
        assert!(!memory.holds_feeds());
        assert_eq!(memory.uncertain.len(), 1, "still open to the journal");
    }

    /// The stream fell behind and dropped what it held: a feed and a
    /// settings change made meanwhile are read again without a reconnect,
    /// the feed replayed, so nothing fires for it.
    #[test]
    fn a_resync_reads_what_the_stream_dropped() {
        use wellfed_server_testkit::feeder;
        use wellfed_server_testkit::{TestServer, ada_login, ada_setup, local, until};

        local(async {
            let sim = feeder::start().await;
            let server = TestServer::start().await;
            let mut owner = server.client();
            owner.setup(&ada_setup()).await.unwrap();
            let adopted = owner
                .adopt(&server::AdoptFeeder {
                    invite: feeder::invite(&sim, server::ClientRole::Server).await,
                    name: "kitchen".into(),
                    address: Some(sim.address.to_string()),
                    phone_key: None,
                })
                .await
                .unwrap();
            until("the server reaches the feeder", || async {
                owner.feeders().await.unwrap()[0].connection == Connection::Online
            })
            .await;
            let url = format!("http://{}", server.address());
            let ada = ada_login();
            let login = account::login(&url, &ada.name, &ada.password, "home-assistant")
                .await
                .unwrap();
            let target = Target {
                url: address(&url).unwrap(),
                token: login.token,
                token_id: login.token_id,
                feeder: adopted.id,
                identity: adopted.identity.0,
            };
            let client = client(&target, Clocks::DEFAULT).unwrap();
            let (updates, heard) = std_mpsc::channel();
            let mut memory = Memory::default();
            let mut session = Session::new(&client, &target, &mut memory, &updates);
            let Ok(_unread) = session.open().await else {
                panic!("the session opens");
            };
            while heard.try_recv().is_ok() {}

            // Meanwhile, with the stream unread: a feed and a settings
            // change.
            let fed = owner.feed(adopted.id, 1, Uuid::new_v4()).await.unwrap();
            let fed = fed.feed.expect("the feed's number");
            let Settings::Feeder(held) = owner.settings(adopted.id).await.unwrap() else {
                panic!("a feeder's settings");
            };
            let quieter = model::FeederSettingsPatch {
                volume: Some(held.volume / 2),
                ..model::FeederSettingsPatch::default()
            };
            let base = held.generation.unwrap();
            owner
                .put_settings(adopted.id, base, &quieter, Uuid::new_v4())
                .await
                .unwrap();
            until("the server has the feed's end", || async {
                let page = owner
                    .events(adopted.id, &EventQuery::default())
                    .await
                    .unwrap();
                page.events.iter().any(|event| {
                    Feed::from_event(&model::Event {
                        epoch: event.epoch,
                        seq: event.seq,
                        at: event.at,
                        body: event.body,
                    })
                    .is_some_and(|feed| feed.feed == fed)
                })
            })
            .await;

            assert!(session.streamed(StreamEvent::Resync).await.is_ok());
            drop(session);
            let settings = memory.facts.settings.expect("the settings");
            assert_eq!(settings.volume, held.volume / 2);
            assert!(memory.facts.feeds.iter().any(|feed| feed.feed == fed));
            let replayed = heard.try_iter().any(|update| {
                update.replayed
                    && update
                        .event
                        .and_then(|event| event.fed)
                        .is_some_and(|event| event.feed == fed)
            });
            assert!(replayed, "the feed is replayed, not news");
            server.stop().await;
            sim.stop().await;
        });
    }

    #[test]
    fn the_side_to_update_comes_first_by_what_it_blocks() {
        use model::Skew as Feeder;

        assert_eq!(skew(ServerSkew::Same, Some(Feeder::Same)), Skew::Same);
        assert_eq!(skew(ServerSkew::Same, None), Skew::Same);
        assert_eq!(
            skew(ServerSkew::ServerBehind, Some(Feeder::Legacy)),
            Skew::Legacy
        );
        assert_eq!(
            skew(ServerSkew::ServerBehind, Some(Feeder::Same)),
            Skew::ServerBehind
        );
        assert_eq!(
            skew(ServerSkew::Same, Some(Feeder::FeederAhead)),
            Skew::ServerBehind
        );
        assert_eq!(
            skew(ServerSkew::ServerAhead, Some(Feeder::FeederBehind)),
            Skew::FeederBehind
        );
        assert_eq!(
            skew(ServerSkew::ServerAhead, Some(Feeder::Same)),
            Skew::ServerAhead
        );
        assert_eq!(
            skew(ServerSkew::Same, Some(Feeder::Unknown)),
            Skew::NewerBuild
        );
    }
}
