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

use std::collections::BTreeSet;

use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use wellfed_model::{
    Event, EventBody, FeedOutcome, Gap, Generation, MissedReason, Plans, Source, Status, UpdateView,
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
        )
    }

    /// Whether it lapses unread after [`NEWS_LIFE`]. A meal that went wrong
    /// does not: it holds until a meal goes right or the owner reads it.
    const fn lapses(self) -> bool {
        matches!(self, Self::MealFed | Self::ManualFeed)
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
    open: Vec<Open>,
    seen: Vec<Seen>,
    /// Recent feeds, oldest first, each with an `at`.
    feeds: Vec<Feed>,
    /// The plans as last read; `None` once an event says they changed.
    plans: Option<Plans>,
    /// Only occurrences after this count as having no record: earlier ones
    /// fell under plans or history this watcher never saw.
    counted_from: Option<Timestamp>,
    link: Link,
    /// When a status last arrived since the link came up.
    status_at: Option<Timestamp>,
    /// The newest event that status counted.
    newest_event: Option<u64>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            preferences: Preferences::default(),
            cursor: None,
            open: Vec::new(),
            seen: Vec::new(),
            feeds: Vec::new(),
            plans: None,
            counted_from: None,
            link: Link::Away,
            status_at: None,
            newest_event: None,
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
            Observation::Event(event) => state.event(&event, now, &mut changes),
            Observation::Gap(gap) => state.gap(gap, now),
            Observation::Status(status) => state.status(status, now),
            Observation::Plans(plans) => state.read_plans(plans, now),
            Observation::Link(link) => state.link(link),
            Observation::Update(_) | Observation::Tick => {}
        }

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
    fn event(&mut self, event: &Event, now: Timestamp, changes: &mut Vec<Change>) {
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
            // Anything may have been among them, feeds too.
            EventBody::Lost { .. } => self.count_from(now),
            EventBody::Boot { .. }
            | EventBody::Lock { .. }
            | EventBody::Settings { .. }
            | EventBody::Wifi { .. }
            | EventBody::Enrolled { .. }
            | EventBody::Revoked { .. }
            | EventBody::ClockSync { .. }
            | EventBody::FeedingBlocked { .. }
            | EventBody::Supply { .. }
            | EventBody::DispenseRefused { .. }
            | EventBody::Food { .. } => {}
        }
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
            Some(_) | None => {}
        }

        self.cursor = Some(Cursor { epoch, seq });
        true
    }

    fn gap(&mut self, gap: Gap, now: Timestamp) {
        // A feed may have left with those events, so no record says nothing
        // about any occurrence so far.
        if self.advance(gap.epoch, gap.to, now) {
            self.count_from(now);
        }
    }

    fn status(&mut self, status: &Status, now: Timestamp) {
        self.link = Link::Connected;
        self.status_at = Some(now);
        self.newest_event = status.last_event;
    }

    fn link(&mut self, link: Link) {
        // A status from before the drop says nothing of what was recorded
        // since, so it takes a new one to judge a meal.
        if link != Link::Connected || self.link != Link::Connected {
            self.status_at = None;
        }
        self.link = link;
    }

    /// The feeder holds the generation, so any change of it is a new table,
    /// whichever way it moved: a wiped store counts again from the start.
    fn read_plans(&mut self, plans: Plans, now: Timestamp) {
        let same = self
            .plans
            .is_some_and(|held| held.generation == plans.generation);

        if !same {
            self.counted_from = Some(self.later_than_counted(now));
        }
        self.plans = Some(plans);
    }

    /// An event can be replayed from behind the table already read: only a
    /// newer one makes that table stale.
    fn plans_changed(&mut self, generation: Generation) {
        if self.plans.is_some_and(|held| generation > held.generation) {
            self.plans = None;
        }
    }

    /// Stops counting occurrences up to `now`, if any were being counted.
    fn count_from(&mut self, now: Timestamp) {
        if self.counted_from.is_some() {
            self.counted_from = Some(self.later_than_counted(now));
        }
    }

    /// A clock stepping back must not reopen what was ruled out.
    fn later_than_counted(&self, now: Timestamp) -> Timestamp {
        self.counted_from.map_or(now, |from| from.max(now))
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
            self.clear(|alert| no_record_of(alert, plan, stamp), changes);
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
    /// holding the meal's feed has arrived.
    fn knows_the_record(&self, thresholds: &Thresholds, now: Timestamp) -> bool {
        let fresh = self
            .status_at
            .is_some_and(|at| elapsed(at, now) <= thresholds.status_fresh);
        let caught_up = self
            .newest_event
            .is_none_or(|newest| self.cursor.is_some_and(|cursor| cursor.seq >= newest));

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
        if !self.knows_the_record(thresholds, now) {
            return;
        }
        let (Some(plans), Some(from)) = (self.plans, self.counted_from) else {
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
                if due > from && !self.recorded(meal.slot, due) {
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

    /// Closes what time alone ends.
    fn lapse(&mut self, now: Timestamp, changes: &mut Vec<Change>) {
        self.clear(
            |alert| match (alert.kind, alert.key.occurrence) {
                (AlertKind::MealSoon, Occurrence::Meal { due, .. }) => due <= now,
                (kind, _) => kind.lapses() && elapsed(alert.opened_at, now) >= NEWS_LIFE,
            },
            changes,
        );
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

    fn raise(&mut self, alert: Alert, changes: &mut Vec<Change>) {
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
