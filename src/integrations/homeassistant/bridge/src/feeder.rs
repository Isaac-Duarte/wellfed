//! One feeder's session for Home Assistant, on a thread of its own.
//!
//! The thread runs a current-thread tokio runtime that opens the session,
//! reads what the feeder knows, follows its events and routes the
//! integration's writes. Home Assistant's side only blocks on
//! [`Feeder::next_update`] and on each write's reply, from its executor.
//!
//! - **On connect** it reads the status, then the settings, the plans, the
//!   time and (for an owner) the calibration, and subscribes to the events.
//!   The status names the newest event: everything up to it is the
//!   backlog, replayed into the facts (the feeds above all) and handed on
//!   flagged as replayed, so the integration fires only what is new. After
//!   the backlog, an event that changes the settings, the plans or the time
//!   has them read again. The event cursor and the facts survive a
//!   reconnect, so a later session replays only what it missed; a history
//!   that started over is a gap, and its backlog is read from the start.
//! - **Writes** go out once, under a fresh request id, and are never sent
//!   again here or by the integration. One the role does not allow
//!   ([`writes::gate`]), or whose op the device's kind does not serve
//!   ([`writes::served`]), is never sent; one while
//!   no session is open is not sent either, so the owner may simply press
//!   again. A change to a store goes on the generation last read, one at a
//!   time per store ([`writes`]).
//! - **Uncertain feeds.** A direct feed is kept from just before it goes
//!   out, in the integration's store too, so a feed out when Home Assistant
//!   stops is lost after the restart rather than forgotten. A feed whose
//!   answer never came may have run. The
//!   session keeps it, and settles it by the one `Fed` the rule the app
//!   shares picks ([`wellfed_client::settle`]): this client's, of the same
//!   portions, after the feed went out, not a feed it has the end of, and
//!   no other feed's it could be. It reports
//!   [`UpdateKind::StillUnknown`](crate::api::UpdateKind) when the history
//!   holds no such feed, never a guessed count. A `Fed` names the client,
//!   not the request, so while one feed is uncertain (or a feed's answer is
//!   still awaited) a new feed is not sent ([`NotSent::Held`]). Every
//!   number this client has the end of goes to the integration
//!   ([`UpdateKind::Answered`](crate::api::UpdateKind)), which hands them
//!   to the next session, so a replayed `Fed` of a feed settled before a
//!   restart settles nothing either.
//! - **Known limits.** The integration's store is written on Home
//!   Assistant's event loop, not before the request goes, so these remain;
//!   both fail toward "may have run", never toward a wrong settlement except
//!   the first, and only on a hard kill:
//!   - A feed's `sending` update reaches the store tens of milliseconds
//!     after its request is queued. Killed in that gap, the store never had
//!     the feed, and after the restart its `Fed` could settle an older open
//!     feed of the same portions. The app writes its pin before it sends.
//!   - Killed after a feed's answer but before its `cleared` update is
//!     stored, the restart finds the feed lost though it was answered: a
//!     false "may have run" repair, and, while it is open (up to a
//!     week), a `Fed` of the same portions settles no other lost feed. Its own `Fed` never settles it, since its number was kept
//!     first.
//! - **Revocation.** A key the feeder no longer knows ends the session for
//!   good: it is reported once and the thread stops reconnecting.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Mutex;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use jiff::tz::TimeZone;
use tokio::sync::{mpsc, oneshot};
use wellfed_client::alerts::{Cursor as AlertCursor, FeederClock};
use wellfed_client::history::{Backlog, Entry, Feed};
use wellfed_client::settle::{self, After, Lost};
use wellfed_client::today::Facts;
use wellfed_connect::{Connection, Cursor, Received, Timing};
use wellfed_model::{
    AlertsView, Answer, Blocked, CalibrationView, ClientRole, CoreEvent, EventBody, FeederEvent,
    FeederSettingsPatch, Generation, Op, Pushed, Readiness, Settings, Skew, Status,
};

use crate::Error;
use crate::alerts::Tracker;
use crate::api::{self, Capabilities, NotSent, Pending, State, Tune, Update, WriteResult, Written};
use crate::key::{ClientKey, locate, random};
use crate::writes::{self, Kind, Landed, Store, Write};

/// The device API's request id.
type RequestId = [u8; 16];

/// How often a quiet session reads the status anew, which also keeps the
/// connection's silence limit fed.
pub(crate) const TURN: Duration = Duration::from_secs(30);
/// How long an uncertain feed holds new feeds back before it is reported
/// still unknown, unless the backlog shows it sooner: longer than the
/// longest feed the feeder runs.
pub(crate) const SETTLE: Duration = Duration::from_secs(15 * 60);
/// How long after it went out a feed whose answer never came stays open to
/// its end: a `Fed` that turns up late, or a journal that settles it late,
/// still settles it. Past this it is forgotten
/// ([`UpdateKind::Forgotten`](crate::api::UpdateKind)), still unknown. The
/// app keeps a lost feed a week from when the owner puts it away; Home
/// Assistant has no such step, so it counts from the send.
pub(crate) const KEEP_MS: i64 = 7 * 24 * 3_600_000;
/// Feeds kept for the day's facts: far more than a day holds.
const FEEDS_KEPT: usize = 512;
/// Feed numbers kept from before the oldest open uncertain feed went out,
/// and kept in any case: a feed answered then may end after it.
const ANSWERED_TAIL: usize = 64;
/// The most feed numbers kept. Every number answered since an uncertain
/// feed went out must be kept while it is open, or a `Fed` of one dropped
/// could be taken for its end, so past this the oldest open one is
/// forgotten instead. A week of feeds is far fewer.
const ANSWERED_MOST: usize = 1024;
/// How long a write may wait for the session before it is refused unsent.
/// A press reaches an idle session within milliseconds; one that waited
/// longer stood behind a connect or a slow write, and by then the owner
/// has seen nothing happen and may have pressed again or walked away, so a
/// feed sent later is not the feed they asked for.
pub(crate) const FRESH: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub(crate) enum Command {
    Write {
        write: Write,
        reply: oneshot::Sender<Written>,
        /// When the integration asked: see [`FRESH`].
        queued: Instant,
    },
    Refresh,
}

/// Whether a write queued at `queued` waited too long to be sent; it is
/// then answered unsent here.
pub(crate) fn stale(
    queued: Instant,
    reply: oneshot::Sender<Written>,
) -> Option<oneshot::Sender<Written>> {
    if queued.elapsed() > FRESH {
        let _ = reply.send(Written::not_sent(NotSent::Disconnected));
        return None;
    }
    Some(reply)
}

/// Run `work` while refusing every write that arrives meanwhile, as
/// between sessions: a connect may take many requests, and a feed held
/// until it ends is not the feed the owner asked for. `None` once the
/// handle is dropped.
pub(crate) async fn refusing<T>(
    work: impl Future<Output = T>,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) -> Option<T> {
    tokio::pin!(work);
    loop {
        tokio::select! {
            done = &mut work => return Some(done),
            command = commands.recv() => match command {
                None => return None,
                Some(Command::Write { reply, .. }) => {
                    let _ = reply.send(Written::not_sent(NotSent::Disconnected));
                }
                Some(Command::Refresh) => {}
            },
        }
    }
}

