//! The feeding owner, stepped from the loop task.
//!
//! The owner opens once the runtime has armed the flash hooks: its journal
//! writes go through them, and their lease authority is the one that blocks
//! the owner's outputs during a lease. Until then, and if it never opens,
//! the outputs stay as startup left them (motor STOP, emitters OFF) and the
//! runtime's watchdog rule is unchanged. At rest the owner holds the
//! board, samples the inputs, and beats.
//!
//! A trial build confirms only once the owner has proved it can feed
//! ([`Proof`], reported to the runtime each pass): open on its journal, any
//! interrupted feed resolved, and at rest with every check passing for
//! [`PROOF_MS`]. An owner that cannot open reports a failure, which resets a
//! trial build at once so KM0 counts it. After a reset storm with nothing
//! to fall back to (`wellfed_sdk::held`), the owner stays closed until the
//! runtime ends the hold, then opens as on any boot.
//!
//! A feed comes from the device API (`wellfed_feeder::feeder::Feeds`): the
//! loop submits it under the next journal id, answers once the owner has
//! reserved it, and when the owner reports its result, logs what the drive
//! measured, records a `Fed` event through the product's event recorder and
//! acknowledges it, which lets the owner take the next. One feed at a
//! time; a feed whose owner faulted without a durable result is recorded
//! as uncertain. So is a feed a reset interrupted: the owner resolves its
//! reservation when it opens, and the loop records that `Fed` event, by the
//! feeder, before the owner takes a feed.
//!
//! The timing limits come from the first pulse and the optics probe (a
//! 200 ms motor lease, 10 ms supervision timer, stock's 100 ms sensor
//! settling); the feed ones from the measured feeds (marks, portions,
//! current in raw scaled counts) and stock (the jam limit and its three
//! reversals). With no confirmed feedback edge a drive stops after
//! `feedback_timeout_ms`, and with no edge at all after `mark_gap_ms`; a
//! jam backs off at most `max_reverse_attempts` times, then ends the feed.
//! None of them restarts a feed.
//!
//! Feeds and Wi-Fi joins take turns (`wellfed_hal::interlock`): an
//! association holds every task for up to 0.72 s, longer than the beat
//! allows while outputs may move, and no task priority avoids it. So a feed
//! is refused `Busy` while a join runs or waits, and a join requested during
//! a feed waits until the owner is at rest with its outputs stopped.
//!
//! Readiness ([`wellfed_feeder::feeder::feeding::readiness`]) starts from
//! the owner's own reason not to feed (`Feeding::blocked`): not open yet
//! or the build not yet confirmed, the journal failed, a fault (held after
//! a reset storm included), an interrupted feed being resolved, a feed in
//! flight, or a join's turn. `submit` keeps its own checks behind it.
//!
//! Each feed is traced (`wellfed_feeder_runtime::traces`): after every
//! owner step the loop hands what the step saw to the trace, so the trace
//! never runs inside the owner's stop and motor decisions, and when the
//! feed's result is recorded the trace is sealed with it and the owner's
//! drive figures, for the settings writer to store once the outputs are
//! stopped.
//!
//! The rotor's phase is the owner's ([`RotorPhase`]): a feed from an
//! unknown phase homes on its first mark, uncredited, and the loop counts
//! those homing feeds and the outlet beam's breaks for the portions
//! statistics (`PLOWN1 marks`), so delivered food can be estimated apart
//! from the requested count ([calibration]).
//!
//! [calibration]: ../../../../../docs/firmware/calibration.md
//!
//! While a feed is driven the loop steps the owner before each pass and,
//! when the pass ran a handshake operation (a client opening a session),
//! again at once after it, so the owner's observations stay
//! inside its 100 ms gap; `PLOWN1 steps` reports, for each feed, the
//! longest gap between two steps and the handshake operations it absorbed.
//!
//! Storage writes take the same turn: a flash lease while the outputs may
//! move fails its stopped-outputs check and resets the board. A store write
//! that comes during a feed waits until the outputs are released (at most
//! `FEED_BOUND_MS`), and a feed that finds a write running keeps asking on
//! each pass, at most `WRITE_WAIT_MS`, with the loop serving everything else
//! meanwhile ([`Claiming`]). The journal writes under the loop's own claim, at
//! the owner's rest boundaries.
//!
//! The food beam is also read at rest (`wellfed_core_feeder::feed::food_read`), so
//! the low-food light and `Status.food` follow a refill or a restart with no
//! feed. The loop drives that from here, before each owner step, and every
//! reason to hold a reading gives way to the feed or write in question:
//! `submit` ends a reading before it claims, a writer waiting ends it within a
//! step, and a trial holds readings until it has confirmed ([`food_read_held`]).
//!
//! The loop's half of all this is [`adapter`], built for the host too: the
//! simulated feeder runs the same adapter over an owner of its own, and host
//! tests run it over a scripted one. Only its runtime (`target`: the
//! owner's static, the runtime's flags, the board's interlock, traces and
//! the log) is the board's.

