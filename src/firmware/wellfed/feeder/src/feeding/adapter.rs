//! The loop's half of the feeding owner, built for the host as well as the
//! target: what turns the owner's status into the answers, `Fed` events
//! and log records clients and the bench see.
//!
//! It reaches the owner and the runtime only through [`FeedOwner`] and
//! [`Runtime`], so one implementation serves the firmware (the owner in its
//! static, the runtime's flags, the board's interlock) and the simulated
//! feeder (an owner of its own over the same feed journal), and host tests
//! drive it against a scripted owner. A change to the `Feeds` contract is
//! made here once.

use core::ops::Deref;

use wellfed_core_feeder::feed::food_read::FoodReads;
use wellfed_core_feeder::feed::observe::Observation;
use wellfed_core_feeder::feed::storage::{OwnerResult, OwnerStorage, RotorPhase};
use wellfed_feeder::feed::{self, Durability};
use wellfed_feeder::feeder::{Feeds, Marks, RefusedClaim};
use wellfed_feeder::trace::DriveFigures;
use wellfed_hal_feeder::owner::{Owner, OwnerFault, OwnerState, OwnerStatus, Step, SubmitResult};
use wellfed_proto::api::{Blocked, EventBody, Phase, Refusal, Source, TraceSummary};
use wellfed_sdk::hal::interlock::{self, Interlock, Turn};
use wellfed_sdk::hal::supervision::supervise;

use super::{
    Cadence, ClaimTurn, Claiming, InFlight, Opened, PowerCycle, Proof, Record, Recovery,
    drive_figures,
};

/// The feeding owner as the loop commands it: the firmware's
/// [`Owner`], or the simulated feeder's own.
pub trait FeedOwner {
    /// One owner iteration: how soon it wants the next, or `None` to be
    /// stepped again at the next pass (stopped, or refused).
    fn step(&self) -> Option<u32>;
    fn status(&self) -> OwnerStatus;
    fn submit(&self, id: u64, portions: u16) -> SubmitResult;
    fn cancel(&self, id: u64) -> bool;
    fn acknowledge(&self, id: u64) -> bool;
    fn high_water_id(&self) -> u64;
    fn faulted(&self) -> bool;
    /// What the last step saw of a feed, for its trace.
    fn take_observation(&self) -> Option<Observation>;
    /// End a food reading now, before a claim.
    fn yield_food_read(&self, interlock: &Interlock);
    /// Start a food reading at rest that is due, or end one `held` stops.
    fn food_read_step(&self, reads: &mut FoodReads, interlock: &Interlock, now_ms: u32, held: bool);
}

impl<S: OwnerStorage> FeedOwner for Owner<S> {
    fn step(&self) -> Option<u32> {
        match Owner::step(self) {
            Step::Wait(ms) => Some(ms),
            Step::Stopped | Step::Refused => None,
        }
    }

    fn status(&self) -> OwnerStatus {
        Owner::status(self)
    }

    fn submit(&self, id: u64, portions: u16) -> SubmitResult {
        Owner::submit(self, id, portions)
    }

    fn cancel(&self, id: u64) -> bool {
        Owner::cancel(self, id)
    }

    fn acknowledge(&self, id: u64) -> bool {
        Owner::acknowledge(self, id)
    }

    fn high_water_id(&self) -> u64 {
        Owner::high_water_id(self)
    }

    fn faulted(&self) -> bool {
        Owner::faulted(self)
    }

    fn take_observation(&self) -> Option<Observation> {
        Owner::take_observation(self)
    }

    fn yield_food_read(&self, interlock: &Interlock) {
        Owner::yield_food_read(self, interlock);
    }

    fn food_read_step(
        &self,
        reads: &mut FoodReads,
        interlock: &Interlock,
        now_ms: u32,
        held: bool,
    ) {
        Owner::food_read_step(self, reads, interlock, now_ms, held);
    }
}

/// The owner, opened: what [`Runtime::open`] came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opening {
    /// `Opened`, `Refused` (tried again next pass) or `Journal` (never
    /// this boot).
    pub record: Record,
    /// The feed a reset interrupted, which the owner resolves as it opens.
    pub interrupted: Option<u64>,
}

