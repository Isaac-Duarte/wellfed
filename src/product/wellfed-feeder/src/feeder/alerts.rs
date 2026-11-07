//! The feeder's alert rules: a small RAM table of what is wrong now, which
//! every client reads, so a phone, the CLI and a server cannot disagree.
//!
//! **Read only.** The rules are told what the loop recorded and read, and
//! open or close entries here. Nothing that decides or runs a meal reads this
//! table, so a feed runs exactly as it would with the rules gone. No alert
//! writes flash.
//!
//! **Two kinds of entry.** A condition opens once its state has held past
//! its threshold and closes when the state ends. Its key is its kind alone.
//! A report opens on something that happened. A past event has no condition
//! that can end it, and the feeder takes no acknowledgment, so a report
//! lapses 48 hours after its own time. One with no time stays until room
//! runs out. Repeated restarts are a report that is tracked like a condition,
//! so that a later episode alerts again.
//!
//! **Keys come from the event's own fields.** The rules see an event before
//! the writer numbers it, and a dropped event never gets a number
//! ([`OpenAlert::key`]). A finished feed's key is completed by the ring's
//! epoch, which holds for a whole boot, so the read carries it once rather
//! than every entry.
//!
//! **Room.** Conditions always fit, as there are fewer of them than slots.
//! A report that finds no room pushes out the oldest of the least urgent
//! reports, or is dropped itself if every report is more urgent. Either way
//! `more` stays set for as long as what left could still have been listed,
//! and the same goes for a boot read that didn't reach all it could have.
//!
//! **`settling`.** A phone takes an alert absent from a complete table to
//! mean it ended. So until the boot read is done and the thresholds that
//! began at boot have run out, and while a reading a condition needs is
//! unknown, `settling` says the table can't vouch for an absence.
//!
//! **After a restart** the boot read passes the newest events of past boots
//! to [`Alerts::replay`], oldest first, then calls [`Alerts::restored`].
//! Reports come back, and so do the food count and the restarts. Conditions
//! run their thresholds again. An old ring is mostly past history, so what
//! the newest time seen shows is over never opens. This boot's own `Boot` and
//! update record reach the recorder before the read ends, and wait for it,
//! since a bank and a run of restarts only mean something in order.
//!
//! **How a boot got there.** Each boot is judged from its `UpdateAtBoot`, what
//! the update store held. That is a record, written before the restart it
//! explains, so nothing here depends on how long anything took. A boot on
//! bank 0 that follows one on bank 1 is a rollback, unless the store holds a
//! pending candidate, which is an install giving way. So is a boot on bank 0
//! that finds a candidate activated, as the build it was activated for has
//! given way. A boot with no record yet is not judged: a later boot's record
//! judges by what the store says then, and the restart counts all the same.

use core::num::NonZeroU32;

use super::CLAIM_WAIT_MS;
use wellfed_core::clock::wall::MAX_UNIX_MS;
use wellfed_device_api::mailbox::PLANS;
use wellfed_proto::api::{
    ALERTS_MAX, Alert, AlertKind, AlertLevel, AlertsView, Battery, Blocked, Desiccant, EventBody,
    FeedOutcome, FoodLevel, OpenAlert, Source, Supply, UpdateCandidate, UpdateStop,
};

/// Slots in the table. A read and the frame carry at most this many.
pub const CAPACITY: usize = ALERTS_MAX;
/// The conditions that can be open at once, repeated restarts among them.
/// There are eight, but stuck plans need a valid clock and no clock needs none.
const CONDITIONS: usize = 7;
const _: () = assert!(CONDITIONS < CAPACITY, "a condition always finds room");

const MINUTE_S: u32 = 60;
const HOUR_S: u32 = 60 * MINUTE_S;
const MINUTE_MS: u32 = 60_000;
const HOUR_MS: u32 = 60 * MINUTE_MS;

const DAY_S: u32 = 24 * HOUR_S;
/// A time past this far ahead of the wall clock isn't one.
const CLOCK_AHEAD_S: u32 = DAY_S;
/// With no clock to go by, a jump this far past the newest time seen isn't
/// one: a feeder off for longer costs at worst old reports listed a while.
const JUMP_AHEAD_S: u32 = 365 * DAY_S;
/// No clock can be set past this.
const LATEST_S: u64 = MAX_UNIX_MS / 1000;

