//! The feeder's stores in the settings writer ([`FeederStores`], the
//! [`Product`] of `wellfed_device_runtime::writer`), and the SDK runtime's
//! settings stage over them (`wellfed_store_step`).
//!
//! Beside the device's stores the writer keeps the feeder's own settings
//! (sectors 48 and 49, `wellfed_feeder::store::settings`, [`settings`]),
//! saved with the device's from one `settings` request, and the portion
//! calibration
//! (sectors 34 and 35, `wellfed_feeder::store::calibration`,
//! [`calibration`]), opened first each boot since a factory reset keeps
//! it, and the feeding plans (sectors 11 and 12,
//! `wellfed_feeder::store::plans`, [`plans`]): a client's table, and the
//! loop's claims and passes. And the meal call (sectors 40-47,
//! `wellfed_feeder::store::call`, [`call`]): an upload a chunk at a time,
//! written only while no call plays from it. The handover from stock
//! clears the plans after the Wi-Fi.
//!
//! It keeps the per-feed traces (sectors 29-31,
//! `wellfed_device::store::traces`): a feed's trace, sealed in RAM once the
//! feed is over, is appended to the ring on the writer's next turn
//! ([`traces`]).
//!
//! It keeps the lifetime statistics too (sectors 32 and 33,
//! `wellfed_feeder::store::stats`): it loads the totals, counts this
//! boot's reset and publishes them for the loop, and saves them with this
//! boot's counts and erases as the boot starts (so a short boot's reset
//! counts), in an intentional restart's grace (before the watchdog's
//! feeding stops), after a finished feed at most every [`FEED_SAVE_MS`],
//! and every [`SAVE_MS`] of uptime; only
//! when nothing else is queued, never per event, and the early ones within
//! the store's tokens. It publishes the event ring's fill for the
//! statistics ([`stats`]). A factory reset rewrites them keeping only the
//! flash erase counts, the hardware's wear.

use core::fmt;

use wellfed_device::store::events::EventsFill;
use wellfed_device::store::settings::{SettingsStatus, Stored};
use wellfed_device::store::traces::TraceRing;
use wellfed_device_api::mailbox::{CallChange, CallClip, FeederSettings, PlansChange, Stamp};
use wellfed_device_runtime::events;
use wellfed_device_runtime::writer::{
    self as device, Core, Describe, EVENTS_POLL_MS, ErrorStage, POLL_MS, Product,
    READ_LINGER_POLLS, Wear, WriteEvent, blank, next,
};
use wellfed_feeder::stats::{FEED_SAVE_MS, SAVE_MS, TOTAL_SECTORS, Totals, reset_cause};
use wellfed_feeder::store::calibration::CalibrationStore;
use wellfed_feeder::store::call::CallStore;
use wellfed_feeder::store::plans::PlansStore;
use wellfed_feeder::store::settings::FeederSettingsStore;
use wellfed_feeder::store::stats::{SaveKind, StatsStatus, StatsStore};
use wellfed_hal::flash_lock::arming::{self, Hooks};
use wellfed_hal::settings::Mailbox;
use wellfed_hal::{audio, sdk, supervision};
use wellfed_sdk::storage::{FlashOps, Region, SETTINGS_BINDING, Slots, TRACES_BASE, TRACES_UNITS};

use crate::calibration::{self, CALIBRATION_BASE};
use crate::call::{self, CALL_BASE, CALL_UNITS, Queued};
use crate::plans::{self, PLANS_BASE};
use crate::settings::{self, FEEDER_SETTINGS_BASE};
use crate::stats::{self, STATS_BASE};
use crate::traces;

/// The settings writer over the feeder's stores.
pub type Writer<F> = device::Writer<F, FeederStores<F>>;
/// A record of the settings writer over the feeder's stores.
pub type Record = device::Record<FeederRecord>;
/// A step of the settings writer over the feeder's stores.
pub type Step = device::Step<FeederRecord>;

