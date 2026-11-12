//! Feeding plans in the loop: which occurrence runs, and how each runs at
//! most once ([schedules]).
//!
//! **Deciding.** With a valid clock, each enabled plan's latest occurrence
//! (`wellfed_core_feeder::schedule::latest_due`, worked out once a wall-clock
//! minute) meets the missed-meal rule (`wellfed_core::clock::wall::occurrence`)
//! against the plan's mark: at or before it, nothing; at most 15 minutes
//! late, it runs; later, it is passed and reported missed. Without a valid
//! clock or a zone offset, plans hold: nothing runs and nothing is passed.
//! A client's skip moves a mark ahead to the occurrence skipped
//! ([`super::skip`]), so it is judged like one already run: nothing.
//!
//! **Claim, then feed.** An occurrence runs in two steps: the writer first
//! saves a claim (the occurrence, and the journal's high water), and only
//! once that is on NOR is the feed queued, under [`Source::Schedule`]. A
//! cut between the two leaves a claim whose feed was never reserved: the
//! next boot finds the journal's high water unchanged, records the
//! occurrence as missed (`Interrupted`) and never runs it. While a claim
//! is being written, or its feed queued, and until the boot has compared
//! a stored claim with the journal, every other feed is refused
//! ([`Schedule::holds`]), so the claimed feed is the very next one the
//! journal reserves. A feed the owner refuses after its claim gives the
//! occurrence back ([`PlansChange::Release`]); it is claimed again after
//! [`RETRY_MS`], within the grace, so a refusing owner costs a few writes
//! rather than two a pass.
//!
//! **A change of zone.** Marks hold occurrences by instant, so a zone
//! further west puts a plan's minute at a later instant, past the mark of
//! the meal already fed that local day. A meal fed or skipped that day
//! never runs again: the schedule keeps each mark with the offset it fell
//! at, and the same instant at another offset means the offsets changed (a
//! new zone, or a client's sync replacing the offsets of a feeder with no
//! zone). If the mark is a meal fed or a client's skip, it then passes,
//! unreported, the plan's occurrence on the date the mark took
//! (`same_day_again`) before anything else runs. A meal missed, or a mark
//! a save left, takes nothing from the day: the meal runs at the new
//! zone's minute if that is still to come. A restart between the zone's
//! save and that pass forgets the old offset, and with it the pass.
//!
//! **Waiting.** An occurrence that comes due while the feeder cannot feed
//! (another feed, a join, an install, a held battery) waits, and runs as
//! soon as it can within the grace; past it, it is missed, with the reason
//! it waited for. Plans due together run one after another, by slot.
//!
//! [schedules]: ../../../../../docs/firmware/schedules.md

use core::fmt;

use wellfed_core::clock::wall::{MISSED_GRACE_MS, Occurrence, occurrence};
use wellfed_core_feeder::schedule::{Plan, latest_due, same_day_again};
use wellfed_device_api::mailbox::{Claim, PLANS, PlansChange, PlansResult, PlansView, PlayResult};
use wellfed_proto::api::{Blocked, EventBody, MissedReason, Readiness, Refusal, Source};

use super::Feeds;
use super::events;
use super::feeding;
use super::platform::Platform;

const MINUTE_MS: u64 = 60_000;
/// How long an occurrence whose claimed feed the owner refused waits
/// before it is claimed again: each try writes the plans twice.
pub const RETRY_MS: u32 = 5_000;

/// Where the schedule stands with the writer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// The boot's stored claim has not been looked at: the plans are
    /// still loading.
    Boot,
    /// The plans hold a claim the journal has not been compared with yet.
    Resolving,
    Idle,
    /// A change that must be written, retried until the mailbox takes it.
    Queue(PlansChange, Expect),
    /// A change the writer has, and what it should leave in the view.
    Wait(Expect),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    Claimed(Claim),
    Released(Claim),
    /// A claim whose feed was never queued settled, reported once it is
    /// with the reason it never ran.
    Settled(Claim, MissedReason),
    /// A missed occurrence passed; reported once it is, if `report`.
    Passed {
        plan: u8,
        due_s: u32,
        report: Option<MissedReason>,
    },
}

