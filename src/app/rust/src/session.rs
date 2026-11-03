//! One feeder's device API session for the app: open it, keep what the Home
//! card and the Today tab show current, read the feeding history, and route
//! Dart's requests. The connection, keepalive and event cursor are
//! `wellfed_connect`'s; this loop only routes commands in and updates out,
//! and [`crate::requests`] builds each request's ops.
//!
//! A feed goes out once, under a fresh request id, and is never sent
//! again: an answer that never came means the feed may have run, and its
//! `Fed` event (or the history) settles it. Every other write goes the same
//! way, and its absent answer comes back as such.
//!
//! A settings, lock or plans write moves the card on its `Done`, and settings
//! and plans writes each go out one at a time on the generation the last one
//! landed: see [`crate::writes`].
//!
//! While this session's own feed runs it reads the status every
//! [`FEED_PROBE`] and gives a silent feeder [`FEED_SILENCE`], not the usual
//! [`QUIET_SILENCE`]: a feeder that lost power sends no reset, so nothing else tells the
//! screen that the feed it is following may have stopped. That only ends the
//! session. The reads are reads, and the feed itself is not sent again.
//!
//! After pairing, or whenever the feeder's clock is not set, the session
//! gives the feeder this phone's clock and zone, as `wellfed time` does.
//! That sync carries the zone's next change too, so a feeder with no zone
//! setting keeps local time across it.
//!
//! A session reads the history from the oldest event the feeder keeps, which
//! is what "last fed" and today's meals need. It holds nothing across
//! sessions, so a new session reads it again: up to the ring's 1,408 records,
//! about a frame a loop pass.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use tokio::sync::{mpsc, oneshot};
use wellfed_client::history::{Backlog, Entry, Feed};
use wellfed_client::plans::Edit;
use wellfed_connect::{Connection, Received};
use wellfed_model::{Answer, EventBody, Generation, Op, PLANS, Plan, Pushed};
use wellfed_session::noise::Keypair;

use crate::api::error::{BridgeError, ErrorKind};
use crate::api::keys::random;
use crate::api::session::{FeedRefusal, HistoryEntry, SessionUpdate};
use crate::link::DirectLink;
use crate::requests::TimeRead;
use crate::today::{Facts, event_entry, gap_entry, result};
use crate::writes::{Line, MealSwitch, Next, PlansQueue, Queued, Reply, SettingsQueue, Write};

/// How long one wait on the connection lasts before the loop looks at its
/// commands again and reads the status anew; the session keeps itself alive
/// meanwhile. So the card's meals and signal are at most this old, and a read
/// this often is what keeps [`QUIET_SILENCE`] fed.
const TURN: Duration = Duration::from_secs(10);
/// How long a feeder may stay silent before the app says it is unreachable. A
/// power cut or a lost network closes nothing, so silence is the only sign.
/// This is room for two unanswered reads. A slot reads nothing while its write
/// waits, and a write waits behind a running feed, so for a while after asking
/// anything the session keeps the usual limit: see [`Serving::keep_limit`].
const QUIET_SILENCE: Duration = Duration::from_secs(30);
/// While the history comes in: a backlog whose newest event this session
/// skips never reaches its end, so a quiet moment ends it instead.
const QUIET: Duration = Duration::from_secs(2);
/// How often a quiet session reads the time again. The feeder records no event
/// for an NTP fix in step, which it takes every 17 minutes, or for a clock it
/// has forgotten, so no event says when, by what or how well it was last set.
const TIME_EVERY: Duration = Duration::from_secs(60);
/// How often the status is read while this session's own feed runs. Nothing
/// else is due from the feeder until the feed's end, and an answer is the only
/// way to hear it is still there.
const FEED_PROBE: Duration = Duration::from_secs(5);
/// How long a feeder that is running this session's feed may stay silent. The
/// feeder answers reads during a feed, so this is room for three unanswered
/// reads. A stall longer than this that the feeder survives only costs a
/// reconnect: its `Fed` event still arrives in the history, and replaces what
/// the screen says.
const FEED_SILENCE: Duration = Duration::from_secs(20);
/// A feed's deadline per portion on the feeder (`portion_ms`), past the about
/// 9 s a portion usually takes.
const FEED_PORTION: Duration = Duration::from_secs(12);
/// What the feeder may add to its portions before a healthy feed's `Fed` is out:
/// 2 s to prepare, 5 s to home, 7.5 s for three jam reversals, 2 s to stop, and
/// 8.5 s to write and push the event.
const FEED_SPARE: Duration = Duration::from_secs(25);

