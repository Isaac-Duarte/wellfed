//! The simulator's feeding: the firmware's own adapter
//! (`wellfed_fw_feeder::feeding::adapter`) over an owner with no motor, on
//! the feeder's own feed journal (`wellfed_core_feeder::feed_journal`) in
//! the storage region's journal sectors. So feed numbers, refusals,
//! readiness, reservations and the resolution of an interrupted feed are
//! the firmware's, decided by the firmware's code.
//!
//! The owner here ([`SimOwner`]) keeps the real owner's states:
//!
//! - A feed is queued under the next journal id, which is its number, and
//!   reserved durably in the journal at the owner's next step: the device
//!   API answers `Feeding` only once it is reserved, so a session that ends
//!   in between leaves a feed that runs with no answer sent. Numbers keep
//!   counting across restarts, as the journal's high water does. A
//!   reservation that cannot be written faults the owner, as the real one
//!   faults on storage, and the feed ends refused `Fault`.
//! - It then "drives" for [`PORTION_MS`] per portion of the loop's time, and
//!   its result goes into the journal before its `Fed` event is recorded; a
//!   result that cannot be written faults the owner and the feed is
//!   recorded as uncertain. A cancel ends it at the next step with the
//!   portions driven so far.
//! - A restart while a feed is reserved leaves the reservation pending: the
//!   next boot's owner resolves it as uncertain under its own number as it
//!   opens, at the loop's first step, and the adapter records that `Fed`
//!   event (by the feeder) and refuses feeds as `Resolving` until it has. A
//!   feed queued but not yet reserved is lost with the restart, as on the
//!   feeder: the motor never ran, and no `Fed` event names it.
//! - The pack's floor is the firmware's (`wellfed_core_feeder::floor::Floor`
//!   against `wellfed_hal_feeder::power::FLOOR`), judged on every step from
//!   the simulated supply: on the pack alone, readiness says `Battery` until
//!   the pack is judged above it. A feed already running is not stopped by a
//!   sag; the firmware's owner and its plant (`wellfed-sim`) cover that.
//! - [`SimFeeds::stall`] holds a feed in flight where it is, as a motor
//!   that stopped moving would, so a test can act while a feed runs.
//! - [`SimFeeds::fault`] makes the next feed end on a fault once it has
//!   driven a portion's time, as the firmware's owner ends it: a blocked
//!   outlet or a jam no reversal freed, `Failed` with one portion delivered
//!   (none for a feed of one, which ends before its mark), never resumed
//!   or retried.
//! - [`SimFeeds::refuse`] makes the owner refuse every feed `Busy` once its
//!   claim is taken, as an owner that lost a race does: the refusal after a
//!   claim the device API and the server must handle.
//! - [`SimFeeds::hold_write`] holds a storage write on a claim's way, on an
//!   interlock of its own, as the settings writer does: a feed then waits to
//!   claim the outputs once a pass, for at most `WRITE_WAIT_MS`, and is
//!   refused past it, the firmware's claim on the firmware's interlock.
//! - The hopper has food until [`SimFeeds::fill`] empties it. Each feed
//!   reads it as the food beam would, runs whatever it reads (as the
//!   firmware's and stock's do) and records the reading in its `Fed`
//!   event; the reading stays the feeder's food level until the next one.
//!   With no feed the beam is read on the firmware's own schedule
//!   (`FoodReads`), on the adapter only, each reading taking no time here.
//!
//! What only the board has stays out: traces, the trial's proof, the
//! recovery restart and the install lease.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wellfed_board::storage;
use wellfed_core_feeder::feed::controller::{FeedReason, FeedRequest};
use wellfed_core_feeder::feed::food_read::{FoodReads, FoodStep, FoodView};
use wellfed_core_feeder::feed::observe::Observation;
use wellfed_core_feeder::feed::progress::{BowlMode, Direction};
use wellfed_core_feeder::feed::storage::{OwnerOutcome, OwnerRecovery, OwnerResult, RotorPhase};
use wellfed_core_feeder::feed_journal::{FeedJournal, JournalConfig, JournalError};
use wellfed_core_feeder::floor::Floor;
use wellfed_feeder::trace::DriveFigures;
use wellfed_fw_feeder::feeding::adapter::{FeedOwner, Feeding, Opening, Runtime};
use wellfed_fw_feeder::feeding::{PowerCycle, Record};
use wellfed_hal::interlock::{Interlock, WriteStep, WriteTurn};
use wellfed_hal::supervision::supervise;
use wellfed_hal_feeder::owner::{FeedTrace, OwnerFault, OwnerState, OwnerStatus, SubmitResult};
use wellfed_proto::api::{EventBody, PORTIONS_MAX, TraceSummary};