/// What the schedule did, for the record lines (`PLSCH1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Record {
    /// An occurrence was taken; its feed is queued once the claim is saved.
    Claimed { plan: u8, due_s: u32 },
    /// The claimed occurrence's feed was queued as `feed`.
    Queued { plan: u8, feed: u32 },
    /// The owner refused the claimed feed; the occurrence is given back.
    Refused { plan: u8, refusal: Refusal },
    Missed {
        plan: u8,
        due_s: u32,
        reason: MissedReason,
    },
    /// The writer dropped the schedule's change: it decides again.
    Dropped,
    /// The meal reminder recorded `MealSoon` for this occurrence.
    Reminded { plan: u8, due_s: u32 },
    /// The meal call was asked of the speaker for this occurrence.
    Called {
        plan: u8,
        due_s: u32,
        result: PlayResult,
    },
}

/// One record's body (`PLSCH1`).
pub fn describe(record: Record, f: &mut impl fmt::Write) -> fmt::Result {
    match record {
        Record::Claimed { plan, due_s } => write!(f, "claimed plan={plan} due={due_s}"),
        Record::Queued { plan, feed } => write!(f, "queued plan={plan} feed={feed}"),
        Record::Refused { plan, refusal } => {
            write!(
                f,
                "refused plan={plan} refusal={}",
                feeding::refusal_name(refusal)
            )
        }
        Record::Missed {
            plan,
            due_s,
            reason,
        } => write!(
            f,
            "missed plan={plan} due={due_s} reason={}",
            missed_name(reason)
        ),
        Record::Dropped => f.write_str("dropped"),
        Record::Reminded { plan, due_s } => write!(f, "reminded plan={plan} due={due_s}"),
        Record::Called {
            plan,
            due_s,
            result,
        } => {
            let result = match result {
                PlayResult::Started => "started",
                PlayResult::Muted => "muted",
                PlayResult::Busy => "busy",
            };
            write!(f, "called plan={plan} due={due_s} result={result}")
        }
    }
}

/// The plans' state in the loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule {
    step: Step,
    /// Each slot's latest occurrence (Unix ms), worked out for `computed`.
    dues: [Option<u64>; PLANS],
    /// The wall-clock minute and the table revision `dues` are for.
    computed: Option<(u64, u32)>,
    /// Each slot's occurrence held while the feeder cannot feed, and why:
    /// every plan due together waits on the same reason, and is missed with
    /// it.
    waiting: [Option<(u64, Blocked)>; PLANS],
    /// No claim before this uptime: the owner refused the last claimed
    /// feed.
    retry_at: Option<u32>,
    /// Each slot's mark (Unix seconds) and the offset it fell at, as last
    /// worked out.
    marked: [Option<(u32, i32)>; PLANS],
    /// Each slot's occurrence (Unix seconds) a change of zone would run on
    /// a local day that already had its meal, with the plan it is for:
    /// passed before anything else runs.
    again: [Option<(Plan, u32)>; PLANS],
}

impl Default for Schedule {
    fn default() -> Self {
        Self::new()
    }
}

impl Schedule {
    pub const fn new() -> Self {
        Self {
            step: Step::Boot,
            dues: [None; PLANS],
            computed: None,
            waiting: [None; PLANS],
            retry_at: None,
            marked: [None; PLANS],
            again: [None; PLANS],
        }
    }

    /// Whether every other feed must wait: a claim is being written, given
    /// back or settled, so the next feed the journal reserves is the
    /// claim's; or a stored claim has not yet been compared with the
    /// journal, which another feed would move first. Plans still loading
    /// hold nothing, so a store that never loads never stops feeding; the
    /// writer publishes it as it opens, before the feeding owner opens.
    pub fn holds(&self) -> bool {
        match self.step {
            Step::Queue(_, expect) | Step::Wait(expect) => !matches!(expect, Expect::Passed { .. }),
            Step::Resolving => true,
            Step::Boot | Step::Idle => false,
        }
    }

    /// Whether a claim is being written, given back or settled: a meal the
    /// schedule has committed to and whose feed is not yet queued. A
    /// restart waits for it, so the meal is not missed. Unlike
    /// [`Self::holds`], a stored claim still waiting for the journal
    /// (`Resolving`) is not one: with a faulted journal that wait lasts the
    /// whole boot.
    pub fn claim_in_flight(&self) -> bool {
        match self.step {
            Step::Queue(_, expect) | Step::Wait(expect) => !matches!(expect, Expect::Passed { .. }),
            Step::Resolving | Step::Boot | Step::Idle => false,
        }
    }

