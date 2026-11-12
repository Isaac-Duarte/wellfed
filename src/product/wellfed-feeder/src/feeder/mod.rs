//! [`Feeder`]: the feeder's loop, one [`Feeder::pass`] at a time, over a
//! [`Platform`]. The PLAF103 firmware runs it in its loop task and the
//! simulator in its serve loop, so both serve the same panel, pairing, BLE
//! setup and device API.
//!
//! A pass, in order:
//! 1. the identity, if the settings writer published a new one
//!    ([`identity`]);
//! 2. the panel's owner: gestures, a client's queued lock change, the idle
//!    relock, pairing's timers and BLE setup mode while it runs ([`owner`],
//!    [`setup`]); then the reboot setup mode asks for, and the lock it now
//!    holds for the device API ([`lock`]);
//! 3. the supply judged, its change recorded, battery mode's radio asked
//!    of the platform and the brownout hold kept ([`supply`]); a Dispense
//!    press the panel allowed, queued for the feeding owner; the feeding
//!    plans ([`schedule`]), the meal reminder ([`reminder`]) and the meal
//!    call ([`call`]); and
//!    readiness noted ([`feeding`]); then the panel's lights from what the pass now
//!    knows ([`lights`]), and a press's beep ([`beep`]);
//! 4. the alert rules' turn, read only, and an install's stop and what the
//!    update store held at boot recorded ([`alert_stage`], [`alerts`]);
//! 5. the device API: its sockets, the server's step and the live samples
//!    ([`serve`]), with the time settings and the NTP exchange ([`time`]);
//! 6. the speaker, while a client's tune or a plan's meal call plays (setup
//!    mode steps it itself). A feed plays none.
//!
//! The feeding owner is the platform's: the firmware steps it before the
//! pass and hands it in as [`Feeds`], and records its `Fed` events through
//! [`Feeder::record_outcome`]. Every durable event goes through the
//! recorder ([`events`]), where the alert rules watch them.

pub mod alert_stage;
pub mod alerts;
pub mod beep;
pub mod call;
pub mod events;
pub mod feeding;
pub mod identity;
pub mod lights;
pub mod lock;
pub mod owner;
pub mod platform;
pub mod reminder;
pub mod schedule;
pub mod serve;
pub mod setup;
pub mod skip;
pub mod supply;
pub mod time;

use core::fmt;

use wellfed_core::ota::install::Phase as InstallPhase;
use wellfed_proto::api::{
    Blocked, EventBody, Phase, Readiness, Refusal, Source, Supply, TraceSummary,
};
use wellfed_session::noise::Keypair;

pub use platform::Platform;

use alert_stage::AlertStage;
use beep::Beeper;
use call::Call;
use identity::Identity;
use lights::Lights;
use lock::LockBox;
use owner::{Owner, ResetRefusal};
use platform::DriverRecord;
use reminder::Reminder;
use schedule::Schedule;
pub use serve::DeviceView;
use serve::{Inputs, Serve};
use wellfed_device_api::Server;
use wellfed_device_api::mailbox::Lock;

