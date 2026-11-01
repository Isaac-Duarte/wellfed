//! What a watcher tells the owner: events, status, the connection and time
//! go in, and alerts opened and cleared come out. The app, a server and any
//! other watcher run this one copy, so they cannot disagree about what is
//! wrong.
//!
//! A notification only reports. Nothing here builds an `Op`, so no rule can
//! feed, retry or change the feeder.
//!
//! Sans-IO: time is always a parameter and the caller keeps [`State`], which
//! is serializable, so a watcher resumes across restarts with its timers and
//! its dedupe intact.
//!
//! An occurrence is one alert. A [`Key`] names it, and once it has opened it
//! never opens again, even after it clears or is acknowledged. [`Engine::open`]
//! is exactly what [`Change`]s have announced and not yet cleared: a kind the
//! owner turned off is still tracked, but says nothing either way.
//!
//! A meal with no record is the one alert computed rather than reported: a
//! meal held for no clock emits nothing, so the plans and the watcher's own
//! time are the only catch. It works with no feeder clock. It stays quiet
//! unless no record is actually known: the link must be up, a status read
//! since it came up, and every event that status counts delivered.
//!
//! A new engine does not replay history, whatever the caller subscribes to:
//! until a status gives it a baseline every event is ignored, and then those
//! at or below the baseline are. The caller subscribes after
//! [`Engine::subscribe_after`].
//!
//! Food has no empty reading. The beam sits about five portions above the
//! rotor, so [`AlertKind::FoodEmpty`] means likely out: that many confirmed
//! since the beam last read low. Word it that way.

use std::collections::BTreeSet;

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use wellfed_model::{
    Battery, Blocked, ClientRole, Desiccant, EnrollPath, Event, EventBody, FeedOutcome, FoodLevel,
    Gap, Generation, Hex, MissedReason, PLANS, Plan, Plans, Readiness, Source, Status, Supply,
    UpdateStop, UpdateView,
};

use crate::history::Feed;
use crate::plans::{MealState, today};
use crate::time::latest_due;

/// How long a feed is kept: long enough for a meal late on one local day to
/// be judged on the next.
const FEEDS_KEPT: SignedDuration = SignedDuration::from_hours(48);
/// How long a dedupe mark is kept. It only has to outlast any replay of the
/// same event or occurrence; after that the cursor has passed it.
const MARKS_KEPT: SignedDuration = SignedDuration::from_hours(8 * 24);
/// How long an alert that reports something finished stays open unread.
const NEWS_LIFE: SignedDuration = SignedDuration::from_hours(24);
/// How far past its occurrence a feed can still be that occurrence's record.
const RECORD_WINDOW: SignedDuration = SignedDuration::from_hours(24);
/// How long the adapter stays back before an outage is over with.
const POWER_BACK: SignedDuration = SignedDuration::from_hours(24);
/// How long without one before restarts are no longer repeating.
const RESTARTS_QUIET: SignedDuration = SignedDuration::from_hours(24);
/// Portions confirmed past a low reading that likely emptied the hopper:
/// what lies between the beam and the rotor.
const PORTIONS_BELOW_THE_BEAM: u32 = 5;
/// `BOOT_Reason`'s system, watchdog, brownout, KM4 system and KM4 watchdog
/// bits: a boot no power-on or deep-sleep wake explains. The firmware's own
/// restarts lapse the watchdog too, so a deliberate reboot or an install
/// counts, and after a real brownout a later boot can carry its bit stale.
const UNPLANNED_RESET: u32 = 0x1f << 16;

/// How soon an alert wants the owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Urgent,
    Soon,
    Security,
    Optional,
}

/// What an alert is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    MealFailed,
    MealUncertain,
    MealMissed,
    MealNoRecord,
    NoClock,
    CannotFeed,
    FoodEmpty,
    Unreachable,
    FoodLow,
    DesiccantDue,
    PowerWasOut,
    BatteryBelowFloor,
    UpdateFailed,
    RolledBack,
    RepeatedRestarts,
    ClientJoined,
    ClientRevoked,
    HistoryLost,
    DailyDigest,
    MealFed,
    MealSoon,
    ManualFeed,
}

impl AlertKind {
    pub const fn level(self) -> Level {
        match self {
            Self::MealFailed
            | Self::MealUncertain
            | Self::MealMissed
            | Self::MealNoRecord
            | Self::NoClock
            | Self::CannotFeed
            | Self::FoodEmpty
            | Self::Unreachable => Level::Urgent,
            Self::FoodLow
            | Self::DesiccantDue
            | Self::PowerWasOut
            | Self::BatteryBelowFloor
            | Self::UpdateFailed
            | Self::RolledBack
            | Self::RepeatedRestarts
            | Self::HistoryLost => Level::Soon,
            Self::ClientJoined | Self::ClientRevoked => Level::Security,
            Self::DailyDigest | Self::MealFed | Self::MealSoon | Self::ManualFeed => {
                Level::Optional
            }
        }
    }

    /// Whether the next scheduled feed that finishes closes it: meals are
    /// running again, whatever this one was about.
    const fn ends_with_a_good_meal(self) -> bool {
        matches!(
            self,
            Self::MealFailed
                | Self::MealUncertain
                | Self::MealMissed
                | Self::MealNoRecord
                | Self::MealFed
                | Self::ManualFeed
        )
    }