    /// Whether the schedule has nothing in hand: what its marks say about an
    /// occurrence is its last word for now, with no write of its own to come.
    pub const fn at_rest(&self) -> bool {
        matches!(self.step, Step::Idle)
    }

    /// Why the schedule is holding a meal it could not start, if it is.
    pub fn meal_held(&self) -> Option<Blocked> {
        self.waiting
            .iter()
            .flatten()
            .map(|&(_, reason)| reason)
            .next()
    }

    /// One pass at `now_ms`; `ready` is the feeder's readiness without the
    /// schedule's own hold.
    pub fn step<P: Platform, F: Feeds>(
        &mut self,
        platform: &mut P,
        feeds: &mut F,
        now_ms: u32,
        ready: Readiness,
        mut record: impl FnMut(Record),
    ) {
        let (view, pending) = match platform.plans() {
            PlansResult::Current { view, pending } => (view, pending),
            // Loading: the boot's claim is still unknown.
            PlansResult::Busy => return,
            // No plans this boot, so no claim to resolve and nothing runs.
            PlansResult::ReadOnly | PlansResult::Unserved => {
                self.step = Step::Idle;
                return;
            }
        };

        match self.step {
            Step::Boot | Step::Resolving => self.boot(&view, feeds),
            Step::Queue(change, expect) => match platform.plans_change(change, None) {
                PlansResult::Current { pending: true, .. } => self.step = Step::Wait(expect),
                // A client's change holds the slot: ask again next pass.
                PlansResult::Busy => {}
                PlansResult::Current { pending: false, .. }
                | PlansResult::ReadOnly
                | PlansResult::Unserved => self.step = Step::Idle,
            },
            // Ours, or a client's queued behind it.
            Step::Wait(_) if pending => {}
            Step::Wait(expect) => {
                self.settle(expect, &view, platform, feeds, now_ms, ready, &mut record)
            }
            // A client's change is being written: decide on what it leaves.
            Step::Idle if pending => {}
            Step::Idle => self.decide(&view, platform, feeds, now_ms, ready, &mut record),
        }
    }

    /// A claim stored before this boot: its feed was reserved if the
    /// journal moved past the claim's high water, and the journal reported
    /// it then. Otherwise the cut came before the feed was queued.
    fn boot<F: Feeds>(&mut self, view: &PlansView, feeds: &F) {
        let Some(claim) = view.claim else {
            self.step = Step::Idle;
            return;
        };
        let Some(high_water) = feeds.high_water() else {
            self.step = Step::Resolving;
            return;
        };
        self.step = if high_water == claim.high_water {
            let settled = Expect::Settled(claim, MissedReason::Interrupted);
            Step::Queue(PlansChange::Settle { claim }, settled)
        } else {
            Step::Idle
        };
    }