/// What the adapter reads from, and tells, the runtime it runs on.
pub trait Runtime {
    type Owner: FeedOwner;
    /// How the owner is held while it is reached.
    type Entered<'a>: Deref<Target = Self::Owner>
    where
        Self: 'a;

    /// The owner, once [`Self::open`] has filled it.
    fn owner(&self) -> Option<Self::Entered<'_>>;
    /// Open the journal (creating it on erased sectors, and noting that in
    /// `created`) and the owner on it; `None` while it cannot be tried.
    fn open(&self, created: &mut bool) -> Option<Opening>;
    /// The turn feeds take with joins and storage writes.
    fn interlock(&self) -> &Interlock;
    fn now_ms(&self) -> u32;
    /// A reset storm holds the feeder: the owner stays closed.
    fn held(&self) -> bool;
    /// A trial build waits to confirm.
    fn confirming(&self) -> bool;
    /// The flash hooks are armed: the journal may be written.
    fn storage_armed(&self) -> bool;
    /// An install holds the flash.
    fn installing(&self) -> bool;
    /// A change waits for the settings writer, which a claim lets go first.
    fn writes_queued(&self) -> bool;
    fn power_cycle(&self) -> PowerCycle;
    /// What the owner has shown, for a trial's confirmation.
    fn report_feeding(&self, feeding: supervise::Feeding);
    /// Restart the feeder (to recover lost sensors).
    fn request_reset(&self);
    /// One line for the log.
    fn record(&self, record: Record);
    /// Feed `feed` starts: its trace begins.
    fn trace_begin(&self, feed: u32);
    fn trace_observe(&self, seen: &Observation);
    /// The feed ended as `fed`: its trace's summary, if it kept one.
    fn trace_seal(&self, fed: &EventBody, drive: &DriveFigures) -> Option<TraceSummary>;
}

/// The feed in flight, from its submission until its result is recorded.
#[derive(Clone, Copy, Debug)]
struct Current {
    feed: u64,
    by: Source,
    /// The owner has reserved it in the journal.
    reserved: bool,
    /// The longest gap between two owner steps since it was queued.
    max_gap_ms: u32,
    /// Handshake operations the loop ran meanwhile.
    handshakes: u32,
}

/// The loop's half of the feeding owner on runtime `R`: when to open it,
/// and what it last logged.
#[derive(Debug, Default)]
pub struct Feeding<R> {
    runtime: R,
    state: State,
}

/// Everything but the runtime, so a method can hold the owner (borrowed
/// from the runtime) and change this at once.
#[derive(Debug, Default)]
struct State {
    /// Opened, or the journal failed: nothing more to try this boot.
    settled: bool,
    /// The journal could not be opened or created: no feed this boot.
    journal_failed: bool,
    /// When the owner was last stepped.
    stepped_ms: Option<u32>,
    /// When the owner is next due a step.
    cadence: Cadence,
    /// The journal was created this boot (by an attempt the owner may
    /// have refused).
    created: bool,
    refused: bool,
    logged: Option<OwnerStatus>,
    current: Option<Current>,
    /// The feed waiting to claim the outputs; never with `current`.
    claiming: Option<Claiming>,
    /// Why the last feed ended before its reservation, for the API's
    /// poll: its id and the refusal.
    unreserved: Option<(u64, Refusal)>,
    /// The interlock's counters as last logged, for its `joins` and
    /// `interlock` records.
    interlock: interlock::Status,
    /// The reservation a reset interrupted, which the owner resolves as
    /// it opens: its id, until its `Fed` event is recorded.
    interrupted: Option<u64>,
    /// What the owner has shown, for a trial's confirmation.
    proof: Proof,
    /// When to restart to recover lost sensors.
    recovery: Recovery,
    /// Feeds this boot that began with a homing mark.
    homed: u32,
    /// Outlet-beam breaks this boot's feeds saw.
    outlet_breaks: u32,
    /// When to read the food beam at rest.
    food_reads: FoodReads,
}

// These two split `interlock::Status` as the `joins` and `interlock`
// records print it: a counter added to the status goes in one of them
// and in that record's `Display` arm, or its moves are never logged.