/// What a session starts from after Home Assistant restarted: the feeds
/// still unsettled, and the feed numbers this client has the end of, as
/// the last [`UpdateKind::Answered`](crate::api::UpdateKind) listed them:
/// every one since the oldest open feed went out, so a `Fed` for one of
/// those settles nothing.
#[derive(Clone, Debug, Default)]
pub struct Carried {
    pub pending: Vec<Pending>,
    pub answered: Vec<u32>,
}

/// The time now in Unix milliseconds.
pub(crate) fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

/// The instant a pending feed went out, from its wall-clock time: a feed
/// carried across a restart keeps its settle window rather than starting
/// a new one.
pub(crate) fn sent_at(sent_ms: i64) -> Instant {
    let ago = u64::try_from(now_ms().saturating_sub(sent_ms)).unwrap_or(0);
    let ago = Duration::from_millis(ago).min(SETTLE);
    Instant::now().checked_sub(ago).unwrap_or_else(Instant::now)
}

/// How long the session waits on the feeder, and between sessions.
/// Home Assistant runs on [`Clocks::DEFAULT`]; a test that loses an answer
/// or a session on purpose shortens them, so it waits a second, not ten.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clocks {
    /// The device API session's own clocks: how long an answer may take
    /// before the write it answers is uncertain.
    pub session: Timing,
    /// How long a lost session waits before opening another.
    pub retry: Duration,
    /// How long a lost feed holds new feeds back before it is reported
    /// still unknown: [`SETTLE`].
    pub settle: Duration,
}

impl Clocks {
    pub const DEFAULT: Self = Self {
        session: Timing::DEFAULT,
        retry: Duration::from_secs(10),
        settle: SETTLE,
    };
}

/// Where the session finds its feeder.
#[derive(Clone, Debug)]
pub struct Target {
    pub identity: [u8; 32],
    /// An address, or `None` to find the feeder over mDNS at each connect.
    pub address: Option<String>,
}

/// A handle on the session thread. Dropping it ends the session.
#[derive(Debug)]
pub struct Feeder {
    commands: mpsc::UnboundedSender<Command>,
    updates: Mutex<std_mpsc::Receiver<Update>>,
}

impl Feeder {
    /// Start the session thread for `target` as `key`, on `clocks`. It
    /// connects, and reconnects after every loss, until the handle is
    /// dropped or the feeder revokes the key.
    /// `carried` is what the last run left unsettled.
    pub fn start(
        key: ClientKey,
        target: Target,
        clocks: Clocks,
        carried: Carried,
    ) -> Result<Self, Error> {
        Self::spawn(move |inbox, outbox| run(key, target, clocks, carried, inbox, outbox))
    }

    /// Start the session thread for the feeder `target` names through the
    /// owner's server ([`crate::server`]), on `clocks`. It connects, and
    /// reconnects after every loss, until the handle is dropped, the server
    /// refuses the token, or the server no longer holds the feeder.
    /// `pending` is what the last run left unsettled.
    pub fn start_server(
        target: crate::server::Target,
        clocks: crate::server::Clocks,
        pending: Vec<Pending>,
    ) -> Result<Self, Error> {
        let client = crate::server::client(&target, clocks)?;
        Self::spawn(move |inbox, outbox| {
            crate::server::run(client, target, clocks, pending, inbox, outbox)
        })
    }

    /// Run the session `serve` makes on a thread of its own, on a
    /// current-thread runtime.
    fn spawn<F, S>(serve: S) -> Result<Self, Error>
    where
        S: FnOnce(mpsc::UnboundedReceiver<Command>, std_mpsc::Sender<Update>) -> F + Send + 'static,
        F: Future<Output = ()>,
    {
        let (commands, inbox) = mpsc::unbounded_channel();
        let (outbox, updates) = std_mpsc::channel();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(Error::Thread)?;
        std::thread::Builder::new()
            .name("wellfed-session".into())
            .spawn(move || runtime.block_on(serve(inbox, outbox)))
            .map_err(Error::Thread)?;

        Ok(Self {
            commands,
            updates: Mutex::new(updates),
        })
    }

    /// The next update within `wait`, or `None`: none came, or the session
    /// thread has stopped.
    pub fn next_update(&self, wait: Duration) -> Option<Update> {
        let updates = self.updates.lock().ok()?;
        updates.recv_timeout(wait).ok()
    }

    /// Start a feed of `portions`; blocks until the feeder answers, or it is
    /// known that no answer will come.
    pub fn feed(&self, portions: u8) -> Written {
        self.write(Write::Feed { portions })
    }

    /// Stop the feed the feeder numbered `feed`, if it still runs.
    pub fn cancel_feed(&self, feed: u32) -> Written {
        self.write(Write::CancelFeed { feed })
    }

    /// Lock or unlock the panel.
    pub fn set_lock(&self, locked: bool) -> Written {
        self.write(Write::SetLock { locked })
    }

    /// Record a fresh desiccant pack today.
    pub fn reset_desiccant(&self) -> Written {
        self.write(Write::ResetDesiccant)
    }

    pub fn play(&self, tune: Tune) -> Written {
        self.write(Write::Play { tune })
    }

    /// Change the settings `changes` names, on the settings last read.
    pub fn put_settings(&self, changes: FeederSettingsPatch) -> Written {
        self.write(Write::Settings(changes))
    }

    /// Turn the plan in `slot` on or off, on the plans last read.
    pub fn set_plan_enabled(&self, slot: u8, enabled: bool) -> Written {
        self.write(Write::PlanEnabled { slot, enabled })
    }

    /// Store the weight of one portion.
    pub fn set_portion_weight(&self, mg: u32) -> Written {
        self.write(Write::PortionWeight { mg })
    }

    /// Read the status now, rather than at the next turn.
    pub fn refresh(&self) {
        let _ = self.commands.send(Command::Refresh);
    }

    fn write(&self, write: Write) -> Written {
        let (reply, answer) = oneshot::channel();
        let queued = Instant::now();
        let command = Command::Write {
            write,
            reply,
            queued,
        };
        if self.commands.send(command).is_err() {
            return Written::not_sent(NotSent::Stopped);
        }
        // The session answers every write it takes, uncertain when it ends
        // first, so a dropped reply is a session that ended mid-write.
        answer
            .blocking_recv()
            .unwrap_or_else(|_| Written::uncertain())
    }
}

/// Where a direct feed of this client's whose end is not known stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// It went out, and its answer is awaited.
    Sending,
    /// Its answer never came: it may have run.
    Lost,
}

/// A direct feed of this client's whose end is not known, kept (in the
/// integration's store too) from just before it went out, as the app pins
/// one: a restart before its answer finds it lost, so its `Fed` is never
/// taken for another lost feed's. Its `Fed` comes after the event
/// `pending.after` names. Once reported still unknown
/// (`pending.reported`) it no longer holds new feeds back, and waits for
/// its `Fed` until [`KEEP_MS`].
#[derive(Clone, Debug)]
struct Uncertain {
    pending: Pending,
    sent: Instant,
    stage: Stage,
    /// How many feed numbers had been kept when it went out: every one
    /// from here on stays while it is open.
    answered_at: usize,
}