    /// The writer is done with the schedule's change: act on what it left.
    #[expect(clippy::too_many_arguments, reason = "one pass's whole context")]
    fn settle<P: Platform, F: Feeds>(
        &mut self,
        expect: Expect,
        view: &PlansView,
        platform: &mut P,
        feeds: &mut F,
        now_ms: u32,
        ready: Readiness,
        record: &mut impl FnMut(Record),
    ) {
        self.step = Step::Idle;
        match expect {
            Expect::Claimed(claim) => {
                let held = view
                    .claim
                    .filter(|held| held.plan == claim.plan && held.due_s == claim.due_s)
                    .filter(|held| held.high_water == claim.high_water);
                let Some(held) = held else {
                    record(Record::Dropped);
                    return;
                };
                // The plan's portions, even if taking it disabled the plan.
                let Some(Some(plan)) = view.table.get(usize::from(held.plan)) else {
                    self.step =
                        Step::Queue(PlansChange::Release { claim: held }, Expect::Released(held));
                    return;
                };
                // Claimed in time, but the claim's write can outlast the
                // grace (a stalled writer), or the clock can be set ahead
                // meanwhile: an occurrence that late never runs. Its claim
                // is spent and cleared, and the meal reported missed.
                let due_ms = u64::from(held.due_s) * 1000;
                let late = platform
                    .wall_ms(now_ms)
                    .is_some_and(|wall| wall.saturating_sub(due_ms) > MISSED_GRACE_MS);
                if late {
                    self.waiting[usize::from(held.plan)] = None;
                    let settled = Expect::Settled(held, MissedReason::Late);
                    self.step = Step::Queue(PlansChange::Settle { claim: held }, settled);
                    return;
                }
                let by = Source::Schedule(held.plan);
                match feeding::queue(feeds, ready, plan.portions, by) {
                    Ok(feed) => {
                        self.waiting[usize::from(held.plan)] = None;
                        record(Record::Queued {
                            plan: held.plan,
                            feed,
                        });
                    }
                    Err(refusal) => self.give_back(held, refusal, now_ms, record),
                }
            }
            Expect::Released(_) => {}
            Expect::Settled(claim, reason) => {
                if view.claim.is_none() {
                    missed(platform, now_ms, claim.plan, claim.due_s, reason, record);
                }
            }
            Expect::Passed {
                plan,
                due_s,
                report,
            } => {
                let passed = view
                    .mark(usize::from(plan))
                    .is_some_and(|mark| mark >= due_s);
                if !passed {
                    record(Record::Dropped);
                    return;
                }
                if let Some(waiting) = self.waiting.get_mut(usize::from(plan)) {
                    *waiting = None;
                }
                if let Some(reason) = report {
                    missed(platform, now_ms, plan, due_s, reason, record);
                }
            }
        }
    }

    /// The owner refused the claimed feed: the occurrence is given back, and
    /// claimed again after [`RETRY_MS`].
    fn give_back(
        &mut self,
        held: Claim,
        refusal: Refusal,
        now_ms: u32,
        record: &mut impl FnMut(Record),
    ) {
        record(Record::Refused {
            plan: held.plan,
            refusal,
        });
        let due = u64::from(held.due_s) * 1000;
        if let Some(waiting) = self.waiting.get_mut(usize::from(held.plan)) {
            *waiting = Some((due, blocked(refusal)));
        }
        self.retry_at = Some(now_ms.wrapping_add(RETRY_MS));
        self.step = Step::Queue(PlansChange::Release { claim: held }, Expect::Released(held));
    }

    /// The feed `plan`'s claim queued was accepted, but its own claim on the
    /// outputs ended refused: as a refusal when it was queued. The schedule
    /// has not stepped since (the loop holds it while a feed waits), so the
    /// plans still hold the claim.
    pub fn claim_refused<P: Platform>(
        &mut self,
        platform: &mut P,
        plan: u8,
        refusal: Refusal,
        now_ms: u32,
        mut record: impl FnMut(Record),
    ) {
        let held = match platform.plans() {
            PlansResult::Current { view, .. } => view.claim.filter(|held| held.plan == plan),
            PlansResult::Busy | PlansResult::ReadOnly | PlansResult::Unserved => None,
        };
        match held {
            Some(held) => self.give_back(held, refusal, now_ms, &mut record),
            None => record(Record::Dropped),
        }
    }