use wellfed_core_feeder::feed::controller::FeedLimits;
use wellfed_core_feeder::feed::progress::BowlMode;
use wellfed_core_feeder::feed::sensor_inputs::SensorLimits;
use wellfed_core_feeder::feed::storage::{OwnerResult, RotorPhase};
use wellfed_core_feeder::feed_journal::{JournalConfig, JournalError};
use wellfed_feeder::feed::Durability;
use wellfed_hal_feeder::events::{EventsConfig, EventsFault, PinWatch};
use wellfed_hal_feeder::owner::{
    FeedTrace, OwnerConfig, OwnerFault, OwnerState, OwnerStatus, SubmitResult,
};
use wellfed_proto::api::{Blocked, Refusal, Source};
use wellfed_sdk::hal::interlock::{self, ClaimStep, ClaimWait, Held, Interlock};
use wellfed_sdk::hal::sdk;
use wellfed_sdk::hal::supervision::config::{SYSTIMER_HZ, WATCHDOG};
use wellfed_sdk::hal::supervision::supervise::{self, LIVENESS_MS};

/// The vendor app's cap on one feed, from the stock inventory. The journal
/// pins it: a journal written with another reads as corrupt.
pub const MAX_PORTIONS: u16 = 48;

/// The PLAF103's single bowl.
pub const BOWL: BowlMode = BowlMode::Single;