/// How long a report stays listed.
const REPORT_LIFE_S: u32 = 48 * HOUR_S;
/// How long `more` outlasts a report that left for want of room, by uptime
/// since the report may have no time on it.
const MORE_MS: u32 = 48 * HOUR_MS;
const NO_CLOCK_MS: u32 = HOUR_MS;
const CANNOT_FEED_MS: u32 = 30 * MINUTE_MS;
/// After a meal is due, how long before no record of it is news: the
/// schedule gives up on a meal 15 minutes late, and a feed then takes time
/// to end.
const NO_RECORD_S: u32 = 25 * MINUTE_S;
/// How long, with a valid clock, plans can stay out of reach before nothing
/// can say whether meals ran: as long as a meal with no record takes.
const PLANS_STUCK_MS: u32 = 25 * MINUTE_MS;
/// Unplanned boots that count as repeated, within the window.
const RESTARTS: usize = 3;
const RESTART_WINDOW_S: u32 = 6 * HOUR_S;
/// How long without an unplanned boot before an episode is over.
const RESTARTS_QUIET_S: u32 = 24 * HOUR_S;
const RESTARTS_QUIET_MS: u32 = 24 * HOUR_MS;
/// Portions confirmed past a low reading that likely emptied the hopper:
/// what lies between the beam and the rotor.
const PORTIONS_BELOW_THE_BEAM: u8 = 5;
/// `BOOT_Reason`'s system, watchdog, brownout, KM4 system and KM4 watchdog
/// bits: a boot no power-on or deep-sleep wake explains. The firmware's own
/// restarts lapse the watchdog too, so a boot after one has its bits set: the
/// records around it say which it was. After a real brownout a later boot can
/// carry its bit stale, which counts.
const UNPLANNED_RESET: u32 = 0x1f << 16;

/// A time in Unix seconds, if the clock was valid: what an entry carries.
type Stamp = Option<NonZeroU32>;

/// What the loop reads for the conditions on one pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Readings {
    /// The wall clock in Unix seconds, if it is valid.
    pub wall_s: Option<u32>,
    /// Why no feed can start now: readiness, or the reason the schedule is
    /// holding a meal.
    pub blocked: Option<Blocked>,
    /// The food beam's latest reading (true with food), once it has one.
    pub food: Option<bool>,
    /// A feed is queued or runs, so a reading may be that feed's own.
    pub feeding: bool,
    pub desiccant: Desiccant,
    pub supply: Supply,
    /// A plan's claim is being written, given back or settled.
    pub claim: bool,
    /// The plans can be read, with no change in hand.
    pub plans_settled: bool,
}

/// How much of the ring's history the boot read reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum History {
    /// All the ring held.
    Whole,
    /// As much as it reads, of a ring that held more: every older record is
    /// older than `oldest_s`, if it found a time.
    Cut { oldest_s: Option<u32> },
    /// It gave up before it finished.
    Unread,
}

/// A state's time past its threshold. Once past, it stays so without looking
/// at the uptime again, which wraps in 49 days.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Timer {
    Off,
    Running { since_ms: u32 },
    Past,
}

impl Timer {
    /// Whether the state `on` has held for `threshold_ms` at `now_ms`.
    fn past(&mut self, on: bool, now_ms: u32, threshold_ms: u32) -> bool {
        if !on {
            *self = Self::Off;
            return false;
        }

        let since_ms = match *self {
            Self::Past => return true,
            Self::Off => now_ms,
            Self::Running { since_ms } => since_ms,
        };
        if now_ms.wrapping_sub(since_ms) >= threshold_ms {
            *self = Self::Past;
            return true;
        }
        *self = Self::Running { since_ms };
        false
    }

    const fn running(self) -> bool {
        matches!(self, Self::Running { .. })
    }
}