    /// Whether it reports something already over, so reading it closes it.
    /// Any other alert stays open until its condition ends.
    const fn ends_when_acknowledged(self) -> bool {
        matches!(
            self,
            Self::MealFailed
                | Self::MealUncertain
                | Self::MealMissed
                | Self::MealFed
                | Self::ManualFeed
                | Self::ClientJoined
                | Self::ClientRevoked
                | Self::HistoryLost
                | Self::PowerWasOut
        )
    }

    /// Whether it lapses unread after [`NEWS_LIFE`]. A meal that went wrong
    /// does not: it holds until a meal goes right or the owner reads it.
    const fn lapses(self) -> bool {
        matches!(
            self,
            Self::MealFed
                | Self::ManualFeed
                | Self::ClientJoined
                | Self::ClientRevoked
                | Self::HistoryLost
        )
    }
}

/// Which occurrence of its kind an alert is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Occurrence {
    /// One feed: the feeder numbers them within an epoch.
    Feed { epoch: u32, feed: u32 },
    /// One plan's occurrence, the same instant in any epoch.
    Meal { plan: u8, due: Timestamp },
    /// One event, or the end of a gap.
    Event { epoch: u32, seq: u64 },
    /// A condition that held from `at`, when it became an alert.
    Since { at: Timestamp },
    /// One local day's digest.
    Day { date: Date },
}

/// An alert's identity: the same key is the same alert, across restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Key {
    pub kind: AlertKind,
    pub occurrence: Occurrence,
}

/// What an alert has to say beyond its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Detail {
    /// The feed as its event recorded it, with its reason and portions. Its
    /// `at` is `None` if the feeder had no clock.
    Feed(Feed),
    /// A plan's occurrence that is coming, or has no record.
    Meal {
        plan: u8,
        due: Timestamp,
        portions: u8,
    },
    /// Nothing beyond the kind.
    None,
    /// What keeps the feeder from feeding, as the alert opened.
    Blocked {
        reason: Blocked,
    },
    /// Portions confirmed since the beam read low.
    Portions {
        confirmed: u32,
    },
    /// Days left of the desiccant's period: 0 or below.
    Desiccant {
        days: i32,
    },
    Battery(Battery),
    Stopped(UpdateStop),
    Joined {
        client: Hex<4>,
        role: ClientRole,
        path: EnrollPath,
    },
    Revoked {
        client: Hex<4>,
        by: Source,
    },
    /// Events that left the feeder's ring unread.
    Gap {
        from: u64,
        to: u64,
    },
    /// Events the feeder could not record.
    Lost {
        count: u16,
    },
    /// Since the last digest: feeds that finished, and urgent alerts.
    Digest {
        fed: u32,
        problems: u32,
    },
    /// A plan's occurrence the feeder says will never run.
    Missed {
        plan: u8,
        due: Timestamp,
        reason: MissedReason,
    },
}

/// One alert.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    pub key: Key,
    pub kind: AlertKind,
    pub level: Level,
    /// When the watcher learned of it, by the watcher's clock. What it
    /// reports may be older: see [`Detail`].
    pub opened_at: Timestamp,
    pub acknowledged: bool,
    pub detail: Detail,
}

impl Alert {
    const fn new(key: Key, detail: Detail, now: Timestamp) -> Self {
        Self {
            key,
            kind: key.kind,
            level: key.kind.level(),
            opened_at: now,
            acknowledged: false,
            detail,
        }
    }
}

/// What one observation changed for the owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Opened(Alert),
    Cleared(Key),
}

/// How the watcher's own connection to the feeder stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Link {
    Connected,
    /// It tried, and the feeder did not answer.
    Failed,
    /// It is not where it could reach the feeder, which says nothing about
    /// the feeder.
    Away,
}

/// What the watcher saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observation<'a> {
    Event(Event),
    Gap(Gap),
    Status(&'a Status),
    Plans(Plans),
    Update(&'a UpdateView),
    Link(Link),
    /// Time passed. Timers only move when something is observed, so a
    /// watcher ticks at least once a minute while it runs.
    Tick,
}

/// When a state becomes an alert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thresholds {
    pub unreachable: SignedDuration,
    pub no_clock: SignedDuration,
    pub cannot_feed: SignedDuration,
    /// After a meal is due, how long before no record of it is news: the
    /// feeder skips a meal 15 minutes late, and a feed then takes time to
    /// end.
    pub meal_grace: SignedDuration,
    /// How old a status may be while a meal is judged: past this the
    /// watcher no longer knows what the feeder has recorded.
    pub status_fresh: SignedDuration,
    /// How long bank 0 must keep answering after bank 1 before it is a
    /// rollback, not an install on its way through.
    pub rolled_back: SignedDuration,
    /// Restarts that are repeated, within `restart_window`.
    pub restarts: u8,
    pub restart_window: SignedDuration,
    /// The local hour the daily digest is due from.
    pub digest_hour: i8,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            unreachable: SignedDuration::from_mins(10),
            no_clock: SignedDuration::from_hours(1),
            cannot_feed: SignedDuration::from_mins(30),
            meal_grace: SignedDuration::from_mins(25),
            status_fresh: SignedDuration::from_mins(15),
            rolled_back: SignedDuration::from_mins(30),
            restarts: 3,
            restart_window: SignedDuration::from_hours(6),
            digest_hour: 8,
        }
    }
}

/// Which kinds the owner hears about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preferences {
    off: BTreeSet<AlertKind>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            off: BTreeSet::from([
                AlertKind::MealFed,
                AlertKind::MealSoon,
                AlertKind::ManualFeed,
            ]),
        }
    }
}

impl Preferences {
    pub fn enabled(&self, kind: AlertKind) -> bool {
        !self.off.contains(&kind)
    }