pub const CONFIG: OwnerConfig = OwnerConfig {
    feed: FeedLimits {
        // A portion is one compartment: PA18 reads LOW for 1.2-1.4 s at each
        // mark and a portion is credited on its rise, about 9.1 s of drive
        // apart empty and 9.9 s with food (bench). Like stock, an edge within
        // 5 s of the drive's start or the last portion does not count.
        feedback_inhibit_ms: 5000,
        // Half again a portion with food; stock waits about 30 s.
        feedback_timeout_ms: 15_000,
        // A jam backs off 1 s (about 13 degrees of rotor, a ninth of a
        // compartment), stops 0.5 s, and drives on, three times at most as
        // stock does; each costs 2.5 s of deadline and feedback window.
        // Stock's reversal length is not recovered: 1 s is provisional
        // until a real jam shows what frees it.
        direction_pause_ms: 500,
        reverse_ms: 1000,
        // A feed's average must stay within 12 s a portion, and every gap
        // within the feedback timeout; the cap covers the largest request.
        max_duration_ms: 12_000 * MAX_PORTIONS as u32,
        portion_ms: 12_000,
        max_observation_gap_ms: 100,
        // A turning rotor shows an edge at least every 8.5 s (a mark's fall
        // 7.8-8.4 s after the last rise, empty or with food, on the
        // adapter), and reads LOW for 1.2-1.4 s at a mark. On batteries the
        // motor may turn slower; a gap past 11 s there would end feeds
        // `FeedbackTimeout`, so the rotor's speed on D cells needs a check.
        mark_gap_ms: 11_000,
        mark_low_ms: 4000,
        // Scaled counts (stock's unit; milliamps need a reference): a drive
        // reads a mean of 59-64 and peaks of 164-168 empty, a mean of 82-87
        // with food, and 15-16 stopped. Stock reverses over 300. A start-up
        // surge read 333 on one poll at 17 ms and up to 447 at 10 ms
        // polling; the 150 ms after each change of drive (50 ms of it the
        // driver's deadtime) count toward no jam, and only a reading past
        // twice the highest surge stops at once.
        //
        // Risk: both come from empty and light-food drives on the adapter.
        // A full hopper's peaks, the peaks on batteries, and a real stall's
        // current are unmeasured. Peaks near 300 would reverse sound feeds
        // (and end them after three); a stall reading over 1000 would stop
        // every jam at once with no reversal, which is safe but never frees
        // one. Both want measuring before the limits are trusted.
        current_limit_ma: 300,
        current_max_ma: 1000,
        surge_ms: 150,
        max_portions: MAX_PORTIONS,
        max_reverse_attempts: 3,
    },
    // Stock: 100 ticks of settling after enable, about 1 s of bad samples
    // before a bad state. Its food flag (`surplusGrain`) clears after 20
    // readings without food; its poll rate is not recovered, so the food
    // filter assumes the nominal 50 ms the outlet's implies: the same 1 s.
    sensors: SensorLimits {
        optical_settle_ms: 100,
        food_low_delay_ms: 1000,
        outlet_block_delay_ms: 1000,
        power_settle_ms: 100,
        max_observation_gap_ms: 100,
        max_age_ms: 100,
        battery: wellfed_hal_feeder::power::FLOOR,
    },
    events: EventsConfig {
        ticks_per_second: SYSTIMER_HZ,
        timer_period_ms: 10,
        max_feedback_age_ms: 100,
        feedback_capacity: 8,
    },
    max_progress_gap_ms: WATCHDOG.max_progress_gap_ms,
    // At rest the loop may be held off by the Wi-Fi driver's handshake for
    // over a second (observed); the runtime's liveness limit for its tasks.
    idle_gap_ms: LIVENESS_MS,
    bowl_mode: BOWL,
    // The loop's own pass; a whole flash lease fits between two beats.
    sample_period_ms: 20,
    // A third of the pack's floor hold, at the sensors' age limit and well
    // inside the motor lease.
    rest_capture_ms: 100,
    prepare_timeout_ms: 2000,
    optics_max_on_ms: 12_000 * MAX_PORTIONS as u32 + 2000,
    motor_deadtime_ms: 50,
    motor_lease_ms: 200,
    adc_poll_limit: 10_000,
    wake_lock_id: sdk::PMU_USER_BASE,
    flash: core::ptr::null_mut(),
};

/// The shortest drive between two marks' rises: 9.1 s with the hopper
/// empty on the adapter, 9.8-9.9 s with food (bench, October 25,
/// docs/firmware/motor-pulse.md, "Food and a full turn").
pub const MARK_PERIOD_MIN_MS: u32 = 9100;

// A homing mark is uncredited, so its window must never hold a whole
// compartment of travel. The most forward drive that window can hold is
// the inhibit plus every reversal's return over the ground it gave up
// (the back-off and pause themselves move the rotor no further forward).
const _: () = assert!(
    CONFIG.feed.feedback_inhibit_ms
        + CONFIG.feed.max_reverse_attempts as u32 * CONFIG.feed.reverse_ms
        < MARK_PERIOD_MIN_MS
);

/// The longest one feed can keep the outputs from rest, and so the longest
/// a Wi-Fi join or a storage write waits for them: the owner keeps the emitters on for at most
/// `optics_max_on_ms` from the request, and its configuration check holds
/// the prepare timeout and the longest drive inside that, then it stops (or
/// faults, which stops too). The margin covers the stop and the pass that
/// sees it. About 9 min 40 s for the largest request.
pub const FEED_BOUND_MS: u32 = CONFIG.optics_max_on_ms + 2000;