/// The feeding owner, as the device API reaches it. Generic rather than
/// `dyn`, so the calls stay direct for the firmware's stack audit.
pub trait Feeds {
    /// A feed is being prepared or driven: only IK session handshakes go
    /// on, and only until one is refused or fails.
    fn feeding(&self) -> bool;
    /// A feed is queued or runs: no install starts, and no restart.
    fn busy(&self) -> bool;
    /// The owner's own reason not to start a feed now, if any
    /// ([`feeding::readiness`] adds the loop's).
    fn blocked(&self) -> Option<Blocked>;
    /// Hand one feed to the owner; the number its journal holds it under.
    fn submit(&mut self, portions: u8, by: Source) -> Result<u32, Refusal>;
    /// Who asked for the feed `submit` took but whose claim on the outputs
    /// still waits for storage writes; `None` with no such feed. The loop
    /// tries it once a pass ([`Self::claim_step`]) and serves everything else
    /// meanwhile; `busy` and `blocked` read it as a feed in flight, and the
    /// device API refuses changes that would queue a write behind it.
    fn claiming(&self) -> Option<Source> {
        None
    }
    /// One try of the waiting feed's claim, `hold` being a reason not to feed
    /// that arose since it was asked for. Its refusal, once it has one: a feed
    /// claimed here has gone to the owner, and [`Self::reserved`] follows it.
    fn claim_step(&mut self, _hold: Option<Blocked>) -> Option<RefusedClaim> {
        None
    }
    /// Whether the feed `feed` is reserved yet: `None` while it waits.
    fn reserved(&self, feed: u32) -> Option<Result<(), Refusal>>;
    /// Stop the feed `feed` if it is still running.
    fn cancel(&self, feed: u32) -> bool;
    /// The food beam's latest reading (true with food), once the owner has
    /// taken one: during a feed, or in a short window at rest.
    fn food(&self) -> Option<bool> {
        None
    }
    /// Where the rotor rests and what the feeds' marks and outlet beam
    /// counted this boot, once the owner has opened.
    fn marks(&self) -> Option<Marks> {
        None
    }
    /// Whether the owner judges the battery pack below the floor a feed
    /// needs; `None` before it has judged, or with no owner.
    fn below_floor(&self) -> Option<bool> {
        None
    }
    /// The number of the newest feed the journal holds, once it is open:
    /// the next feed is reserved as the one after it.
    fn high_water(&self) -> Option<u32> {
        None
    }
}

/// A feed `submit` accepted and numbered, whose claim on the outputs ended
/// refused: nothing moved, and the next feed takes the same number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefusedClaim {
    pub feed: u32,
    pub by: Source,
    pub refusal: Refusal,
}

/// The rotor's phase and this boot's mark counts, from the feeding owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Marks {
    pub phase: Phase,
    /// Feeds that began with a homing mark, uncredited.
    pub homed: u32,
    /// Outlet-beam breaks seen during feeds.
    pub outlet_breaks: u32,
}

/// A feeder with no feeding owner: every feed is refused before it
/// reaches the motor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoFeeds;

impl Feeds for NoFeeds {
    fn feeding(&self) -> bool {
        false
    }

    fn busy(&self) -> bool {
        false
    }

    fn blocked(&self) -> Option<Blocked> {
        Some(Blocked::Unserved)
    }

    fn submit(&mut self, _: u8, _: Source) -> Result<u32, Refusal> {
        Err(Refusal::Unavailable)
    }

    fn reserved(&self, _: u32) -> Option<Result<(), Refusal>> {
        Some(Err(Refusal::Unavailable))
    }

    fn cancel(&self, _: u32) -> bool {
        false
    }
}

/// What one pass did, for the record lines. No record carries a key, a
/// secret, a credential or the setup code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Record<D> {
    /// `PLID1`.
    Identity(identity::Record),
    /// `PLPANEL1`, `PLSET1`, or a setup driver's own tag.
    Owner(owner::Record<D>),
    /// `PLAPI1`.
    Serve(serve::Record),
    /// `PLFEED1`.
    Feeding(feeding::Record),
    /// `PLIND1`.
    Lights(lights::Record),
    /// `PLSCH1`.
    Schedule(schedule::Record),
    /// `PLBAT2`.
    Supply(supply::Record),
    /// The speaker's own record while a requested tune plays, under its tag.
    Driver(D),
}

impl<D: DriverRecord> Record<D> {
    /// The line's tag.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Identity(_) => "PLID1",
            Self::Owner(owner::Record::Setup(setup::Record::Driver(record))) => record.tag(),
            Self::Owner(owner::Record::Setup(_)) => "PLSET1",
            Self::Owner(_) => "PLPANEL1",
            Self::Serve(_) => "PLAPI1",
            Self::Feeding(_) => "PLFEED1",
            Self::Lights(_) => "PLIND1",
            Self::Schedule(_) => "PLSCH1",
            Self::Supply(_) => "PLBAT2",
            Self::Driver(record) => record.tag(),
        }
    }
}