/// When a feed of `portions` is overdue: a feeder that stayed on has ended it by
/// then.
pub(crate) fn feed_due(portions: u8) -> Duration {
    FEED_PORTION * u32::from(portions) + FEED_SPARE
}

/// The device API's request id.
type RequestId = [u8; 16];

#[derive(Debug)]
pub enum Command {
    /// A feed; `reply`, when given, hears its answer too (the router's
    /// feeds), beside the updates every feed makes.
    Feed {
        portions: u8,
        reply: Option<Reply>,
    },
    CancelFeed {
        feed: u32,
    },
    Refresh,
    /// One op; `reply` hears how it ended, [`Heard::NotSent`] when it never
    /// went out. A settings write may wait behind another: see
    /// [`crate::writes`]. A reply dropped unanswered is read as one that
    /// may have run.
    Ask {
        op: Op,
        reply: oneshot::Sender<Heard>,
    },
    /// A plans write: the table `edit` made of the one on `base`. A meal
    /// switch that waits behind another plans write goes again on the table
    /// that one made; see [`crate::writes`].
    EditPlans {
        base: Generation,
        plans: [Option<Plan>; PLANS],
        edit: Edit,
        reply: oneshot::Sender<Heard>,
    },
    Close,
}

/// How one [`Command::Ask`] ended.
#[derive(Clone, Debug)]
pub enum Heard {
    Answer(Answer),
    /// It went out and no answer came: it may have run.
    Unanswered,
    /// A settings or plans write asked while another was out, which never
    /// went: see [`crate::writes`].
    Held,
    /// The session ended before it went out.
    NotSent,
}

impl From<Option<Answer>> for Heard {
    fn from(answer: Option<Answer>) -> Self {
        answer.map_or(Self::Unanswered, Self::Answer)
    }
}

/// What an outstanding request was for.
enum Asked {
    Status,
    Settings,
    Plans,
    Time,
    Feed {
        portions: u8,
        reply: Option<Reply>,
    },
    Cancel,
    Clock,
    /// This phone's zone, stored on a feeder just paired.
    Zone,
    Ask(Reply),
    /// A write the card takes on its `Done`, and everyone its answer goes to:
    /// more than one for settings writes merged while they waited.
    Write {
        write: Write,
        replies: Vec<Reply>,
    },
}

impl Asked {
    /// Tell whoever waits on it that it never went out.
    fn never_sent(self) {
        let replies = match self {
            Self::Feed {
                reply: Some(reply), ..
            }
            | Self::Ask(reply) => vec![reply],
            Self::Write { replies, .. } => replies,
            _ => Vec::new(),
        };
        for reply in replies {
            // Nobody waiting means nobody to tell.
            let _ = reply.send(Heard::NotSent);
        }
    }
}

/// Run the session until Dart closes it or the feeder goes away. `update`
/// returns `false` once nobody is listening, which ends the session too.
pub async fn run(
    keys: Keypair,
    feeder: [u8; 32],
    address: SocketAddr,
    sync_clock: bool,
    commands: mpsc::UnboundedReceiver<Command>,
    update: impl FnMut(SessionUpdate) -> bool,
) {
    run_linked(None, keys, feeder, address, sync_clock, commands, update).await;
}

