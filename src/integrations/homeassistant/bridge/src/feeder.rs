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
//! - **Uncertain feeds.** A feed whose answer never came may have run. The
//!   session keeps it, and settles it when a later `Fed` event names this
//!   client as its source and is not a feed it was already answered for;
//!   it reports [`UpdateKind::StillUnknown`](crate::api::UpdateKind) when
//!   the history holds no such feed, never a guessed count.
//! - **Revocation.** A key the feeder no longer knows ends the session for
//!   good: it is reported once and the thread stops reconnecting.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Mutex;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};
use wellfed_client::history::{Backlog, Entry, Feed};
use wellfed_client::today::Facts;
use wellfed_connect::{Connection, Cursor, Received};
use wellfed_model::{
    Answer, Blocked, CalibrationView, ClientRole, CoreEvent, EventBody, FeederEvent,
    FeederSettingsPatch, Generation, Op, Pushed, Readiness, Settings, Source, Status,
};

use crate::Error;
use crate::api::{self, Capabilities, NotSent, State, Tune, Update, Written};
use crate::key::{ClientKey, locate, random};
use crate::writes::{self, Kind, Landed, Store, Write};

/// The device API's request id.
type RequestId = [u8; 16];

/// How often a quiet session reads the status anew, which also keeps the
/// connection's silence limit fed.
const TURN: Duration = Duration::from_secs(30);
/// How long a lost session waits before opening another.
const RETRY: Duration = Duration::from_secs(10);
/// How long an uncertain feed waits for its `Fed` event within one
/// session: longer than the longest feed the feeder runs.
const SETTLE: Duration = Duration::from_secs(15 * 60);
/// Feeds kept for the day's facts, and feed numbers this client was
/// answered for: far more than a day holds.
const FEEDS_KEPT: usize = 512;

#[derive(Debug)]
enum Command {
    Write {
        write: Write,
        reply: oneshot::Sender<Written>,
    },
    Refresh,
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
    /// Start the session thread for `target` as `key`. It connects, and
    /// reconnects after every loss, until the handle is dropped or the
    /// feeder revokes the key.
    pub fn start(key: ClientKey, target: Target) -> Result<Self, Error> {
        let (commands, inbox) = mpsc::unbounded_channel();
        let (outbox, updates) = std_mpsc::channel();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(Error::Thread)?;
        std::thread::Builder::new()
            .name("wellfed-session".into())
            .spawn(move || runtime.block_on(run(key, target, inbox, outbox)))
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
        if self.commands.send(Command::Write { write, reply }).is_err() {
            return Written::not_sent(NotSent::Stopped);
        }
        // The session answers every write it takes, uncertain when it ends
        // first, so a dropped reply is a session that ended mid-write.
        answer
            .blocking_recv()
            .unwrap_or_else(|_| Written::uncertain())
    }
}

/// A feed that went out and was never answered.
#[derive(Clone, Copy, Debug)]
struct Uncertain {
    portions: u8,
    /// The newest event known when it went out: its `Fed` comes after.
    after: Option<u64>,
    sent: Instant,
}

/// What outlives one session: where the history was read to, what it said,
/// and the feeds still to settle.
#[derive(Debug, Default)]
struct Memory {
    cursor: Option<Cursor>,
    facts: Facts,
    calibration: Option<CalibrationView>,
    /// Feed numbers the feeder answered this client with, newest last: a
    /// `Fed` for one of them settles nothing.
    answered: VecDeque<u32>,
    uncertain: Vec<Uncertain>,
}

impl Memory {
    fn answered(&mut self, feed: u32) {
        if self.answered.len() == FEEDS_KEPT {
            self.answered.pop_front();
        }
        self.answered.push_back(feed);
    }