/// The line's body.
impl<D: DriverRecord> fmt::Display for Record<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Identity(record) => identity::describe(record, f),
            Self::Owner(record) => owner::describe(record, f),
            Self::Serve(record) => serve::describe(record, f),
            Self::Feeding(record) => feeding::describe(record, f),
            Self::Lights(record) => lights::describe(record, f),
            Self::Schedule(record) => schedule::describe(record, f),
            Self::Supply(record) => supply::describe(record, f),
            Self::Driver(record) => fmt::Display::fmt(&record, f),
        }
    }
}

pub struct Feeder<P: Platform> {
    platform: P,
    identity: Identity,
    owner: Owner,
    lock: LockBox,
    serve: Serve<P::Socket>,
    readiness: feeding::Watch,
    supply: supply::Watch,
    lights: Lights,
    beep: Beeper,
    schedule: Schedule,
    reminder: Reminder,
    alert_stage: AlertStage,
    /// When a due restart began waiting for a plan's claim in flight.
    claim_wait: Option<u32>,
    call: Call,
}

/// The longest a due restart waits for a plan's claim in flight: a writer
/// turn, a feed's outputs to stop, and room to spare.
pub const CLAIM_WAIT_MS: u32 = 30_000;

impl<P: Platform> Feeder<P> {
    /// Const, so the firmware builds it in its static: the server's table
    /// and setup mode's state would not fit the loop's stack.
    pub const fn new(platform: P) -> Self {
        Self {
            platform,
            identity: Identity::new(),
            owner: Owner::new(),
            lock: LockBox::new(),
            serve: Serve::new(),
            readiness: feeding::Watch::new(),
            supply: supply::Watch::new(),
            lights: Lights::new(),
            beep: Beeper::new(),
            schedule: Schedule::new(),
            reminder: Reminder::new(),
            alert_stage: AlertStage::new(),
            claim_wait: None,
            call: Call::new(),
        }
    }

    pub fn platform(&self) -> &P {
        &self.platform
    }

    pub fn platform_mut(&mut self) -> &mut P {
        &mut self.platform
    }

    /// The identity key pair, once the writer has published one.
    pub fn identity(&self) -> Option<&Keypair> {
        self.identity.keypair()
    }

    /// The panel's lock, once the first pass has run.
    pub fn lock(&self) -> Option<Lock> {
        self.owner.lock()
    }

    /// Whether BLE setup mode runs.
    pub fn setup_on(&self) -> bool {
        self.owner.setup_on()
    }

    /// The device API's server, for a host test to look into.
    pub fn server(&mut self) -> &mut Server {
        self.serve.server()
    }

    /// Whether the last pass ran a handshake operation, its costly part: a
    /// loop with a feed in flight steps the feeding owner again at once.
    pub fn handshook(&mut self) -> bool {
        self.serve.server().handshook()
    }

    /// Whether a feed can start now, with the feeding owner `feeds`.
    pub fn readiness<F: Feeds>(&self, feeds: &F) -> Readiness {
        feeding::held(self.ready(feeds), self.schedule.holds())
    }

    /// As [`Self::readiness`], without the schedule's own hold: what the
    /// schedule's feed is asked under. A setup the boot started for want
    /// of a network lets the panel feed between sessions; the device API's
    /// clients, who need a network, still see setup mode block.
    fn ready<F: Feeds>(&self, feeds: &F) -> Readiness {
        feeding::readiness(
            feeds,
            self.platform.restarting(),
            self.owner.setup_holds_feeds(),
            self.platform.installing(),
            self.supply.holds(),
        )
    }