/// A feed waiting to claim the outputs, from its `submit` until the claim is
/// taken or refused: its number is promised, and nothing has moved. While one
/// waits the loop reads it as a feed in flight (`busy`, `Blocked::Feeding`) so
/// nothing else starts, and the device API refuses changes that would queue a
/// write behind it, as if the outputs were claimed. It holds nothing in the
/// interlock, so the writes it waits for can finish.
#[derive(Debug)]
pub struct Claiming {
    /// The number `submit` answered, the journal's next.
    pub feed: u32,
    pub portions: u8,
    pub by: Source,
    wait: ClaimWait,
}

/// What one pass's try of a [`Claiming`] came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimTurn {
    /// The outputs are claimed: hand the feed to the owner in this pass.
    Claimed,
    /// Try again on the next pass.
    Waiting,
    /// Over, nothing claimed and nothing moved.
    Refused(Refusal),
}

impl Claiming {
    pub const fn new(feed: u32, portions: u8, by: Source, now_ms: u32) -> Self {
        Self {
            feed,
            portions,
            by,
            wait: ClaimWait::new(now_ms),
        }
    }

    /// One try at `now_ms`. `hold` is a reason not to feed that arose since
    /// the feed was asked for (setup mode, an install, a restart, the battery
    /// hold): a pass runs between two tries, so what readiness said at `submit`
    /// no longer speaks for this moment. A storage write running or waiting,
    /// or a change `queued` in a mailbox, goes first, for at most
    /// `WRITE_WAIT_MS`. A join running or waiting, or a write still running or
    /// a change still queued past that wait, refuses the feed `Busy`, final
    /// for that write id (clients never retry a feed on their own); `held`
    /// hears why, for the log.
    pub fn step(
        &mut self,
        interlock: &Interlock,
        hold: Option<Blocked>,
        queued: bool,
        now_ms: u32,
        held: impl FnOnce(Held),
    ) -> ClaimTurn {
        if let Some(blocked) = hold {
            return ClaimTurn::Refused(blocked.refusal());
        }
        match interlock.claim_feed_after_writes(&mut self.wait, FEED_BOUND_MS, queued, now_ms) {
            ClaimStep::Claimed => ClaimTurn::Claimed,
            ClaimStep::Wait => ClaimTurn::Waiting,
            ClaimStep::Refused(why) => {
                held(why);
                ClaimTurn::Refused(Refusal::Busy)
            }
        }
    }
}

/// Whether a client's change waits in a mailbox for the settings writer,
/// which takes its write turn only on its next poll: a claim lets it go
/// first, so the client hears its answer before the feed rather than after.
pub fn writes_queued() -> bool {
    use wellfed_device_runtime::{keys, network};
    use wellfed_feeder_runtime::{settings, traces};
    settings::SETTINGS.waiting() || network::saving() || keys::queued() || traces::queued()
}

/// Whether the firmware's own reasons hold food readings at rest
/// (`Owner::food_read_step` gathers the rest).
///
/// A trial is the one that needs a hold of its own. Its confirmation requires
/// outputs verified stopped for 30 s and then [`Proof`]'s [`PROOF_MS`] of
/// emitters read back off, so lit emitters would restart both; and its
/// deadline resets the boot however close that came. So a trial reads nothing
/// until it has confirmed, and its low-food light stays dark until then.
///
/// Bluetooth `setup` holds them too. Setup's prebuilt Bluetooth and Wi-Fi
/// libraries have never run with lit emitters, and one that wrote flash would
/// fail the lease's stopped check and reset the board. The first reading comes
/// `FOOD_READ_FIRST_MS` into the boot after setup's restart. Setup's start ends a reading already
/// running, by way of the platform's `ble_restart`.
pub const fn food_read_held(
    confirming: bool,
    setup: bool,
    in_flight: InFlight,
    interrupted: bool,
    queued: bool,
) -> bool {
    confirming || setup || matches!(in_flight, InFlight::Feed) || interrupted || queued
}