    fn fed(&mut self, feed: Feed) {
        // A backlog replays from the cursor, so a feed is seen once; the
        // check keeps a replay after a lost cursor from doubling it.
        if self
            .facts
            .feeds
            .last()
            .is_some_and(|last| last.seq >= feed.seq)
        {
            return;
        }
        if self.facts.feeds.len() == FEEDS_KEPT {
            self.facts.feeds.remove(0);
        }
        self.facts.feeds.push(feed);
    }
}

/// Connect, serve until the session ends, and go again, until the handle is
/// dropped or the key is revoked.
async fn run(
    key: ClientKey,
    target: Target,
    mut commands: mpsc::UnboundedReceiver<Command>,
    updates: std_mpsc::Sender<Update>,
) {
    let mut fingerprint = [0; 4];
    fingerprint.copy_from_slice(&key.public()[..4]);
    let mut memory = Memory::default();

    loop {
        let ended = match open(&key, &target).await {
            Ok(connection) => {
                let session = Session::new(connection, fingerprint, &mut memory, &updates);
                session.serve(&mut commands).await
            }
            Err(Error::Connect(wellfed_connect::Error::UnknownKey)) => Ended::Revoked,
            Err(error) => Ended::Lost(error.to_string()),
        };
        let reason = match ended {
            Ended::Dropped => return,
            Ended::Revoked => {
                revoked(&mut commands, &updates).await;
                return;
            }
            Ended::Lost(reason) => reason,
        };
        if updates.send(Update::disconnected(reason)).is_err() {
            return;
        }

        // While no session is open a write is refused at once, never held
        // for the next one: a feed sent minutes after its press is not the
        // feed the owner asked for.
        let pause = tokio::time::sleep(RETRY);
        tokio::pin!(pause);
        loop {
            tokio::select! {
                () = &mut pause => break,
                command = commands.recv() => match command {
                    None => return,
                    Some(Command::Write { reply, .. }) => {
                        let _ = reply.send(Written::not_sent(NotSent::Disconnected));
                    }
                    Some(Command::Refresh) => {}
                },
            }
        }
    }
}

/// The key is gone from the feeder: say so once, then refuse every write
/// until the handle is dropped. A new key comes from a new invite.
async fn revoked(
    commands: &mut mpsc::UnboundedReceiver<Command>,
    updates: &std_mpsc::Sender<Update>,
) {
    if updates.send(Update::revoked()).is_err() {
        return;
    }
    while let Some(command) = commands.recv().await {
        if let Command::Write { reply, .. } = command {
            let _ = reply.send(Written::not_sent(NotSent::Stopped));
        }
    }
}

async fn open(key: &ClientKey, target: &Target) -> Result<Connection, Error> {
    let address: SocketAddr = locate(&target.identity, target.address.as_deref()).await?;
    Ok(Connection::open(key.keypair(), target.identity, address, random()?).await?)
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
}

impl Read {
    const fn op(self) -> Op {
        match self {
            Self::Status => Op::Status,
            Self::Settings => Op::GetSettings,
            Self::Plans => Op::GetPlans,
            Self::Time => Op::GetTime,
            Self::Calibration => Op::GetCalibration,
        }
    }
}

/// What an outstanding request was for.
#[derive(Debug)]
enum Pending {
    Write {
        reply: oneshot::Sender<Written>,
        kind: Kind,
        /// What went out, to put on the facts once it lands.
        op: Op,
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
    /// The replay at connect, until its newest event has come.
    backlog: Option<Backlog>,
    pending: HashMap<RequestId, Pending>,
}

/// The session's own failure, or the handle gone.
type Step = Result<(), Ended>;

impl<'a> Session<'a> {
    fn new(
        connection: Connection,
        fingerprint: [u8; 4],
        memory: &'a mut Memory,
        updates: &'a std_mpsc::Sender<Update>,
    ) -> Self {
        Self {
            role: connection.role(),
            capabilities: Capabilities::of(connection.kind()),
            connection,
            fingerprint,
            memory,
            updates,
            backlog: None,
            pending: HashMap::new(),
        }
    }

