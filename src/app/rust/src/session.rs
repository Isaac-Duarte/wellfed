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
//! While this session's own feed runs it reads the status every
//! [`FEED_PROBE`] and gives a silent feeder [`FEED_SILENCE`], not the usual
//! 90 s: a feeder that lost power sends no reset, so nothing else tells the
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
use wellfed_connect::{Connection, Received};
use wellfed_model::{Answer, EventBody, Op, Pushed};
use wellfed_session::noise::Keypair;

use crate::api::error::{BridgeError, ErrorKind};
use crate::api::keys::random;
use crate::api::session::{FeedRefusal, HistoryEntry, SessionUpdate};
use crate::requests::TimeRead;
use crate::today::{Facts, event_entry, gap_entry, result};

/// How long one wait on the connection lasts before the loop looks at its
/// commands again and reads the status anew; the session keeps itself alive
/// meanwhile. So the card's meals and signal are at most this old.
const TURN: Duration = Duration::from_secs(30);
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
fn feed_due(portions: u8) -> Duration {
    FEED_PORTION * u32::from(portions) + FEED_SPARE
}

/// The device API's request id.
type RequestId = [u8; 16];

#[derive(Debug)]
pub enum Command {
    Feed {
        portions: u8,
    },
    CancelFeed {
        feed: u32,
    },
    Refresh,
    /// One op; `reply` hears its answer, `None` if none came, and is dropped
    /// unused if it never went out.
    Ask {
        op: Op,
        reply: oneshot::Sender<Option<Answer>>,
    },
    Close,
}

/// What an outstanding request was for.
enum Asked {
    Status,
    Settings,
    Plans,
    Time,
    Feed { portions: u8 },
    Cancel,
    Clock,
    Ask(oneshot::Sender<Option<Answer>>),
}

/// Run the session until Dart closes it or the feeder goes away. `update`
/// returns `false` once nobody is listening, which ends the session too.
pub async fn run(
    keys: Keypair,
    feeder: [u8; 32],
    address: SocketAddr,
    sync_clock: bool,
    mut commands: mpsc::UnboundedReceiver<Command>,
    mut update: impl FnMut(SessionUpdate) -> bool,
) {
    let mut asked = HashMap::new();
    let error = serve(
        &keys,
        feeder,
        address,
        sync_clock,
        &mut commands,
        &mut update,
        &mut asked,
    )
    .await
    .err();

    // A session that ends answers nothing still outstanding, and each of
    // those writes may have run: say so before the end.
    for what in asked.into_values() {
        match what {
            Asked::Feed { .. } => {
                update(SessionUpdate::FeedUnanswered);
            }
            Asked::Ask(reply) => {
                // Nobody waiting means nobody to tell.
                let _ = reply.send(None);
            }
            _ => {}
        }
    }
    update(SessionUpdate::Closed { error });
}

/// The session's own requests and the history as it comes in.
struct Serving<'a, U> {
    connection: Connection,
    update: &'a mut U,
    asked: &'a mut HashMap<RequestId, Asked>,
    facts: Facts,
    /// Until it ends, events only fill `batch`; after it they are news.
    backlog: Option<Backlog>,
    batch: Vec<HistoryEntry>,
    /// When this session last asked for the time by itself.
    time_asked: Instant,
    /// This session's own feed and when it is overdue, from the feeder's
    /// `Feeding` until its `Fed` or that time: a `Fed` lost with a gap never comes.
    running: Option<(u32, tokio::time::Instant)>,
}

async fn serve(
    keys: &Keypair,
    feeder: [u8; 32],
    address: SocketAddr,
    sync_clock: bool,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    update: &mut impl FnMut(SessionUpdate) -> bool,
    asked: &mut HashMap<RequestId, Asked>,
) -> Result<(), BridgeError> {
    let mut connection = Connection::open(keys, feeder, address, random()?).await?;
    if !update(SessionUpdate::Opened {
        build: connection.build().to_owned(),
        role: connection.role(),
    }) {
        return Ok(());
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
    let mut serving = Serving {
        connection,
        update,
        asked,
        facts: Facts::default(),
        backlog: Some(backlog),
        batch: Vec::new(),
        time_asked: Instant::now(),
        running: None,
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
                let (op, what) = match command {
                    Command::Close => return Ok(()),
                    Command::Refresh => (Op::Status, Asked::Status),
                    Command::Feed { portions } => (Op::Feed { portions }, Asked::Feed { portions }),
                    Command::CancelFeed { feed } => (Op::CancelFeed { feed }, Asked::Cancel),
                    Command::Ask { op, reply } => (op, Asked::Ask(reply)),
                };
                serving.request(&op, what)?;
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
    fn request(&mut self, op: &Op, what: Asked) -> Result<(), BridgeError> {
        let id = random()?;
        self.connection.request(id, op)?;
        self.asked.insert(id, what);
        Ok(())
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
        match self.facts.card() {
            Some(card) => (self.update)(SessionUpdate::Status { card }),
            None => true,
        }
    }

    /// Read the status again, unless a read is already on its way.
    fn refresh(&mut self) -> Result<(), BridgeError> {
        if !self
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
            self.asked.values().any(|what| matches!(what, Asked::Time)),
            self.running.is_some()
                || self
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

    /// Nothing came for a whole wait.
    fn turn(&mut self) -> Result<bool, BridgeError> {
        if self
            .running
            .is_some_and(|(_, due)| due <= tokio::time::Instant::now())
        {
            self.running = None;
            self.connection.set_silence(wellfed_connect::SILENCE);
        }
        if self.backlog.is_some() && !self.end_backlog() {
            return Ok(false);
        }
        self.refresh()?;
        self.reread_time()?;
        Ok(true)
    }

    fn answered(&mut self, id: RequestId, answer: Option<Answer>) -> Result<bool, BridgeError> {
        let Some(what) = self.asked.remove(&id) else {
            return Ok(true);
        };
        match (what, answer) {
            (Asked::Status, Some(Answer::Status(status))) => {
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
                self.facts.time = Some(TimeRead::just_read(time));
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
                let _ = reply.send(answer);
                Ok(true)
            }
            (Asked::Clock, Some(Answer::Done { .. })) => {
                self.request(&Op::GetTime, Asked::Time)?;
                Ok((self.update)(SessionUpdate::ClockSynced))
            }
            (Asked::Clock, _) => Ok((self.update)(SessionUpdate::ClockNotSynced)),
            (what, answer) => {
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
        if let Some(feed) = fed {
            self.facts.feeds.push(feed);
            if self.running.is_some_and(|(ours, _)| ours == feed.feed) {
                self.running = None;
                self.connection.set_silence(wellfed_connect::SILENCE);
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
        (Asked::Feed { portions }, Some(Answer::Feeding { feed })) => {
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

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use wellfed_model::{FeedOutcome, FeedReason, FoodLevel, Generation, MissedReason, Source};

    use super::*;

    /// Past this a feeder that stayed on ended the feed itself: its largest
    /// request, with every reversal and homing, still fits.
    #[test]
    fn a_feed_s_due_time_covers_what_the_feeder_can_take() {
        assert_eq!(feed_due(2), Duration::from_secs(49));
        assert_eq!(feed_due(48), Duration::from_secs(601));
    }

    #[test]
    fn the_time_is_read_again_about_once_a_minute_and_never_twice_at_once() {
        // A turn comes at least 30 s after the last, so the second is due.
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