/// The rules' state in the loop. Neither `Copy` nor `Clone`: it is too large
/// for the loop's stack, and lives in a static.
#[derive(Debug)]
pub struct Alerts {
    /// Open alerts, oldest first, with the free slots at the end.
    table: [Option<OpenAlert>; CAPACITY],
    generation: u32,
    /// The uptime a report last left for want of room, while `more` holds.
    room_ran_out_ms: Option<u32>,
    /// The boot read is done.
    restored: bool,
    /// The boot read has held sessions back for as long as it may.
    welcome: bool,
    /// The newest time the boot read and the boot's first events have shown,
    /// in Unix seconds; 0 with none. An old report is judged against it, as
    /// the clock itself is seldom valid yet.
    newest_s: u32,
    /// This boot's `Boot`, waiting for the boot read to end.
    this_boot: Option<(Stamp, u8, u32)>,
    /// This boot's update record, waiting with it.
    this_candidate: Option<UpdateCandidate>,
    /// The thresholds that began at boot have run out.
    settled: bool,
    settling: bool,
    no_clock: Timer,
    cannot_feed: Timer,
    /// A plan's claim in flight, which the writer should finish in seconds.
    claim: Timer,
    plans_stuck: Timer,
    /// A `Supply` event reached the recorder before the read ended, so what
    /// the read finds is older.
    supply_live: bool,
    /// Portions confirmed since the beam read low, while it reads low.
    low_portions: Option<u8>,
    power_out: bool,
    /// The bank and time of the newest boot seen.
    bank: Option<u8>,
    boot_at: Stamp,
    /// The newest boot waits for its update record, and whether it would
    /// count as unplanned.
    unjudged: Option<bool>,
    /// The last boot judged, and what the store held then.
    judged: Option<(u8, UpdateCandidate)>,
    /// A restart someone asked for is on its way.
    restart_asked: bool,
    /// The newest unplanned boots' times, oldest first; the last `restarts`
    /// of them are real.
    restart_at: [Stamp; RESTARTS],
    restarts: u8,
    /// The newest boot's time, or for one with no time the newest seen before
    /// it, which it came after; 0 with none. Only the boot read uses it.
    restart_after_s: u32,
    /// The boots counted so far were close enough to count as repeated.
    episode: bool,
    /// The occurrence each plan was last reported as having no record, so one
    /// that leaves for want of room does not come straight back.
    no_record: [u32; PLANS],
}

impl Default for Alerts {
    fn default() -> Self {
        Self::new()
    }
}

impl Alerts {
    /// Const, so the firmware builds it inside the feeder's static.
    pub const fn new() -> Self {
        Self {
            table: [None; CAPACITY],
            generation: 0,
            room_ran_out_ms: None,
            restored: false,
            welcome: false,
            newest_s: 0,
            this_boot: None,
            this_candidate: None,
            settled: false,
            settling: true,
            no_clock: Timer::Off,
            cannot_feed: Timer::Off,
            claim: Timer::Off,
            plans_stuck: Timer::Off,
            supply_live: false,
            low_portions: None,
            power_out: false,
            bank: None,
            boot_at: None,
            unjudged: None,
            judged: None,
            restart_asked: false,
            restart_at: [None; RESTARTS],
            restarts: 0,
            restart_after_s: 0,
            episode: false,
            no_record: [0; PLANS],
        }
    }

    /// Changes whenever anything a client reads here does.
    pub const fn generation(&self) -> u32 {
        self.generation
    }

    /// A report left for want of room, and could still have been listed.
    pub const fn more(&self) -> bool {
        self.room_ran_out_ms.is_some()
    }

    /// An absent alert doesn't yet mean it ended.
    pub const fn settling(&self) -> bool {
        self.settling
    }

    /// The table as a client reads it, in the ring's `epoch`.
    pub const fn view(&self, epoch: u32) -> AlertsView {
        AlertsView {
            epoch,
            generation: self.generation,
            more: self.more(),
            settling: self.settling,
            alerts: self.table,
        }
    }

    /// The open alerts, oldest first.
    pub fn open(&self) -> impl Iterator<Item = &OpenAlert> {
        self.table.iter().flatten()
    }

    /// Whether the boot read is done.
    pub const fn is_restored(&self) -> bool {
        self.restored
    }

    /// Whether the device API may welcome a session.
    pub const fn welcoming(&self) -> bool {
        self.restored || self.welcome
    }

    /// The boot read has held sessions back for as long as it may.
    pub const fn welcome(&mut self) {
        self.welcome = true;
    }

    /// The boot read is done: what the ring could restore, it has. This
    /// boot's `Boot` now takes its place after the boots it read.
    pub fn restored(&mut self, now_ms: u32, history: History) {
        if self.restored {
            return;
        }

        self.lapse(self.newest_s);
        self.history_missed(now_ms, history);
        self.end_quiet_episode();
        if let Some((at, bank, reset_flags)) = self.this_boot.take() {
            self.booted(at, bank, reset_flags);
        }
        if let Some(candidate) = self.this_candidate.take() {
            self.judge_boot(now_ms, None, candidate);
        }
        self.restored = true;
    }