/// Polls at [`EVENTS_POLL_MS`] after the last chunk of a clip still
/// `Receiving`: about 1 s. Bounded, because a client that walks away leaves
/// the clip `Receiving` until the next upload or delete, for as long as the
/// feeder then runs, and battery mode pays for every wake.
pub const RECEIVE_LINGER_POLLS: u32 = READ_LINGER_POLLS;

/// The feeder's own stores' records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeederRecord {
    /// The feeder's settings (sectors 48 and 49), as the writer's settings
    /// lines name them after the device's.
    Settings {
        status: SettingsStatus,
        generation: u64,
        settings: FeederSettings,
    },
    /// The lifetime statistics (sectors 32 and 33).
    StatsOpen {
        status: StatsStatus,
        generation: u64,
        /// The early saves since the last settled one, and whether a run of
        /// them ever spent the budget, so a boot may have been missed.
        early: u8,
        missed: bool,
    },
    StatsSaved {
        generation: u64,
        early: u8,
    },
    StatsError {
        stage: ErrorStage,
    },
    /// The portion calibration (sectors 34 and 35).
    CalibrationSaved {
        generation: u64,
        portion_mg: Option<u32>,
    },
    CalibrationError {
        stage: ErrorStage,
    },
    /// The feeding plans (sectors 11 and 12).
    PlansSaved {
        sequence: u32,
        revision: u32,
        event: WriteEvent,
    },
    /// A change of the loop's taken against plans that have moved on:
    /// nothing written, and the loop decides again.
    PlansDropped,
    PlansError {
        stage: ErrorStage,
    },
    /// The meal call (sectors 40-47) after an upload's start or end, or a
    /// delete; a chunk mid-upload logs nothing.
    Call {
        clip: CallClip,
    },
    CallError {
        stage: ErrorStage,
    },
    /// A feed's trace went to the ring (`seq`), or did not (`None`).
    Trace {
        feed: u32,
        seq: Option<u32>,
    },
}

impl Describe for FeederRecord {
    fn describe(self, f: &mut impl fmt::Write) -> fmt::Result {
        match self {
            Self::Settings {
                status,
                generation,
                settings,
            } => {
                write!(
                    f,
                    "feeder_status={} feeder_generation={generation} meal_reminder={} \
                     desiccant_days={} desiccant_reset=",
                    status.code(),
                    settings.meal_reminder,
                    settings.desiccant_days,
                )?;
                match settings.desiccant_reset {
                    Some(day) => write!(f, "{day}"),
                    None => f.write_str("none"),
                }
            }
            Self::StatsOpen {
                status,
                generation,
                early,
                missed,
            } => {
                write!(
                    f,
                    "stats open status={} generation={generation} early={early} missed={}",
                    status.name(),
                    u8::from(missed)
                )?;
                // A record this build could not read: the totals and the
                // wear counts it held are gone.
                if status == StatsStatus::Lost {
                    f.write_str(" wear_lost=1")?;
                }
                Ok(())
            }
            Self::StatsSaved { generation, early } => {
                write!(f, "stats saved generation={generation} early={early}")
            }
            Self::StatsError { stage } => write!(f, "stats error stage={stage}"),
            Self::CalibrationSaved {
                generation,
                portion_mg,
            } => {
                write!(f, "calibration saved generation={generation} portion_mg=")?;
                match portion_mg {
                    Some(mg) => write!(f, "{mg}"),
                    None => f.write_str("none"),
                }
            }
            Self::CalibrationError { stage } => write!(f, "calibration error stage={stage}"),
            Self::PlansSaved {
                sequence,
                revision,
                event,
            } => write!(
                f,
                "plans saved sequence={sequence} revision={revision} event={event}"
            ),
            Self::PlansDropped => f.write_str("plans dropped"),
            Self::PlansError { stage } => write!(f, "plans error stage={stage}"),
            Self::Call { clip } => match clip {
                CallClip::None => f.write_str("call none"),
                CallClip::Receiving { bytes, .. } => write!(f, "call receiving bytes={bytes}"),
                CallClip::Stored { bytes, crc } => {
                    write!(f, "call stored bytes={bytes} crc={crc:08x}")
                }
            },
            Self::CallError { stage } => write!(f, "call error stage={stage}"),
            Self::Trace {
                feed,
                seq: Some(seq),
            } => write!(f, "trace saved feed={feed} seq={seq}"),
            Self::Trace { feed, seq: None } => write!(f, "trace error feed={feed}"),
        }
    }
}