/// Whether a feed sent at `sent_ms` has waited as long as one is kept.
pub(crate) fn forgotten(sent_ms: i64) -> bool {
    now_ms().saturating_sub(sent_ms) >= KEEP_MS
}

/// What outlives one session: where the history was read to, what it said,
/// and the feeds still to settle.
#[derive(Debug, Default)]
struct Memory {
    cursor: Option<Cursor>,
    facts: Facts,
    calibration: Option<CalibrationView>,
    /// Feed numbers this client has the end of, newest last: a `Fed` for
    /// one of them settles nothing.
    answered: VecDeque<u32>,
    /// How many numbers `answered` was ever given, so the oldest it holds
    /// is number `answered_total - answered.len()` of them.
    answered_total: usize,
    uncertain: Vec<Uncertain>,
    /// The alert tables read so far; `None` until the first session.
    alerts: Option<Tracker>,
}

impl Memory {
    /// What the last run left: each feed, lost (one still sending when it
    /// stopped never had its answer). Which of its numbers came after which
    /// feed is not kept, so every one stays while any of its feeds is open.
    fn carried(carried: Carried) -> Self {
        let uncertain = carried
            .pending
            .into_iter()
            .map(|pending| Uncertain {
                sent: sent_at(pending.sent_ms),
                stage: Stage::Lost,
                answered_at: 0,
                pending,
            })
            .collect();
        Self {
            answered_total: carried.answered.len(),
            answered: carried.answered.into(),
            uncertain,
            ..Self::default()
        }
    }

    /// Keep `feed` among the numbers this client has the end of. Those no
    /// open uncertain feed needs go; any uncertain feed whose numbers would
    /// not all fit is forgotten, and returned.
    fn answered(&mut self, feed: u32) -> Vec<Uncertain> {
        self.answered.push_back(feed);
        self.answered_total += 1;

        let mut forgotten = Vec::new();
        loop {
            let oldest = self
                .uncertain
                .iter()
                .map(|uncertain| uncertain.answered_at)
                .min();
            let from = oldest
                .unwrap_or(self.answered_total)
                .saturating_sub(ANSWERED_TAIL);
            let first = self.answered_total - self.answered.len();
            for _ in first..from {
                self.answered.pop_front();
            }
            if self.answered.len() <= ANSWERED_MOST {
                return forgotten;
            }
            let Some(index) = self
                .uncertain
                .iter()
                .enumerate()
                .filter(|(_, uncertain)| uncertain.stage == Stage::Lost)
                .min_by_key(|(_, uncertain)| uncertain.answered_at)
                .map(|(index, _)| index)
            else {
                // Only feeds still waiting on their answer, seconds old:
                // the oldest numbers go.
                while self.answered.len() > ANSWERED_MOST {
                    self.answered.pop_front();
                }
                return forgotten;
            };
            forgotten.push(self.uncertain.remove(index));
        }
    }

    /// The lost feeds the history read settles, by index, with their
    /// `Fed`s, by the rule the app shares ([`settle`]). Every feed kept
    /// takes part, one still waiting on its answer too, so no lost feed
    /// takes a `Fed` that may be its; only a lost one is settled. `client`
    /// is this client's key's first 4 bytes.
    fn settlements(&self, client: [u8; 4]) -> Vec<(usize, Feed)> {
        let answered: Vec<u32> = self.answered.iter().copied().collect();
        let feeds: Vec<Lost<'_>> = self
            .uncertain
            .iter()
            .map(|uncertain| Lost {
                portions: uncertain.pending.portions,
                // A feed goes out only once `open` has read a status, so a
                // mark of `None` is a history that held no event then, not
                // one not yet read: any `Fed` is after it.
                after: uncertain.pending.after.map_or(After::Start, After::Event),
                excluded: &answered,
            })
            .collect();

        settle::settled(&feeds, &self.facts.feeds, client)
            .into_iter()
            .zip(&self.uncertain)
            .enumerate()
            .filter(|(_, (_, uncertain))| uncertain.stage == Stage::Lost)
            .filter_map(|(index, (fed, _))| fed.map(|fed| (index, *fed)))
            .collect()
    }

    /// Take out the lost feeds past [`KEEP_MS`]. One still waiting on its
    /// answer is never forgotten, whatever the clock says: a wall clock
    /// that jumps ahead must not let go of a feed that may yet run.
    fn past_keep(&mut self) -> Vec<Uncertain> {
        let (gone, open) = std::mem::take(&mut self.uncertain)
            .into_iter()
            .partition(|uncertain| {
                uncertain.stage == Stage::Lost && forgotten(uncertain.pending.sent_ms)
            });
        self.uncertain = open;
        gone
    }

    /// The uncertain feed whose pending has `id`.
    fn position(&self, id: &str) -> Option<usize> {
        self.uncertain
            .iter()
            .position(|uncertain| uncertain.pending.id == id)
    }

    fn fed(&mut self, feed: Feed) {
        fed(&mut self.facts, feed);
    }

    /// Whether a feed of this client's has no end yet and is not reported
    /// still unknown: one still waiting on its answer, which may yet be
    /// lost, or a lost one, carried from the last run included. A new feed
    /// waits for it.
    fn holds_feeds(&self) -> bool {
        self.uncertain
            .iter()
            .any(|uncertain| !uncertain.pending.reported)
    }
}

/// Add a feed the history gave to the day's facts.
pub(crate) fn fed(facts: &mut Facts, feed: Feed) {
    // A backlog replays from the cursor, so a feed is seen once; the check
    // keeps a replay after a lost cursor from doubling it.
    if facts.feeds.last().is_some_and(|last| last.seq >= feed.seq) {
        return;
    }
    if facts.feeds.len() == FEEDS_KEPT {
        facts.feeds.remove(0);
    }
    facts.feeds.push(feed);
}

/// Connect, serve until the session ends, and go again, until the handle is
/// dropped, the key is revoked, or the feeder runs another build.
async fn run(
    key: ClientKey,
    target: Target,
    clocks: Clocks,
    carried: Carried,
    mut commands: mpsc::UnboundedReceiver<Command>,
    updates: std_mpsc::Sender<Update>,
) {
    let mut fingerprint = [0; 4];
    fingerprint.copy_from_slice(&key.public()[..4]);
    let mut memory = Memory::carried(carried);
    memory.alerts = Some(Tracker::new(fingerprint));
    // A lost feed carried from the last run that still holds feeds back is
    // announced again, so the integration's repair says so.
    for uncertain in &memory.uncertain {
        if !uncertain.pending.reported
            && updates
                .send(Update::pending(uncertain.pending.clone()))
                .is_err()
        {
            return;
        }
    }

    loop {
        let Some(opened) = refusing(open(&key, &target, clocks.session), &mut commands).await
        else {
            return;
        };
        let ended = match opened {
            Ok(connection) => {
                let session = Session::new(
                    connection,
                    fingerprint,
                    &mut memory,
                    &updates,
                    clocks.settle,
                );
                session.serve(&mut commands).await
            }
            Err(Error::Connect(wellfed_connect::Error::UnknownKey)) => Ended::Revoked,
            Err(error) => Ended::Lost(error.to_string()),
        };
        let reason = match ended {
            Ended::Dropped => return,
            Ended::Revoked => {
                stopped(Update::revoked(), &mut commands, &updates).await;
                return;
            }
            Ended::Lost(reason) => reason,
        };
        if updates.send(Update::disconnected(reason)).is_err() {
            return;
        }
        if !pause(clocks.retry, &mut commands).await {
            return;
        }
    }
}