    /// What a read that fell short may have missed holds `more` for as long
    /// as it could still have been listed: a whole report's life, less the
    /// age of the oldest time it found, as everything it missed is older.
    fn history_missed(&mut self, now_ms: u32, history: History) {
        let age_s = match history {
            History::Whole => return,
            History::Unread | History::Cut { oldest_s: None } => 0,
            History::Cut {
                oldest_s: Some(oldest_s),
            } => self.newest_s.saturating_sub(oldest_s),
        };
        if age_s >= REPORT_LIFE_S {
            return;
        }

        // As if room had run out when that time was now. A later report that
        // truly finds no room moves this on.
        self.room_ran_out_ms = Some(now_ms.wrapping_sub(age_s * 1000));
        self.changed();
    }

    /// An event as the recorder sees it at `now_ms`, stamped `at` in Unix
    /// seconds if the clock is valid.
    pub fn event(&mut self, now_ms: u32, at: Option<u32>, body: &EventBody) {
        if matches!(body, EventBody::Supply(_)) && !self.restored {
            self.supply_live = true;
        }

        match *body {
            EventBody::Boot { bank, reset_flags } if !self.restored => {
                self.note_time(at);
                self.this_boot = Some((at.and_then(NonZeroU32::new), bank, reset_flags));
            }
            EventBody::UpdateAtBoot { candidate } if !self.restored => {
                self.note_time(at);
                self.this_candidate = Some(candidate);
            }
            _ => self.apply(now_ms, at, body),
        }
    }

    /// An event of a past boot, as the boot read finds it, oldest first, with
    /// the wall clock if it is valid yet to judge its time against.
    pub fn replay(&mut self, now_ms: u32, wall_s: Option<u32>, at: Option<u32>, body: &EventBody) {
        // A time that can't be right says nothing of how far on the ring got.
        let at_for_newest = at.filter(|&at| plausible(at, wall_s, self.newest_s));
        self.note_time(at_for_newest);

        // What this boot already reported of the supply is newer.
        if matches!(body, EventBody::Supply(_)) && self.supply_live {
            return;
        }
        self.apply_noted(now_ms, at, body);
    }

    fn apply(&mut self, now_ms: u32, at: Option<u32>, body: &EventBody) {
        self.note_time(at);
        self.apply_noted(now_ms, at, body);
    }

    fn apply_noted(&mut self, now_ms: u32, at: Option<u32>, body: &EventBody) {
        let at = at.and_then(NonZeroU32::new);

        match *body {
            EventBody::Fed {
                completed, hopper, ..
            } => {
                self.portions(completed, hopper);
                if let Some(alert) = meal_problem(body) {
                    self.report(now_ms, OpenAlert { at, alert });
                }
            }
            EventBody::Missed { plan, due, reason } => {
                let alert = Alert::MealMissed { plan, due, reason };
                self.report(now_ms, OpenAlert { at, alert });
            }
            EventBody::Food { level } => self.food(level),
            EventBody::Supply(supply) => match supply.adapter {
                Some(false) if !self.power_out => {
                    self.power_out = true;
                    let alert = Alert::PowerWasOut;
                    self.report(now_ms, OpenAlert { at, alert });
                }
                Some(true) => self.power_out = false,
                Some(false) | None => {}
            },
            EventBody::Boot { bank, reset_flags } => self.booted(at, bank, reset_flags),
            EventBody::UpdateAtBoot { candidate } => self.judge_boot(now_ms, at, candidate),
            EventBody::Restarting => self.restart_asked = true,
            EventBody::UpdateStop { stop } => {
                // A cancel is the owner's doing, not a failure.
                if stop != UpdateStop::Cancelled {
                    let alert = Alert::UpdateFailed { stop };
                    self.report(now_ms, OpenAlert { at, alert });
                }
            }
            EventBody::Enrolled { client, role, path } => {
                let alert = Alert::ClientJoined { client, role, path };
                self.report(now_ms, OpenAlert { at, alert });
            }
            EventBody::Revoked { client, by } => {
                let alert = Alert::ClientRevoked { client, by };
                self.report(now_ms, OpenAlert { at, alert });
            }
            EventBody::Lock { .. }
            | EventBody::Settings { .. }
            | EventBody::Wifi { .. }
            | EventBody::ClockSync { .. }
            | EventBody::Lost { .. }
            | EventBody::FeedingBlocked { .. }
            | EventBody::Plans { .. }
            | EventBody::DispenseRefused { .. }
            | EventBody::MealSoon { .. }
            // A skip moves the plan's mark to the meal it skips, so the
            // meal is judged as one already run: no record is news, and no
            // `Missed` comes ([`super::skip`]).
            | EventBody::Skipped { .. }
            | EventBody::Unskipped { .. } => {}
        }
    }