/// [`run`], telling `link` (the path router's view of this session) as it
/// opens, reads the history and ends.
pub(crate) async fn run_linked(
    link: Option<&DirectLink>,
    keys: Keypair,
    feeder: [u8; 32],
    address: SocketAddr,
    sync_clock: bool,
    mut commands: mpsc::UnboundedReceiver<Command>,
    mut update: impl FnMut(SessionUpdate) -> bool,
) {
    let mut out = Outstanding::default();
    let error = serve(
        link,
        &keys,
        feeder,
        address,
        sync_clock,
        &mut commands,
        &mut update,
        &mut out,
    )
    .await
    .err();
    if let Some(link) = link {
        link.closed();
    }

    // A session that ends answers nothing still outstanding, and each of
    // those writes may have run: say so before the end.
    let Outstanding {
        asked,
        mut settings,
        mut plans,
    } = out;
    for what in asked.into_values() {
        match what {
            Asked::Feed { reply, .. } => {
                if let Some(reply) = reply {
                    // Nobody waiting means nobody to tell.
                    let _ = reply.send(Heard::Unanswered);
                }
                update(SessionUpdate::FeedUnanswered);
            }
            Asked::Ask(reply) => {
                // Nobody waiting means nobody to tell.
                let _ = reply.send(Heard::Unanswered);
            }
            Asked::Write { replies, .. } => {
                for reply in replies {
                    let _ = reply.send(Heard::Unanswered);
                }
            }
            _ => {}
        }
    }
    // The writes still waiting never went, nor did the commands the loop
    // never took: each caller hears so. Closing first means no command
    // lands after the drain unanswered.
    for reply in settings.take().into_iter().chain(plans.take()) {
        let _ = reply.send(Heard::NotSent);
    }
    commands.close();
    while let Ok(command) = commands.try_recv() {
        never_sent(command);
    }
    update(SessionUpdate::Closed { error });
}

/// Tell whoever waits on `command` that it never went out.
fn never_sent(command: Command) {
    let reply = match command {
        Command::Feed {
            reply: Some(reply), ..
        }
        | Command::Ask { reply, .. }
        | Command::EditPlans { reply, .. } => reply,
        Command::Feed { reply: None, .. }
        | Command::CancelFeed { .. }
        | Command::Refresh
        | Command::Close => return,
    };
    // Nobody waiting means nobody to tell.
    let _ = reply.send(Heard::NotSent);
}

/// What a session has out or waiting, which is answered as it ends.
#[derive(Default)]
struct Outstanding {
    asked: HashMap<RequestId, Asked>,
    settings: SettingsQueue,
    plans: PlansQueue,
}

/// The session's own requests and the history as it comes in.
struct Serving<'a, U> {
    link: Option<&'a DirectLink>,
    connection: Connection,
    update: &'a mut U,
    out: &'a mut Outstanding,
    facts: Facts,
    /// Until it ends, events only fill `batch`; after it they are news.
    backlog: Option<Backlog>,
    batch: Vec<HistoryEntry>,
    /// When this session last asked for the time by itself.
    time_asked: Instant,
    /// This session's own feed and when it is overdue, from the feeder's
    /// `Feeding` until its `Fed` or that time: a `Fed` lost with a gap never comes.
    running: Option<(u32, tokio::time::Instant)>,
    /// When Dart last asked for something, which may be waiting behind a feed.
    asked_at: Option<tokio::time::Instant>,
    /// This session gave the feeder its offset, once: a feeder that refuses
    /// the sync is not asked again on every read of the time.
    offset_given: bool,
    /// A feeder just paired with no zone gets this phone's, once. Only then:
    /// later, no zone is the owner's choice (the clock screen clears it).
    zone_due: bool,
}