/// The owner's figures for a feed's drive, as its trace's summary keeps
/// them: the same that `fed` logs.
pub fn drive_figures(trace: &FeedTrace) -> wellfed_feeder::trace::DriveFigures {
    wellfed_feeder::trace::DriveFigures {
        drive_ms: trace.drive_ms,
        samples: u32::from(trace.current_samples),
        current_peak: trace.current_max,
        current_mean: trace.current_mean(),
        rising: trace.rising,
        falling: trace.falling,
        first_edge_ms: trace.first_edge_ms,
        reversals: u16::from(trace.reversals),
    }
}

/// Whether the loop has a feed between its submission and its recorded
/// result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InFlight {
    Nothing,
    Feed,
}

/// Whether the loop may hand the outputs' turn back to Wi-Fi joins: no feed
/// in flight, the owner in a state that never drives (a queued request is
/// about to), and motor STOP and the emitters off read back. A failed stop
/// keeps the turn, and a waiting join then goes ahead only past the bound.
pub fn may_release(in_flight: InFlight, status: &OwnerStatus) -> bool {
    let resting = matches!(
        status.state,
        OwnerState::Starting
            | OwnerState::Idle
            | OwnerState::Terminal
            | OwnerState::Fault
            | OwnerState::Stopped
    );
    in_flight == InFlight::Nothing && resting && status.stopped && status.optics_off
}

/// How long the owner rests Idle with every check passing before it has
/// proved it can feed: a hundred of its 20 ms polls, each reading the sensor
/// inputs and verifying motor STOP and the emitters off, and about twenty
/// battery conversions among them. The motor current's channel, on the same
/// converter, is first converted when a feed prepares.
pub const PROOF_MS: u32 = 2_000;

/// When the loop next steps the owner: once the wait the owner asked for
/// has passed, timed from the start of its last step.
///
/// A step can hold a sensor capture (up to two ADC channels, about 11 ms for
/// both on hardware), and the loop passes every millisecond while the device
/// API has work. Stepping the owner on each of those passes polled more often
/// than its period asks, and made each pass, so each event a subscriber reads,
/// cost the step. Its configuration bounds the period below every lease and
/// age limit (`OwnerConfig`), so stepping at the period is the supervision it
/// was designed for; an idle loop steps it at this cadence too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cadence {
    due: Option<u32>,
}

impl Cadence {
    /// The wait left at `now` before the owner is due; `None` once it is.
    pub fn wait(&self, now_ms: u32) -> Option<u32> {
        let due = self.due?;
        (!wellfed_core::deadline_passed(now_ms, due)).then(|| wellfed_core::until(now_ms, due))
    }

    /// The owner was stepped from `now` and asked to wait `delay`; with
    /// none (stopped or refused) the next pass steps it again.
    pub fn stepped(&mut self, now_ms: u32, delay: Option<u32>) {
        self.due = delay.map(|delay| now_ms.wrapping_add(delay));
    }
}

/// What the owner has shown this boot, for the runtime: a trial build
/// confirms only on [`supervise::Feeding::Proven`], and one whose owner cannot open
/// ([`supervise::Feeding::Failed`]) is reset at once and counted.
#[derive(Clone, Copy, Debug, Default)]
pub struct Proof {
    /// When the owner was first seen at rest with every check passing,
    /// since it last was not.
    resting_since: Option<u32>,
}

/// The owner as the loop sees it, for [`Proof::step`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opened<'a> {
    /// Not opened yet: the hooks are unarmed, or a lease holds the flash.
    Not,
    /// The journal could not be opened or created: no owner this boot.
    JournalFailed,
    /// Opened; `resolving` while a feed a reset interrupted is still being
    /// recorded as uncertain.
    Owner {
        status: &'a OwnerStatus,
        resolving: bool,
    },
}