    /// One pass at `now_ms`, with the feeding owner `feeds`; `record`
    /// receives what happened, in order. The delay until the next pass.
    /// The caller reads `now_ms` before the pass, so it comes before the
    /// identity step (an X25519 derivation when a new identity arrives).
    pub fn pass<F: Feeds>(
        &mut self,
        now_ms: u32,
        feeds: &mut F,
        mut record: impl FnMut(Record<P::Record>),
    ) -> u32 {
        let owner_delay = self.owner_stage(now_ms, feeds, &mut record);
        let (local_delay, reading) = self.local_stage(now_ms, feeds, &mut record);
        self.alert_turn(now_ms, feeds, reading);

        let reset = self.owner.reset_pending();
        let inputs = Inputs {
            now_ms,
            setup: self.owner.setup_on(),
            locked: self.owner.lock().map(Lock::locked),
            recovery: self.owner.take_recovery(),
            signer: self.owner.take_signer(),
            pairing: self.owner.pairing(),
            identity: self.identity.keypair(),
            supply: reading,
            battery_hold: self.supply.holds(),
            meal: self.schedule.holds(),
            reset,
            claim: self.schedule.claim_in_flight(),
            call_lead: self.call.lead_at(),
            feeds,
        };
        let serve_delay = self
            .serve
            .step(&mut self.platform, &mut self.lock, inputs, |r| {
                record(Record::Serve(r))
            });

        // After the owner and the device API, so neither waits on playback
        // within the pass. A feed in flight still pays for this step in its
        // gap between owner steps.
        let speaker_delay = self.speaker_stage(&mut record);

        owner_delay
            .min(local_delay)
            .min(serve_delay)
            .min(speaker_delay)
    }

    /// The speaker's step, unless setup mode steps it itself; the delay it
    /// asks for. Out of line so its record's frame is not under the device
    /// API's paths.
    #[inline(never)]
    fn speaker_stage(&mut self, record: &mut impl FnMut(Record<P::Record>)) -> u32 {
        if self.owner.setup_on() || self.platform.speaker_idle() {
            return u32::MAX;
        }

        let step = self.platform.speaker_step();
        if let Some(r) = step.record {
            record(Record::Driver(r));
        }
        step.delay_ms
    }

    /// The pass's first two steps: the identity, the panel's owner and the
    /// restart it may have made due; the delay the owner asks for. Out of
    /// line, apart from [`Self::local_stage`], because setup mode's session
    /// handshake under the owner's step is one of the loop's deepest paths
    /// and needs none of that stage's frame.
    #[inline(never)]
    fn owner_stage<F: Feeds>(
        &mut self,
        now_ms: u32,
        feeds: &mut F,
        record: &mut impl FnMut(Record<P::Record>),
    ) -> u32 {
        if let Some(r) = self.identity.step(&mut self.platform) {
            record(Record::Identity(r));
        }

        let remote = self.lock.take();
        let refusal = self.reset_refusal(feeds);
        let delay = self.owner.step(
            &mut self.platform,
            now_ms,
            refusal,
            remote,
            self.identity.keypair(),
            |r| record(Record::Owner(r)),
        );
        // Setup mode's and the factory reset's restart, and the one the
        // writer asks for when a reset could not finish, wait for a feed in
        // flight, which they would otherwise leave uncertain; none starts a
        // feed itself, and once asked for no feed starts (`restarting`). They
        // wait for a plan's claim in flight too, which a restart before its
        // feed is queued would leave missed; that wait is bounded
        // (`CLAIM_WAIT_MS`), so a claim the writer never finishes cannot
        // hold a restart for ever.
        let due = self.owner.reboot_due(now_ms) || self.platform.restart_wanted();
        let claim = due && self.claim_holds_restart(now_ms);
        if due && !feeds.busy() && !claim && !self.platform.restarting() {
            // The writer's restart save takes this boot's counts as they are now.
            let counts = self.serve.stats().counts(now_ms);
            self.platform.hand_counts(counts);
            events::record(&mut self.platform, now_ms, EventBody::Restarting);
            self.platform.reset();
        }
        delay
    }