/// Wait `retry` before the next session, refusing every write meanwhile;
/// `false` once the handle is dropped. A write is refused at once, never
/// held for the next session: a feed sent minutes after its press is not
/// the feed the owner asked for.
pub(crate) async fn pause(
    retry: Duration,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) -> bool {
    let pause = tokio::time::sleep(retry);
    tokio::pin!(pause);
    loop {
        tokio::select! {
            () = &mut pause => return true,
            command = commands.recv() => match command {
                None => return false,
                Some(Command::Write { reply, .. }) => {
                    let _ = reply.send(Written::not_sent(NotSent::Disconnected));
                }
                Some(Command::Refresh) => {}
            },
        }
    }
}

/// The session cannot go on: say why once, then refuse every write until
/// the handle is dropped. A revoked key needs a new invite; a refused
/// token, a new login.
pub(crate) async fn stopped(
    why: Update,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    updates: &std_mpsc::Sender<Update>,
) {
    if updates.send(why).is_err() {
        return;
    }
    while let Some(command) = commands.recv().await {
        if let Command::Write { reply, .. } = command {
            let _ = reply.send(Written::not_sent(NotSent::Stopped));
        }
    }
}

async fn open(key: &ClientKey, target: &Target, timing: Timing) -> Result<Connection, Error> {
    let address: SocketAddr = locate(&target.identity, target.address.as_deref()).await?;
    let ephemeral = random()?;
    Ok(Connection::open_with(key.keypair(), target.identity, address, ephemeral, timing).await?)
}

enum Ended {
    /// The handle is gone.
    Dropped,
    Revoked,
    Lost(String),
}

/// A read the session asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Read {
    Status,
    Settings,
    Plans,
    Time,
    Calibration,
    Alerts,
}

impl Read {
    const fn op(self) -> Op {
        match self {
            Self::Status => Op::Status,
            Self::Settings => Op::GetSettings,
            Self::Plans => Op::GetPlans,
            Self::Time => Op::GetTime,
            Self::Calibration => Op::GetCalibration,
            Self::Alerts => Op::GetAlerts,
        }
    }
}

/// What an outstanding request was for.
#[derive(Debug)]
enum Outstanding {
    Write {
        reply: oneshot::Sender<Written>,
        kind: Kind,
        /// What went out, to put on the facts once it lands.
        op: Op,
        /// For a feed, its pending's id among the uncertain feeds.
        feed: Option<String>,
    },
    Read(Read),
}

struct Session<'a> {
    connection: Connection,
    fingerprint: [u8; 4],
    role: ClientRole,
    capabilities: Capabilities,
    memory: &'a mut Memory,
    updates: &'a std_mpsc::Sender<Update>,
    /// How long a lost feed holds new feeds back: [`Clocks::settle`].
    settle: Duration,
    /// The replay at connect, until its newest event has come.
    backlog: Option<Backlog>,
    pending: HashMap<RequestId, Outstanding>,
}

/// The session's own failure, or the handle gone.
type Step = Result<(), Ended>;

impl<'a> Session<'a> {
    fn new(
        connection: Connection,
        fingerprint: [u8; 4],
        memory: &'a mut Memory,
        updates: &'a std_mpsc::Sender<Update>,
        settle: Duration,
    ) -> Self {
        Self {
            role: connection.role(),
            capabilities: Capabilities::of(connection.kind()),
            connection,
            fingerprint,
            memory,
            updates,
            settle,
            backlog: None,
            pending: HashMap::new(),
        }
    }

    async fn serve(mut self, commands: &mut mpsc::UnboundedReceiver<Command>) -> Ended {
        let ended = match refusing(self.open(), commands).await {
            None => Ended::Dropped,
            Some(Ok(())) => self.follow(commands).await,
            Some(Err(ended)) => ended,
        };

        // Each write still outstanding went out and was never answered.
        let outstanding: Vec<Outstanding> =
            self.pending.drain().map(|(_, pending)| pending).collect();
        for pending in outstanding {
            if let Outstanding::Write {
                reply, kind, feed, ..
            } = pending
            {
                let _ = reply.send(self.unanswered(kind, feed.as_deref()));
            }
        }
        ended
    }

    /// Announce the session, read the facts and subscribe.
    async fn open(&mut self) -> Step {
        let connected = Update::connected(
            self.connection.build(),
            self.role,
            self.capabilities,
            self.connection.skew(),
        );
        self.send(connected)?;
        // A pre-revision build takes only the frozen update path: nothing
        // this session reads or writes. Its connect update says so; the
        // session ends and tries again later, by which time the app may
        // have updated it.
        if self.connection.skew() == Skew::Legacy {
            return Err(Ended::Lost(wellfed_connect::Error::Legacy.to_string()));
        }

        let status = match self
            .connection
            .ask(random().map_err(lost)?, &Op::Status)
            .await
        {
            Ok(Answer::Status(status)) => status,
            Ok(other) => return Err(Ended::Lost(format!("a status read answered {other:?}"))),
            Err(error) => return Err(lost(error)),
        };
        let newest = status.last_event;
        self.memory.facts.status = Some(status);
        for read in [Read::Settings, Read::Plans, Read::Time, Read::Calibration] {
            if self.serves(read) {
                self.read(read)?;
            }
        }
        self.alerts_due()?;

        // A cursor from another history is a gap: what happened meanwhile
        // is unknown, and the new history's backlog is read from its start.
        let epoch = self.connection.epoch();
        let from = match self.memory.cursor {
            Some(held) if held.epoch == epoch => Some(held),
            Some(_) => {
                self.memory.facts.feeds.clear();
                self.send(Update::gap(None))?;
                None
            }
            None => None,
        };
        self.connection.subscribe(from, false).await.map_err(lost)?;
        let backlog = Backlog { newest };
        if backlog.read(from.map(|cursor| cursor.seq)) {
            self.end_backlog()
        } else {
            self.backlog = Some(backlog);
            Ok(())
        }
    }

    async fn follow(&mut self, commands: &mut mpsc::UnboundedReceiver<Command>) -> Ended {
        loop {
            // A lost feed is reported when its time comes, not at the next
            // message: one whose answer is held back may bring none.
            let due = self.next_report();
            let step = tokio::select! {
                () = until(due) => Ok(()),
                command = commands.recv() => match command {
                    None => return Ended::Dropped,
                    Some(Command::Write { write, reply, queued }) => match stale(queued, reply) {
                        Some(reply) => self.write(write, reply),
                        None => Ok(()),
                    },
                    Some(Command::Refresh) => self.read(Read::Status),
                },
                received = self.connection.receive(TURN) => match received {
                    Ok(Received::Answer { id, answer }) => self.answered(id, answer),
                    Ok(Received::Pushed(pushed)) => self.pushed(&pushed),
                    Err(wellfed_connect::Error::Timeout) => self.read(Read::Status),
                    Err(wellfed_connect::Error::UnknownKey) => Err(Ended::Revoked),
                    Err(error) => Err(lost(error)),
                },
            };
            if let Err(ended) = step.and_then(|()| self.expire()) {
                return ended;
            }
        }
    }