    /// Run the first occurrence that is due, or pass the first that was
    /// missed.
    fn decide<P: Platform, F: Feeds>(
        &mut self,
        view: &PlansView,
        platform: &mut P,
        feeds: &mut F,
        now_ms: u32,
        ready: Readiness,
        record: &mut impl FnMut(Record),
    ) {
        // Marks must be kept to run anything at most once.
        if !view.writable {
            return;
        }
        let Some(wall_ms) = platform.wall_ms(now_ms) else {
            self.waiting = [None; PLANS];
            self.retry_at = None;
            return;
        };
        self.compute(view, platform, wall_ms);
        // A plan changed or moved on to its next occurrence holds nothing.
        for (waiting, due) in self.waiting.iter_mut().zip(self.dues) {
            if waiting.is_some_and(|(held, _)| Some(held) != due) {
                *waiting = None;
            }
        }
        if self.pass_again(view, platform) {
            return;
        }

        let judged: [Option<(usize, u64, Occurrence)>; PLANS] = core::array::from_fn(|slot| {
            let due = self.dues[slot]?;
            let mark = view.mark(slot).map(|s| u64::from(s) * 1000);
            let judged = match occurrence(due, mark, wall_ms) {
                // A plan saved without a valid clock starts from its first
                // occurrence after the clock returns: one before then may
                // have fallen before the save, so it passes, unreported.
                Occurrence::Run if mark.is_none() => Occurrence::Missed,
                judged => judged,
            };
            Some((slot, due, judged))
        });
        let first = |wanted| judged.iter().flatten().find(|j| j.2 == wanted).copied();

        if let Some((slot, due, _)) = first(Occurrence::Run) {
            // Every plan due with the first waits on what holds it.
            let held = self.run(view, platform, feeds, (slot, due), now_ms, ready, record);
            if let Some(reason) = held {
                for &(slot, due, judged) in judged.iter().flatten() {
                    if judged == Occurrence::Run {
                        self.waiting[slot] = Some((due, reason));
                    }
                }
            }
            return;
        }
        // The refused occurrence is past running: a deadline kept longer
        // would read as ahead again once the uptime wraps half its range.
        self.retry_at = None;
        if let Some((slot, due, _)) = first(Occurrence::Missed) {
            let plan = u8::try_from(slot).unwrap_or(u8::MAX);
            let reason = match self.waiting[slot] {
                Some((held_due, blocked)) if held_due == due => MissedReason::Blocked(blocked),
                _ => MissedReason::Late,
            };
            // A plan saved without a valid clock has no mark: its first
            // occurrence was never expected, so it passes unreported.
            let expected = view.mark(slot).is_some() || reason != MissedReason::Late;
            let due_s = seconds(due);
            let change = PlansChange::Pass {
                plan,
                due_s,
                revision: view.revision,
            };
            if let PlansResult::Current { pending: true, .. } = platform.plans_change(change, None)
            {
                self.step = Step::Wait(Expect::Passed {
                    plan,
                    due_s,
                    report: expected.then_some(reason),
                });
            }
        }
    }

    /// Claim slot `slot`'s occurrence `due` if the feeder can feed now;
    /// otherwise what holds it.
    #[expect(clippy::too_many_arguments, reason = "one pass's whole context")]
    fn run<P: Platform, F: Feeds>(
        &mut self,
        view: &PlansView,
        platform: &mut P,
        feeds: &F,
        (slot, due): (usize, u64),
        now_ms: u32,
        ready: Readiness,
        record: &mut impl FnMut(Record),
    ) -> Option<Blocked> {
        let plan = u8::try_from(slot).unwrap_or(u8::MAX);
        if let Some(at) = self.retry_at {
            if !wellfed_core::deadline_passed(now_ms, at) {
                // What the owner's refusal of this occurrence held it on.
                return self.waiting[slot].map(|(_, reason)| reason);
            }
            self.retry_at = None;
        }
        let blocked = match ready {
            Readiness::Blocked(blocked) => Some(blocked),
            Readiness::Ready if feeds.busy() => Some(Blocked::Feeding),
            Readiness::Ready => None,
        };
        let high_water = feeds.high_water();
        let (None, Some(high_water)) = (blocked, high_water) else {
            return Some(blocked.unwrap_or(Blocked::Starting));
        };

        let claim = Claim {
            plan,
            due_s: seconds(due),
            high_water,
            disabled: false,
        };
        let change = PlansChange::Claim {
            claim,
            revision: view.revision,
        };
        if let PlansResult::Current { pending: true, .. } = platform.plans_change(change, None) {
            self.step = Step::Wait(Expect::Claimed(claim));
            record(Record::Claimed {
                plan,
                due_s: claim.due_s,
            });
        }
        None
    }