    /// The alert rules' turn, after every stage that decides or runs a meal
    /// and before the device API, so a status in this pass carries what they
    /// make of it. Out of line so its inputs are not in [`Self::pass`]'s
    /// frame under the device API's paths.
    #[inline(never)]
    fn alert_turn<F: Feeds>(&mut self, now_ms: u32, feeds: &F, supply: Supply) {
        let claim = self.schedule.claim_in_flight();
        let inputs = alert_stage::Inputs {
            ready: self.readiness(feeds),
            meal_held: self.schedule.meal_held(),
            food: feeds.food(),
            feeding: feeds.busy(),
            supply,
            claim,
            busy: feeds.busy() || feeds.feeding() || claim,
            schedule_at_rest: self.schedule.at_rest(),
            booted: self.serve.booted(),
        };
        self.alert_stage.step(&mut self.platform, now_ms, &inputs);
    }

    /// Why a factory reset cannot run now, if so. Out of line so the install
    /// progress it reads is not in [`Self::owner_stage`]'s frame.
    #[inline(never)]
    fn reset_refusal<F: Feeds>(&mut self, feeds: &F) -> Option<ResetRefusal> {
        if feeds.busy() {
            Some(ResetRefusal::Feeding)
        } else if self.platform.installing()
            || self
                .platform
                .update_progress()
                .is_some_and(|progress| progress.phase != InstallPhase::Idle)
        {
            Some(ResetRefusal::Updating)
        } else {
            None
        }
    }

    /// The pass's third step, up to the device API: the delay it asks for and
    /// the supply it judged. Out of line because its frame would otherwise
    /// stay live under every request and handshake, the loop's deepest paths.
    #[inline(never)]
    fn local_stage<F: Feeds>(
        &mut self,
        now_ms: u32,
        feeds: &mut F,
        record: &mut impl FnMut(Record<P::Record>),
    ) -> (u32, Supply) {
        // The boot's facts (and its `Boot` event) first, so the supply's
        // brownout hold is armed before the first readiness.
        let boot = self.serve.boot(&mut self.platform, now_ms);
        let reading = supply::supply(self.platform.supply(now_ms), feeds.below_floor());
        let reading = self.supply.cells(now_ms, reading);
        let reset = crate::stats::reset_cause(boot.reset_flags);
        let changed = self
            .supply
            .note(now_ms, reset, reading, |r| record(Record::Supply(r)));
        if let Some(changed) = changed {
            events::record(&mut self.platform, now_ms, EventBody::Supply(changed));
        }
        let mut went_off = false;
        let radio = self.supply.radio(now_ms, self.owner.setup_on(), |r| {
            went_off |= r == supply::Record::Radio(supply::Radio::Off);
            record(Record::Supply(r));
        });
        self.platform.radio(radio);
        // No chunk can reach an install once the radio is off: stop it now
        // rather than hold the write turn until it times out. Bank 1 stays
        // invalid, as at any stop before activation.
        if went_off
            && self
                .platform
                .update_progress()
                .is_some_and(|p| p.phase == InstallPhase::Receiving)
        {
            let stopped = self.platform.update_cancel().is_ok();
            record(Record::Supply(supply::Record::InstallStopped { stopped }));
        }

        self.claim_turn(feeds, now_ms, record);
        if self.owner.take_dispense() {
            let ready = self.readiness(feeds);
            let portions = feeding::DISPENSE_PORTIONS;
            let result = feeding::queue(feeds, ready, portions, Source::Panel);
            record(Record::Feeding(feeding::Record::Dispense { result }));
            if result.is_err() {
                let reason = feeding::refused_reason(feeds, ready);
                self.readiness.refused(&mut self.platform, now_ms, reason);
                self.lights.refused(now_ms, |r| record(Record::Lights(r)));
            }
        }
        let ready = self.ready(feeds);
        // A waiting feed is the next one, so nothing is decided around it: a
        // plan's refused claim comes back to the schedule as it left, and
        // no write of the plans joins the writes the feed waits for.
        if feeds.claiming().is_none() {
            self.schedule
                .step(&mut self.platform, feeds, now_ms, ready, |r| {
                    record(Record::Schedule(r))
                });
        }
        // After the schedule, so a pass that runs a meal decides it first;
        // the reminder only reads the plans.
        self.reminder
            .step(&mut self.platform, now_ms, |r| record(Record::Schedule(r)));
        // Read only too: the call starts ahead of the meal it announces,
        // and none plays for a meal a fault or setup mode will not let run.
        let quiet = self.owner.setup_on()
            || matches!(
                ready,
                Readiness::Blocked(Blocked::Fault | Blocked::Journal | Blocked::Setup)
            );
        self.call.step(&mut self.platform, now_ms, quiet, |r| {
            record(Record::Schedule(r))
        });
        let ready = self.readiness(feeds);
        self.readiness.note(&mut self.platform, now_ms, ready, |r| {
            record(Record::Feeding(r))
        });
        let flight = feeding::Flight::of(feeds);
        if let Some(level) = self.readiness.food(feeds.food(), flight) {
            events::record(
                &mut self.platform,
                now_ms,
                EventBody::Food {
                    level: level.into(),
                },
            );
        }

        let local_ms = self
            .platform
            .clock_state(now_ms)
            .reading
            .and_then(|r| r.local_ms());
        let states = self.states(feeds, ready, local_ms);
        let lights_delay = self.lights.step(&mut self.platform, now_ms, states, |r| {
            record(Record::Lights(r))
        });

        // Every press beeps when sound may play; not before the settings load.
        let pressed = self.owner.take_pressed();
        let allowed = self
            .platform
            .settings()
            .is_some_and(|view| view.sound_at(local_ms));
        let at_rest = !feeds.busy();
        let beep_delay = self
            .beep
            .step(&mut self.platform, now_ms, pressed, allowed, at_rest);

        // Requests read what the owner holds after this step.
        if let Some(lock) = self.owner.lock() {
            self.lock.publish(lock.locked());
        }
        self.serve.stats().presses(self.owner.presses());

        (lights_delay.min(beep_delay), reading)
    }