    /// When the next lost feed not yet reported is due to be.
    fn next_report(&self) -> Option<Instant> {
        self.memory
            .uncertain
            .iter()
            .filter(|uncertain| uncertain.stage == Stage::Lost && !uncertain.pending.reported)
            .map(|uncertain| uncertain.sent + self.settle)
            .min()
    }

    fn send(&self, update: Update) -> Step {
        self.updates.send(update).map_err(|_| Ended::Dropped)
    }

    /// The state as it now stands, once the backlog is read.
    fn state(&self) -> Step {
        if self.backlog.is_some() {
            return Ok(());
        }
        let zone = TimeZone::system();
        match State::of(&self.memory.facts, self.memory.calibration.as_ref(), &zone) {
            Some(state) => self.send(Update::state(state)),
            None => Ok(()),
        }
    }

    /// Ask for `read`, unless the same read is already out.
    fn read(&mut self, read: Read) -> Step {
        if self
            .pending
            .values()
            .any(|pending| matches!(pending, Outstanding::Read(out) if *out == read))
        {
            return Ok(());
        }
        let op = read.op();
        let id = random().map_err(lost)?;
        self.connection.request(id, &op).map_err(lost)?;
        self.pending.insert(id, Outstanding::Read(read));
        Ok(())
    }

    /// Whether the device's kind serves `read`, by its op's own kind
    /// ([`Op::kind`]): one source of truth with the writes.
    fn serves(&self, read: Read) -> bool {
        read.op()
            .kind()
            .is_none_or(|kind| kind == self.connection.kind())
    }

    fn write(&mut self, write: Write, reply: oneshot::Sender<Written>) -> Step {
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
        let built = writes::build(&write, &read).and_then(|(op, kind)| {
            writes::gate(self.role, &op)?;
            writes::served(&op, self.connection.kind())?;
            let busy = matches!(kind, Kind::Store { store, .. } if self.store_busy(store));
            wellfed_formats::require(!busy, || NotSent::Busy)?;
            let held = matches!(kind, Kind::Feed { .. }) && self.feed_unsettled();
            wellfed_formats::require(!held, || NotSent::Held)?;
            Ok((op, kind))
        });
        let (op, kind) = match built {
            Ok(built) => built,
            Err(why) => {
                let _ = reply.send(Written::not_sent(why));
                return Ok(());
            }
        };

        let id = random().map_err(lost)?;
        let feed = match kind {
            Kind::Feed { portions } => Some(self.sending(portions)),
            Kind::Once | Kind::Store { .. } => None,
        };
        match self.connection.request(id, &op) {
            Ok(()) => {
                let write = Outstanding::Write {
                    reply,
                    kind,
                    op,
                    feed,
                };
                self.pending.insert(id, write);
                Ok(())
            }
            // `Connection::request` only queues: an error from it means
            // nothing of this request was queued, whether or not the session
            // survives ([`wellfed_connect::Error::keeps_session`]), as `ask`
            // reads the same errors `Unsent`. So a feed refused here never
            // went and is cleared. A feed whose request was queued may have
            // run; it is lost when its answer never comes, or when the
            // session ends with it outstanding ([`Session::serve`]).
            //
            // Refused before any byte went out, on a session that goes on:
            // the writes still out stay out.
            Err(error) if error.keeps_session() => {
                self.clear(feed.as_deref());
                let why = match error {
                    wellfed_connect::Error::Legacy => NotSent::Unsupported,
                    _ => NotSent::Invalid,
                };
                let _ = reply.send(Written::not_sent(why));
                Ok(())
            }
            Err(error) => {
                self.clear(feed.as_deref());
                let _ = reply.send(Written::not_sent(NotSent::Disconnected));
                Err(lost(error))
            }
        }
    }

    /// Keep a feed of `portions` about to go out, and announce it before
    /// it goes, so the integration's store holds it if Home Assistant stops
    /// before its answer. Its pending's id.
    fn sending(&mut self, portions: u8) -> String {
        let pending = Pending {
            id: uuid::Uuid::new_v4().to_string(),
            portions,
            sent_ms: now_ms(),
            after: self.newest(),
            reported: false,
        };
        let id = pending.id.clone();
        // A handle gone has no one to tell.
        let _ = self.updates.send(Update::sending(pending.clone()));
        self.memory.uncertain.push(Uncertain {
            pending,
            sent: Instant::now(),
            stage: Stage::Sending,
            answered_at: self.memory.answered_total,
        });
        id
    }

    /// The feed kept as `feed` has its answer, or never went: no longer
    /// kept.
    fn clear(&mut self, feed: Option<&str>) {
        let Some(index) = feed.and_then(|feed| self.memory.position(feed)) else {
            return;
        };
        let uncertain = self.memory.uncertain.remove(index);
        let _ = self.updates.send(Update::cleared(uncertain.pending));
    }

    /// Whether a feed this client sent has no end yet: uncertain, or
    /// still waiting on its answer, which may yet be lost.
    fn feed_unsettled(&self) -> bool {
        self.memory.holds_feeds()
    }

    fn store_busy(&self, store: Store) -> bool {
        self.pending.values().any(|pending| {
            matches!(pending, Outstanding::Write { kind: Kind::Store { store: out, .. }, .. } if *out == store)
        })
    }

    fn answered(&mut self, id: RequestId, answer: Option<Answer>) -> Step {
        match self.pending.remove(&id) {
            Some(Outstanding::Write {
                reply,
                kind,
                op,
                feed,
            }) => {
                let written = self.written(kind, &op, feed.as_deref(), answer.as_ref())?;
                let _ = reply.send(written);
                // A feed that ends here no longer keeps a lost one from a
                // `Fed` it could have claimed.
                self.settle()
            }
            Some(Outstanding::Read(read)) => self.fact(read, answer),
            None => Ok(()),
        }
    }

    /// How the write of `kind` ended, by its answer; `feed` names a feed's
    /// pending.
    fn written(
        &mut self,
        kind: Kind,
        op: &Op,
        feed: Option<&str>,
        answer: Option<&Answer>,
    ) -> Result<Written, Ended> {
        let Some(answer) = readable(answer) else {
            return Ok(self.unanswered(kind, feed));
        };
        match kind {
            Kind::Once => Ok(once(answer)),
            Kind::Feed { .. } => {
                let written = once(answer);
                // An answer that names no end may follow a feed that ran.
                if written.result == WriteResult::Uncertain {
                    return Ok(self.unanswered(kind, feed));
                }
                // Its number is kept before the feed is let go, so a
                // restart in between leaves it lost, never its `Fed` free.
                if let Answer::Feeding { feed } = answer {
                    self.answer(*feed);
                }
                self.clear(feed);
                Ok(written)
            }
            Kind::Store { store, base } => {
                // The store is read again whatever the answer, so the
                // state shows what it now holds.
                self.read(store_read(store))?;
                Ok(match writes::landed(Some(answer), base) {
                    Landed::Done => {
                        self.landed(op);
                        self.state()?;
                        Written::done()
                    }
                    Landed::Stale | Landed::Refused(_) => once(answer),
                    Landed::Unknown => self.unanswered(kind, None),
                })
            }
        }
    }