impl Proof {
    pub fn step(&mut self, opened: Opened<'_>, now_ms: u32) -> supervise::Feeding {
        let (status, resolving) = match opened {
            Opened::Not => return self.pending(),
            Opened::JournalFailed => return self.failed(),
            Opened::Owner { status, resolving } => (status, resolving),
        };
        // A fault latches until a restart: setup, the journal's recovery,
        // storage, an output or input check, or the owner's own beat.
        if status.state == OwnerState::Fault || status.fault != OwnerFault::Ok {
            return self.failed();
        }

        let resting = status.state == OwnerState::Idle
            && status.supervisor_fault == EventsFault::Ok
            && status.stopped
            && status.optics_off
            && !resolving;
        if !resting {
            return self.pending();
        }
        let since = *self.resting_since.get_or_insert(now_ms);
        if now_ms.wrapping_sub(since) >= PROOF_MS {
            supervise::Feeding::Proven
        } else {
            supervise::Feeding::Pending
        }
    }

    fn pending(&mut self) -> supervise::Feeding {
        self.resting_since = None;
        supervise::Feeding::Pending
    }

    fn failed(&mut self) -> supervise::Feeding {
        self.resting_since = None;
        supervise::Feeding::Failed
    }
}

/// How long the owner holds lost sensors, at rest, before the loop asks
/// for the one restart a power cycle allows.
pub const RECOVERY_HOLD_MS: u32 = 10 * 60_000;

/// Which boot of its power cycle this is: the KM4 boot count the flash
/// arming keeps in a backup register, which power loss clears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerCycle {
    FirstBoot,
    /// Some reset came before this boot: a restart, an update, a watchdog.
    Restarted,
}

/// When a feeder whose owner holds lost sensors restarts by itself.
///
/// Lost sensors are held, not reset on (the owner's own rule), so a feeder
/// left alone would never feed again. A converter that stopped once may
/// convert again after a reset, so after `RECOVERY_HOLD_MS` of holding them
/// at rest with nothing in flight, the loop asks for one restart, only from
/// the first boot of a power cycle: if the sensors are lost again after it,
/// the owner holds them for good, until the owner restarts it or power is
/// cut. No backup register bit is free to count restarts, so a power cycle
/// that already had any reset gives up its recovery restart.
#[derive(Clone, Copy, Debug, Default)]
pub struct Recovery {
    /// Since when the owner has held lost sensors at rest.
    since: Option<u32>,
    asked: bool,
}

impl Recovery {
    /// One pass at `now_ms`; `held` names the boot while the owner holds
    /// lost sensors with nothing in flight and no install running. True
    /// once, when the restart is due.
    pub fn step(&mut self, now_ms: u32, held: Option<PowerCycle>) -> bool {
        let Some(cycle) = held else {
            self.since = None;
            return false;
        };
        let since = *self.since.get_or_insert(now_ms);
        let due = cycle == PowerCycle::FirstBoot
            && !self.asked
            && now_ms.wrapping_sub(since) >= RECOVERY_HOLD_MS;
        self.asked |= due;
        due
    }

    /// How long the owner has held them, for the log.
    pub fn held_ms(&self, now_ms: u32) -> u32 {
        self.since.map_or(0, |since| now_ms.wrapping_sub(since))
    }
}

pub const JOURNAL: JournalConfig = JournalConfig {
    binding: wellfed_sdk::storage::JOURNAL_BINDING,
    mode: BOWL,
    max_portions: MAX_PORTIONS,
};

