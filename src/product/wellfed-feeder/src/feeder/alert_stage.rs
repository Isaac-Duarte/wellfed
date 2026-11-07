//! The alert rules' turn in the loop ([`super::alerts`]): the boot read of
//! the ring, the readings each look at the conditions takes, and each plan's
//! latest occurrence against the schedule's record.
//!
//! **Read only.** It reads what the loop already keeps, and writes only the
//! table, so nothing it does can delay, take or block a meal. It runs after
//! the feeding plans. While a feed is queued or driven or a plan's claim is
//! in flight, the boot read and the look at each plan's meal wait, so a
//! drive's gaps see no flash read and no plan's table. The conditions go on
//! being looked at, or a claim that never finishes would freeze them.
//!
//! **The boot read.** A restart loses the table. Once the ring is open, the
//! newest [`BOOT_READ`] records it held then go to the rules, oldest first.
//! Records past that mark are this boot's, and came through the recorder, so
//! each event reaches the rules once. A read gets [`BOOT_READ_MS`] from the
//! ring's opening, and if the ring never opens the rules go on without it
//! after [`RING_WAIT_MS`]. On bank 1 the device API welcomes no session for
//! the first [`WELCOME_MS`] of the boot unless the read ends sooner, which
//! keeps it from taking a turn from theirs: sessions matter more than old
//! reports. Bank 0, the recovery bank, holds none back, so an installer's
//! reconnect after a give-way never waits on the read. A
//! read cut short says so, and the table's `more` holds until what it missed
//! could no longer have been listed.
//!
//! **An install's stop.** A stop is live state, which going idle erases. So
//! it is recorded when the stage first shows it, as an event of its own.
//! This does not depend on the rules, and writes nothing a rule asked for.
//!
//! **What the update store held.** Once in every boot, after its `Boot`, the
//! store's candidate goes to the ring, which is how the rules tell a fall to
//! bank 0 from an install's own restarts. It can't be recorded before the
//! restart: an activation or a give-way keeps the write turn until the reset,
//! so nothing queued then could land. The store's own record lands, as both
//! are written before the marker changes, and this boot reads it. A build
//! this boot activated that then confirms clears the candidate with no
//! restart, so that is recorded too, as `UpdateConfirmed`.

use wellfed_core::schedule::latest_due;
use wellfed_device_api::SESSIONS;
use wellfed_device_api::mailbox::{PLANS, PlansResult};
use wellfed_proto::api::{Blocked, EventBody, Readiness, Supply, UpdateCandidate, UpdateStop};

use super::alerts::History;
use super::alerts::Readings;
use super::events;
use super::platform::Platform;
use super::schedule::seconds;
use super::serve::desiccant_now;
use wellfed_device::store::events::Read;

/// Records the boot read covers. Sized for the 48 hours a report can last: a
/// busy day records well under this. What it costs at boot is unmeasured.
pub const BOOT_READ: u64 = 128;
/// How long the boot read may run, from the ring's opening.
pub const BOOT_READ_MS: u32 = 10_000;
/// How long the rules wait for the ring to open at all before going on
/// without its history.
pub const RING_WAIT_MS: u32 = 60_000;
/// The longest the boot read holds sessions back, from the boot's first pass.
pub const WELCOME_MS: u32 = 10_000;
/// Records the boot read decodes in one pass.
const READ_PER_PASS: usize = 32;
/// How often the conditions are looked at: their thresholds are minutes.
const LOOK_MS: u32 = 1_000;
const MINUTE_MS: u64 = 60_000;

/// A reader outside the session slots: it takes no turn from any.
const READER: wellfed_device_api::Reader = wellfed_device_api::Reader::slot(SESSIONS);

/// What the loop's pass knows that the stage reads.
#[derive(Clone, Copy, Debug)]
pub struct Inputs {
    pub ready: Readiness,
    /// Why the schedule is holding a meal, if it is.
    pub meal_held: Option<Blocked>,
    pub food: Option<bool>,
    /// A feed is queued or runs.
    pub feeding: bool,
    pub supply: Supply,
    /// A plan's claim is in flight.
    pub claim: bool,
    /// A feed is queued or driven, or a plan's claim is in flight.
    pub busy: bool,
    /// The schedule has nothing in hand, so its marks are its last word.
    pub schedule_at_rest: bool,
    /// This boot's `Boot` is in the queue.
    pub booted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BootRead {
    /// The ring is not open yet.
    Waiting,
    /// Next the record after `after`, up to `last`. `cut` says the ring held
    /// more than the read covers, and `oldest_s` is the oldest time it found.
    Reading {
        after: Option<u64>,
        last: u64,
        cut: bool,
        oldest_s: Option<u32>,
    },
    Done,
}

/// The stage's state in the loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlertStage {
    read: BootRead,
    /// The uptime of the first pass.
    began_ms: Option<u32>,
    /// The uptime the ring was first seen open.
    ring_open_ms: Option<u32>,
    /// The next look at the conditions.
    look_ms: Option<u32>,
    /// The wall-clock minute and the plans' revision the meals were judged at.
    meals: Option<(u64, u32)>,
    /// An install held the flash at the last look.
    installing: bool,
    /// The stop the stage showed at the last look.
    stop: Option<UpdateStop>,
    /// This boot's update store record is in the queue.
    candidate_noted: bool,
    /// This boot runs a build it activated, not yet seen to confirm.
    on_trial: bool,
    /// The next look at whether it has.
    trial_look_ms: Option<u32>,
}