    pub fn enable(&mut self, kind: AlertKind) {
        self.off.remove(&kind);
    }

    pub fn disable(&mut self, kind: AlertKind) {
        self.off.insert(kind);
    }
}

/// How far the events have been read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Cursor {
    epoch: u32,
    seq: u64,
}

/// An open alert, and whether a [`Change`] announced it: one that did not
/// must not be announced as cleared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Open {
    alert: Alert,
    announced: bool,
}

/// Something that has happened and must not count twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mark {
    /// By the feed, not an alert's key: an uncertain feed's second `Fed`
    /// can end differently, and most feeds open no alert at all.
    Feed {
        epoch: u32,
        feed: u32,
    },
    Alert(Key),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Seen {
    mark: Mark,
    at: Timestamp,
}

/// All an engine remembers: the caller stores it and hands it back to
/// [`Engine::resume`]. Lists, not maps, as a JSON map's keys are strings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    preferences: Preferences,
    cursor: Option<Cursor>,
    /// The newest event when the first status came, with no cursor yet. A
    /// status names no epoch, so this cannot tell a ring wiped since.
    baseline: Option<u64>,
    open: Vec<Open>,
    seen: Vec<Seen>,
    /// Recent feeds, oldest first, each with an `at`.
    feeds: Vec<Feed>,
    /// The plans as last read.
    plans: Option<Plans>,
    /// An event said the plans changed, and they have not been read since.
    plans_stale: bool,
    /// By slot, only occurrences after this count as having no record:
    /// earlier ones fell under a plan or a history this watcher never saw.
    counted_from: [Option<Timestamp>; PLANS],
    link: Link,
    /// When the link began failing, with no connection or absence since.
    failing_since: Option<Timestamp>,
    /// When a status last arrived since the link came up.
    status_at: Option<Timestamp>,
    /// The newest event that status counted.
    newest_event: Option<u64>,
    /// When a status first read no valid clock, of those in a row.
    no_clock_since: Option<Timestamp>,
    /// When a status first read blocked, of those in a row.
    blocked_since: Option<Timestamp>,
    /// Portions confirmed since food read low; `None` unless it reads low.
    low_portions: Option<u32>,
    /// The adapter was last seen out.
    power_out: bool,
    /// When the adapter was seen back after an outage.
    power_back_at: Option<Timestamp>,
    /// The bank last seen.
    bank: Option<u8>,
    /// When the bank was seen to fall from 1 to 0, while it stays there.
    fell_back_at: Option<Timestamp>,
    /// Recent unplanned restarts, as many as it takes to alert.
    restarts: Vec<Timestamp>,
    /// When the last unplanned restart was read.
    last_restart: Option<Timestamp>,
    /// The local day of the last digest, and what the next will say.
    digest_day: Option<Date>,
    fed_since_digest: u32,
    problems_since_digest: u32,
}

impl Default for State {
    fn default() -> Self {
        Self {
            preferences: Preferences::default(),
            cursor: None,
            baseline: None,
            open: Vec::new(),
            seen: Vec::new(),
            feeds: Vec::new(),
            plans: None,
            plans_stale: false,
            counted_from: [None; PLANS],
            link: Link::Away,
            failing_since: None,
            status_at: None,
            newest_event: None,
            no_clock_since: None,
            blocked_since: None,
            low_portions: None,
            power_out: false,
            power_back_at: None,
            bank: None,
            fell_back_at: None,
            restarts: Vec::new(),
            last_restart: None,
            digest_day: None,
            fed_since_digest: 0,
            problems_since_digest: 0,
        }
    }
}

/// The rules, over a [`State`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Engine {
    state: State,
    thresholds: Thresholds,
}

impl Engine {
    pub fn new(thresholds: Thresholds) -> Self {
        Self {
            state: State::default(),
            thresholds,
        }
    }

    /// An engine going on from a stored `state`.
    pub const fn resume(state: State, thresholds: Thresholds) -> Self {
        Self { state, thresholds }
    }

    /// What to store for [`Self::resume`]. Every observation can change it.
    pub const fn state(&self) -> &State {
        &self.state
    }

    pub const fn preferences(&self) -> &Preferences {
        &self.state.preferences
    }

    pub const fn preferences_mut(&mut self) -> &mut Preferences {
        &mut self.state.preferences
    }

    /// Where a subscription resumes: the epoch and the last event read.
    pub fn cursor(&self) -> Option<(u32, u64)> {
        self.state.cursor.map(|cursor| (cursor.epoch, cursor.seq))
    }

    /// The event to subscribe after: the cursor, or with none yet the
    /// baseline. `None` until a status has been read, and before that the
    /// engine ignores any event it is given.
    pub fn subscribe_after(&self) -> Option<u64> {
        self.state.read_to()
    }

    /// The alerts [`Change`]s have opened and not cleared.
    pub fn open(&self) -> impl Iterator<Item = &Alert> {
        self.state
            .open
            .iter()
            .filter(|open| open.announced)
            .map(|open| &open.alert)
    }