    /// Put a store write that landed on the facts at its next generation,
    /// as the feeder now holds it: the next change is built on it at once,
    /// rather than on a generation the re-read has yet to bring.
    fn landed(&mut self, op: &Op) {
        let facts = &mut self.memory.facts;
        match op {
            Op::PutSettings { base, changes } => {
                if let Some(settings) = facts
                    .settings
                    .as_mut()
                    .filter(|held| held.generation == Some(*base))
                {
                    settings.apply(*changes);
                    settings.generation = Some(next(*base));
                    if let Some(status) = facts.status.as_mut().and_then(Status::feeder_mut) {
                        status.settings_generation = settings.generation;
                    }
                }
            }
            Op::PutPlans { base, plans } => {
                if let Some(held) = facts.plans.as_mut().filter(|held| held.generation == *base) {
                    held.plans = *plans;
                    held.generation = next(*base);
                }
            }
            Op::PutCalibration { base, calibration } => {
                let view = self.memory.calibration.as_mut();
                if let Some(view) = view.filter(|view| view.generation == *base) {
                    view.calibration = *calibration;
                    view.generation = next(*base);
                }
            }
            _ => {}
        }
    }

    /// The newest event this session knows the feeder holds: the cursor,
    /// or the newest the status named, whichever is later (the backlog may
    /// still be on its way).
    fn newest(&self) -> Option<u64> {
        let cursor = self.memory.cursor.map(|cursor| cursor.seq);
        let status = self
            .memory
            .facts
            .status
            .as_ref()
            .and_then(|status| status.last_event);
        cursor.max(status)
    }

    /// A write of `kind` went out and its answer never came: uncertain,
    /// and a feed, kept as `feed` since it went out, is lost until its `Fed`
    /// settles it.
    fn unanswered(&mut self, kind: Kind, feed: Option<&str>) -> Written {
        let Kind::Feed { .. } = kind else {
            return Written::uncertain();
        };
        let Some(index) = feed.and_then(|feed| self.memory.position(feed)) else {
            return Written::uncertain();
        };
        let uncertain = &mut self.memory.uncertain[index];
        uncertain.stage = Stage::Lost;
        let pending = uncertain.pending.clone();
        // Announced on the updates before anything can settle it, so the
        // integration's repair says it may have run; a handle gone has no
        // one to tell.
        let _ = self.updates.send(Update::pending(pending.clone()));
        Written::uncertain_feed(pending)
    }

    fn fact(&mut self, read: Read, answer: Option<Answer>) -> Step {
        let facts = &mut self.memory.facts;
        match (read, answer) {
            (Read::Status, Some(Answer::Status(status))) => {
                facts.status = Some(status);
                self.alerts_due()?;
            }
            (Read::Alerts, Some(Answer::Alerts(view))) => return self.alerts_read(&view),
            (Read::Settings, Some(Answer::Settings(Settings::Feeder(settings)))) => {
                facts.settings = Some(settings);
            }
            (Read::Plans, Some(Answer::Plans(plans))) => facts.plans = Some(plans),
            (Read::Time, Some(Answer::Time(time))) => facts.time = Some(time),
            (Read::Calibration, Some(Answer::Calibration(view))) => {
                self.memory.calibration = Some(view);
            }
            // A read that failed is asked again at the next turn or event.
            _ => return Ok(()),
        }
        self.state()
    }

    fn pushed(&mut self, pushed: &Pushed) -> Step {
        let replayed = match pushed {
            Pushed::Event(event) => self
                .backlog
                .is_some_and(|backlog| backlog.newest.is_some_and(|newest| event.seq <= newest)),
            Pushed::Gap(_) | Pushed::Unknown(_) => self.backlog.is_some(),
            Pushed::Sample(_) | Pushed::Log(_) => return Ok(()),
        };
        self.memory.cursor = self.connection.cursor();

        if let Some(Entry::Feed(feed)) = Entry::from_pushed(pushed) {
            self.memory.fed(feed);
            self.settle()?;
        }
        match pushed {
            Pushed::Event(event) => {
                self.send(Update::event(event, replayed))?;
                if !replayed {
                    self.news(&event.body)?;
                }
            }
            Pushed::Gap(gap) => self.send(Update::gap(Some(gap)))?,
            // A newer feeder build's event, which this integration cannot
            // show: it only moves the cursor and may end the backlog.
            Pushed::Unknown(_) | Pushed::Sample(_) | Pushed::Log(_) => {}
        }

        match self.backlog {
            Some(backlog) if backlog.ends_at(pushed) => self.end_backlog(),
            Some(_) => Ok(()),
            None => self.state(),
        }
    }

    /// Read the alert table when the status shows it moved since the last
    /// one applied; otherwise let an alert held absent past its hold end.
    fn alerts_due(&mut self) -> Step {
        let Some((cursor, clock)) = self.alerts_cursor() else {
            return Ok(());
        };
        let Some(tracker) = self.memory.alerts.as_mut() else {
            return Ok(());
        };
        if tracker.needs_read(cursor) {
            return self.read(Read::Alerts);
        }
        match tracker.unchanged(clock) {
            Some(update) => self.send(update),
            None => Ok(()),
        }
    }

    /// The table as read, under the cursor the status gave with the
    /// table's own generation.
    fn alerts_read(&mut self, view: &AlertsView) -> Step {
        let Some((cursor, clock)) = self.alerts_cursor() else {
            return Ok(());
        };
        let cursor = AlertCursor {
            epoch: view.epoch,
            generation: view.generation,
            ..cursor
        };
        let Some(tracker) = self.memory.alerts.as_mut() else {
            return Ok(());
        };
        match tracker.table(cursor, view, clock) {
            Some(update) => self.send(update),
            None => Ok(()),
        }
    }

    /// The table's cursor and the feeder's clock, from the status.
    fn alerts_cursor(&self) -> Option<(AlertCursor, FeederClock)> {
        let status = self.memory.facts.status.as_ref()?;
        let cursor = AlertCursor {
            epoch: self.connection.epoch(),
            boot: status.boot_count,
            generation: status.alerts_generation,
        };
        let clock = match status.clock {
            Some(_) => FeederClock::Known,
            None => FeederClock::Unknown,
        };
        Some((cursor, clock))
    }

    /// An event as it happened: any may move the status, and some the
    /// settings, the plans or the time, so those are read again.
    fn news(&mut self, body: &EventBody) -> Step {
        match body {
            EventBody::Feeder(FeederEvent::Plans { .. })
            | EventBody::Feeder(FeederEvent::Fed { .. })
            | EventBody::Feeder(FeederEvent::Missed { .. })
                if self.serves(Read::Plans) =>
            {
                self.read(Read::Plans)?;
            }
            EventBody::Feeder(FeederEvent::Settings { .. }) => self.read(Read::Settings)?,
            EventBody::Core(CoreEvent::ClockSync { .. }) if self.serves(Read::Time) => {
                self.read(Read::Time)?
            }
            _ => {}
        }
        self.read(Read::Status)
    }