    async fn serve(mut self, commands: &mut mpsc::UnboundedReceiver<Command>) -> Ended {
        let ended = match self.open().await {
            Ok(()) => self.follow(commands).await,
            Err(ended) => ended,
        };

        // Each write still outstanding went out and was never answered.
        let outstanding: Vec<Pending> = self.pending.drain().map(|(_, pending)| pending).collect();
        for pending in outstanding {
            if let Pending::Write { reply, kind, .. } = pending {
                self.unanswered(kind);
                let _ = reply.send(Written::uncertain());
            }
        }
        ended
    }

    /// Announce the session, read the facts and subscribe.
    async fn open(&mut self) -> Step {
        let connected = Update::connected(self.connection.build(), self.role, self.capabilities);
        self.send(connected)?;

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
            let step = tokio::select! {
                command = commands.recv() => match command {
                    None => return Ended::Dropped,
                    Some(Command::Write { write, reply }) => self.write(write, reply),
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

    fn send(&self, update: Update) -> Step {
        self.updates.send(update).map_err(|_| Ended::Dropped)
    }

    /// The state as it now stands, once the backlog is read.
    fn state(&self) -> Step {
        if self.backlog.is_some() {
            return Ok(());
        }
        match State::of(&self.memory.facts, self.memory.calibration.as_ref()) {
            Some(state) => self.send(Update::state(state)),
            None => Ok(()),
        }
    }

    /// Ask for `read`, unless the same read is already out.
    fn read(&mut self, read: Read) -> Step {
        if self
            .pending
            .values()
            .any(|pending| matches!(pending, Pending::Read(out) if *out == read))
        {
            return Ok(());
        }
        let op = read.op();
        let id = random().map_err(lost)?;
        self.connection.request(id, &op).map_err(lost)?;
        self.pending.insert(id, Pending::Read(read));
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
            settings: facts.settings.as_ref().map(|settings| settings.generation),
            plans: facts.plans.as_ref(),
            calibration: self.memory.calibration.as_ref(),
        };
        let built = writes::gate(self.role)
            .and_then(|()| writes::build(&write, &read))
            .and_then(|(op, kind)| {
                writes::served(&op, self.connection.kind())?;
                let busy = matches!(kind, Kind::Store { store, .. } if self.store_busy(store));
                wellfed_formats::require(!busy, || NotSent::Busy)?;
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
        if let Err(error) = self.connection.request(id, &op) {
            // Refused before any byte went out.
            let _ = reply.send(Written::not_sent(NotSent::Disconnected));
            return Err(lost(error));
        }
        self.pending.insert(id, Pending::Write { reply, kind, op });
        Ok(())
    }

    fn store_busy(&self, store: Store) -> bool {
        self.pending.values().any(|pending| {
            matches!(pending, Pending::Write { kind: Kind::Store { store: out, .. }, .. } if *out == store)
        })
    }

    fn answered(&mut self, id: RequestId, answer: Option<Answer>) -> Step {
        match self.pending.remove(&id) {
            Some(Pending::Write { reply, kind, op }) => {
                let written = self.written(kind, &op, answer.as_ref())?;
                let _ = reply.send(written);
                Ok(())
            }
            Some(Pending::Read(read)) => self.fact(read, answer),
            None => Ok(()),
        }
    }

    /// How the write of `kind` ended, by its answer.
    fn written(&mut self, kind: Kind, op: &Op, answer: Option<&Answer>) -> Result<Written, Ended> {
        let Some(answer) = answer else {
            self.unanswered(kind);
            return Ok(Written::uncertain());
        };
        match kind {
            Kind::Once | Kind::Feed { .. } => {
                if let Answer::Feeding { feed } = answer {
                    self.memory.answered(*feed);
                }
                Ok(once(answer))
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
                    Landed::Unknown => Written::uncertain(),
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
                    .filter(|held| held.generation == *base)
                {
                    settings.apply(*changes);
                    settings.generation = next(*base);
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

    /// A write of `kind` went out and its answer never came.
    fn unanswered(&mut self, kind: Kind) {
        if let Kind::Feed { portions } = kind {
            self.memory.uncertain.push(Uncertain {
                portions,
                after: self.memory.cursor.map(|cursor| cursor.seq).or_else(|| {
                    self.memory
                        .facts
                        .status
                        .as_ref()
                        .and_then(|status| status.last_event)
                }),
                sent: Instant::now(),
            });
        }
    }

    fn fact(&mut self, read: Read, answer: Option<Answer>) -> Step {
        let facts = &mut self.memory.facts;
        match (read, answer) {
            (Read::Status, Some(Answer::Status(status))) => facts.status = Some(status),
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
            Pushed::Gap(_) => self.backlog.is_some(),
            Pushed::Sample(_) | Pushed::Log(_) => return Ok(()),
        };
        self.memory.cursor = self.connection.cursor();

        if let Some(Entry::Feed(feed)) = Entry::from_pushed(pushed) {
            self.memory.fed(feed);
            self.settle(&feed)?;
        }
        match pushed {
            Pushed::Event(event) => {
                self.send(Update::event(event, replayed))?;
                if !replayed {
                    self.news(&event.body)?;
                }
            }
            Pushed::Gap(gap) => self.send(Update::gap(Some(gap)))?,
            Pushed::Sample(_) | Pushed::Log(_) => {}
        }

        match self.backlog {
            Some(backlog) if backlog.ends_at(pushed) => self.end_backlog(),
            Some(_) => Ok(()),
            None => self.state(),
        }
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
    /// unknown.
    fn end_backlog(&mut self) -> Step {
        self.backlog = None;
        let feeding = self.memory.facts.status.as_ref().is_some_and(feeding_now);
        if !feeding {
            for uncertain in std::mem::take(&mut self.memory.uncertain) {
                self.send(Update::still_unknown(uncertain.portions))?;
            }
        }
        self.state()
    }

    /// A `Fed` this client asked for and was never answered about settles
    /// the oldest uncertain feed it can be.
    fn settle(&mut self, feed: &Feed) -> Step {
        let ours =
            matches!(feed.by, Some(Source::Client { client }) if client.0 == self.fingerprint);
        if !ours || self.memory.answered.contains(&feed.feed) {
            return Ok(());
        }
        let found = self
            .memory
            .uncertain
            .iter()
            .position(|uncertain| uncertain.after.is_none_or(|after| feed.seq > after));
        let Some(found) = found else {
            return Ok(());
        };

        self.memory.uncertain.remove(found);
        self.memory.answered(feed.feed);
        self.send(Update::settled(api::Feed::from(feed)))
    }

    /// Uncertain feeds past [`SETTLE`] with no `Fed`: still unknown.
    fn expire(&mut self) -> Step {
        let now = Instant::now();
        let (expired, kept) = std::mem::take(&mut self.memory.uncertain)
            .into_iter()
            .partition(|uncertain| now.duration_since(uncertain.sent) >= SETTLE);
        self.memory.uncertain = kept;
        for uncertain in expired {
            self.send(Update::still_unknown(uncertain.portions))?;
        }
        Ok(())
    }
}

/// Whether the feeder reports a feed running now.
fn feeding_now(status: &Status) -> bool {
    status
        .feeder()
        .is_some_and(|feeder| feeder.readiness == Readiness::Blocked(Blocked::Feeding))
}

/// The generation a store moves to from `base`.
const fn next(base: Generation) -> Generation {
    Generation(base.0.wrapping_add(1))
}

const fn store_read(store: Store) -> Read {
    match store {
        Store::Settings => Read::Settings,
        Store::Plans => Read::Plans,
        Store::Calibration => Read::Calibration,
    }
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