/// What the `joins` record logs.
fn join_counters(status: &interlock::Status) -> [u32; 5] {
    [
        status.refused,
        status.deferred,
        status.forced,
        status.waited_max_ms,
        status.long_joins,
    ]
}

/// What the `interlock` record logs.
fn write_counters(status: &interlock::Status) -> [u32; 4] {
    [
        status.writes_deferred,
        status.writes_forced,
        status.write_waited_max_ms,
        status.feeds_delayed,
    ]
}

impl<R: Runtime> Feeding<R> {
    pub fn new(runtime: R) -> Self {
        Self {
            runtime,
            state: State::default(),
        }
    }

    pub fn runtime(&self) -> &R {
        &self.runtime
    }

    /// One pass; the owner's delay while it runs. `setup` is whether
    /// Bluetooth setup runs. A feed's `Fed` event and its trace's summary
    /// go to `outcome`.
    pub fn step(
        &mut self,
        setup: bool,
        mut outcome: impl FnMut(EventBody, Option<&TraceSummary>),
    ) -> Option<u32> {
        let rt = &self.runtime;
        let state = &mut self.state;
        let now = rt.now_ms();
        if let Some(wait) = state.cadence.wait(now) {
            return Some(wait);
        }
        // While a reset storm holds the feeder the owner stays closed,
        // the outputs as startup left them, stopped. It opens once the
        // hold ends.
        if rt.held() {
            return None;
        }
        if !state.settled {
            if !rt.storage_armed() {
                rt.report_feeding(state.proof.step(Opened::Not, now));
                return None;
            }
            state.open(rt);
        }
        if state.journal_failed || !state.settled {
            let opened = if state.journal_failed {
                Opened::JournalFailed
            } else {
                Opened::Not
            };
            rt.report_feeding(state.proof.step(opened, now));
        }

        let owner = rt.owner()?;
        if let (Some(current), Some(last)) = (state.current.as_mut(), state.stepped_ms) {
            current.max_gap_ms = current.max_gap_ms.max(now.wrapping_sub(last));
        }
        state.stepped_ms = Some(now);
        let held = super::food_read_held(
            rt.confirming(),
            setup,
            state.in_flight(),
            state.interrupted.is_some(),
            rt.writes_queued(),
        );
        owner.food_read_step(&mut state.food_reads, rt.interlock(), now, held);
        let delay = owner.step();
        state.cadence.stepped(now, delay);
        // After the step, so the trace never delays a stop or a
        // motor decision.
        if let Some(seen) = owner.take_observation() {
            rt.trace_observe(&seen);
        }

        let status = owner.status();
        let key = |s: &OwnerStatus| {
            (
                s.state,
                s.fault,
                s.supervisor_fault,
                s.stopped,
                s.optics_off,
            )
        };
        if state.logged.as_ref().map(key) != Some(key(&status)) {
            state.logged = Some(status);
            rt.record(Record::Status {
                state: status.state,
                fault: status.fault,
                supervisor: status.supervisor_fault,
                stopped: status.stopped,
                optics_off: status.optics_off,
            });
        }

        state.follow(rt, &*owner, &status, &mut outcome);
        state.release(rt, &status);
        if state.settled && !state.journal_failed {
            let opened = Opened::Owner {
                status: &status,
                resolving: state.interrupted.is_some(),
            };
            rt.report_feeding(state.proof.step(opened, now));
        }
        state.recover(rt, &status, now);
        delay
    }

    /// The pass ran a handshake operation during the feed in flight.
    pub fn handshook(&mut self) {
        if let Some(current) = self.state.current.as_mut() {
            current.handshakes = current.handshakes.saturating_add(1);
        }
    }

    /// The feed whose motor the owner is driving, for the
    /// fault-injection variants.
    #[cfg(any(feature = "feed-stall", feature = "stop-fail"))]
    pub fn driving(&self) -> Option<crate::faults::Driving> {
        let status = self.runtime.owner()?.status();
        (status.state == OwnerState::Feeding).then_some(crate::faults::Driving {
            feed: status.result.id,
            portions: status.result.requested,
        })
    }