/// Where the meal call's queued work stands after the writer's look.
enum CallSave {
    /// Written (or refused), with the record it makes, if any.
    Done {
        record: Option<FeederRecord>,
        delay_ms: u32,
    },
    /// A feed holds the outputs.
    Turn,
    /// A call plays from the clip.
    Held,
}

/// Why the writer saves the lifetime totals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SaveTrigger {
    /// The boot's own, as it starts, so its reset counts.
    Boot,
    /// An intentional restart was asked for.
    Restart,
    /// The timed save, every `SAVE_MS` of uptime.
    Due,
    /// A feed ended since the last save, at most every `FEED_SAVE_MS`.
    Feed,
}

impl SaveTrigger {
    /// Only the timed save gives an early token back, and a restart's
    /// spends one only early in a boot (`SaveKind::restart`).
    const fn kind(self, uptime_ms: u32) -> SaveKind {
        match self {
            Self::Boot => SaveKind::Early,
            Self::Restart => SaveKind::restart(uptime_ms),
            Self::Due => SaveKind::Settled,
            Self::Feed => SaveKind::Feed,
        }
    }
}

/// The feeder's stores, which the settings writer keeps beside the
/// device's.
pub struct FeederStores<F> {
    /// The feeder's own settings, beside the device's.
    settings: FeederSettingsStore<Slots<F, FEEDER_SETTINGS_BASE>>,
    /// Opened first each boot; its status shows in the device API's
    /// calibration view (writable, generation) rather than a record.
    calibration: CalibrationStore<Slots<F, CALIBRATION_BASE>>,
    /// As for the calibration: its status shows in the plans view.
    plans: PlansStore<Slots<F, PLANS_BASE>>,
    /// Its status shows in the call's view.
    call: CallStore<Region<F, CALL_BASE, CALL_UNITS>>,
    traces: TraceRing<Region<F, TRACES_BASE, TRACES_UNITS>>,
    stats: StatsStore<Slots<F, STATS_BASE>>,
    stats_opened: bool,
    /// The stored totals with this boot's reset counted: what each save
    /// adds this boot's figures to.
    stats_base: Totals,
    /// When the next settled save of the totals is due, in uptime ms;
    /// `None` once a save failed (no retry this boot).
    stats_due: Option<u32>,
    /// This boot's feeds in the last save: any others want a save.
    stats_feeds: u32,
    /// When a feed last asked for a save, in uptime ms.
    stats_feed_at: Option<u32>,
    /// The boot's own early save, and the intentional restart's, had their
    /// one attempt.
    stats_boot_saved: bool,
    stats_restart_saved: bool,
}

impl<F> Default for FeederStores<F> {
    fn default() -> Self {
        Self {
            settings: FeederSettingsStore::default(),
            calibration: CalibrationStore::default(),
            plans: PlansStore::default(),
            call: CallStore::default(),
            traces: TraceRing::default(),
            stats: StatsStore::default(),
            stats_opened: false,
            stats_base: Totals::ZERO,
            stats_due: Some(SAVE_MS),
            stats_feeds: 0,
            stats_feed_at: None,
            stats_boot_saved: false,
            stats_restart_saved: false,
        }
    }
}

/// The factory reset's step for the lifetime statistics: rewrite them
/// keeping only the erase counts ([`Totals::wear_only`]), as one dual-slot
/// save, so a cut leaves the old totals or the wear-only ones and the boot
/// after does it again. A store with nothing in it, one this build cannot
/// read (its next save writes over it), or one a newer major wrote, is left
/// as it is.
///
/// The statistics pair is the region's most written, so it may be the one
/// to wear out. A save that fails must not stop the reset for good (every
/// boot after would serve nothing): the pair is erased instead, losing the
/// erase counts the reset meant to keep. A cut part-way leaves a pair the
/// next boot saves or erases again.
fn keep_wear<F: FlashOps>() -> Wear {
    let mut stats = StatsStore::default();
    stats.open(Slots::<F, STATS_BASE>::new());
    if stats.status != StatsStatus::Loaded || !stats.writable() {
        return Wear::Kept;
    }
    let wear = stats.totals.wear_only();
    if stats.totals == wear || stats.save(&wear, SaveKind::Restart) {
        return Wear::Kept;
    }

    let sectors = STATS_BASE..STATS_BASE + wellfed_hal::board::storage::STATS.count;
    if sectors
        .into_iter()
        .all(|sector| F::erase(sector) && blank::<F>(sector))
    {
        Wear::Lost
    } else {
        Wear::Failed
    }
}