    /// One pass's try of a feed waiting to claim the outputs, and the end of
    /// one refused. What held for it when it was asked for is asked again,
    /// since passes run between two tries: a gesture may have started setup
    /// mode, and an install or a restart may have begun. Out of line so its
    /// frame is not under every session's deepest path.
    #[inline(never)]
    fn claim_turn<F: Feeds>(
        &mut self,
        feeds: &mut F,
        now_ms: u32,
        record: &mut impl FnMut(Record<P::Record>),
    ) {
        let Some(by) = feeds.claiming() else {
            return;
        };
        // As readiness tells each asker: a client always sees setup mode block.
        let setup = match by {
            Source::Client(_) => self.owner.setup_on(),
            _ => self.owner.setup_holds_feeds(),
        };
        let hold = feeding::claim_hold(
            self.platform.restarting(),
            setup,
            self.platform.installing(),
            self.supply.holds(),
        );
        let Some(refused) = feeds.claim_step(hold) else {
            return;
        };

        // Before anything can take its number again.
        self.serve
            .server()
            .feed_refused(refused.feed, refused.refusal);
        match refused.by {
            Source::Panel => {
                let result = Err(refused.refusal);
                record(Record::Feeding(feeding::Record::Dispense { result }));
                let reason = feeding::refused_reason(feeds, self.readiness(feeds));
                self.readiness.refused(&mut self.platform, now_ms, reason);
                self.lights.refused(now_ms, |r| record(Record::Lights(r)));
            }
            Source::Schedule(plan) => self.schedule.claim_refused(
                &mut self.platform,
                plan,
                refused.refusal,
                now_ms,
                |r| record(Record::Schedule(r)),
            ),
            Source::Client(_) | Source::Feeder => {}
        }
    }