#[expect(
    clippy::too_many_arguments,
    reason = "the session's inputs, passed once from its two entry points"
)]
async fn serve(
    link: Option<&DirectLink>,
    keys: &Keypair,
    feeder: [u8; 32],
    address: SocketAddr,
    sync_clock: bool,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    update: &mut impl FnMut(SessionUpdate) -> bool,
    out: &mut Outstanding,
) -> Result<(), BridgeError> {
    let mut connection = Connection::open(keys, feeder, address, random()?).await?;
    connection.set_silence(QUIET_SILENCE);
    if !update(SessionUpdate::Opened {
        build: connection.build().to_owned(),
        role: connection.role(),
    }) {
        return Ok(());
    }
    if let Some(link) = link {
        link.opened();
    }

    let Answer::Status(status) = connection.ask(random()?, &Op::Status).await? else {
        return Err(BridgeError::new(
            ErrorKind::Protocol,
            "the feeder did not answer with its status",
        ));
    };
    let backlog = Backlog {
        newest: status.last_event,
    };
    if let Some(link) = link {
        link.event_at(status.last_event);
    }
    let mut serving = Serving {
        link,
        connection,
        update,
        out,
        facts: Facts::default(),
        backlog: Some(backlog),
        batch: Vec::new(),
        time_asked: Instant::now(),
        running: None,
        asked_at: None,
        offset_given: false,
        zone_due: sync_clock,
    };
    if sync_clock || status.clock.is_none() {
        let sync = wellfed_client::time::sync(Timestamp::now(), &TimeZone::system());
        serving.request(&Op::TimeSync(sync), Asked::Clock)?;
    }
    serving.request(&Op::GetSettings, Asked::Settings)?;
    serving.request(&Op::GetPlans, Asked::Plans)?;
    serving.request(&Op::GetTime, Asked::Time)?;
    serving.connection.subscribe(None, false).await?;
    serving.facts.status = Some(status);
    if !serving.card() {
        return Ok(());
    }
    if backlog.read(None) && !serving.end_backlog() {
        return Ok(());
    }

    loop {
        let wait = serving.wait();
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { return Ok(()) };
                if !serving.command(command)? {
                    return Ok(());
                }
            }
            received = serving.connection.receive(wait) => {
                let more = match received {
                    Err(wellfed_connect::Error::Timeout) => serving.turn()?,
                    other => match other? {
                        Received::Answer { id, answer } => serving.answered(id, answer)?,
                        Received::Pushed(pushed) => serving.pushed(&pushed)?,
                    },
                };
                if !more {
                    return Ok(());
                }
            }
        }
    }
}