impl<F: FlashOps> FeederStores<F> {
    /// Save a queued calibration: the record, or `None` while a feed holds
    /// the outputs (the change stays queued).
    fn save_calibration<H: Hooks>(
        &mut self,
        core: &mut Core<F>,
        change: &wellfed_device_api::mailbox::Calibration,
    ) -> Option<FeederRecord> {
        let saved = |store: &CalibrationStore<_>| FeederRecord::CalibrationSaved {
            generation: store.generation,
            portion_mg: store.calibration.portion_mg,
        };
        if core.unarmed::<H>() {
            return Some(FeederRecord::CalibrationError {
                stage: ErrorStage::Unarmed,
            });
        }
        if *change == self.calibration.calibration && self.calibration.generation != 0 {
            return Some(saved(&self.calibration));
        }

        let _turn = core.take_turn()?;
        arming::set_attempt();
        let ok = self.calibration.save(change);
        arming::clear_attempt();
        Some(if ok {
            saved(&self.calibration)
        } else {
            FeederRecord::CalibrationError {
                stage: ErrorStage::Save,
            }
        })
    }

    /// Apply a queued plans change and save it: the record, or `None`
    /// while a feed holds the outputs (the change stays queued).
    fn save_plans<H: Hooks>(
        &mut self,
        core: &mut Core<F>,
        change: &PlansChange,
        asked: Option<Stamp>,
    ) -> Option<FeederRecord> {
        if core.unarmed::<H>() {
            return Some(FeederRecord::PlansError {
                stage: ErrorStage::Unarmed,
            });
        }
        // Checked before the turn, so a dropped change waits for no feed.
        if self.plans.plans.changed(change).is_none() {
            return Some(FeederRecord::PlansDropped);
        }

        let _turn = core.take_turn()?;
        arming::set_attempt();
        let ok = self.plans.apply(change) == Some(true);
        arming::clear_attempt();
        // A client's change records its event in the turn that saved it
        // ([`Core::append_written`]); the schedule's own carry no stamp.
        let event = match asked {
            Some(asked) if ok => match self.plans.event(change, asked.by) {
                Some(body) => core.append_written::<Self>(asked.at, &body),
                None => WriteEvent::None,
            },
            _ => WriteEvent::None,
        };
        Some(if ok {
            FeederRecord::PlansSaved {
                sequence: self.plans.sequence,
                revision: self.plans.plans.revision,
                event,
            }
        } else {
            FeederRecord::PlansError {
                stage: ErrorStage::Save,
            }
        })
    }

    /// Apply the call's queued work, or say why it waits (it stays
    /// queued).
    fn save_call<H: Hooks>(&mut self, core: &mut Core<F>, queued: Queued, data: &[u8]) -> CallSave {
        // An upload's next chunk follows at once. A start or a delete erases,
        // and a client alternating them must wear the sector no faster than
        // the idle poll let it, so it keeps that delay whole.
        let delay_ms = match queued {
            Queued::Chunk { .. } => EVENTS_POLL_MS,
            Queued::Change(_) => POLL_MS,
        };

        if core.unarmed::<H>() {
            return CallSave::Done {
                record: Some(FeederRecord::CallError {
                    stage: ErrorStage::Unarmed,
                }),
                delay_ms,
            };
        }
        // The sectors first: a call that plays reads them.
        if !audio::call_write_begin() {
            return CallSave::Held;
        }
        let Some(_turn) = core.take_turn() else {
            audio::call_write_end();
            return CallSave::Turn;
        };

        audio::publish_call(None);
        arming::set_attempt();
        let ok = match queued {
            Queued::Change(CallChange::Start { bytes, crc }) => self.call.start(bytes, crc),
            Queued::Change(CallChange::Delete) => self.call.delete(),
            Queued::Chunk { offset, .. } => self.call.chunk(offset, data),
        };
        arming::clear_attempt();
        audio::publish_call(self.call.stored());
        audio::call_write_end();

        let clip = self.call.clip;
        let record = if !ok {
            Some(FeederRecord::CallError {
                stage: ErrorStage::Save,
            })
        } else if matches!(
            (queued, clip),
            (Queued::Chunk { .. }, CallClip::Receiving { .. })
        ) {
            None
        } else {
            Some(FeederRecord::Call { clip })
        };
        CallSave::Done { record, delay_ms }
    }