    /// The backlog is read: the state is whole, and an uncertain feed with
    /// no `Fed` in it, on a feeder that is not feeding now, is still
    /// unknown. It no longer holds feeds back, and its `Fed` still settles
    /// it if it turns up later.
    fn end_backlog(&mut self) -> Step {
        self.backlog = None;
        let feeding = self.memory.facts.status.as_ref().is_some_and(feeding_now);
        if !feeding {
            self.report(|_| true)?;
        }
        self.state()
    }

    /// Report each lost feed `due` picks, not yet reported, as still
    /// unknown. One still waiting on its answer is not lost yet.
    fn report(&mut self, due: impl Fn(&Uncertain) -> bool) -> Step {
        let mut reported = Vec::new();
        for uncertain in &mut self.memory.uncertain {
            if uncertain.stage == Stage::Lost && !uncertain.pending.reported && due(uncertain) {
                uncertain.pending.reported = true;
                reported.push(uncertain.pending.clone());
            }
        }
        for pending in reported {
            self.send(Update::still_unknown(pending))?;
        }
        Ok(())
    }

    /// Settle each lost feed the history read gives the one `Fed` of
    /// ([`Memory::settlements`]).
    fn settle(&mut self) -> Step {
        let settled = self.memory.settlements(self.fingerprint);
        let mut ended = Vec::with_capacity(settled.len());
        for (index, fed) in settled.into_iter().rev() {
            ended.push((self.memory.uncertain.remove(index), fed));
        }
        for (uncertain, fed) in ended.into_iter().rev() {
            self.answer(fed.feed);
            self.send(Update::settled(api::Feed::from(&fed), uncertain.pending))?;
        }
        Ok(())
    }

    /// This client has the end of the feed numbered `feed`: a `Fed` for it
    /// settles nothing, in this session or, through the integration's
    /// store, the next. Announced before the write's reply or the
    /// settlement, so the integration has it first; a handle gone has no
    /// one to tell.
    fn answer(&mut self, feed: u32) {
        for uncertain in self.memory.answered(feed) {
            self.forget(uncertain.pending);
        }
        let kept = self.memory.answered.iter().copied().collect();
        let _ = self.updates.send(Update::answered(kept));
    }

    /// Stop waiting on a lost feed: still unknown, and reported so first if
    /// it was not. A handle gone has no one to tell.
    fn forget(&self, pending: Pending) {
        if !pending.reported {
            let _ = self.updates.send(Update::still_unknown(pending.clone()));
        }
        let _ = self.updates.send(Update::forgotten(pending));
    }

    /// Lost feeds past [`Clocks::settle`] with no `Fed` are still unknown;
    /// those past [`KEEP_MS`] are forgotten.
    fn expire(&mut self) -> Step {
        for uncertain in self.memory.past_keep() {
            self.forget(uncertain.pending);
        }

        let (now, settle) = (Instant::now(), self.settle);
        self.report(|uncertain| now.duration_since(uncertain.sent) >= settle)
    }
}

/// Wait until `due`, or forever when there is none.
async fn until(due: Option<Instant>) {
    match due {
        Some(due) => tokio::time::sleep_until(due.into()).await,
        None => std::future::pending().await,
    }
}

/// Whether the feeder reports a feed running now.
pub(crate) fn feeding_now(status: &Status) -> bool {
    status
        .feeder()
        .is_some_and(|feeder| feeder.readiness == Readiness::Blocked(Blocked::Feeding))
}

/// The generation a store moves to from `base`.
pub(crate) const fn next(base: Generation) -> Generation {
    Generation(base.0.wrapping_add(1))
}

const fn store_read(store: Store) -> Read {
    match store {
        Store::Settings => Read::Settings,
        Store::Plans => Read::Plans,
        Store::Calibration => Read::Calibration,
    }
}

/// The answer a write is judged by. One this build cannot read (a newer
/// feeder's) says no more than none: the write may have run, so it takes
/// the unanswered path, and a feed so answered waits for its `Fed` event.
fn readable(answer: Option<&Answer>) -> Option<&Answer> {
    answer.filter(|answer| !matches!(answer, Answer::Unreadable))
}

/// How a write whose answer is its whole result ended.
fn once(answer: &Answer) -> Written {
    match answer {
        Answer::Feeding { feed } => Written::feeding(*feed),
        Answer::Done { .. } => Written::done(),
        Answer::Refused(refusal) => Written::refused(refusal),
        // A write answered with anything else ran or not as the feeder
        // decided; this client cannot tell which.
        _ => Written::uncertain(),
    }
}

fn lost(error: impl std::fmt::Display) -> Ended {
    Ended::Lost(error.to_string())
}

#[cfg(test)]
mod tests {
    use wellfed_model::Source;

    use super::*;