    /// The feed the owner has, from its queueing until its result, for
    /// the jam and sensor-fault variants.
    #[cfg(any(feature = "feed-jam", feature = "sensor-fault"))]
    pub fn phase(&self) -> Option<crate::feed_faults::Phase> {
        let status = self.runtime.owner()?.status();
        matches!(
            status.state,
            OwnerState::Queued | OwnerState::Preparing | OwnerState::Feeding
        )
        .then_some(crate::feed_faults::Phase {
            feed: status.result.id,
            portions: status.result.requested,
            driving: status.state == OwnerState::Feeding,
        })
    }
}

impl State {
    /// Open the owner, logging how that went: a refusal once, until it
    /// opens or the journal fails.
    fn open<R: Runtime>(&mut self, rt: &R) {
        let Some(opening) = rt.open(&mut self.created) else {
            return;
        };
        match opening.record {
            Record::Refused if self.refused => {}
            Record::Refused => {
                self.refused = true;
                rt.record(Record::Refused);
            }
            opened => {
                self.settled = true;
                self.journal_failed = matches!(opened, Record::Journal { .. });
                if !self.journal_failed {
                    self.interrupted = opening.interrupted;
                }
                rt.record(opened);
            }
        }
    }

    /// The one restart a power cycle allows to recover lost sensors
    /// (`Recovery`), through the deliberate restart every reboot takes,
    /// which no reset storm counts. A held boot never gets here: its
    /// owner stays closed.
    fn recover<R: Runtime>(&mut self, rt: &R, status: &OwnerStatus, now: u32) {
        let held = status.state == OwnerState::Fault
            && status.fault == OwnerFault::Sensor
            && status.stopped
            && status.optics_off
            && self.in_flight() == InFlight::Nothing
            && !rt.installing();
        let held_ms = self.recovery.held_ms(now);
        if self.recovery.step(now, held.then(|| rt.power_cycle())) {
            rt.record(Record::Recover { held_ms });
            rt.request_reset();
        }
    }

    /// Hands the outputs' turn back by `may_release`, and watches for a
    /// join that holds its turn too long.
    fn release<R: Runtime>(&mut self, rt: &R, status: &OwnerStatus) {
        let interlock = rt.interlock();
        if super::may_release(self.in_flight(), status) {
            interlock.release_feed();
        }
        interlock.watch_join(rt.now_ms());

        // Each half is logged whenever one of its counters moves: a
        // refused feed, a join that starts waiting, waits longer than
        // any before, is forced, or holds its turn past the long-join
        // limit; a write that waits or is forced, or a feed delayed.
        let now = interlock.status();
        let before = core::mem::replace(&mut self.interlock, now);
        if join_counters(&now) != join_counters(&before) {
            rt.record(Record::Joins(now));
        }
        if write_counters(&now) != write_counters(&before) {
            rt.record(Record::Writes(now));
        }
    }

    /// The feed in flight against the owner's status: note its
    /// reservation, and record its end.
    fn follow<R: Runtime>(
        &mut self,
        rt: &R,
        owner: &R::Owner,
        status: &OwnerStatus,
        outcome: &mut impl FnMut(EventBody, Option<&TraceSummary>),
    ) {
        let Some(current) = self.current.as_mut() else {
            self.report_interrupted(rt, owner, status, outcome);
            return;
        };
        let ours = status.result.id == current.feed;
        if ours && status.intent_reserved {
            current.reserved = true;
        }

        let durability = match status.state {
            OwnerState::Terminal if ours && status.result_durable => Durability::Durable,
            OwnerState::Fault if ours && status.result_durable => Durability::Durable,
            OwnerState::Fault if current.reserved => Durability::Lost,
            OwnerState::Fault => {
                self.unreserved = Some((current.feed, Refusal::Fault));
                self.current = None;
                return;
            }
            _ => return,
        };

        let current = *current;
        self.current = None;
        let result = if ours {
            status.result
        } else {
            OwnerResult {
                id: current.feed,
                ..OwnerResult::default()
            }
        };
        rt.record(Record::Fed {
            result,
            durability,
            trace: status.trace,
        });
        rt.record(Record::Current {
            feed: result.id,
            trace: status.trace,
        });
        rt.record(Record::Edges {
            feed: result.id,
            trace: status.trace,
        });
        rt.record(Record::Steps {
            feed: result.id,
            max_gap_ms: current.max_gap_ms,
            handshakes: current.handshakes,
        });
        let outlet_breaks = super::outlet_breaks(&status.trace.pins);
        if status.trace.homed_ms.is_some() {
            self.homed = self.homed.saturating_add(1);
        }
        self.outlet_breaks = self.outlet_breaks.saturating_add(outlet_breaks);
        rt.record(Record::Marks {
            feed: result.id,
            homed_ms: status.trace.homed_ms,
            outlet_breaks,
            phase: status.phase,
        });
        if status.trace.pins.start.is_some() {
            rt.record(Record::Pins {
                feed: result.id,
                pins: status.trace.pins,
            });
        }
        // The feed id fits the event: `submit` refused any that does not.
        let feed = u32::try_from(current.feed).unwrap_or(u32::MAX);
        // The trace is this feed's only if the owner's result is.
        let hopper = status.trace.hopper.filter(|_| ours);
        let body = feed::fed(feed, &result, hopper, durability, current.by);
        let summary = rt.trace_seal(&body, &drive_figures(&status.trace));
        outcome(body, summary.as_ref());
        if status.state == OwnerState::Terminal {
            owner.acknowledge(current.feed);
        }
    }