    /// Append the sealed trace, if one waits: under the write turn, which a
    /// feed's claim holds until its outputs are stopped. One that cannot be
    /// written (unarmed, read-only) is let go at once, so the next feed's
    /// claim never waits for it.
    fn trace_step(&mut self, core: &mut Core<F>, writable: bool) -> Option<Step> {
        if !traces::queued() {
            return None;
        }
        let turn = if writable {
            let Some(turn) = core.take_turn() else {
                return Some(core.waiting());
            };
            Some(turn)
        } else {
            None
        };

        let ring = &mut self.traces;
        let mut seq = None;
        let written = traces::write_sealed(|summary, body| {
            if turn.is_none() {
                return false;
            }
            arming::set_attempt();
            seq = ring.append(summary, body).ok();
            arming::clear_attempt();
            seq.is_some()
        });
        drop(turn);
        traces::publish_fill(self.traces.kept(), self.traces.capacity());
        written.map(|(feed, _)| {
            next(
                POLL_MS,
                Some(Record::Product(FeederRecord::Trace { feed, seq })),
            )
        })
    }

    /// Open the stats store, count this boot's reset and publish the
    /// totals: none from a store this build cannot write.
    fn open_stats(&mut self) -> FeederRecord {
        self.stats.open(Slots::new());
        let mut base = self.stats.totals;
        base.count_boot(reset_cause(sdk::boot_reason()));
        self.stats_base = base;
        stats::publish_totals(self.stats.writable().then_some(base));
        FeederRecord::StatsOpen {
            status: self.stats.status,
            generation: self.stats.generation,
            early: self.stats.budget.early,
            missed: self.stats.budget.missed,
        }
    }

    /// The save of the totals wanted now, if any: the boot's own as it
    /// starts, an intentional restart's (in its grace, before the feeding
    /// stops), the timed one, or a finished feed's. A trigger the store refuses (no early token
    /// left, or a store it cannot write) is dropped for the boot.
    fn stats_wanted(&mut self) -> Option<SaveTrigger> {
        let now = sdk::uptime_ms();
        let trigger = if !self.stats_boot_saved {
            SaveTrigger::Boot
        } else if supervision::restart_grace() && !self.stats_restart_saved {
            SaveTrigger::Restart
        } else if self
            .stats_due
            .is_some_and(|due| wellfed_core::deadline_passed(now, due))
        {
            SaveTrigger::Due
        } else if self.feed_unsaved(now) {
            SaveTrigger::Feed
        } else {
            return None;
        };

        if self.stats.may_save(trigger.kind(now)) {
            return Some(trigger);
        }
        self.stats_done(trigger);
        None
    }

    /// Whether a feed ended since the last save, and none asked for one
    /// within `FEED_SAVE_MS`. A boot's first feed asks at once: the boot may
    /// not last long enough for anything later.
    fn feed_unsaved(&self, now: u32) -> bool {
        // A failed save stops the store's saves for the boot (no retry).
        self.stats_due.is_some()
            && self
                .stats_feed_at
                .is_none_or(|at| now.wrapping_sub(at) >= FEED_SAVE_MS)
            && stats::counts().is_some_and(|counts| counts.feeds != self.stats_feeds)
    }