    /// The feeder's states as the lights show them, after this pass's
    /// owner step and readiness, at the local wall time `local_ms`.
    fn states<F: Feeds>(
        &mut self,
        feeds: &F,
        ready: Readiness,
        local_ms: Option<i64>,
    ) -> lights::States {
        let blocked = feeds.blocked();
        lights::States {
            fault: matches!(blocked, Some(Blocked::Fault | Blocked::Journal)),
            updating: self.platform.installing(),
            setup: self.owner.setup_on(),
            pairing: self.owner.pairing().awaiting().is_some(),
            battery: ready == Readiness::Blocked(Blocked::Battery)
                || blocked == Some(Blocked::Battery),
            low_food: feeds.food() == Some(false),
            locked: self.owner.lock().is_some_and(Lock::locked),
            online: self.platform.ipv4().is_some(),
            // True until the settings load.
            lights: self
                .platform
                .settings()
                .is_none_or(|view| view.lights_at(local_ms)),
        }
    }

    /// Whether a plan's claim in flight still holds a restart that is due:
    /// until it is settled, or for at most [`CLAIM_WAIT_MS`].
    fn claim_holds_restart(&mut self, now_ms: u32) -> bool {
        if !self.schedule.claim_in_flight() {
            self.claim_wait = None;
            return false;
        }
        let since = *self.claim_wait.get_or_insert(now_ms);
        !wellfed_core::deadline_passed(now_ms, since.wrapping_add(CLAIM_WAIT_MS))
    }

    /// Record a feed's `Fed` event, which may take the queue's reserved
    /// slot. A restart's uncertain feed that a plan had claimed is the
    /// plan's, and happened at the occurrence's due time: the boot resolves
    /// it before any source has set the clock after a power cut, and the
    /// claim on NOR is the only time it still has. Its feed began within
    /// the grace after it, so the history places it where it ran rather
    /// than when the boot found it.
    pub fn record_outcome(&mut self, now_ms: u32, mut body: EventBody) {
        let mut ran_s = None;
        if let EventBody::Fed { feed, by, .. } = &mut body
            && let Some(claim) = Schedule::claim_behind(&mut self.platform, *feed, *by)
        {
            *by = Source::Schedule(claim.plan);
            ran_s = Some(claim.due_s);
        }
        self.serve.stats().fed(&body);
        events::record_outcome(&mut self.platform, now_ms, ran_s, body);
        // The writer saves the totals soon after a feed (`FEED_SAVE_MS`), and
        // behind every queued record, so the event goes in first.
        let counts = self.serve.stats().counts(now_ms);
        self.platform.hand_counts(counts);
    }

    /// A feed's trace was sealed: its figures join the statistics.
    pub fn traced(&mut self, summary: &TraceSummary) {
        self.serve.stats().traced(summary);
    }

    /// The last loop pass (the feeding owner's step and [`Self::pass`])
    /// took `us`, for the loop metric.
    pub fn pass_took(&mut self, us: u32) {
        self.serve.stats().pass_took(us);
    }

    /// The device API's view of the feeder at `now_ms`, with the feeding
    /// owner `feeds`, as the server sees it in a pass.
    pub fn device<'a, F: Feeds>(
        &'a mut self,
        now_ms: u32,
        feeds: &'a mut F,
    ) -> DeviceView<'a, P, F> {
        let boot = self.serve.boot(&mut self.platform, now_ms);
        let supply = supply::supply(self.platform.supply(now_ms), feeds.below_floor());
        let supply = self.supply.cells(now_ms, supply);
        let ntp_last = self.serve.ntp_last();
        let (stats, tasks, page) = self.serve.stats_parts();
        DeviceView {
            platform: &mut self.platform,
            lock: &mut self.lock,
            feeds,
            boot,
            locked: self.owner.lock().map(Lock::locked),
            setup: self.owner.setup_on(),
            now_ms,
            ntp_last,
            stats,
            tasks,
            page,
            battery_hold: self.supply.holds(),
            supply,
            meal: self.schedule.holds(),
            reset: self.owner.reset_pending(),
            claim: self.schedule.claim_in_flight(),
            call_lead: self.call.lead_at(),
        }
    }
}