impl<U: FnMut(SessionUpdate) -> bool> Serving<'_, U> {
    /// Send `op`. It is outstanding before a byte goes, so a send that
    /// fails part way is answered as one that may have run when the
    /// session ends.
    fn request(&mut self, op: &Op, what: Asked) -> Result<(), BridgeError> {
        let id = match random() {
            Ok(id) => id,
            Err(error) => {
                what.never_sent();
                return Err(error);
            }
        };
        self.out.asked.insert(id, what);
        self.connection.request(id, op)?;
        Ok(())
    }

    /// Send what Dart asked for; `false` once it closes the session.
    fn command(&mut self, command: Command) -> Result<bool, BridgeError> {
        let (op, what) = match command {
            Command::Close => return Ok(false),
            Command::Refresh => (Op::Status, Asked::Status),
            Command::Feed { portions, reply } => {
                (Op::Feed { portions }, Asked::Feed { portions, reply })
            }
            Command::CancelFeed { feed } => (Op::CancelFeed { feed }, Asked::Cancel),
            Command::EditPlans {
                base,
                plans,
                edit,
                reply,
            } => {
                let switch = match edit {
                    Edit::Enabled { slot, enabled } => Some(MealSwitch { slot, enabled }),
                    _ => None,
                };
                let queued = self.out.plans.ask(base, plans, switch, reply);
                return self.queued(queued);
            }
            Command::Ask { op, reply } => match Write::of(&op) {
                Some(write @ (Write::Settings { .. } | Write::Reset { .. })) => {
                    let known = self.facts.settings.map(|settings| settings.generation);
                    let queued = self.out.settings.ask(write, known, reply);
                    return self.queued(queued);
                }
                Some(Write::Plans { base, plans }) => {
                    let queued = self.out.plans.ask(base, plans, None, reply);
                    return self.queued(queued);
                }
                Some(write) => (
                    op,
                    Asked::Write {
                        write,
                        replies: vec![reply],
                    },
                ),
                None => (op, Asked::Ask(reply)),
            },
        };
        self.asked(&op, what)?;
        Ok(true)
    }

    /// Send a write the queue let go now; one that waits goes once the write
    /// out lands, and one held hears so.
    fn queued(&mut self, queued: Queued) -> Result<bool, BridgeError> {
        match queued {
            Queued::Send(outgoing) => self.asked(
                &outgoing.op,
                Asked::Write {
                    write: outgoing.write,
                    replies: outgoing.replies,
                },
            )?,
            Queued::Waits => {}
            Queued::Held(reply) => {
                // Dart stopped waiting: nothing to tell it.
                let _ = reply.send(Heard::Held);
            }
        }
        Ok(true)
    }

    /// Send a request a screen made, which may wait behind a feed.
    fn asked(&mut self, op: &Op, what: Asked) -> Result<(), BridgeError> {
        self.request(op, what)?;
        self.asked_at = Some(tokio::time::Instant::now());
        self.keep_limit();
        Ok(())
    }

    /// A write's answer: everyone who asked hears it, the card takes a `Done`
    /// that landed, and the settings writes that waited go or end.
    ///
    /// The callers hear first: whatever ends the session after, a write the
    /// feeder answered must never read as not sent, which is safe to resend.
    fn wrote(
        &mut self,
        write: &Write,
        replies: Vec<Reply>,
        answer: Option<&Answer>,
    ) -> Result<bool, BridgeError> {
        let heard = Heard::from(answer.cloned());
        for reply in replies {
            // Dart stopped waiting: nothing to tell it.
            let _ = reply.send(heard.clone());
        }
        let next = match write.queue() {
            Some(Line::Settings) => self.out.settings.answered(answer),
            Some(Line::Plans) => self.out.plans.answered(answer),
            None => Next::Idle,
        };
        let follow = match next {
            Next::Idle => None,
            Next::Send(outgoing) => Some(outgoing),
            Next::Unsent(waiting) => {
                for reply in waiting {
                    let _ = reply.send(Heard::Held);
                }
                None
            }
        };

        if write.landed(&mut self.facts, answer) && !self.card() {
            return Ok(false);
        }
        if let Some(outgoing) = follow {
            self.asked(
                &outgoing.op,
                Asked::Write {
                    write: outgoing.write,
                    replies: outgoing.replies,
                },
            )?;
        }
        Ok(true)
    }

    /// How long to wait on the connection before [`Self::turn`].
    const fn wait(&self) -> Duration {
        match (self.backlog, self.running) {
            (Some(_), _) => QUIET,
            (None, Some(_)) => FEED_PROBE,
            (None, None) => TURN,
        }
    }

    /// Say the card again; `false` once nobody listens.
    fn card(&mut self) -> bool {
        match crate::today::card(&self.facts) {
            Some(card) => (self.update)(SessionUpdate::Status { card }),
            None => true,
        }
    }

    /// Read the status again, unless a read is already on its way.
    fn refresh(&mut self) -> Result<(), BridgeError> {
        if !self
            .out
            .asked
            .values()
            .any(|what| matches!(what, Asked::Status))
        {
            self.request(&Op::Status, Asked::Status)?;
        }
        Ok(())
    }

    /// Read the time again if it is due, unless a read is already on its way
    /// or this session's feed is running.
    fn reread_time(&mut self) -> Result<(), BridgeError> {
        let due = time_read_due(
            self.time_asked.elapsed(),
            self.out
                .asked
                .values()
                .any(|what| matches!(what, Asked::Time)),
            self.running.is_some()
                || self
                    .out
                    .asked
                    .values()
                    .any(|what| matches!(what, Asked::Feed { .. })),
        );
        if due {
            self.time_asked = Instant::now();
            self.request(&Op::GetTime, Asked::Time)?;
        }
        Ok(())
    }

    /// Hand over the history so far.
    fn end_backlog(&mut self) -> bool {
        self.backlog = None;
        let entries = std::mem::take(&mut self.batch);
        (self.update)(SessionUpdate::History {
            entries,
            caught_up: true,
        }) && self.card()
    }

    /// Set how long a silent feeder is given: a running feed's limit, the usual
    /// one while something asked may still be waiting behind a feed, or the
    /// quiet session's.
    fn keep_limit(&mut self) {
        let waiting = self
            .asked_at
            .is_some_and(|at| at.elapsed() < wellfed_connect::SILENCE);
        self.connection.set_silence(match (self.running, waiting) {
            (Some(_), _) => FEED_SILENCE,
            (None, true) => wellfed_connect::SILENCE,
            (None, false) => QUIET_SILENCE,
        });
    }

    /// Nothing came for a whole wait.
    fn turn(&mut self) -> Result<bool, BridgeError> {
        if self
            .running
            .is_some_and(|(_, due)| due <= tokio::time::Instant::now())
        {
            self.running = None;
        }
        self.keep_limit();
        if self.backlog.is_some() && !self.end_backlog() {
            return Ok(false);
        }
        self.refresh()?;
        self.reread_time()?;
        Ok(true)
    }

    fn answered(&mut self, id: RequestId, answer: Option<Answer>) -> Result<bool, BridgeError> {
        let Some(what) = self.out.asked.remove(&id) else {
            return Ok(true);
        };
        match (what, answer) {
            (Asked::Status, Some(Answer::Status(status))) => {
                if let Some(link) = self.link {
                    link.event_at(status.last_event);
                }
                self.facts.status = Some(status);
                Ok(self.card())
            }
            (Asked::Settings, Some(Answer::Settings(settings))) => {
                self.facts.settings = Some(settings);
                Ok(self.card())
            }
            (Asked::Plans, Some(Answer::Plans(plans))) => {
                self.facts.plans = Some(plans);
                Ok(self.card())
            }
            (Asked::Time, Some(Answer::Time(time))) => {
                let give = !self.offset_given && needs_offset(&time);
                let zone = self.zone_due.then(|| zone_for(&time)).flatten();
                self.zone_due = false;
                self.facts.time = Some(TimeRead::just_read(time));
                if let Some(op) = zone {
                    self.request(&op, Asked::Zone)?;
                }
                if give {
                    self.offset_given = true;
                    let sync = wellfed_client::time::sync(Timestamp::now(), &TimeZone::system());
                    self.request(&Op::TimeSync(sync), Asked::Clock)?;
                }
                Ok(self.card())
            }
            (Asked::Ask(reply), answer) => {
                // A screen's read is news for the card as well.
                let news = match &answer {
                    Some(Answer::Plans(plans)) => {
                        self.facts.plans = Some(*plans);
                        true
                    }
                    Some(Answer::Time(time)) => {
                        self.facts.time = Some(TimeRead::just_read(time.clone()));
                        true
                    }
                    _ => false,
                };
                if news && !self.card() {
                    return Ok(false);
                }
                // Dart stopped waiting: nothing to tell it.
                let _ = reply.send(Heard::from(answer));
                Ok(true)
            }
            (Asked::Write { write, replies }, answer) => {
                self.wrote(&write, replies, answer.as_ref())
            }
            (Asked::Clock, Some(Answer::Done { .. })) => {
                self.request(&Op::GetTime, Asked::Time)?;
                Ok((self.update)(SessionUpdate::ClockSynced))
            }
            (Asked::Clock, _) => Ok((self.update)(SessionUpdate::ClockNotSynced)),
            (Asked::Zone, Some(Answer::Done { .. })) => {
                self.request(&Op::GetTime, Asked::Time)?;
                Ok(true)
            }
            // Refused or unanswered: the clock screen still offers it.
            (Asked::Zone, _) => Ok(true),
            (what, answer) => {
                let what = match what {
                    Asked::Feed {
                        portions,
                        reply: Some(reply),
                    } => {
                        // The router stopped waiting: nothing to tell it.
                        let _ = reply.send(Heard::from(answer.clone()));
                        Asked::Feed {
                            portions,
                            reply: None,
                        }
                    }
                    other => other,
                };
                let message = feed_answered(what, answer);
                if let Some(SessionUpdate::FeedStarted {
                    feed, due_seconds, ..
                }) = &message
                {
                    let due = Duration::from_secs(u64::from(*due_seconds));
                    self.running = Some((*feed, tokio::time::Instant::now() + due));
                    self.connection.set_silence(FEED_SILENCE);
                }
                Ok(message.is_none_or(|message| (self.update)(message)))
            }
        }
    }

    fn pushed(&mut self, pushed: &Pushed) -> Result<bool, BridgeError> {
        if let Pushed::Log(log) = pushed {
            return Ok((self.update)(SessionUpdate::Log {
                seq: log.seq,
                dropped: log.dropped,
                lines: log.lines.clone(),
            }));
        }

        let fed = match Entry::from_pushed(pushed) {
            Some(Entry::Feed(feed)) => Some(feed),
            _ => None,
        };
        if let (Some(link), Pushed::Event(event)) = (self.link, pushed) {
            link.event_at(Some(event.seq));
        }
        if let (Some(link), Some(feed)) = (self.link, fed.as_ref()) {
            link.fed(feed);
        }
        if let Some(feed) = fed {
            self.facts.feeds.push(feed);
            if self.running.is_some_and(|(ours, _)| ours == feed.feed) {
                self.running = None;
                self.keep_limit();
            }
        }
        let entry = match pushed {
            Pushed::Event(event) => event_entry(event),
            Pushed::Gap(gap) => gap_entry(gap),
            Pushed::Sample(_) | Pushed::Log(_) => return Ok(true),
        };

        if let Some(backlog) = self.backlog {
            self.batch.push(entry);
            return Ok(!backlog.ends_at(pushed) || self.end_backlog());
        }
        if !(self.update)(SessionUpdate::History {
            entries: vec![entry],
            caught_up: true,
        }) {
            return Ok(false);
        }
        self.news(pushed, fed.as_ref())
    }

    /// An event after the backlog: any may move the card (a feed, the lock,
    /// the hopper), so it is read again.
    fn news(&mut self, pushed: &Pushed, fed: Option<&Feed>) -> Result<bool, BridgeError> {
        if let Pushed::Event(event) = pushed {
            match event.body {
                ref body if moves_plans(body) => self.request(&Op::GetPlans, Asked::Plans)?,
                EventBody::Settings { .. } => self.request(&Op::GetSettings, Asked::Settings)?,
                EventBody::ClockSync { .. } => self.request(&Op::GetTime, Asked::Time)?,
                _ => {}
            }
        }
        self.refresh()?;
        match fed {
            Some(fed) => Ok((self.update)(SessionUpdate::FeedEnded {
                result: result(fed),
            })),
            None => Ok(true),
        }
    }
}