use crate::flash::{Flash, Region};
use crate::platform::SimSupply;

/// How long one portion takes to drive.
pub const PORTION_MS: u32 = 200;

/// How often the owner asks to be stepped while a feed runs: the firmware
/// owner's period, so a cancel lands as soon as it would there.
const STEP_MS: u32 = 20;

/// The simulator's journal: its own binding (nonzero, never a real
/// board's), the firmware's bowl and per-feed cap.
const JOURNAL: JournalConfig = JournalConfig {
    binding: 0x5349_4d5f_4645_4544,
    mode: BowlMode::Single,
    max_portions: PORTIONS_MAX as u16,
};

/// The simulated feeder's feeding: the firmware's adapter on [`SimRuntime`].
#[derive(Debug)]
pub struct SimFeeds {
    feeding: Feeding<SimRuntime>,
}

impl SimFeeds {
    /// The feeding of a boot on `flash`'s journal. The owner opens at the
    /// first step, creating the journal on erased sectors and resolving a
    /// feed a restart interrupted.
    pub fn open(flash: &Rc<Flash>) -> Self {
        let runtime = SimRuntime {
            flash: Rc::clone(flash),
            owner: SimOwner::default(),
            opened: Cell::new(false),
            interlock: Interlock::new(),
        };
        Self {
            feeding: Feeding::new(runtime),
        }
    }

    /// The `Feeds` the loop's pass and the device API reach.
    pub fn feeds(&mut self) -> &mut Feeding<SimRuntime> {
        &mut self.feeding
    }

    /// The same, to read.
    pub fn adapter(&self) -> &Feeding<SimRuntime> {
        &self.feeding
    }

    fn owner(&self) -> &SimOwner {
        &self.feeding.runtime().owner
    }

    /// Judge the supply at `now_ms`, as the owner does on each poll: the
    /// adapter, and the battery channel's pin millivolts. With no pack the
    /// floor stays unjudged, so the simulator's default (the adapter, no
    /// cells) records no floor crossing; on the pack alone that leaves
    /// feeds refused.
    pub fn sense(&mut self, now_ms: u32, supply: SimSupply) {
        let owner = self.owner();
        owner.now_ms.set(now_ms);
        let mut inner = owner.inner.borrow_mut();
        inner.adapter = supply.adapter;
        if let Some(pin_mv) = supply.pin_mv {
            inner
                .floor
                .update(now_ms, Some(pin_mv), &wellfed_hal_feeder::power::FLOOR);
        }
    }

    /// End the next feed on `reason` once it has driven a portion's time:
    /// an outlet that blocks or a jam that holds. A feed of more than one
    /// portion ends with that one delivered; a feed of one ends `Failed`
    /// with none, since it ends before its last mark.
    pub fn fault(&mut self, reason: FeedReason) {
        self.owner().inner.borrow_mut().inject = Some(reason);
    }

    /// Put food in front of the beam (`true`), or empty the hopper.
    pub fn fill(&mut self, food: bool) {
        self.owner().inner.borrow_mut().hopper = food;
    }

    /// Hold a feed in flight where it is, or let it go on.
    pub fn stall(&mut self, stalled: bool) {
        self.owner().inner.borrow_mut().stalled = stalled;
    }

    /// Refuse every feed `Busy` once its claim is taken, or stop.
    pub fn refuse(&mut self, refusing: bool) {
        self.owner().inner.borrow_mut().refusing = refusing;
    }

    /// Hold a storage write in the way of a feed's claim, or end it. A
    /// hold is refused while a feed holds the outputs or a write is held
    /// already, where the firmware's writer would wait.
    pub fn hold_write(&mut self, held: bool) -> Result<(), WriteWaits> {
        let runtime = self.feeding.runtime();
        if !held {
            runtime.interlock.end_write();
            return Ok(());
        }

        // Refused before the turn is asked for: a writer the interlock makes
        // wait counts among its waiters until its turn comes, and a hold
        // that is refused never asks again, so its count would hold every
        // later claim back for the boot. The simulator runs on one thread
        // and nothing here begins a food reading, so these two are all that
        // would make it wait (bar held writes, which count no waiter).
        if runtime.interlock.outputs_claimed() || runtime.interlock.writing() {
            return Err(WriteWaits);
        }
        let now = runtime.owner.now_ms.get();
        match WriteTurn::new().begin(&runtime.interlock, now, || true) {
            WriteStep::Go(_) => Ok(()),
            WriteStep::Wait => Err(WriteWaits),
        }
    }

