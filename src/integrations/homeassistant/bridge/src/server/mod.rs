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
//!   connect replays the newest events only ([`BACKLOG`]); a later one
//!   replays from its cursor, and a history that started over is a gap.
//! - **The feeder's connection** is the server's: while the server cannot
//!   reach it the session reads as disconnected and every write is refused
//!   unsent, as the direct session's are between sessions.
//! - **Writes** go out once, under a fresh idempotency key, and are never
//!   sent again here. A feed whose answer never came is tracked by its key
//!   and settled from the server's journal: by its `Fed` event once the
//!   journal names its number, as refused, or as unknown when the journal
//!   says the feeder restarted with no record of it. One unsettled after
//!   [`SETTLE`] is still unknown, never resent. A write that waited longer
//!   than [`FRESH`](crate::feeder::FRESH) for the session, or arrived while it connects, is
//!   refused unsent.
//! - **The event stream** is read by a task of its own, so a slow write or
//!   a long connect never leaves it unread past its read timeout.
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
use crate::feeder::{self as direct, Command, SETTLE, TURN, now_ms, refusing, sent_at, stale};
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

/// A feed that went out and was never answered, by the key it went under.
#[derive(Clone, Debug)]
struct Uncertain {
    key: Uuid,
    pending: Pending,
    /// Its number, once the server's journal names it.
    feed: Option<u32>,
    sent: Instant,
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
    pending: Vec<Pending>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    updates: std_mpsc::Sender<Update>,
) {
    let mut memory = Memory::default();
    for pending in pending {
        // Only a key this integration drew names a command on the server.
        match Uuid::parse_str(&pending.id) {
            Ok(key) => memory.uncertain.push(Uncertain {
                key,
                sent: sent_at(pending.sent_ms),
                pending,
                feed: None,
            }),
            Err(_) => {
                if updates.send(Update::still_unknown(pending)).is_err() {
                    return;
                }
            }
        }
    }
    let mut lost_feeder = false;

    loop {
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
        loop {
            let step = tokio::select! {
                command = commands.recv() => match command {
                    None => return Ended::Dropped,
                    Some(Command::Write { write, reply, queued }) => match stale(queued, reply) {
                        Some(reply) => self.write(write, reply).await,
                        None => Ok(()),
                    },
                    Some(Command::Refresh) => self.refresh().await,
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
        let (client, id) = (self.client, self.target.feeder);
        if self.serves(&Op::GetSettings)
            && let Some(Settings::Feeder(settings)) = read(client.settings(id).await)?
        {
            self.memory.facts.settings = Some(settings);
        }
        if self.serves(&Op::GetPlans)
            && let Some(plans) = read(client.plans(id).await)?
        {
            self.memory.facts.plans = Some(plans);
        }
        if self.serves(&Op::GetCalibration)
            && let Some(view) = read(client.calibration(id).await)?
        {
            self.memory.calibration = Some(view);
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
        for key in keys {
            match self.client.command(key).await {
                Ok(command) => self.journalled(&command)?,
                // Not journalled yet: the request may still be on its way
                // to the server, so it may still run. Unauthorized ends
                // the session; anything else is asked at the next connect.
                Err(error) if error.failure() == Failure::Unauthorized => {
                    return Err(Ended::TokenRefused);
                }
                Err(_) => {}
            }
        }
        Ok(())
    }

    async fn streamed(&mut self, event: StreamEvent) -> Step {
        let ours = self.target.feeder;
        match event {
            StreamEvent::Event { feeder, event } if feeder == ours => {
                self.event(&event, false)?;
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
            // The stream could not resume: read what it would have said.
            StreamEvent::Resync => self.refresh().await,
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

    /// One event of the history: `replayed` when read from the history at
    /// connect rather than as it happened.
    fn event(&mut self, event: &HistoryEvent, replayed: bool) -> Step {
        match self.memory.cursor {
            // The stream repeats what the backlog read.
            Some(held) if held.epoch == event.epoch && event.seq <= held.seq => return Ok(()),
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
        if !replayed {
            self.news(&event.body);
        }
        Ok(())
    }

    /// An event as it happened: some change what the facts hold, which the
    /// next turn reads again.
    fn news(&mut self, body: &model::EventBody) {
        use model::{EventBody, FeederEvent};
        match body {
            EventBody::Feeder(FeederEvent::Settings { .. }) => self.memory.facts.settings = None,
            EventBody::Feeder(FeederEvent::Plans { .. }) => self.memory.facts.plans = None,
            _ => {}
        }
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

    /// Read the feeder anew: at each turn, on the integration's asking,
    /// and when the stream could not resume.
    async fn refresh(&mut self) -> Step {
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
        writes::gate(self.role)?;
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
        };
        // Announced on the updates before anything can settle it, so the
        // integration keeps it first; a handle gone has no one to tell.
        let _ = self.updates.send(Update::pending(pending.clone()));
        self.memory.uncertain.push(Uncertain {
            key,
            pending: pending.clone(),
            feed,
            sent: Instant::now(),
        });
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
            Some(Settled::Unrecorded) => {
                let uncertain = self.memory.uncertain.remove(index);
                self.send(Update::still_unknown(uncertain.pending))
            }
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

    /// Uncertain feeds past [`SETTLE`] the journal has not settled: still
    /// unknown.
    fn expire(&mut self) -> Step {
        let now = Instant::now();
        let (expired, kept) = std::mem::take(&mut self.memory.uncertain)
            .into_iter()
            .partition(|uncertain| now.duration_since(uncertain.sent) >= SETTLE);
        self.memory.uncertain = kept;
        for uncertain in expired {
            self.send(Update::still_unknown(uncertain.pending))?;
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
    use super::*;

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