    /// Takes in `observation`, seen at `now`. `zone` is the feeder's, for
    /// its local days. Rules that wait on time run on every observation, so
    /// any of them does for a tick.
    pub fn observe(
        &mut self,
        observation: Observation<'_>,
        now: Timestamp,
        zone: &TimeZone,
    ) -> Vec<Change> {
        let mut changes = Vec::new();
        let state = &mut self.state;

        match observation {
            Observation::Event(event) => {
                state.event(&event, &self.thresholds, now, &mut changes);
            }
            Observation::Gap(gap) => state.gap(gap, now, &mut changes),
            Observation::Status(status) => {
                state.status(status, &self.thresholds, now, &mut changes);
            }
            Observation::Plans(plans) => state.read_plans(plans, now),
            Observation::Update(update) => state.update(update, now, &mut changes),
            Observation::Link(link) => state.link(link, now, &mut changes),
            Observation::Tick => state.digest(&self.thresholds, now, zone, &mut changes),
        }

        state.fail_link(&self.thresholds, now, &mut changes);
        state.reach_meals(&self.thresholds, now, zone, &mut changes);
        state.lapse(now, &mut changes);
        state.prune(now);
        changes
    }

    /// The owner has read the alert `key`. One that reports something over
    /// closes, and the answer is its [`Change::Cleared`]; any other stays
    /// open, acknowledged, until its condition ends. Neither opens again
    /// for this occurrence.
    pub fn acknowledge(&mut self, key: Key) -> Option<Change> {
        let open = &mut self.state.open;
        let at = open.iter().position(|open| open.alert.key == key)?;

        if key.kind.ends_when_acknowledged() {
            let closed = open.remove(at);
            return closed.announced.then_some(Change::Cleared(key));
        }

        if let Some(open) = open.get_mut(at) {
            open.alert.acknowledged = true;
        }
        None
    }
}

impl State {
    fn event(
        &mut self,
        event: &Event,
        thresholds: &Thresholds,
        now: Timestamp,
        changes: &mut Vec<Change>,
    ) {
        if !self.advance(event.epoch, event.seq, now) {
            return;
        }

        match event.body {
            EventBody::Fed { .. } => {
                if let Some(feed) = Feed::from_event(event) {
                    self.fed(feed, now, changes);
                }
            }
            EventBody::Missed { plan, due, reason } => self.missed(plan, due, reason, now, changes),
            EventBody::MealSoon {
                plan,
                due,
                portions,
                ..
            } => self.meal_soon(plan, due, portions, now, changes),
            EventBody::Plans { generation, .. } => self.plans_changed(generation),
            EventBody::Lost { count } => {
                // Anything may have been among them, feeds too.
                self.count_from(now);
                let occurrence = Occurrence::Event {
                    epoch: event.epoch,
                    seq: event.seq,
                };
                self.raise_event(
                    AlertKind::HistoryLost,
                    occurrence,
                    Detail::Lost { count },
                    now,
                    changes,
                );
            }
            EventBody::Enrolled { client, role, path } => {
                let occurrence = Occurrence::Event {
                    epoch: event.epoch,
                    seq: event.seq,
                };
                let detail = Detail::Joined { client, role, path };
                self.raise_event(AlertKind::ClientJoined, occurrence, detail, now, changes);
            }
            EventBody::Revoked { client, by } => {
                let occurrence = Occurrence::Event {
                    epoch: event.epoch,
                    seq: event.seq,
                };
                let detail = Detail::Revoked { client, by };
                self.raise_event(AlertKind::ClientRevoked, occurrence, detail, now, changes);
            }
            EventBody::Boot { bank, reset_flags } => {
                self.see_bank(bank, now, changes);
                if reset_flags & UNPLANNED_RESET != 0 {
                    let at = event.at.unwrap_or(now);
                    self.restarted(thresholds, at, now, changes);
                }
            }
            EventBody::Supply {
                adapter,
                battery,
                below_floor,
            } => {
                let supply = Supply {
                    adapter,
                    battery,
                    below_floor,
                };
                self.supply(supply, now, changes);
            }
            EventBody::Food { level } => self.food(level, now, changes),
            EventBody::Lock { .. }
            | EventBody::Settings { .. }
            | EventBody::Wifi { .. }
            | EventBody::ClockSync { .. }
            | EventBody::FeedingBlocked { .. }
            | EventBody::DispenseRefused { .. } => {}
        }
    }

    /// The newest event this engine has accounted for.
    fn read_to(&self) -> Option<u64> {
        self.cursor.map(|cursor| cursor.seq).or(self.baseline)
    }

    /// Moves the cursor to `seq`, or refuses a duplicate. A new epoch is a
    /// new history: its numbers start again, and what the old one held is
    /// gone, but what is open stays open.
    fn advance(&mut self, epoch: u32, seq: u64, now: Timestamp) -> bool {
        match self.cursor {
            Some(cursor) if cursor.epoch == epoch && seq <= cursor.seq => return false,
            Some(cursor) if cursor.epoch != epoch => {
                self.newest_event = None;
                self.count_from(now);
            }
            // Before any cursor only what follows the baseline is news, and
            // with no baseline yet nothing can be told from history.
            None if self.baseline.is_none_or(|baseline| seq <= baseline) => return false,
            Some(_) | None => {}
        }

        self.cursor = Some(Cursor { epoch, seq });
        true
    }

    fn gap(&mut self, gap: Gap, now: Timestamp, changes: &mut Vec<Change>) {
        if !self.advance(gap.epoch, gap.to, now) {
            return;
        }

        // A feed may have left with those events, so no record says nothing
        // about any occurrence so far.
        self.count_from(now);
        let occurrence = Occurrence::Event {
            epoch: gap.epoch,
            seq: gap.to,
        };
        let detail = Detail::Gap {
            from: gap.from,
            to: gap.to,
        };
        self.raise_event(AlertKind::HistoryLost, occurrence, detail, now, changes);
    }