    /// Whether the outputs are claimed: a feed's claim was taken and it has
    /// not ended.
    pub fn outputs_claimed(&self) -> bool {
        self.feeding.runtime().interlock.outputs_claimed()
    }

    /// Step the adapter at `now_ms`: the `Fed` event of a feed that ended,
    /// if one did, and how soon to step again while the owner asks.
    pub fn step(&mut self, now_ms: u32) -> (Option<EventBody>, Option<u32>) {
        self.owner().now_ms.set(now_ms);
        let mut fed = None;
        let delay = self.feeding.step(false, |body, _| fed = Some(body));
        (fed, delay)
    }
}

/// A held write the interlock made wait: a feed holds the outputs, or a
/// write is held already.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the write waits: a feed holds the outputs or another write is held")]
pub struct WriteWaits;

/// The adapter's runtime in the simulator: the owner below, an interlock
/// of its own, and nothing of the board's (no trial, no reset storm, no
/// install lease, no traces).
#[derive(Debug)]
pub struct SimRuntime {
    flash: Rc<Flash>,
    owner: SimOwner,
    /// The owner has opened on a journal that opened.
    opened: Cell<bool>,
    interlock: Interlock,
}

impl Runtime for SimRuntime {
    type Owner = SimOwner;
    type Entered<'a> = &'a SimOwner;

    fn owner(&self) -> Option<&SimOwner> {
        self.opened.get().then_some(&self.owner)
    }

    fn open(&self, created: &mut bool) -> Option<Opening> {
        let io = Region::new(&self.flash, storage::JOURNAL);
        let mut journal = FeedJournal::default();
        let mut error = journal.open(io.clone(), JOURNAL);
        if error == JournalError::Empty {
            *created = true;
            error = journal.create(io, JOURNAL, Direction::D0, true, true);
        }
        if error != JournalError::Ok {
            return Some(Opening {
                record: Record::Journal {
                    error,
                    created: *created,
                },
                interrupted: None,
            });
        }

        let high_water = journal.view.recovery.high_water_id;
        let interrupted = journal.view.recovery.pending_request.then_some(high_water);
        self.owner.open(journal, interrupted.is_some());
        self.opened.set(true);
        Some(Opening {
            record: Record::Opened {
                created: *created,
                high_water,
            },
            interrupted,
        })
    }

    fn interlock(&self) -> &Interlock {
        &self.interlock
    }

    fn now_ms(&self) -> u32 {
        self.owner.now_ms.get()
    }

    fn held(&self) -> bool {
        false
    }

    fn confirming(&self) -> bool {
        false
    }

    fn storage_armed(&self) -> bool {
        true
    }

    fn installing(&self) -> bool {
        false
    }

    fn writes_queued(&self) -> bool {
        false
    }

    fn power_cycle(&self) -> PowerCycle {
        PowerCycle::Restarted
    }

    fn report_feeding(&self, _: supervise::Feeding) {}

    fn request_reset(&self) {}

    fn record(&self, _: Record) {}

    fn trace_begin(&self, _: u32) {}

    fn trace_observe(&self, _: &Observation) {}

    fn trace_seal(&self, _: &EventBody, _: &DriveFigures) -> Option<TraceSummary> {
        None
    }
}

/// An owner with no motor, on the feed journal: the real owner's states
/// and refusals, a drive that is only time.
#[derive(Debug, Default)]
pub struct SimOwner {
    /// The uptime as the last step or sense read it.
    now_ms: Cell<u32>,
    inner: RefCell<Inner>,
}