/// Whether a quiet turn should read the time again: after [`TIME_EVERY`], with
/// no read on its way, and not while this session's feed runs, when reads have
/// a more urgent job.
fn time_read_due(since_asked: Duration, on_its_way: bool, feeding: bool) -> bool {
    since_asked >= TIME_EVERY && !on_its_way && !feeding
}

/// The screen's update for a feed's or a cancel's answer, or its absence.
fn feed_answered(what: Asked, answer: Option<Answer>) -> Option<SessionUpdate> {
    match (what, answer) {
        (Asked::Feed { portions, .. }, Some(Answer::Feeding { feed })) => {
            Some(SessionUpdate::FeedStarted {
                feed,
                portions,
                due_seconds: u32::try_from(feed_due(portions).as_secs()).unwrap_or(u32::MAX),
            })
        }
        (Asked::Feed { .. }, Some(Answer::Refused(refusal))) => Some(SessionUpdate::FeedRefused {
            refusal: FeedRefusal::from(refusal),
        }),
        // The feed may have run; it is never sent again.
        (Asked::Feed { .. }, None) => Some(SessionUpdate::FeedUnanswered),
        (Asked::Cancel, Some(Answer::Refused(refusal))) => Some(SessionUpdate::CancelUnconfirmed {
            refusal: Some(FeedRefusal::from(refusal)),
        }),
        (Asked::Cancel, None) => Some(SessionUpdate::CancelUnconfirmed { refusal: None }),
        _ => None,
    }
}