    /// The interrupted feed's uncertain result, once the owner holds it
    /// durably: recorded once, then acknowledged. An owner that faulted
    /// after the resolution still holds it; its event is recorded all
    /// the same, with nothing to acknowledge.
    fn report_interrupted<R: Runtime>(
        &mut self,
        rt: &R,
        owner: &R::Owner,
        status: &OwnerStatus,
        outcome: &mut impl FnMut(EventBody, Option<&TraceSummary>),
    ) {
        let Some(feed) = self.interrupted else {
            return;
        };
        if !status.result_durable || status.result.id != feed {
            return;
        }
        match status.state {
            OwnerState::Terminal if owner.acknowledge(feed) => {}
            OwnerState::Fault => {}
            _ => return,
        }

        self.interrupted = None;
        rt.record(Record::Fed {
            result: status.result,
            durability: Durability::Durable,
            trace: status.trace,
        });
        rt.record(Record::Current {
            feed,
            trace: status.trace,
        });
        // The journal holds the id under the high water `submit` checks.
        let number = u32::try_from(feed).unwrap_or(u32::MAX);
        // Its trace, if it had one, was lost with the RAM it was in.
        let body = feed::interrupted(number, &status.result, Source::Feeder);
        outcome(body, None);
    }

    /// One try of `claiming`. A reading ends first, each try: a feed goes
    /// before one, and lit emitters would outlast the writes it waits for.
    fn try_claim<R: Runtime>(
        rt: &R,
        claiming: &mut Claiming,
        owner: &R::Owner,
        hold: Option<Blocked>,
    ) -> ClaimTurn {
        owner.yield_food_read(rt.interlock());
        let portions = claiming.portions;
        let now = rt.now_ms();
        claiming.step(rt.interlock(), hold, rt.writes_queued(), now, |held| {
            rt.record(Record::Held { portions, held });
        })
    }

    /// The claim is taken: the owner gets the feed under the number it was
    /// promised. Only a reservation moves the journal's high water and no
    /// other feed starts while one waits, so that is still the next.
    fn hand_over<R: Runtime>(
        &mut self,
        rt: &R,
        owner: &R::Owner,
        claiming: &Claiming,
    ) -> Result<u32, Refusal> {
        let (number, portions, by) = (claiming.feed, claiming.portions, claiming.by);
        let feed = u64::from(number);
        if owner.high_water_id().wrapping_add(1) != feed {
            return Err(Refusal::Busy);
        }

        let result = owner.submit(feed, u16::from(portions));
        rt.record(Record::Submitted {
            feed,
            portions,
            result,
        });
        match result {
            SubmitResult::Enqueued => {
                rt.trace_begin(number);
                self.current = Some(Current {
                    feed,
                    by,
                    reserved: false,
                    max_gap_ms: 0,
                    handshakes: 0,
                });
                // Gaps count from the first step after the feed was
                // queued, not from one before it.
                self.stepped_ms = None;
                self.unreserved = None;
                Ok(number)
            }
            SubmitResult::RejectRequest => Err(Refusal::Portions { asked: portions }),
            SubmitResult::RejectStopped if owner.faulted() => Err(Refusal::Fault),
            // Still opening, busy, or a phase only a double bowl needs.
            SubmitResult::RejectStopped
            | SubmitResult::RejectBusy
            | SubmitResult::RejectId
            | SubmitResult::RejectPhase => Err(Refusal::Busy),
        }
    }