    /// Timers on a status judge only a status: with no status coming there
    /// is nothing to say about the feeder but that it is unreachable.
    fn status(
        &mut self,
        status: &Status,
        thresholds: &Thresholds,
        now: Timestamp,
        changes: &mut Vec<Change>,
    ) {
        self.connected(changes);
        self.status_at = Some(now);
        self.newest_event = status.last_event;
        if self.cursor.is_none() && self.baseline.is_none() {
            // Events count from 1, so a feeder with none has no history.
            self.baseline = Some(status.last_event.unwrap_or(0));
        }

        self.clock(status, thresholds, now, changes);
        self.readiness(status, thresholds, now, changes);
        self.food(status.food, now, changes);
        self.desiccant(status.desiccant, now, changes);
        if let Some(supply) = status.supply {
            self.supply(supply, now, changes);
        }

        self.see_bank(status.bank, now, changes);
        if self
            .fell_back_at
            .is_some_and(|at| elapsed(at, now) >= thresholds.rolled_back)
        {
            self.raise_condition(AlertKind::RolledBack, Detail::None, now, changes);
        }
    }

    fn clock(
        &mut self,
        status: &Status,
        thresholds: &Thresholds,
        now: Timestamp,
        changes: &mut Vec<Change>,
    ) {
        if status.clock.is_some() {
            self.no_clock_since = None;
            self.clear(|alert| alert.kind == AlertKind::NoClock, changes);
            return;
        }

        let since = *self.no_clock_since.get_or_insert(now);
        if elapsed(since, now) >= thresholds.no_clock {
            self.raise_condition(AlertKind::NoClock, Detail::None, now, changes);
        }
    }

    fn readiness(
        &mut self,
        status: &Status,
        thresholds: &Thresholds,
        now: Timestamp,
        changes: &mut Vec<Change>,
    ) {
        match status.readiness {
            Some(Readiness::Ready) => {
                self.blocked_since = None;
                self.clear(|alert| alert.kind == AlertKind::CannotFeed, changes);
            }
            Some(Readiness::Blocked(reason)) => {
                let since = *self.blocked_since.get_or_insert(now);
                if elapsed(since, now) >= thresholds.cannot_feed {
                    let detail = Detail::Blocked { reason };
                    self.raise_condition(AlertKind::CannotFeed, detail, now, changes);
                }
            }
            None => {}
        }
    }

    fn food(&mut self, level: FoodLevel, now: Timestamp, changes: &mut Vec<Change>) {
        match level {
            FoodLevel::Low => {
                self.low_portions.get_or_insert(0);
                self.raise_condition(AlertKind::FoodLow, Detail::None, now, changes);
            }
            FoodLevel::Present => {
                self.low_portions = None;
                self.clear(
                    |alert| matches!(alert.kind, AlertKind::FoodLow | AlertKind::FoodEmpty),
                    changes,
                );
            }
            FoodLevel::Unread => {}
        }
    }

    /// Without a clock the days cannot be counted, which says nothing about
    /// the desiccant, so an open alert holds through a cold boot.
    fn desiccant(&mut self, desiccant: Desiccant, now: Timestamp, changes: &mut Vec<Change>) {
        match desiccant {
            Desiccant::Remaining { days, due: true } => {
                let detail = Detail::Desiccant { days };
                self.raise_condition(AlertKind::DesiccantDue, detail, now, changes);
            }
            Desiccant::Remaining { due: false, .. } | Desiccant::Unset => {
                self.clear(|alert| alert.kind == AlertKind::DesiccantDue, changes);
            }
            Desiccant::Unknown => {}
        }
    }

    /// The feeder drops Wi-Fi on its batteries, so an outage is usually read
    /// after it ended. `below_floor` is `None` with no pack, but an older
    /// build judged the empty holder's near-zero channel: on the adapter it
    /// counts only with a pack that reads, and off the adapter it always
    /// does, as the feeder is then running on whatever is there.
    fn supply(&mut self, supply: Supply, now: Timestamp, changes: &mut Vec<Change>) {
        match supply.adapter {
            Some(false) => {
                self.power_back_at = None;
                if !self.power_out {
                    self.power_out = true;
                    self.raise_condition(AlertKind::PowerWasOut, Detail::None, now, changes);
                }
            }
            Some(true) if self.power_out => {
                self.power_out = false;
                self.power_back_at = Some(now);
            }
            Some(true) | None => {}
        }

        let pack = matches!(supply.battery, Battery::Reading { .. });
        match supply.below_floor {
            Some(true) if pack || supply.adapter == Some(false) => {
                let detail = Detail::Battery(supply.battery);
                self.raise_condition(AlertKind::BatteryBelowFloor, detail, now, changes);
            }
            Some(_) => self.clear(|alert| alert.kind == AlertKind::BatteryBelowFloor, changes),
            None => {}
        }
    }

    /// A cancelled install was the owner's doing, and a new stop replaces
    /// the one before, so it ends the alert as no stop does.
    fn update(&mut self, update: &UpdateView, now: Timestamp, changes: &mut Vec<Change>) {
        match update.stopped {
            Some(UpdateStop::Cancelled) | None => {
                self.clear(|alert| alert.kind == AlertKind::UpdateFailed, changes);
            }
            Some(stop) => {
                let detail = Detail::Stopped(stop);
                self.raise_condition(AlertKind::UpdateFailed, detail, now, changes);
            }
        }
    }