    /// One plan's latest occurrence `due_s` against the schedule's own record
    /// of it: `mark`, the newest occurrence it took or passed. The caller asks
    /// only with a valid clock, and only while the schedule has nothing in
    /// hand, or a meal it is about to pass would read as having no record. A
    /// plan with no mark was saved without a clock, and its first occurrence
    /// was never expected.
    pub fn meal(&mut self, now_ms: u32, wall_s: u32, plan: u8, due_s: u32, mark: Option<u32>) {
        let Some(mark) = mark else {
            return;
        };

        // The record arrived after all: it was only late.
        let answered = |open: &OpenAlert| {
            matches!(open.alert, Alert::MealNoRecord { plan: slot, due }
                if slot == plan && due <= mark)
        };
        self.remove_where(answered);

        let reached = wall_s >= due_s.saturating_add(NO_RECORD_S);
        let Some(noted) = self.no_record.get_mut(usize::from(plan)) else {
            return;
        };
        if due_s <= mark || !reached || *noted == due_s {
            return;
        }

        *noted = due_s;
        let at = NonZeroU32::new(wall_s);
        let alert = Alert::MealNoRecord { plan, due: due_s };
        self.report(now_ms, OpenAlert { at, alert });
    }

    /// One pass's look at the conditions and at what time alone ends.
    pub fn step(&mut self, now_ms: u32, readings: &Readings) {
        let wall_s = readings.wall_s;
        if let Some(wall_s) = wall_s {
            self.lapse(wall_s);
        }
        self.conditions(now_ms, readings);
        self.restarts_end_or_open(now_ms, wall_s);

        if self
            .room_ran_out_ms
            .is_some_and(|at| now_ms.wrapping_sub(at) >= MORE_MS)
        {
            self.room_ran_out_ms = None;
            self.changed();
        }

        self.settled = self.settled
            || (self.restored && !self.no_clock.running() && !self.cannot_feed.running());
        let settling = !self.settled || unknown_reading(readings);
        if settling != self.settling {
            self.settling = settling;
            self.changed();
        }
    }

    fn conditions(&mut self, now_ms: u32, readings: &Readings) {
        let at = readings.wall_s.and_then(NonZeroU32::new);

        if self
            .no_clock
            .past(readings.wall_s.is_none(), now_ms, NO_CLOCK_MS)
        {
            self.condition(now_ms, at, Alert::NoClock);
        } else {
            self.close(AlertKind::NoClock);
        }

        // A claim the writer never finishes holds every feed, and much sooner
        // than a state that may clear on its own is worth saying.
        let cannot_feed = self
            .cannot_feed
            .past(readings.blocked.is_some(), now_ms, CANNOT_FEED_MS);
        let claim_stuck = self.claim.past(readings.claim, now_ms, CLAIM_WAIT_MS);
        match readings.blocked.filter(|_| cannot_feed || claim_stuck) {
            Some(reason) => self.condition(now_ms, at, Alert::CannotFeed { reason }),
            None => self.close(AlertKind::CannotFeed),
        }

        // With no clock no meal is due, and `NoClock` says so.
        let unsettled = readings.wall_s.is_some() && !readings.plans_settled;
        if self.plans_stuck.past(unsettled, now_ms, PLANS_STUCK_MS) {
            self.condition(now_ms, at, Alert::PlansStuck);
        } else {
            self.close(AlertKind::PlansStuck);
        }

        match readings.food {
            Some(true) => {
                self.food(FoodLevel::Present);
                self.close(AlertKind::FoodLow);
            }
            Some(false) => {
                // A feed's own low reading starts the count only with its
                // `Fed`, which counts none of that feed's portions: the beam
                // may have read low at its end.
                if !readings.feeding {
                    self.food(FoodLevel::Low);
                }
                self.condition(now_ms, at, Alert::FoodLow);
            }
            None => {}
        }
        match self.low_portions {
            Some(confirmed) if confirmed >= PORTIONS_BELOW_THE_BEAM => {
                self.condition(now_ms, at, Alert::FoodLikelyOut { confirmed });
            }
            Some(_) | None => self.close(AlertKind::FoodLikelyOut),
        }

        // With no clock the days can't be counted, which says nothing about
        // the desiccant, so an open alert holds through a cold boot.
        match readings.desiccant {
            Desiccant::Remaining { days } if days <= 0 => {
                self.condition(now_ms, at, Alert::DesiccantDue { days });
            }
            Desiccant::Remaining { .. } | Desiccant::Unset => self.close(AlertKind::DesiccantDue),
            Desiccant::Unknown => {}
        }

        // `None` is no verdict: a pack gone stale leaves an open alert as it
        // stands.
        match readings.supply.below_floor {
            Some(true) => self.condition(now_ms, at, Alert::BatteryBelowFloor),
            Some(false) => self.close(AlertKind::BatteryBelowFloor),
            None => {}
        }
    }