    /// Whether the loop has a feed between its asking and its recorded
    /// result, its claim taken or not.
    fn in_flight(&self) -> InFlight {
        if self.current.is_some() || self.claiming.is_some() {
            InFlight::Feed
        } else {
            InFlight::Nothing
        }
    }
}

impl<R: Runtime> Feeds for Feeding<R> {
    /// A feed is being prepared or driven: the loop defers handshakes.
    fn feeding(&self) -> bool {
        self.runtime.owner().is_some_and(|owner| {
            matches!(
                owner.status().state,
                OwnerState::Preparing | OwnerState::Feeding
            )
        })
    }

    /// A feed is queued or runs: no install starts.
    fn busy(&self) -> bool {
        self.state.in_flight() == InFlight::Feed || self.feeding()
    }

    /// The owner's own reason not to start a feed now.
    fn blocked(&self) -> Option<Blocked> {
        let (rt, state) = (&self.runtime, &self.state);
        if state.journal_failed {
            return Some(Blocked::Journal);
        }
        // While a reset storm holds the feeder, until the hold ends.
        if rt.held() {
            return Some(Blocked::Fault);
        }
        // A trial build feeds once confirmed: the confirmation must
        // never wait for a feed towards the trial's deadline.
        if rt.confirming() {
            return Some(Blocked::Starting);
        }
        let owner = rt.owner();
        let Some(status) = owner
            .as_ref()
            .filter(|_| state.settled)
            .map(|owner| owner.status())
        else {
            return Some(Blocked::Starting);
        };
        match status.state {
            // A stop asked for (a restart on its way) serves no feed
            // either, until the next boot.
            OwnerState::Fault | OwnerState::Stopped => return Some(Blocked::Fault),
            OwnerState::Starting => return Some(Blocked::Starting),
            OwnerState::Idle
            | OwnerState::Queued
            | OwnerState::Preparing
            | OwnerState::Feeding
            | OwnerState::Reporting
            | OwnerState::Terminal => {}
        }
        if state.interrupted.is_some() {
            return Some(Blocked::Resolving);
        }
        if state.in_flight() == InFlight::Feed || status.state != OwnerState::Idle {
            return Some(Blocked::Feeding);
        }
        // On the pack below its floor, a feed would only sag it
        // further; the owner would refuse it at the start anyway.
        if status.supply_low {
            return Some(Blocked::Battery);
        }
        match rt.interlock().turn() {
            Turn::Joining | Turn::FeedingJoinWaits => Some(Blocked::Joining),
            Turn::Free | Turn::Feeding => None,
        }
    }

    /// The owner's verdict on the pack's floor, once it has judged.
    fn below_floor(&self) -> Option<bool> {
        self.runtime
            .owner()
            .and_then(|owner| owner.status().below_floor)
    }

    /// The newest feed the journal holds, once the owner has opened
    /// it: a schedule's claim names it, and its feed is the next.
    fn high_water(&self) -> Option<u32> {
        let owner = self.opened()?;
        u32::try_from(owner.status().high_water_id).ok()
    }

    /// Hand one feed to the owner on `by`'s behalf: the number its
    /// journal will hold it under.
    fn submit(&mut self, portions: u8, by: Source) -> Result<u32, Refusal> {
        // The scheduling measurement runs with nobody at the motor, so its
        // image cannot start a feed: the owner stays at rest.
        if cfg!(feature = "gap-probe") {
            return Err(Refusal::Unavailable);
        }
        let (rt, state) = (&self.runtime, &mut self.state);
        let owner = rt.owner().ok_or(Refusal::Busy)?;
        // An install holds the flash; the owner refuses it as well.
        if state.in_flight() == InFlight::Feed || rt.installing() {
            return Err(Refusal::Busy);
        }
        let number = u32::try_from(owner.high_water_id().wrapping_add(1))
            .map_err(|_| Refusal::NotWritable)?;

        let mut claiming = Claiming::new(number, portions, by, rt.now_ms());
        match State::try_claim(rt, &mut claiming, &owner, None) {
            ClaimTurn::Claimed => state.hand_over(rt, &owner, &claiming),
            ClaimTurn::Refused(refusal) => Err(refusal),
            ClaimTurn::Waiting => {
                state.claiming = Some(claiming);
                Ok(number)
            }
        }
    }