impl Default for AlertStage {
    fn default() -> Self {
        Self::new()
    }
}

impl AlertStage {
    pub const fn new() -> Self {
        Self {
            read: BootRead::Waiting,
            began_ms: None,
            ring_open_ms: None,
            look_ms: None,
            meals: None,
            installing: false,
            stop: None,
            candidate_noted: false,
            on_trial: false,
            trial_look_ms: None,
        }
    }

    /// One pass at `now_ms`.
    #[inline(never)]
    pub fn step<P: Platform>(&mut self, platform: &mut P, now_ms: u32, inputs: &Inputs) {
        self.watch_install(platform, now_ms);
        self.note_candidate(platform, now_ms, inputs.booted);
        self.watch_trial(platform, now_ms);

        if !platform.alerts_on() {
            platform.alerts_mut().restored(now_ms, History::Whole);
            return;
        }
        self.give_up_reading(platform, now_ms);
        if !inputs.busy {
            self.read_boot(platform, now_ms);
        }

        let due = self
            .look_ms
            .is_none_or(|at| wellfed_core::deadline_passed(now_ms, at));
        if !due || self.read != BootRead::Done {
            return;
        }
        self.look_ms = Some(now_ms.wrapping_add(LOOK_MS));

        let wall_ms = platform.wall_ms(now_ms);
        let readings = Readings {
            wall_s: wall_ms.map(seconds),
            zoned: wall_ms.is_some_and(|ms| platform.offset_at(ms).is_some()),
            blocked: match inputs.ready {
                Readiness::Blocked(reason) => Some(reason),
                Readiness::Ready => inputs.meal_held,
            },
            food: inputs.food,
            feeding: inputs.feeding,
            desiccant: desiccant_now(platform, now_ms),
            supply: inputs.supply,
            claim: inputs.claim,
            plans_settled: plans_settled(platform),
        };
        platform.alerts_mut().step(now_ms, &readings);

        if inputs.schedule_at_rest && !inputs.busy {
            self.judge_meals(platform, now_ms);
        }
    }

    /// Records a stop as the update stage first shows it. Only around an
    /// install, as the progress is a copy under a paused scheduler.
    #[inline(never)]
    fn watch_install<P: Platform>(&mut self, platform: &mut P, now_ms: u32) {
        let installing = platform.installing();
        let was_installing = core::mem::replace(&mut self.installing, installing);
        if !installing && !was_installing {
            return;
        }

        let stop = platform
            .update_progress()
            .and_then(|progress| progress.stopped)
            .map(wellfed_device::update::stop);
        if stop != self.stop
            && let Some(stop) = stop
        {
            events::record(platform, now_ms, EventBody::UpdateStop { stop });
        }
        self.stop = stop;
    }

    /// Records what the update store held, once it is open.
    #[inline(never)]
    fn note_candidate<P: Platform>(&mut self, platform: &mut P, now_ms: u32, booted: bool) {
        if self.candidate_noted || !booted {
            return;
        }
        let Some(progress) = platform.update_progress() else {
            return;
        };

        let candidate = wellfed_device::update::candidate(progress.candidate);
        let body = EventBody::UpdateAtBoot { candidate };
        self.candidate_noted = events::record_or_wait(platform, now_ms, body);
        self.on_trial = self.candidate_noted && candidate == UpdateCandidate::Activated;
    }

    /// Records the build this boot activated confirming itself, which clears
    /// the store's candidate with no restart. Looked at once a second, and
    /// only on trial, as the progress is a copy under a paused scheduler. The
    /// event goes through the queue like any other, so the writer appends it
    /// under the write turn, never during a drive.
    #[inline(never)]
    fn watch_trial<P: Platform>(&mut self, platform: &mut P, now_ms: u32) {
        let due = self
            .trial_look_ms
            .is_none_or(|at| wellfed_core::deadline_passed(now_ms, at));
        if !self.on_trial || !due {
            return;
        }
        self.trial_look_ms = Some(now_ms.wrapping_add(LOOK_MS));

        let Some(progress) = platform.update_progress() else {
            return;
        };
        if wellfed_device::update::candidate(progress.candidate) == UpdateCandidate::None {
            self.on_trial = !events::record_or_wait(platform, now_ms, EventBody::UpdateConfirmed);
        }
    }