#[derive(Debug)]
struct Inner {
    journal: Option<FeedJournal<Region>>,
    state: OwnerState,
    fault: OwnerFault,
    /// The feed queued, driven or last ended.
    result: OwnerResult,
    intent_reserved: bool,
    result_durable: bool,
    /// How long the feed has driven, and when it was last stepped.
    driven_ms: u32,
    stepped_ms: Option<u32>,
    cancelled: bool,
    stalled: bool,
    refusing: bool,
    /// The fault the next feed ends on, after its first portion.
    inject: Option<FeedReason>,
    /// The pack against its floor, and whether the adapter was in at the
    /// last step.
    floor: Floor,
    adapter: bool,
    /// Whether the hopper has food in front of the beam.
    hopper: bool,
    /// What the beam last read this boot, in a feed or at rest.
    food: Option<bool>,
    /// Readings at rest this boot.
    reads: u32,
    trace: FeedTrace,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            journal: None,
            state: OwnerState::Starting,
            fault: OwnerFault::Ok,
            result: OwnerResult::default(),
            intent_reserved: false,
            result_durable: false,
            driven_ms: 0,
            stepped_ms: None,
            cancelled: false,
            stalled: false,
            refusing: false,
            inject: None,
            floor: Floor::default(),
            adapter: true,
            hopper: true,
            food: None,
            reads: 0,
            trace: FeedTrace::default(),
        }
    }
}

impl Inner {
    fn high_water(&self) -> u64 {
        self.journal
            .as_ref()
            .map_or(0, |journal| journal.view.recovery.high_water_id)
    }

    /// On the pack alone and not judged above its floor.
    fn supply_low(&self) -> bool {
        !self.adapter && !self.floor.above()
    }

    /// The queued feed's reservation, written before anything moves.
    fn reserve(&mut self, now_ms: u32) -> Option<u32> {
        let journal = self.journal.as_mut()?;
        let recovery = journal.view.recovery;
        let request = FeedRequest {
            id: self.result.id,
            portions: self.result.requested,
            mode: JOURNAL.mode,
            direction: recovery.direction,
            credit_next_event: recovery.credit_next_event,
            phase_known: recovery.phase_known,
            homing: false,
        };
        if !journal.reserve(&request) {
            // Nothing ran: the adapter answers the feed refused.
            self.state = OwnerState::Fault;
            self.fault = OwnerFault::Storage;
            return None;
        }
        self.intent_reserved = true;
        self.state = OwnerState::Feeding;
        self.driven_ms = 0;
        self.stepped_ms = Some(now_ms);
        Some(0)
    }

    /// One step of the feed being driven: how soon to step again, until it
    /// ends.
    fn drive(&mut self, now_ms: u32) -> Option<u32> {
        if !self.stalled
            && let Some(stepped) = self.stepped_ms
        {
            self.driven_ms = self.driven_ms.saturating_add(now_ms.wrapping_sub(stepped));
        }
        self.stepped_ms = Some(now_ms);
        let portions = self.result.requested;
        let full = u32::from(portions) * PORTION_MS;
        // A fault comes after a portion's drive and ends the feed there:
        // stopped, never resumed. A feed of one has not reached its mark.
        let faulted = self.inject.filter(|_| self.driven_ms >= PORTION_MS);
        if !self.cancelled && faulted.is_none() && self.driven_ms < full {
            let next = if self.inject.is_some() {
                PORTION_MS
            } else {
                full
            };
            return Some(next.saturating_sub(self.driven_ms).clamp(1, STEP_MS));
        }

        let (outcome, reason, completed) = if let Some(reason) = faulted {
            self.inject = None;
            (OwnerOutcome::Failed, reason, u16::from(portions > 1))
        } else if self.cancelled {
            let driven = self.driven_ms.min(full) / PORTION_MS;
            let driven = u16::try_from(driven).unwrap_or(portions);
            (OwnerOutcome::Cancelled, FeedReason::Cancelled, driven)
        } else {
            (OwnerOutcome::Done, FeedReason::Done, portions)
        };
        self.result = OwnerResult {
            completed,
            outcome,
            reason,
            motion_possible: true,
            ..self.result
        };
        self.food = Some(self.hopper);
        self.trace.hopper = Some(self.hopper);
        self.finish();
        None
    }

    /// Write the result into the journal: `Terminal` until it is
    /// acknowledged, or a fault with the result uncertain.
    fn finish(&mut self) {
        let result = self.result;
        let written = self.journal.as_mut().is_some_and(|journal| {
            let next = OwnerRecovery {
                high_water_id: result.id,
                pending_request: false,
                ..journal.view.recovery
            };
            journal.finish(&result, &next)
        });
        self.result_durable = written;
        if written {
            self.state = OwnerState::Terminal;
        } else {
            self.state = OwnerState::Fault;
            self.fault = OwnerFault::Storage;
        }
    }
}