/// Whether `body` may mean the plans moved. A one-off plan disables itself when
/// it is claimed or passed, which moves the generation with no `Plans` event:
/// the feed or the missed meal is the only news of it.
const fn moves_plans(body: &EventBody) -> bool {
    matches!(
        body,
        EventBody::Plans { .. } | EventBody::Fed { .. } | EventBody::Missed { .. }
    )
}

/// Whether the feeder runs a valid clock with no local offset. Plans hold
/// then, and nothing says so: an NTP fix carries no offset, and one a
/// client's sync gave lives in RAM. So a feeder with no zone set that lost
/// power keeps the right time but holds every meal, until a client syncs.
/// The session gives it this phone's.
fn needs_offset(time: &wellfed_model::TimeView) -> bool {
    time.clock_ms.is_some() && time.offset_s.is_none() && time.zone.is_none()
}

/// This phone's zone for a feeder that keeps none and may store one. A zone
/// is kept on flash, so the offset survives a power cut, and its rule
/// follows daylight saving.
fn zone_for(time: &wellfed_model::TimeView) -> Option<Op> {
    if time.zone.is_some() || !time.writable {
        return None;
    }
    let changes =
        crate::requests::time_changes(Some(crate::api::feeder::ZoneChange::Phone), None).ok()?;
    Some(Op::PutTime {
        base: time.generation,
        changes,
    })
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use wellfed_model::{FeedOutcome, FeedReason, FoodLevel, Generation, MissedReason, Source};

    use super::*;

    /// A feeder's time view: a valid clock set by NTP, no zone, no offset.
    fn after_a_power_cut() -> wellfed_model::TimeView {
        wellfed_model::TimeView {
            generation: Generation(1),
            writable: true,
            zone: None,
            ntp: wellfed_model::NtpServer::Gateway { port: 123 },
            clock_ms: Some(1_793_000_000_000),
            error_ms: Some(30),
            offset_s: None,
            rate_ppb: 0,
            rate_bound_ppb: 5_000_000,
            learned: false,
            syncs: 1,
            last: Some(wellfed_model::TimeSource::Ntp),
            last_age_s: Some(60),
            ntp_last: Some(wellfed_model::NtpResult::Used),
        }
    }

    /// The state that held a meal on the bench feeder on November 2: the
    /// clock right, the offset gone with the power, no zone to bring it back.
    #[test]
    fn a_valid_clock_with_no_offset_and_no_zone_needs_this_phone_s() {
        assert!(needs_offset(&after_a_power_cut()));
    }

    #[test]
    fn an_offset_a_zone_or_no_clock_needs_nothing() {
        let with_offset = wellfed_model::TimeView {
            offset_s: Some(-7 * 3600),
            ..after_a_power_cut()
        };
        let with_zone = wellfed_model::TimeView {
            zone: Some("MST7".into()),
            ..after_a_power_cut()
        };
        // An invalid clock gets the session's opening sync already.
        let no_clock = wellfed_model::TimeView {
            clock_ms: None,
            ..after_a_power_cut()
        };

        assert!(!needs_offset(&with_offset));
        assert!(!needs_offset(&with_zone));
        assert!(!needs_offset(&no_clock));
    }

    /// A zone the owner set is never replaced, and a store that cannot be
    /// written is not asked.
    #[test]
    fn a_zone_set_or_a_read_only_store_gets_no_zone() {
        let set = wellfed_model::TimeView {
            zone: Some("MST7".into()),
            ..after_a_power_cut()
        };
        let read_only = wellfed_model::TimeView {
            writable: false,
            ..after_a_power_cut()
        };

        assert!(zone_for(&set).is_none());
        assert!(zone_for(&read_only).is_none());
    }

    /// Past this a feeder that stayed on ended the feed itself: its largest
    /// request, with every reversal and homing, still fits.
    #[test]
    fn a_feed_s_due_time_covers_what_the_feeder_can_take() {
        assert_eq!(feed_due(2), Duration::from_secs(49));
        assert_eq!(feed_due(48), Duration::from_secs(601));
    }

    #[test]
    fn the_time_is_read_again_about_once_a_minute_and_never_twice_at_once() {
        // A minute after the last read, and not before.
        assert!(!time_read_due(Duration::from_secs(30), false, false));
        assert!(time_read_due(Duration::from_secs(60), false, false));
        assert!(!time_read_due(Duration::from_secs(60), true, false));
        assert!(!time_read_due(Duration::from_secs(60), false, true));
    }

    #[test]
    fn a_feed_or_a_missed_meal_reads_the_plans_again() {
        let fed = EventBody::Fed {
            feed: 1,
            requested: 1,
            completed: 1,
            outcome: FeedOutcome::Done,
            reason: FeedReason::Done,
            hopper: FoodLevel::Present,
            by: Source::Schedule { plan: 0 },
        };
        let missed = EventBody::Missed {
            plan: 0,
            due: Timestamp::UNIX_EPOCH,
            reason: MissedReason::Late,
        };

        assert!(moves_plans(&fed));
        assert!(moves_plans(&missed));
        assert!(!moves_plans(&EventBody::Settings {
            generation: Generation(1),
            by: Source::Feeder,
        }));
    }
}