    /// Falling from bank 1 to bank 0 starts the wait; bank 1 ends it. The
    /// bank alone cannot say whether a boot followed an install in progress.
    fn see_bank(&mut self, bank: u8, now: Timestamp, changes: &mut Vec<Change>) {
        if bank == 1 {
            self.fell_back_at = None;
            self.clear(|alert| alert.kind == AlertKind::RolledBack, changes);
        } else if bank == 0 && self.bank == Some(1) {
            self.fell_back_at = Some(now);
        }
        self.bank = Some(bank);
    }

    /// `at` is when it happened, so restarts read late still count as close
    /// together; `now` starts the quiet time.
    fn restarted(
        &mut self,
        thresholds: &Thresholds,
        at: Timestamp,
        now: Timestamp,
        changes: &mut Vec<Change>,
    ) {
        self.last_restart = Some(now);
        self.restarts
            .retain(|restart| elapsed(*restart, at) <= thresholds.restart_window);
        self.restarts.push(at);

        let needed = usize::from(thresholds.restarts);
        if self.restarts.len() >= needed {
            self.raise_condition(AlertKind::RepeatedRestarts, Detail::None, now, changes);
        }
        let surplus = self.restarts.len().saturating_sub(needed);
        self.restarts.drain(..surplus);
    }

    fn connected(&mut self, changes: &mut Vec<Change>) {
        self.link = Link::Connected;
        self.failing_since = None;
        self.clear(|alert| alert.kind == AlertKind::Unreachable, changes);
    }

    fn link(&mut self, link: Link, now: Timestamp, changes: &mut Vec<Change>) {
        // A status from before the drop says nothing of what was recorded
        // since, so it takes a new one to judge a meal.
        if link != Link::Connected || self.link != Link::Connected {
            self.status_at = None;
        }

        match link {
            Link::Connected => self.connected(changes),
            Link::Failed => {
                self.link = link;
                self.failing_since.get_or_insert(now);
            }
            // Being out must never read as a dead feeder: the timer starts
            // again, and only a connection says anything about what is open.
            Link::Away => {
                self.link = link;
                self.failing_since = None;
            }
        }
    }

    fn fail_link(&mut self, thresholds: &Thresholds, now: Timestamp, changes: &mut Vec<Change>) {
        if self
            .failing_since
            .is_some_and(|since| elapsed(since, now) >= thresholds.unreachable)
        {
            self.raise_condition(AlertKind::Unreachable, Detail::None, now, changes);
        }
    }

    /// Once a local day from the digest hour. A clock stepping back to an
    /// earlier day does not send it again.
    fn digest(
        &mut self,
        thresholds: &Thresholds,
        now: Timestamp,
        zone: &TimeZone,
        changes: &mut Vec<Change>,
    ) {
        let local = now.to_zoned(zone.clone());
        let date = local.date();
        let due = local.hour() >= thresholds.digest_hour
            && self.digest_day.is_none_or(|last| last < date);
        if !due {
            return;
        }

        self.clear(|alert| alert.kind == AlertKind::DailyDigest, changes);
        let key = Key {
            kind: AlertKind::DailyDigest,
            occurrence: Occurrence::Day { date },
        };
        let detail = Detail::Digest {
            fed: self.fed_since_digest,
            problems: self.problems_since_digest,
        };
        self.raise(Alert::new(key, detail, now), changes);

        self.digest_day = Some(date);
        self.fed_since_digest = 0;
        self.problems_since_digest = 0;
    }

    /// The feeder's generation moves for more than a table replaced: taking
    /// a once plan disables it with no event at all. So a slot starts afresh
    /// only by the feeder's own rule, when its times changed or it came back
    /// on, and a change to one plan leaves the others' occurrences counted.
    fn read_plans(&mut self, plans: Plans, now: Timestamp) {
        let held = self.plans.map(|held| held.plans);

        for (slot, new) in plans.plans.iter().enumerate() {
            let old = held.and_then(|held| held.get(slot).copied().flatten());
            let Some(from) = self.counted_from.get_mut(slot) else {
                continue;
            };
            *from = match (new, old) {
                (None, _) => None,
                (Some(new), Some(old)) if keeps_occurrences(new, &old) => *from,
                // A clock stepping back must not reopen what was ruled out.
                (Some(_), _) => Some(from.map_or(now, |from| from.max(now))),
            };
        }

        self.plans = Some(plans);
        self.plans_stale = false;
    }

    /// An event can be replayed from behind the table already read: only a
    /// newer one makes that table stale.
    fn plans_changed(&mut self, generation: Generation) {
        if self.plans.is_some_and(|held| generation > held.generation) {
            self.plans_stale = true;
        }
    }

    /// The feeder disables a once plan as it takes it, unannounced. Until
    /// the table is next read this keeps it from reading as tomorrow's meal.
    fn spend_once(&mut self, slot: u8) {
        if let Some(plans) = &mut self.plans
            && let Some(Some(plan)) = plans.plans.get_mut(usize::from(slot))
            && plan.days.once()
        {
            plan.enabled = false;
        }
    }

    /// Stops counting occurrences up to `now`, for every plan being counted.
    fn count_from(&mut self, now: Timestamp) {
        for from in self.counted_from.iter_mut().flatten() {
            *from = (*from).max(now);
        }
    }