    /// An episode of restarts: open, or gone quiet. It ends its own state, so
    /// it doesn't look at the uptime after it has wrapped.
    fn restarts_end_or_open(&mut self, now_ms: u32, wall_s: Option<u32>) {
        if self.restarts > 0 {
            let newest = self.restart_at[RESTARTS - 1];
            if now_ms >= RESTARTS_QUIET_MS || held_for(newest, wall_s, RESTARTS_QUIET_S) {
                self.forget_restarts();
                self.close(AlertKind::RepeatedRestarts);
            }
        }
        if self.episode && self.restored {
            let at = wall_s.and_then(NonZeroU32::new);
            self.condition(now_ms, at, Alert::RepeatedRestarts);
        }
    }

    /// Before the boot read ends, anything with a time says how far on the
    /// ring has got.
    fn note_time(&mut self, at: Option<u32>) {
        if !self.restored {
            self.newest_s = self.newest_s.max(at.unwrap_or(0));
        }
    }

    fn lapse(&mut self, wall_s: u32) {
        self.remove_where(|open| {
            !open.alert.kind().is_condition()
                && open
                    .at
                    .is_some_and(|at| wall_s.saturating_sub(at.get()) >= REPORT_LIFE_S)
        });
    }

    /// The beam's word: a low reading starts the count, food ends it.
    fn food(&mut self, level: FoodLevel) {
        match level {
            FoodLevel::Low => {
                self.low_portions.get_or_insert(0);
            }
            FoodLevel::Present => self.low_portions = None,
            FoodLevel::Unread => {}
        }
    }

    /// A feed's portions count towards an empty hopper only past a low reading
    /// that came before it: the feed that first reads low may have read it
    /// at its end.
    fn portions(&mut self, completed: u8, hopper: FoodLevel) {
        if let Some(low) = &mut self.low_portions {
            *low = low.saturating_add(completed);
        }
        self.food(hopper);
    }

    /// A boot, which its update record will judge. If the boot before it
    /// never got one, that boot counts as what its flags say.
    fn booted(&mut self, at: Stamp, bank: u8, reset_flags: u32) {
        if self.unjudged == Some(true) {
            self.restarted(self.boot_at);
        }

        let asked = core::mem::take(&mut self.restart_asked);
        self.unjudged = Some(reset_flags & UNPLANNED_RESET != 0 && !asked);
        self.bank = Some(bank);
        self.boot_at = at;
    }

    /// What the update store held as the newest boot came up: whether that
    /// boot was an install's own restart, or a fall back to bank 0.
    fn judge_boot(&mut self, now_ms: u32, at: Stamp, candidate: UpdateCandidate) {
        let (Some(bank), Some(unplanned)) = (self.bank, self.unjudged.take()) else {
            return;
        };
        let before = self.judged.replace((bank, candidate));
        let bank_before = before.map(|(bank, _)| bank);

        let fell = match (bank, candidate) {
            (0, UpdateCandidate::None) => bank_before == Some(1),
            // Once only: bank 0 goes on finding it, boot after boot. With no
            // boot before it, this is one long past the ring's reach.
            (0, UpdateCandidate::Activated) => {
                before.is_some_and(|before| before != (0, UpdateCandidate::Activated))
            }
            _ => false,
        };
        if fell {
            let alert = Alert::RolledBack;
            let at = self.boot_at.or(at);
            self.report(now_ms, OpenAlert { at, alert });
        }

        let install = match (bank, candidate) {
            (0, UpdateCandidate::Pending) => bank_before != Some(0),
            (1, UpdateCandidate::Activated) => bank_before != Some(1),
            _ => false,
        };
        if unplanned && !install {
            self.restarted(self.boot_at);
        }
    }