/// What the loop logs, for the bench.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Record {
    /// The journal could not be opened or created; the owner never opens
    /// this boot.
    Journal { error: JournalError, created: bool },
    /// The owner refused to open (a lease in progress, or the authority
    /// not yet ready); retried each pass, logged once.
    Refused,
    /// Opened, on a journal created this boot or found.
    Opened { created: bool, high_water: u64 },
    /// The owner's published state changed.
    Status {
        state: OwnerState,
        fault: OwnerFault,
        supervisor: EventsFault,
        stopped: bool,
        optics_off: bool,
    },
    /// A feed was handed to the owner, or refused before it was.
    Submitted {
        feed: u64,
        portions: u8,
        result: SubmitResult,
    },
    /// A feed ended: its result, and how long its drive ran and the edges
    /// it counted.
    Fed {
        result: OwnerResult,
        durability: Durability,
        trace: FeedTrace,
    },
    /// The rest of what that feed's drive measured: its first edge, its
    /// current, its reversals and the food beam. A line of its own, so
    /// neither this nor `fed` outgrows a record.
    Current { feed: u64, trace: FeedTrace },
    /// Which pads changed during that feed, to find the feedback input.
    Pins { feed: u64, pins: PinWatch },
    /// When each feedback edge came, from the drive's start.
    Edges { feed: u64, trace: FeedTrace },
    /// A feed was refused before the owner saw it: a Wi-Fi join runs or
    /// waits, or a storage write ran past the claim's wait.
    Held { portions: u8, held: Held },
    /// Joins waited for the outputs, or went ahead past a feed's bound: the
    /// interlock's join counters.
    Joins(interlock::Status),
    /// Storage writes waited for the outputs, or a feed for a write: the
    /// interlock's write counters, which the bench reads. A line apart from
    /// the joins', so neither outgrows a record.
    Writes(interlock::Status),
    /// What the feed's marks and the outlet beam showed: how far into the
    /// drive a homing mark came, the beam's breaks, and the phase after.
    Marks {
        feed: u64,
        homed_ms: Option<u32>,
        outlet_breaks: u32,
        phase: RotorPhase,
    },
    /// While the feed was in flight: the longest gap between two owner
    /// steps, and the handshake operations the loop ran.
    Steps {
        feed: u64,
        max_gap_ms: u32,
        handshakes: u32,
    },
    /// The owner held lost sensors this long: the loop restarts the feeder
    /// once to recover them.
    Recover { held_ms: u32 },
}

/// The most pads a `pins` record lists, so the line fits a record.
const PINS_LISTED: usize = 10;

/// The outlet beam's breaks in a feed's pin watch: each break is two
/// changes of PA16, a half-seen one at either end counts whole. Sampled on
/// the 10 ms supervision tick, so a shorter passage can fall between two
/// samples: a lower bound.
pub fn outlet_breaks(pins: &PinWatch) -> u32 {
    let outlet = usize::try_from(wellfed_sdk::hal::board::pins::OUTLET).unwrap_or(usize::MAX);
    let toggles = pins.toggles[0].get(outlet).copied().unwrap_or(0);
    u32::from(toggles).div_ceil(2)
}