    fn fed(&mut self, feed: Feed, now: Timestamp, changes: &mut Vec<Change>) {
        let (epoch, id) = (feed.epoch, feed.feed);
        if !self.mark(Mark::Feed { epoch, feed: id }, now) {
            return;
        }

        // With no clock when it ended, the watcher's own time stands in. It
        // is only ever later than the feed, which is all a record of an
        // earlier occurrence needs.
        let stamp = feed.at.unwrap_or(now);
        self.feeds.push(Feed {
            at: Some(stamp),
            ..feed
        });

        if let Some(Source::Schedule { plan }) = feed.by {
            self.spend_once(plan);
            self.clear(|alert| no_record_of(alert, plan, stamp), changes);
        }
        if feed.outcome == FeedOutcome::Done {
            self.fed_since_digest = self.fed_since_digest.saturating_add(1);
        }
        if let Some(low) = self.low_portions {
            let confirmed = low.saturating_add(u32::from(feed.delivered.bounds().0));
            self.low_portions = Some(confirmed);
            if confirmed >= PORTIONS_BELOW_THE_BEAM {
                let detail = Detail::Portions { confirmed };
                self.raise_condition(AlertKind::FoodEmpty, detail, now, changes);
            }
        }

        let kind = match (feed.outcome, feed.by) {
            (FeedOutcome::Failed | FeedOutcome::Expired, _) => Some(AlertKind::MealFailed),
            (FeedOutcome::Uncertain, _) => Some(AlertKind::MealUncertain),
            (FeedOutcome::Done, Some(Source::Schedule { .. })) => {
                self.clear(|alert| alert.kind.ends_with_a_good_meal(), changes);
                Some(AlertKind::MealFed)
            }
            (FeedOutcome::Done, Some(Source::Client { .. } | Source::Panel)) => {
                Some(AlertKind::ManualFeed)
            }
            (FeedOutcome::Done, Some(Source::Feeder) | None) | (FeedOutcome::Cancelled, _) => None,
        };

        if let Some(kind) = kind {
            let occurrence = Occurrence::Feed { epoch, feed: id };
            let alert = Alert::new(Key { kind, occurrence }, Detail::Feed(feed), now);
            self.raise(alert, changes);
        }
    }

    fn missed(
        &mut self,
        plan: u8,
        due: Timestamp,
        reason: MissedReason,
        now: Timestamp,
        changes: &mut Vec<Change>,
    ) {
        self.spend_once(plan);
        let occurrence = Occurrence::Meal { plan, due };
        let no_record = Key {
            kind: AlertKind::MealNoRecord,
            occurrence,
        };
        self.clear(|alert| alert.key == no_record, changes);

        let key = Key {
            kind: AlertKind::MealMissed,
            occurrence,
        };
        let detail = Detail::Missed { plan, due, reason };
        self.raise_once(key, detail, now, changes);
    }

    fn meal_soon(
        &mut self,
        plan: u8,
        due: Timestamp,
        portions: u8,
        now: Timestamp,
        changes: &mut Vec<Change>,
    ) {
        // Read late, it is no longer soon. It would only open and clear.
        if due <= now {
            return;
        }

        let key = Key {
            kind: AlertKind::MealSoon,
            occurrence: Occurrence::Meal { plan, due },
        };
        let detail = Detail::Meal {
            plan,
            due,
            portions,
        };
        self.raise_once(key, detail, now, changes);
    }

    /// Whether a missing record means anything yet. The cursor against the
    /// status catches the moment after a reconnect, before the backlog
    /// holding the meal's feed has arrived. A ring wiped since the status
    /// can pass this for a moment, as a status names no epoch: the first
    /// event of the new epoch rules those meals out, and until it arrives a
    /// meal can read as having no record.
    fn knows_the_record(&self, thresholds: &Thresholds, now: Timestamp) -> bool {
        let fresh = self
            .status_at
            .is_some_and(|at| elapsed(at, now) <= thresholds.status_fresh);
        let read_to = self.read_to();
        let caught_up = self
            .newest_event
            .is_none_or(|newest| read_to.is_some_and(|read_to| read_to >= newest));

        self.link == Link::Connected && fresh && caught_up
    }

    /// Opens an alert for each occurrence that has reached its grace with
    /// no record. Judged as of `now` less the grace, so an occurrence is
    /// reached exactly at due plus grace, on either side of midnight. The
    /// end of the day before is judged too, since a hold or a stopped
    /// watcher can run past midnight; the marks keep that from repeating.
    fn reach_meals(
        &mut self,
        thresholds: &Thresholds,
        now: Timestamp,
        zone: &TimeZone,
        changes: &mut Vec<Change>,
    ) {
        if self.plans_stale || !self.knows_the_record(thresholds, now) {
            return;
        }
        let Some(plans) = self.plans else {
            return;
        };
        let Ok(reached) = now.checked_sub(thresholds.meal_grace) else {
            return;
        };

        let day_before = reached
            .to_zoned(zone.clone())
            .start_of_day()
            .ok()
            .and_then(|start| {
                start
                    .timestamp()
                    .checked_sub(SignedDuration::from_secs(1))
                    .ok()
            });
        for moment in day_before.into_iter().chain([reached]) {
            for meal in today(&plans, &self.feeds, moment, zone) {
                if meal.state != MealState::Passed {
                    continue;
                }

                // The feeder's reckoning, not the meal's minute: in a skipped
                // hour `today` calls a meal passed before it is due, and this
                // then lands on the day before, which has its record.
                let plan = plans.plans.get(usize::from(meal.slot)).copied().flatten();
                let Some(due) = plan.and_then(|plan| latest_due(&plan, moment, zone)) else {
                    continue;
                };
                let counted = self
                    .counted_from
                    .get(usize::from(meal.slot))
                    .copied()
                    .flatten()
                    .is_some_and(|from| due > from);
                if counted && !self.recorded(meal.slot, due) {
                    let key = Key {
                        kind: AlertKind::MealNoRecord,
                        occurrence: Occurrence::Meal {
                            plan: meal.slot,
                            due,
                        },
                    };
                    let detail = Detail::Meal {
                        plan: meal.slot,
                        due,
                        portions: meal.portions,
                    };
                    self.raise_once(key, detail, now, changes);
                }
            }
        }
    }