    /// One more unplanned boot. A boot is stamped only with a valid clock,
    /// so one with no time counts as recent: a false alert costs less than
    /// silence. Read back from the ring, boots the times show are long over
    /// don't count towards it.
    fn restarted(&mut self, at: Stamp) {
        self.end_quiet_episode();

        self.restart_at.rotate_left(1);
        self.restart_at[RESTARTS - 1] = at;
        self.restarts = self.restarts.saturating_add(1);
        self.restart_after_s = at.map_or(self.newest_s, NonZeroU32::get);

        let timed = || self.restart_at.iter().flatten().map(|stamp| stamp.get());
        let spread = match (timed().min(), timed().max()) {
            (Some(oldest), Some(newest)) => newest - oldest,
            _ => 0,
        };
        self.episode |= usize::from(self.restarts) >= RESTARTS && spread <= RESTART_WINDOW_S;
    }

    /// While the ring is read: drops boots the newest time seen shows were
    /// followed by a quiet day. A time of 0 is none, which shows nothing.
    fn end_quiet_episode(&mut self) {
        let after = self.restart_after_s;
        if !self.restored && after != 0 && self.newest_s.saturating_sub(after) >= RESTARTS_QUIET_S {
            self.forget_restarts();
        }
    }

    fn forget_restarts(&mut self) {
        self.restarts = 0;
        self.restart_at = [None; RESTARTS];
        self.restart_after_s = 0;
        self.episode = false;
    }

    /// Opens a report, or replaces the one with its key: an uncertain feed
    /// can record a second `Fed`, and the boot read can find what is open.
    ///
    /// Time passes with the report's own, so one that has lapsed never counts
    /// as room running out. While the ring is read that time is history, so
    /// the newest seen stands in, and a report already past its life stays
    /// out. Once it is read a report's time is now, whatever the clock did.
    fn report(&mut self, now_ms: u32, open: OpenAlert) {
        if let Some(at) = open.at.map(NonZeroU32::get) {
            let now_s = if self.restored { at } else { self.newest_s };
            self.lapse(now_s);
            if now_s.saturating_sub(at) >= REPORT_LIFE_S {
                return;
            }
        }

        let key = open.key();
        let same = self
            .table
            .iter_mut()
            .flatten()
            .find(|held| held.key() == key);
        // The read goes oldest first but this boot's own come through at once,
        // so while it runs a report with an older time is the older report.
        let restored = self.restored;
        match same {
            Some(held) if *held == open => {}
            Some(held) if !restored && older(open.at, held.at) => {}
            Some(held) => {
                *held = open;
                self.changed();
            }
            None => self.insert(now_ms, open),
        }
    }

    /// Opens a condition, or updates what it says. It keeps the time it
    /// opened at, so it isn't new each time its detail moves.
    fn condition(&mut self, now_ms: u32, at: Stamp, alert: Alert) {
        let kind = alert.kind();
        let same = self
            .table
            .iter_mut()
            .flatten()
            .find(|held| held.alert.kind() == kind);
        match same {
            Some(held) if held.alert == alert => {}
            Some(held) => {
                held.alert = alert;
                self.changed();
            }
            None => self.insert(now_ms, OpenAlert { at, alert }),
        }
    }

    fn close(&mut self, kind: AlertKind) {
        self.remove_where(|open| open.alert.kind() == kind);
    }

    /// Puts `open` in a free slot, making room if there is none.
    fn insert(&mut self, now_ms: u32, open: OpenAlert) {
        if self.table.iter().all(Option::is_some) && !self.make_room(now_ms, &open) {
            return;
        }

        if let Some(free) = self.table.iter_mut().find(|slot| slot.is_none()) {
            *free = Some(open);
            self.changed();
        }
    }