    /// Who asked for the feed waiting to claim the outputs.
    fn claiming(&self) -> Option<Source> {
        self.state.claiming.as_ref().map(|claiming| claiming.by)
    }

    /// One pass's try of the feed waiting to claim the outputs: its
    /// refusal, once it has one. `hold` is why it must not start now.
    fn claim_step(&mut self, hold: Option<Blocked>) -> Option<RefusedClaim> {
        let (rt, state) = (&self.runtime, &mut self.state);
        let owner = rt.owner()?;
        let mut claiming = state.claiming.take()?;

        let turn = State::try_claim(rt, &mut claiming, &owner, hold);
        let refusal = match turn {
            ClaimTurn::Waiting => {
                state.claiming = Some(claiming);
                return None;
            }
            ClaimTurn::Refused(refusal) => refusal,
            // The owner's own checks may still refuse it; the claim then
            // goes back at its next step, as for any refused feed.
            ClaimTurn::Claimed => match state.hand_over(rt, &owner, &claiming) {
                Ok(_) => return None,
                Err(refusal) => refusal,
            },
        };
        state.unreserved = Some((u64::from(claiming.feed), refusal));
        Some(RefusedClaim {
            feed: claiming.feed,
            by: claiming.by,
            refusal,
        })
    }

    /// Whether the feed `feed` is reserved yet: `None` while it waits.
    fn reserved(&self, feed: u32) -> Option<Result<(), Refusal>> {
        let state = &self.state;
        let feed = u64::from(feed);
        let claiming = state.claiming.as_ref();
        if claiming.is_some_and(|claiming| u64::from(claiming.feed) == feed) {
            return None;
        }
        match (state.current, state.unreserved) {
            (Some(current), _) if current.feed == feed => current.reserved.then_some(Ok(())),
            (_, Some((ended, refusal))) if ended == feed => Some(Err(refusal)),
            // Not the feed in flight nor the last one unreserved: it was
            // reserved and has ended. A feed ends unreserved when the
            // owner faults before reserving it, and a faulted owner takes
            // no feed until a restart (which empties the server's memory
            // of ids); or when its claim is refused, which the server
            // hears at once (`RefusedClaim`) and answers itself, since the
            // next feed takes the same number. So nothing still asks after
            // a feed a later one replaced here.
            _ => Some(Ok(())),
        }
    }

    /// Stop the feed `feed` if it is still running. One still waiting to
    /// claim the outputs is not running: its number has been announced to
    /// nobody, a cancel could not promise the `Fed` event a cancel means,
    /// and a refused claim's number goes to the next feed.
    fn cancel(&self, feed: u32) -> bool {
        let feed = u64::from(feed);
        self.state
            .current
            .is_some_and(|current| current.feed == feed)
            && self.runtime.owner().is_some_and(|owner| owner.cancel(feed))
    }

    /// The food beam's latest reading, from a feed or a reading at rest.
    fn food(&self) -> Option<bool> {
        self.runtime.owner()?.status().food
    }

    /// Where the rotor rests, and this boot's homing feeds and outlet
    /// breaks, once the owner has opened.
    fn marks(&self) -> Option<Marks> {
        let owner = self.opened()?;
        let phase = match owner.status().phase {
            RotorPhase::AtMark => Phase::AtMark,
            RotorPhase::Unknown => Phase::Unknown,
        };
        Some(Marks {
            phase,
            homed: self.state.homed,
            outlet_breaks: self.state.outlet_breaks,
        })
    }
}

impl<R: Runtime> Feeding<R> {
    /// The owner, once it has opened on a journal that opened.
    fn opened(&self) -> Option<R::Entered<'_>> {
        let state = &self.state;
        self.runtime
            .owner()
            .filter(|_| state.settled && !state.journal_failed)
    }
}