    /// Whether the feeder recorded anything of this occurrence. `today`
    /// looks only within a local day, and a feed can end past midnight.
    fn recorded(&self, plan: u8, due: Timestamp) -> bool {
        let missed = Mark::Alert(Key {
            kind: AlertKind::MealMissed,
            occurrence: Occurrence::Meal { plan, due },
        });
        let fed = self.feeds.iter().any(|feed| {
            feed.by == Some(Source::Schedule { plan }) && feed.at.is_some_and(|at| at >= due)
        });

        fed || self.seen.iter().any(|seen| seen.mark == missed)
    }

    /// Closes what time alone ends. A meal problem goes last, with the
    /// marks, so a dead feeder cannot grow the state without limit.
    fn lapse(&mut self, now: Timestamp, changes: &mut Vec<Change>) {
        self.clear(
            |alert| {
                let age = elapsed(alert.opened_at, now);
                match (alert.kind, alert.key.occurrence) {
                    (AlertKind::MealSoon, Occurrence::Meal { due, .. }) => due <= now,
                    (kind, _) if kind.lapses() => age >= NEWS_LIFE,
                    (kind, _) => kind.ends_with_a_good_meal() && age >= MARKS_KEPT,
                }
            },
            changes,
        );

        if self
            .power_back_at
            .is_some_and(|at| elapsed(at, now) >= POWER_BACK)
        {
            self.power_back_at = None;
            self.clear(|alert| alert.kind == AlertKind::PowerWasOut, changes);
        }
        if self
            .last_restart
            .is_some_and(|at| elapsed(at, now) >= RESTARTS_QUIET)
        {
            self.last_restart = None;
            self.restarts.clear();
            self.clear(|alert| alert.kind == AlertKind::RepeatedRestarts, changes);
        }
    }

    fn prune(&mut self, now: Timestamp) {
        self.feeds
            .retain(|feed| feed.at.is_some_and(|at| elapsed(at, now) <= FEEDS_KEPT));
        self.seen.retain(|seen| elapsed(seen.at, now) <= MARKS_KEPT);
    }

    /// Records `mark`, and says whether it was new.
    fn mark(&mut self, mark: Mark, now: Timestamp) -> bool {
        if self.seen.iter().any(|seen| seen.mark == mark) {
            return false;
        }
        self.seen.push(Seen { mark, at: now });
        true
    }

    fn raise_once(&mut self, key: Key, detail: Detail, now: Timestamp, changes: &mut Vec<Change>) {
        if self.mark(Mark::Alert(key), now) {
            self.raise(Alert::new(key, detail, now), changes);
        }
    }

    /// The event's own number makes it one alert, so it needs no mark.
    fn raise_event(
        &mut self,
        kind: AlertKind,
        occurrence: Occurrence,
        detail: Detail,
        now: Timestamp,
        changes: &mut Vec<Change>,
    ) {
        self.raise(Alert::new(Key { kind, occurrence }, detail, now), changes);
    }

    /// One alert for as long as the condition lasts, acknowledged or not.
    fn raise_condition(
        &mut self,
        kind: AlertKind,
        detail: Detail,
        now: Timestamp,
        changes: &mut Vec<Change>,
    ) {
        if self.open.iter().any(|open| open.alert.kind == kind) {
            return;
        }

        let occurrence = Occurrence::Since { at: now };
        self.raise(Alert::new(Key { kind, occurrence }, detail, now), changes);
    }

    fn raise(&mut self, alert: Alert, changes: &mut Vec<Change>) {
        if alert.level == Level::Urgent {
            self.problems_since_digest = self.problems_since_digest.saturating_add(1);
        }

        let announced = self.preferences.enabled(alert.kind);
        if announced {
            changes.push(Change::Opened(alert));
        }
        self.open.push(Open { alert, announced });
    }

    fn clear(&mut self, ended: impl Fn(&Alert) -> bool, changes: &mut Vec<Change>) {
        self.open.retain(|open| {
            if !ended(&open.alert) {
                return true;
            }
            if open.announced {
                changes.push(Change::Cleared(open.alert.key));
            }
            false
        });
    }
}

/// Whether a plan kept its occurrences through a change: the feeder's own
/// rule for its marks. Portions alone keep them, and so does turning off,
/// but turning back on starts afresh.
const fn keeps_occurrences(new: &Plan, old: &Plan) -> bool {
    new.minute == old.minute && new.days.bits() == old.days.bits() && (old.enabled || !new.enabled)
}

/// Whether `alert` says plan `plan` has no record of an occurrence a feed
/// stamped `stamp` can be the record of. A later day's feed is not.
fn no_record_of(alert: &Alert, plan: u8, stamp: Timestamp) -> bool {
    let Occurrence::Meal { plan: slot, due } = alert.key.occurrence else {
        return false;
    };

    alert.kind == AlertKind::MealNoRecord
        && slot == plan
        && due <= stamp
        && elapsed(due, stamp) < RECORD_WINDOW
}

/// The time from `from` to `now`, never negative: a clock can step back.
fn elapsed(from: Timestamp, now: Timestamp) -> SignedDuration {
    now.duration_since(from).max(SignedDuration::ZERO)
}

#[cfg(test)]
mod tests;