    /// Pushes out the oldest of the least urgent reports, unless `open` is a
    /// report less urgent than all of them. Whether `open` now has a slot.
    fn make_room(&mut self, now_ms: u32, open: &OpenAlert) -> bool {
        self.room_ran_out_ms = Some(now_ms);
        self.changed();

        // `min_by_key` keeps the first of equals: the oldest.
        let victim = self
            .open()
            .enumerate()
            .filter(|(_, held)| !held.alert.kind().is_condition())
            .min_by_key(|(_, held)| rank(held.alert.level()))
            .map(|(at, held)| (at, rank(held.alert.level())));
        let Some((at, victim_rank)) = victim else {
            return false;
        };

        let incoming = open.alert.kind();
        if !incoming.is_condition() && rank(incoming.level()) < victim_rank {
            return false;
        }
        self.remove(at);
        true
    }

    fn remove_where(&mut self, ended: impl Fn(&OpenAlert) -> bool) {
        // From the end, so a removal doesn't move what is still to be looked at.
        for at in (0..CAPACITY).rev() {
            if self.table[at].as_ref().is_some_and(&ended) {
                self.remove(at);
            }
        }
    }

    /// Closes the gap, so the table stays oldest first.
    fn remove(&mut self, at: usize) {
        let Some(tail) = self.table.get_mut(at..) else {
            return;
        };

        tail.rotate_left(1);
        if let Some(last) = tail.last_mut() {
            *last = None;
        }
        self.changed();
    }

    /// Any change at all: a client compares for equality, and rereads.
    const fn changed(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
}

/// The alert a finished feed opens, if it went wrong. Which feed asked makes
/// no difference: a failure is a failure, and a later good meal doesn't undo
/// it. A clean or cancelled ending opens nothing, and doesn't close what an
/// uncertain first record of the same feed opened.
const fn meal_problem(body: &EventBody) -> Option<Alert> {
    let EventBody::Fed {
        feed,
        requested,
        completed,
        outcome,
        reason,
        by,
        ..
    } = *body
    else {
        return None;
    };
    let plan = match by {
        Source::Schedule(plan) => Some(plan),
        Source::Panel | Source::Client(_) | Source::Feeder => None,
    };

    match outcome {
        FeedOutcome::Failed | FeedOutcome::Expired => Some(Alert::MealFailed {
            feed,
            plan,
            requested,
            completed,
            outcome,
            reason,
        }),
        FeedOutcome::Uncertain => Some(Alert::MealUncertain {
            feed,
            plan,
            requested,
            completed,
            reason,
        }),
        FeedOutcome::Done | FeedOutcome::Cancelled => None,
    }
}

/// Whether a reading some condition needs is unknown. A battery with no pack
/// fitted has no verdict to wait for.
const fn unknown_reading(readings: &Readings) -> bool {
    let no_verdict = readings.supply.below_floor.is_none()
        && !matches!(readings.supply.battery, Battery::Absent);

    readings.food.is_none() || matches!(readings.desiccant, Desiccant::Unknown) || no_verdict
}

/// Whether the stamp `from` is at least `span_s` before `to`. A clock can
/// step back, and a missing time tells nothing.
fn held_for(from: Stamp, to: Option<u32>, span_s: u32) -> bool {
    match (from, to) {
        (Some(from), Some(to)) => to.saturating_sub(from.get()) >= span_s,
        _ => false,
    }
}

/// Whether a time can be right. The wall clock is the best judge, once it is
/// valid; before that, only how far it jumps past what the ring has shown.
fn plausible(at: u32, wall_s: Option<u32>, newest_s: u32) -> bool {
    let ceiling = match wall_s {
        Some(wall_s) => wall_s.saturating_add(CLOCK_AHEAD_S),
        None if newest_s != 0 => newest_s.saturating_add(JUMP_AHEAD_S),
        None => u32::MAX,
    };
    at <= ceiling && u64::from(at) <= LATEST_S
}

/// Whether `this` is a time before `that`. No time tells nothing.
fn older(this: Stamp, that: Stamp) -> bool {
    matches!((this, that), (Some(this), Some(that)) if this < that)
}

/// Which report leaves first when room runs out, lowest first.
const fn rank(level: AlertLevel) -> u8 {
    match level {
        AlertLevel::Soon => 0,
        AlertLevel::Security => 1,
        AlertLevel::Urgent => 2,
    }
}

#[cfg(test)]
mod tests;