    /// The welcome's bound and the read's own, which start apart: a ring that
    /// opens late still gets its whole read, and holds no session longer.
    fn give_up_reading<P: Platform>(&mut self, platform: &mut P, now_ms: u32) {
        let began = *self.began_ms.get_or_insert(now_ms);
        if now_ms.wrapping_sub(began) >= WELCOME_MS {
            platform.alerts_mut().welcome();
        }
        if self.read == BootRead::Done {
            return;
        }

        if self.ring_open_ms.is_none() && platform.epoch().is_some() {
            self.ring_open_ms = Some(now_ms);
        }
        let over = match self.ring_open_ms {
            Some(open) => now_ms.wrapping_sub(open) >= BOOT_READ_MS,
            None => now_ms.wrapping_sub(began) >= RING_WAIT_MS,
        };
        if over {
            self.finish_reading(platform, now_ms, History::Unread);
        }
    }

    fn finish_reading<P: Platform>(&mut self, platform: &mut P, now_ms: u32, history: History) {
        self.read = BootRead::Done;
        platform.alerts_mut().restored(now_ms, history);
    }

    #[inline(never)]
    fn read_boot<P: Platform>(&mut self, platform: &mut P, now_ms: u32) {
        if self.read == BootRead::Waiting {
            if platform.epoch().is_none() {
                return;
            }
            let Some(last) = platform.ring_opened_at() else {
                self.finish_reading(platform, now_ms, History::Whole);
                return;
            };
            let after = last.checked_sub(BOOT_READ).filter(|&after| after > 0);
            self.read = BootRead::Reading {
                after,
                last,
                cut: after.is_some(),
                oldest_s: None,
            };
        }

        let wall_s = wall_seconds(platform, now_ms);
        for _ in 0..READ_PER_PASS {
            let BootRead::Reading {
                after,
                last,
                cut,
                oldest_s,
            } = self.read
            else {
                return;
            };
            // `None` is the writer's turn: the next pass asks again.
            let Some(read) = platform.event_after(READER, after) else {
                return;
            };

            let mut oldest_s = oldest_s;
            let next = match read {
                Read::Record(record) if record.seq <= last => {
                    if let Some(body) = wellfed_proto::decode_padded::<EventBody>(&record.body) {
                        platform
                            .alerts_mut()
                            .replay(now_ms, wall_s, record.at, &body);
                    }
                    oldest_s = oldest_s.or(record.at);
                    record.seq
                }
                Read::Gap { to, .. } if to < last => to,
                Read::Record(_) | Read::Gap { .. } | Read::End => last,
            };
            self.read = BootRead::Reading {
                after: Some(next),
                last,
                cut,
                oldest_s,
            };
            if next >= last {
                let history = if cut {
                    History::Cut { oldest_s }
                } else {
                    History::Whole
                };
                self.finish_reading(platform, now_ms, history);
            }
        }
    }

    /// Each enabled plan's latest occurrence, once a wall-clock minute or when
    /// the plans change, as the schedule works its own out. A store that can't
    /// be written runs nothing, and so comes to have no record of any meal,
    /// which is just what this is here to say.
    #[inline(never)]
    fn judge_meals<P: Platform>(&mut self, platform: &mut P, now_ms: u32) {
        let Some(wall_ms) = platform.wall_ms(now_ms) else {
            return;
        };
        let PlansResult::Current {
            view,
            pending: false,
        } = platform.plans()
        else {
            return;
        };
        let key = (wall_ms / MINUTE_MS, view.revision);
        if self.meals == Some(key) {
            return;
        }
        self.meals = Some(key);

        let wall_s = seconds(wall_ms);
        for slot in 0..PLANS {
            let Some(Some(plan)) = view
                .table
                .get(slot)
                .filter(|plan| plan.is_some_and(|plan| plan.enabled))
            else {
                continue;
            };
            let Some(due_ms) = latest_due(plan, wall_ms, |ms| platform.offset_at(ms)) else {
                continue;
            };
            let Ok(plan) = u8::try_from(slot) else {
                continue;
            };
            let mark = view.mark(slot);
            platform
                .alerts_mut()
                .meal(now_ms, wall_s, plan, seconds(due_ms), mark);
        }
    }
}

fn wall_seconds<P: Platform>(platform: &mut P, now_ms: u32) -> Option<u32> {
    platform.wall_ms(now_ms).map(seconds)
}

/// Whether the plans are in reach with no change in hand. Out of line, as the
/// plans' table is a large value to hold in the stage's frame.
#[inline(never)]
fn plans_settled<P: Platform>(platform: &mut P) -> bool {
    match platform.plans() {
        PlansResult::Current { pending, .. } => !pending,
        PlansResult::Busy => false,
        // Nothing to judge, which is no change that never ends.
        PlansResult::ReadOnly | PlansResult::Unserved => true,
    }
}