    /// `trigger` has had its one attempt.
    fn stats_done(&mut self, trigger: SaveTrigger) {
        match trigger {
            SaveTrigger::Boot => self.stats_boot_saved = true,
            SaveTrigger::Restart => {
                self.stats_restart_saved = true;
                // Saved or not, the restart need not wait any longer.
                supervision::restart_flushed();
            }
            SaveTrigger::Due => {}
            SaveTrigger::Feed => self.stats_feed_at = Some(sdk::uptime_ms()),
        }
    }

    /// Save the totals: the stored ones with this boot's counts, as the
    /// loop last handed them, and this boot's erases, the save's own
    /// included. Only on a turn with the outputs released: the turn is
    /// asked for as never stopped, so a claim past its bound, which lets
    /// another store's write go ahead, keeps this one waiting. A restart
    /// waits for it only through its grace; once the feeding stops no turn
    /// is granted, so no save starts into the reset.
    fn save_stats(&mut self, core: &mut Core<F>, trigger: SaveTrigger) -> Step {
        let now = sdk::uptime_ms();
        let Some(turn) = core.take_released_turn(now) else {
            return core.waiting();
        };

        let mut erases = [0; TOTAL_SECTORS];
        for (sector, count) in (0u32..).zip(erases.iter_mut()) {
            *count = sdk::storage_erase_count(sector);
        }
        let own = STATS_BASE + self.stats.next_slot();
        if let Some(count) = usize::try_from(own)
            .ok()
            .and_then(|own| erases.get_mut(own))
        {
            *count = count.saturating_add(1);
        }
        let boot = stats::counts().unwrap_or_default();
        let totals = self
            .stats_base
            .plus(&boot, &erases, sdk::bank1_erase_count());

        let kind = trigger.kind(now);
        arming::set_attempt();
        let saved = self.stats.save(&totals, kind);
        arming::clear_attempt();
        drop(turn);

        self.stats_done(trigger);
        self.stats_feeds = boot.feeds;
        let record = if saved {
            if kind == SaveKind::Settled {
                self.stats_due = Some(now.wrapping_add(SAVE_MS));
            }
            FeederRecord::StatsSaved {
                generation: self.stats.generation,
                early: self.stats.budget.early,
            }
        } else {
            self.stats_due = None;
            FeederRecord::StatsError {
                stage: ErrorStage::Save,
            }
        };
        next(POLL_MS, Some(Record::Product(record)))
    }
}

impl<F: FlashOps> Product<F> for FeederStores<F> {
    type Record = FeederRecord;
    type Settings = FeederSettings;