    /// An answer a newer feeder sent that this build cannot read is no
    /// answer: the write is judged unanswered, never done or refused.
    #[test]
    fn an_unreadable_answer_is_judged_as_none() {
        assert_eq!(readable(Some(&Answer::Unreadable)), None);
        assert_eq!(readable(None), None);
        let done = Answer::Done { generation: None };
        assert_eq!(readable(Some(&done)), Some(&done));
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    /// A lost feed handed over from the last run holds a new feed as one
    /// lost in this run does, until the session settles it.
    #[test]
    fn a_lost_feed_carried_from_the_last_run_holds_new_feeds() {
        assert!(!Memory::carried(Carried::default()).holds_feeds());

        let carried = Carried {
            pending: vec![lost("a-lost-feed", now_ms())],
            answered: Vec::new(),
        };
        assert!(Memory::carried(carried).holds_feeds());
    }

    /// A write that waited past [`FRESH`] is answered unsent; a fresh one
    /// goes on to the session.
    #[test]
    fn a_write_that_waited_too_long_is_not_sent() {
        let (reply, answer) = oneshot::channel();
        let long_ago = Instant::now() - FRESH - Duration::from_secs(1);
        assert!(stale(long_ago, reply).is_none());
        assert_eq!(
            answer.blocking_recv().unwrap(),
            Written::not_sent(NotSent::Disconnected)
        );

        let (reply, _answer) = oneshot::channel();
        assert!(stale(Instant::now(), reply).is_some());
    }

    /// A write that arrives while a session connects is refused at once,
    /// never held until the connect ends.
    #[test]
    fn a_write_while_connecting_is_refused_at_once() {
        runtime().block_on(async {
            let (commands, mut inbox) = mpsc::unbounded_channel();
            let (reply, answer) = oneshot::channel();
            commands
                .send(Command::Write {
                    write: Write::Feed { portions: 1 },
                    reply,
                    queued: Instant::now(),
                })
                .unwrap();

            let connect = tokio::time::sleep(Duration::from_millis(200));
            assert_eq!(refusing(connect, &mut inbox).await, Some(()));
            assert_eq!(
                answer.await.unwrap(),
                Written::not_sent(NotSent::Disconnected)
            );

            drop(commands);
            let forever = std::future::pending::<()>();
            assert_eq!(refusing(forever, &mut inbox).await, None);
        });
    }

    fn lost(id: &str, sent_ms: i64) -> Pending {
        Pending {
            id: id.into(),
            portions: 1,
            sent_ms,
            after: Some(4),
            reported: false,
        }
    }

    /// A lost feed carried from a run that already reported it still
    /// unknown waits for its `Fed`, but holds no feed back, whenever it
    /// went out: the report is kept, not judged again by the clock.
    #[test]
    fn a_carried_feed_reported_before_holds_no_feed_back() {
        let reported = Pending {
            reported: true,
            ..lost("reported", now_ms() - 1000)
        };
        let carried = Carried {
            pending: vec![reported],
            answered: Vec::new(),
        };
        let memory = Memory::carried(carried);
        assert_eq!(memory.uncertain.len(), 1, "still waited on");
        assert!(!memory.holds_feeds());
    }

    /// One of this client's `Fed`s of 1 portion, ended at event `seq`.
    fn ours(seq: u64, feed: u32) -> Feed {
        Feed {
            epoch: 1,
            seq,
            feed,
            at: None,
            by: Some(Source::Client {
                client: wellfed_model::Hex(US),
            }),
            requested: 1,
            delivered: wellfed_client::history::Delivered::Counted(1),
            outcome: wellfed_model::FeedOutcome::Done,
            reason: wellfed_model::FeedReason::Done,
            hopper: wellfed_model::FoodLevel::Present,
        }
    }

    const US: [u8; 4] = [1, 2, 3, 4];

    fn kept(pending: Pending, stage: Stage) -> Uncertain {
        Uncertain {
            pending,
            sent: Instant::now(),
            stage,
            answered_at: 0,
        }
    }

    /// A feed still waiting on its answer may yet be the lost one's `Fed`'s
    /// owner: the lost one takes nothing while it waits, and is settled
    /// once it is alone.
    #[test]
    fn a_feed_awaiting_its_answer_keeps_a_lost_one_from_its_fed() {
        let mut memory = Memory::default();
        memory.fed(ours(10, 7));
        memory
            .uncertain
            .push(kept(lost("lost", now_ms()), Stage::Lost));
        memory
            .uncertain
            .push(kept(lost("sending", now_ms()), Stage::Sending));
        assert!(memory.settlements(US).is_empty());

        memory.uncertain.pop();
        let settled = memory.settlements(US);
        assert_eq!(settled.len(), 1);
        assert_eq!((settled[0].0, settled[0].1.feed), (0, 7));
    }

    /// A feed still waiting on its answer is never settled by a `Fed`: its
    /// answer says how it ended.
    #[test]
    fn a_feed_awaiting_its_answer_is_not_settled() {
        let mut memory = Memory::default();
        memory.fed(ours(10, 7));
        memory
            .uncertain
            .push(kept(lost("sending", now_ms()), Stage::Sending));
        assert!(memory.settlements(US).is_empty());
        assert!(memory.holds_feeds(), "and it holds new feeds back");
    }

    /// A feed sent before Home Assistant stopped, with no answer, is lost
    /// after the restart: it takes part, so a feed reported unknown before
    /// it never takes its `Fed`.
    #[test]
    fn a_feed_sending_at_a_restart_keeps_its_fed_from_a_reported_one() {
        let older = Pending {
            reported: true,
            ..lost("reported", now_ms() - 60 * 60 * 1000)
        };
        let carried = Carried {
            pending: vec![older, lost("sending", now_ms())],
            answered: Vec::new(),
        };
        let mut memory = Memory::carried(carried);
        memory.fed(ours(10, 7));
        assert!(memory.settlements(US).is_empty());
        assert!(memory.holds_feeds());
    }

    /// Every number answered since an open lost feed went out is kept,
    /// with the ones just before it; with none open, only the last few.
    #[test]
    fn the_numbers_an_open_lost_feed_needs_are_kept() {
        let mut memory = Memory::default();
        for feed in 0..100 {
            assert!(memory.answered(feed).is_empty());
        }
        assert_eq!(memory.answered.len(), ANSWERED_TAIL);
        assert_eq!(memory.answered.front(), Some(&36));

        memory.uncertain.push(Uncertain {
            answered_at: memory.answered_total,
            ..kept(lost("open", now_ms()), Stage::Lost)
        });
        for feed in 100..400 {
            assert!(memory.answered(feed).is_empty());
        }
        assert_eq!(
            memory.answered.front(),
            Some(&36),
            "the tail before it stays"
        );
        assert_eq!(memory.answered.back(), Some(&399));

        memory.uncertain.clear();
        assert!(memory.answered(400).is_empty());
        assert_eq!(memory.answered.len(), ANSWERED_TAIL);
        assert_eq!(memory.answered.back(), Some(&400));
    }

    /// Numbers carried from the last run are all kept while one of its
    /// feeds is open: which came after it is not known.
    #[test]
    fn carried_numbers_stay_while_a_carried_feed_is_open() {
        let carried = Carried {
            pending: vec![lost("open", now_ms())],
            answered: (0..200).collect(),
        };
        let mut memory = Memory::carried(carried);
        assert!(memory.answered(200).is_empty());
        assert_eq!(memory.answered.len(), 201);
        assert_eq!(memory.answered.front(), Some(&0));
    }

    /// A lost feed whose numbers would not all fit is forgotten rather
    /// than have one dropped that its `Fed` could be taken for.
    #[test]
    fn a_lost_feed_whose_numbers_would_not_fit_is_forgotten() {
        let carried = Carried {
            pending: vec![lost("open", now_ms())],
            answered: Vec::new(),
        };
        let mut memory = Memory::carried(carried);
        for feed in 0..u32::try_from(ANSWERED_MOST).unwrap() {
            assert!(memory.answered(feed).is_empty());
        }
        let forgotten = memory.answered(9_999);
        assert_eq!(forgotten.len(), 1);
        assert_eq!(forgotten[0].pending.id, "open");
        assert!(memory.uncertain.is_empty());
        assert_eq!(memory.answered.len(), ANSWERED_TAIL);
    }

    /// After a wall clock jump a week ahead, a lost feed is forgotten, but
    /// a feed still waiting on its answer stays.
    #[test]
    fn a_clock_jump_forgets_no_feed_still_sending() {
        let long_ago = now_ms() - KEEP_MS - 1000;
        let mut memory = Memory::default();
        memory
            .uncertain
            .push(kept(lost("sending", long_ago), Stage::Sending));
        memory
            .uncertain
            .push(kept(lost("lost", long_ago), Stage::Lost));

        let gone = memory.past_keep();
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].pending.id, "lost");
        assert_eq!(memory.uncertain.len(), 1);
        assert_eq!(memory.uncertain[0].pending.id, "sending");
    }

    /// A lost feed is forgotten after the keep window, by the time it
    /// went out.
    #[test]
    fn a_lost_feed_is_forgotten_by_its_age() {
        assert!(!forgotten(now_ms() - 16 * 60 * 1000));
        assert!(forgotten(now_ms() - KEEP_MS));
    }

    /// A feed carried across a restart keeps its settle window: one sent
    /// longer ago than [`SETTLE`] is already past it.
    #[test]
    fn a_carried_feed_keeps_its_settle_window() {
        let old = sent_at(now_ms() - 20 * 60 * 1000);
        assert!(old.elapsed() >= SETTLE);
        let recent = sent_at(now_ms() - 1000);
        assert!(recent.elapsed() < Duration::from_secs(5));
    }
}