/// The body of a `PLOWN1` line.
impl core::fmt::Display for Record {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Traces and pin watches are borrowed, not bound by value: a copy
        // would sit in this frame, which the stack audit charges at every
        // nesting level of every line the loop formats.
        match *self {
            Record::Journal { error, created } => write!(
                f,
                "open journal={} created={}",
                error.code(),
                u8::from(created)
            ),
            Record::Refused => f.write_str("open refused=1"),
            Record::Opened {
                created,
                high_water,
            } => write!(
                f,
                "open created={} high_water={high_water}",
                u8::from(created)
            ),
            Record::Status {
                state,
                fault,
                supervisor,
                stopped,
                optics_off,
            } => write!(
                f,
                "state={} fault={} supervisor={} stopped={} optics_off={}",
                state.code(),
                fault.code(),
                supervisor.code(),
                u8::from(stopped),
                u8::from(optics_off)
            ),
            Record::Submitted {
                feed,
                portions,
                result,
            } => write!(
                f,
                "feed id={feed} portions={portions} submit={}",
                result.code()
            ),
            Record::Fed {
                ref result,
                durability,
                ref trace,
            } => write!(
                f,
                "fed id={} requested={} completed={} outcome={} reason={} durable={} drive_ms={} rising={} falling={}",
                result.id,
                result.requested,
                result.completed,
                result.outcome.code(),
                result.reason.code(),
                u8::from(durability == Durability::Durable),
                trace.drive_ms,
                trace.rising,
                trace.falling,
            ),
            Record::Current { feed, ref trace } => {
                write!(f, "current id={feed} first_edge_ms=")?;
                match trace.first_edge_ms {
                    Some(ms) => write!(f, "{ms}")?,
                    None => f.write_str("none")?,
                }
                write!(f, " current_max={} current_mean=", trace.current_max)?;
                match trace.current_mean() {
                    Some(mean) => write!(f, "{mean}")?,
                    None => f.write_str("none")?,
                }
                write!(
                    f,
                    " samples={} reversals={} hopper={}",
                    trace.current_samples,
                    trace.reversals,
                    match trace.hopper {
                        None => "unread",
                        Some(true) => "present",
                        Some(false) => "low",
                    }
                )
            }
            Record::Edges { feed, ref trace } => {
                write!(f, "edges id={feed} at=")?;
                let mut logged = trace.edge_ms.iter().flatten().peekable();
                if logged.peek().is_none() {
                    return f.write_str("none");
                }
                for (index, &(ms, high)) in logged.enumerate() {
                    if index != 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{ms}{}", if high { 'H' } else { 'L' })?;
                }
                Ok(())
            }
            Record::Marks {
                feed,
                homed_ms,
                outlet_breaks,
                phase,
            } => {
                write!(f, "marks id={feed} homed_ms=")?;
                match homed_ms {
                    Some(ms) => write!(f, "{ms}")?,
                    None => f.write_str("none")?,
                }
                let phase = match phase {
                    RotorPhase::AtMark => "at_mark",
                    RotorPhase::Unknown => "unknown",
                };
                write!(f, " outlet_breaks={outlet_breaks} phase={phase}")
            }
            Record::Steps {
                feed,
                max_gap_ms,
                handshakes,
            } => write!(
                f,
                "steps id={feed} max_gap_ms={max_gap_ms} handshakes={handshakes}"
            ),
            Record::Recover { held_ms } => write!(f, "recover restart=1 held_ms={held_ms}"),
            Record::Held { portions, held } => {
                write!(f, "feed portions={portions} held={}", held.code())
            }
            Record::Joins(ref status) => write!(
                f,
                "joins refused={} deferred={} forced={} waited_max_ms={} long_joins={}",
                status.refused,
                status.deferred,
                status.forced,
                status.waited_max_ms,
                status.long_joins,
            ),
            Record::Writes(ref status) => write!(
                f,
                "interlock writes_deferred={} writes_forced={} write_waited_max_ms={} \
                 feeds_delayed={}",
                status.writes_deferred,
                status.writes_forced,
                status.write_waited_max_ms,
                status.feeds_delayed
            ),
            Record::Pins { feed, ref pins } => {
                let [start_a, start_b] = pins.start.unwrap_or_default();
                let [changed_a, changed_b] = pins.changed;
                write!(
                    f,
                    "pins id={feed} start={start_a:08x},{start_b:08x} changed={changed_a:08x},{changed_b:08x} toggles="
                )?;
                let toggles = pins
                    .toggles
                    .iter()
                    .zip(['A', 'B'])
                    .flat_map(|(port, name)| {
                        (0..32).filter_map(move |pin| {
                            let count = port[pin];
                            (count != 0).then_some((name, pin, count))
                        })
                    });
                let mut listed = 0;
                for (name, pin, count) in toggles {
                    if listed == PINS_LISTED {
                        return f.write_str(",+");
                    }
                    if listed != 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{name}{pin}:{count}")?;
                    listed += 1;
                }
                if listed == 0 {
                    f.write_str("none")?;
                }
                Ok(())
            }
        }
    }
}

pub mod adapter;
#[cfg(target_os = "none")]
mod target;

#[cfg(target_os = "none")]
pub use target::{Feeding, yield_food_read};