    fn settings_mailbox() -> &'static Mailbox<FeederSettings> {
        &settings::SETTINGS
    }

    fn settings(&self) -> Stored<FeederSettings> {
        self.settings.stored()
    }

    fn save_settings(&mut self, settings: &FeederSettings, generation: u64) -> bool {
        self.settings.save_at(settings, generation)
    }

    fn reopen_settings(&mut self) {
        self.settings.reopen();
    }

    fn settings_record(&self) -> Option<FeederRecord> {
        Some(FeederRecord::Settings {
            status: self.settings.status,
            generation: self.settings.generation,
            settings: self.settings.settings,
        })
    }

    fn open_kept(&mut self) {
        self.calibration.open(Slots::new());
    }

    fn open_cleared(&mut self) {
        self.settings.open(Slots::new(), SETTINGS_BINDING);
        self.plans.open(Slots::new());
        self.call.open(Region::new());
    }

    fn reopen_handed_over(&mut self) {
        self.settings = FeederSettingsStore::default();
        self.plans = PlansStore::default();
        self.call = CallStore::default();
        self.open_cleared();
    }

    fn publish(&self, armed: bool) {
        calibration::publish(&self.calibration, armed);
        plans::publish(&self.plans, armed);
        call::publish(&self.call, armed);
    }

    fn open_with_events(&mut self) {
        self.traces.open(Region::new(), TRACES_UNITS as usize);
        traces::publish_fill(self.traces.kept(), self.traces.capacity());
    }

    fn open_late(&mut self) -> Option<FeederRecord> {
        if self.stats_opened {
            return None;
        }
        self.stats_opened = true;
        Some(self.open_stats())
    }

    fn events_filled(fill: EventsFill) {
        stats::publish_fill(fill);
    }

    fn mailboxes<H: Hooks>(&mut self, core: &mut Core<F>) -> Option<Step> {
        // A change left unsaved (its guard dropped) waits for the next pass.
        if let Some(taken) = calibration::take() {
            let Some(record) = self.save_calibration::<H>(core, taken.get()) else {
                return Some(core.waiting());
            };
            core.publish(self);
            taken.finish();
            return Some(next(POLL_MS, Some(Record::Product(record))));
        }
        if let Some(taken) = plans::take() {
            let plans::Asked { change, asked } = *taken.get();
            let Some(record) = self.save_plans::<H>(core, &change, asked) else {
                return Some(core.waiting());
            };
            core.publish(self);
            taken.finish();
            return Some(next(POLL_MS, Some(Record::Product(record))));
        }
        // The chunk's bytes are copied out of the slot only as the flash
        // takes them.
        let taken = call::take()?;
        let work = taken.get();
        match self.save_call::<H>(core, work.queued, work.bytes()) {
            CallSave::Done { record, delay_ms } => {
                core.publish(self);
                taken.finish();
                core.linger(if matches!(self.call.clip, CallClip::Receiving { .. }) {
                    RECEIVE_LINGER_POLLS
                } else {
                    0
                });
                Some(next(delay_ms, record.map(Record::Product)))
            }
            CallSave::Turn => Some(core.waiting()),
            // A call plays from the clip for seconds: the work below
            // goes on meanwhile, and the change is tried each step.
            CallSave::Held => None,
        }
    }

    fn rest(&mut self, core: &mut Core<F>, writable: bool) -> Option<Step> {
        if let Some(step) = self.trace_step(core, writable) {
            return Some(step);
        }
        // The totals go last, behind every queued record.
        if writable
            && events::pending().is_none()
            && let Some(trigger) = self.stats_wanted()
        {
            return Some(self.save_stats(core, trigger));
        }
        None
    }

    fn keep_on_reset() -> Option<(u32, Wear)> {
        Some((STATS_BASE, keep_wear::<F>()))
    }
}

/// The service task's settings stage; only `wellfed_store_step` enters it.
#[cfg(target_os = "none")]
static WRITER: wellfed_sdk::Slot<Writer<wellfed_sdk::SdkStorage>> = wellfed_sdk::Slot::new();

#[cfg(target_os = "none")]
struct Line(Record);

#[cfg(target_os = "none")]
impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        device::describe(self.0, f)
    }
}

/// The SDK runtime's settings stage: one writer step, logged as a `PLCFG1`
/// record; the delay to the next, 0 to end. Its signature must match the
/// runtime's `extern "Rust"` declaration in `wellfed-sdk`'s `runtime.rs`
/// (the linker does not check it); the assertion below pins it.
///
/// # Safety
/// Called only from the service task, with the runtime's fixed provider,
/// never otherwise borrowed.
#[cfg(target_os = "none")]
#[unsafe(no_mangle)]
pub unsafe fn wellfed_store_step(
    provider: *mut wellfed_hal::flash_lock::provider::Provider,
) -> u32 {
    use wellfed_hal::flash::park::Hardware;
    use wellfed_hal::startup;

    let Some(mut writer) = WRITER.enter_or_init(Default::default) else {
        return 0;
    };
    // SAFETY: the caller's contract.
    let step = unsafe { writer.step::<Hardware, wellfed_sdk::SdkHooks>(provider) };
    wellfed_hal::writer_wake::settle(step.rested);

    if let Some(record) = step.record {
        let mut report = startup::Report::new();
        if let Ok(line) = report.record(format_args!("PLCFG1 {}\r\n", Line(record))) {
            sdk::log(line);
        }
    }
    step.delay_ms
}

#[cfg(target_os = "none")]
const _: unsafe fn(*mut wellfed_hal::flash_lock::provider::Provider) -> u32 = wellfed_store_step;