    /// Each enabled plan's latest occurrence, worked out again only when
    /// the wall-clock minute or the table changes: the zone's rule is
    /// costly to evaluate every pass. So is whether the zone changed under
    /// a mark, which then names the occurrence to pass.
    fn compute<P: Platform>(&mut self, view: &PlansView, platform: &P, wall_ms: u64) {
        let key = (wall_ms / MINUTE_MS, view.revision);
        if self.computed == Some(key) {
            return;
        }
        let offset_at = |ms| platform.offset_at(ms);
        for (due, plan) in self.dues.iter_mut().zip(&view.table) {
            *due = plan
                .filter(|plan| plan.enabled)
                .and_then(|plan| latest_due(&plan, wall_ms, offset_at));
        }

        for slot in 0..PLANS {
            let mark = view.mark(slot);
            // The instant last seen keeps its offset unless the zone
            // changed; the mark may have moved since, within the day.
            let changed = self.marked[slot].and_then(|(seen, offset)| {
                let now = offset_at(u64::from(seen) * 1000)?;
                (now != offset).then_some(offset)
            });
            // Only a meal fed or skipped took the day's meal.
            let taken = view.fed(slot) || view.skipped(slot).is_some();
            let plan = view.table[slot].filter(|plan| plan.enabled && taken);
            if let (Some(old_offset), Some(plan), Some(mark)) = (changed, plan, mark) {
                let again = same_day_again(&plan, u64::from(mark) * 1000, old_offset, offset_at);
                if let Some(due_s) = again.and_then(|due| u32::try_from(due / 1000).ok()) {
                    self.again[slot] = Some((plan, due_s));
                }
            }
            self.marked[slot] = mark.and_then(|mark| {
                let offset = offset_at(u64::from(mark) * 1000)?;
                Some((mark, offset))
            });
        }
        self.computed = Some(key);
    }

    /// Pass the first occurrence a change of zone would run again, if one
    /// is still ahead of its plan's mark: whether a pass was queued. One
    /// for a plan since changed, disabled or already past it is dropped.
    fn pass_again<P: Platform>(&mut self, view: &PlansView, platform: &mut P) -> bool {
        for slot in 0..PLANS {
            let Some((plan, due_s)) = self.again[slot] else {
                continue;
            };
            let behind = view.mark(slot).is_some_and(|mark| mark >= due_s);
            if view.table[slot] != Some(plan) || behind {
                self.again[slot] = None;
                continue;
            }

            let plan = u8::try_from(slot).unwrap_or(u8::MAX);
            let change = PlansChange::Pass {
                plan,
                due_s,
                revision: view.revision,
            };
            if let PlansResult::Current { pending: true, .. } = platform.plans_change(change, None)
            {
                self.step = Step::Wait(Expect::Passed {
                    plan,
                    due_s,
                    report: None,
                });
            }
            return true;
        }
        false
    }

    /// The stored claim behind a restart's uncertain feed, which the
    /// resolution reports as the feeder's own: the feed that followed the
    /// claim's high water is the claimed occurrence's.
    pub fn claim_behind<P: Platform>(platform: &mut P, feed: u32, by: Source) -> Option<Claim> {
        let PlansResult::Current { view, .. } = platform.plans() else {
            return None;
        };
        view.claim
            .filter(|claim| by == Source::Feeder && claim.high_water.wrapping_add(1) == feed)
    }
}

/// Record a missed occurrence: the event and the line.
fn missed<P: Platform>(
    platform: &mut P,
    now_ms: u32,
    plan: u8,
    due_s: u32,
    reason: MissedReason,
    record: &mut impl FnMut(Record),
) {
    events::record(
        platform,
        now_ms,
        EventBody::Missed {
            plan,
            due: due_s,
            reason,
        },
    );
    record(Record::Missed {
        plan,
        due_s,
        reason,
    });
}

/// What a refused claimed feed waited on, for a `Missed` event if the
/// refusals outlast the grace.
const fn blocked(refusal: Refusal) -> Blocked {
    match refusal {
        Refusal::Fault => Blocked::Fault,
        Refusal::NotWritable => Blocked::Journal,
        Refusal::Unavailable => Blocked::Unserved,
        Refusal::Busy
        | Refusal::Forbidden
        | Refusal::Stale { .. }
        | Refusal::Invalid
        | Refusal::Full
        | Refusal::NotFound
        | Refusal::Muted
        | Refusal::Time
        | Refusal::Unverified
        | Refusal::Portions { .. }
        | Refusal::NoClock
        | Refusal::Update(_) => Blocked::Feeding,
    }
}

/// Unix seconds of an occurrence, which is whole seconds (offsets are).
pub(super) fn seconds(due_ms: u64) -> u32 {
    u32::try_from(due_ms / 1000).unwrap_or(u32::MAX)
}

const fn missed_name(reason: MissedReason) -> &'static str {
    match reason {
        MissedReason::Late => "late",
        MissedReason::Interrupted => "interrupted",
        MissedReason::Blocked(_) => "blocked",
    }
}