impl SimOwner {
    /// Open on `journal`, resolving the feed a restart `interrupted` as
    /// uncertain: reported like any durable result until it is
    /// acknowledged.
    fn open(&self, mut journal: FeedJournal<Region>, interrupted: bool) {
        let resolved = if interrupted { journal.resolve() } else { None };
        let mut inner = self.inner.borrow_mut();
        inner.journal = Some(journal);
        inner.state = match resolved {
            Some(result) => {
                inner.result = result;
                inner.result_durable = true;
                OwnerState::Terminal
            }
            None if interrupted => {
                inner.fault = OwnerFault::Storage;
                OwnerState::Fault
            }
            None => OwnerState::Idle,
        };
    }
}

impl FeedOwner for SimOwner {
    fn step(&self) -> Option<u32> {
        let now_ms = self.now_ms.get();
        let mut inner = self.inner.borrow_mut();
        match inner.state {
            OwnerState::Queued => inner.reserve(now_ms),
            OwnerState::Feeding => inner.drive(now_ms),
            _ => None,
        }
    }

    fn status(&self) -> OwnerStatus {
        let inner = self.inner.borrow();
        let resting = !matches!(inner.state, OwnerState::Feeding);
        OwnerStatus {
            state: inner.state,
            fault: inner.fault,
            result: inner.result,
            high_water_id: inner.high_water(),
            phase: RotorPhase::Unknown,
            intent_reserved: inner.intent_reserved,
            result_durable: inner.result_durable,
            stopped: resting,
            optics_off: resting,
            trace: inner.trace,
            food: inner.food,
            food_reads: inner.reads,
            on_adapter: inner.adapter,
            below_floor: inner.floor.below(),
            supply_low: inner.supply_low(),
            ..OwnerStatus::default()
        }
    }

    fn submit(&self, id: u64, portions: u16) -> SubmitResult {
        let mut inner = self.inner.borrow_mut();
        if inner.state == OwnerState::Fault {
            return SubmitResult::RejectStopped;
        }
        if inner.state != OwnerState::Idle || inner.refusing {
            return SubmitResult::RejectBusy;
        }
        if id == 0 || id <= inner.high_water() {
            return SubmitResult::RejectId;
        }
        if portions == 0 || portions > JOURNAL.max_portions {
            return SubmitResult::RejectRequest;
        }

        inner.state = OwnerState::Queued;
        inner.result = OwnerResult {
            id,
            requested: portions,
            ..OwnerResult::default()
        };
        inner.intent_reserved = false;
        inner.result_durable = false;
        inner.cancelled = false;
        inner.trace = FeedTrace::default();
        SubmitResult::Enqueued
    }

    fn cancel(&self, id: u64) -> bool {
        let mut inner = self.inner.borrow_mut();
        let ours = matches!(inner.state, OwnerState::Queued | OwnerState::Feeding)
            && inner.result.id == id;
        inner.cancelled |= ours;
        ours
    }

    fn acknowledge(&self, id: u64) -> bool {
        let mut inner = self.inner.borrow_mut();
        let ours =
            inner.state == OwnerState::Terminal && inner.result.id == id && inner.result_durable;
        if ours {
            inner.state = OwnerState::Idle;
        }
        ours
    }

    fn high_water_id(&self) -> u64 {
        self.inner.borrow().high_water()
    }

    fn faulted(&self) -> bool {
        self.inner.borrow().state == OwnerState::Fault
    }

    fn take_observation(&self) -> Option<Observation> {
        None
    }

    /// A reading here takes no time, so none is ever lit to end.
    fn yield_food_read(&self, _: &Interlock) {}

    fn food_read_step(&self, reads: &mut FoodReads, _: &Interlock, now_ms: u32, held: bool) {
        let mut inner = self.inner.borrow_mut();
        let view = FoodView {
            reading: false,
            reads: inner.reads,
        };
        let held = held || inner.state != OwnerState::Idle || !inner.adapter;
        if reads.step(now_ms, view, held) == FoodStep::Start {
            reads.started();
            inner.food = Some(inner.hopper);
            inner.reads = inner.reads.wrapping_add(1);
        }
    }
}

/// How many feeds the journal on `flash` has ever reserved: its high
/// water, which only a reservation moves. `None` before the journal
/// exists or when it does not open.
pub fn reserved(flash: &Rc<Flash>) -> Option<u64> {
    let mut journal = FeedJournal::default();
    let opened = journal.open(Region::new(flash, storage::JOURNAL), JOURNAL);
    (opened == JournalError::Ok).then_some(journal.view.recovery.high_water_id)
}
