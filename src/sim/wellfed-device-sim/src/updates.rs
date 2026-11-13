//! [`Updates`], a simulated device's boot and update side: the loader's
//! choice and KM0's trial ([`Banks::boot`]) as it boots, the confirmation a
//! new build in trial makes once it has run healthy a while, the hold a
//! reset storm puts on a boot, and the firmware's own update stage
//! (`wellfed_hal::ota::update`) stepped as the service task gives it a
//! turn.
//!
//! As on the feeder:
//! - **A trial build confirms** after [`CONFIRM_MS`], raising the floor
//!   first and then writing the confirmation record, as the firmware's
//!   confirmation does once its healthy window and the feeding owner's
//!   proof are over. Until then it is confirming ([`Updates::confirming`]),
//!   and the firmware takes no feed. A build that never confirms, or whose
//!   confirmation fails (the floor or the record not written), stays in
//!   trial and resets after [`TRIAL_MS`], as the supervisor's deadline makes
//!   it; one that storms resets the same way. KM0 then counts the reset.
//! - **A trial that can never confirm gives way** at once, as the
//!   firmware's runtime does (`wellfed_sdk::give_way`): its confirmation
//!   failed, it has no network to join (no build network and none
//!   stored), or the owner held Reset. KM0's count is set to its limit and
//!   the device restarts, so the next boot gives way to bank 0.
//! - **A storm holds a boot** that has nothing to give way to
//!   ([`Storm::Held`]): the feeding owner stays closed
//!   ([`Updates::held`]) for `trial::hold_ms` of the storm's level, then the
//!   runtime releases it one level up.
//! - **A boot settles** once it has run `trial::STABLE_MS` (a trial only
//!   once confirmed, a released boot only once it has fed for its new
//!   level's hold), and KM0 takes its storm count back.
//!
//! A halted boot (`Storm::Halted`) starts nothing on the feeder, not even
//! the device API; the simulator does not model it and boots it as usual.
//!
//! The simulation begins on a device whose first bank-1 build has run its
//! trial already: at the first boot on new banks, an erased confirmation
//! sector is written to name that build, as its first boot's confirmation
//! would have. A sector erased later is left erased.
//!
//! The settings writer starts on bank 1 only once the confirmation ends,
//! confirmed now or already (bank 0 starts it at once); the feeder's
//! simulator skips the writer's turn while [`Updates::confirming`] holds.
//!
//! Nothing here reads a clock: every call takes the time, so a test can run
//! virtual time and a driver real time through the same code.

use std::cell::RefCell;
use std::rc::Rc;

use wellfed_board::storage;
use wellfed_core::ota::{self, CONFIRM_LEN};
use wellfed_core::store::slot::SlotIo;
use wellfed_core::store::update::{State, UpdateStore};
use wellfed_device::store::net::{BuildWifi, NetStatus, NetStore};
use wellfed_hal::ota::confirm::GiveWayReason;
use wellfed_hal::ota::mailbox;
use wellfed_hal::ota::update::{self as update, Floor, Update};
use wellfed_hal::trial::{self, Storm};
use wellfed_signed::offer::ImageKind;

use crate::banks::{BANK1_BUILD, Banks, Boot, Bus, Trial};
use crate::flash::{Flash, Region};

/// A build that never confirms resets this long into each trial boot: the
/// firmware's supervisor waits ten minutes; the simulator does not.
pub const TRIAL_MS: u32 = 2_000;

/// A trial build confirms this long into its boot: the firmware's does
/// after 30 s of health and the feeding owner's 2 s proof, scaled as
/// [`TRIAL_MS`] is, so a test sees the window and still runs quickly.
pub const CONFIRM_MS: u32 = 500;
const _: () = assert!(CONFIRM_MS < TRIAL_MS);

/// What a boot did: the bank the loader chose, whether its build is in
/// trial, what a reset storm left it, and the build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Booted {
    pub bank: u8,
    pub trial: bool,
    pub storm: Storm,
    pub build: [u8; 32],
}

/// One turn of the update stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Turn {
    /// The trial build's confirmation, on the turn it ran.
    pub confirmed: Option<Floor>,
    /// The trial gave way to bank 0 on this turn, for this reason.
    pub gave_way: Option<GiveWayReason>,
    /// The stage's record, if it made one.
    pub record: Option<update::Record>,
    /// The device must restart after this pass: the stage asked, or a
    /// trial or storm reached its deadline.
    pub restart: bool,
    /// The stage has work waiting and wants its next turn soon.
    pub busy: bool,
}

/// The update side of one boot.
pub struct Updates {
    flash: Rc<Flash>,
    banks: Rc<RefCell<Banks>>,
    bank: u8,
    build: [u8; 32],
    stage: Box<Update<Region>>,
    /// This boot runs a build in trial.
    trial: bool,
    /// When a trial build confirms, until it has tried; `None` for one
    /// that never will.
    confirm_at: Option<u32>,
    /// The trial build confirmed this boot.
    confirmed: bool,
    /// Every boot of this build resets soon after it starts.
    storms: bool,
    /// When this boot ends in a reset nobody asked for: a trial build at
    /// its deadline unless it confirmed, or one that storms.
    fault_reset: Option<u32>,
    /// The restart that comes is `fault_reset`'s, not one the device asked
    /// for.
    faulted: bool,
    /// A reset storm holds this boot: the feeding owner stays closed.
    held: bool,
    /// This boot started held, and when its hold ended.
    started_held: bool,
    released_at: Option<u32>,
    /// This boot has run long enough to end a storm's count.
    settled: bool,
    /// The trial gave way: the restart that follows is neither a fault nor
    /// one that ends a storm's count.
    gave_way: bool,
}

impl Updates {
    /// Boot from `flash` on `banks` at `now_ms`, trusting `update_key`. The
    /// update mailbox and the interlock are RAM, so a boot forgets every
    /// request and chunk, and the write turn a give-way kept.
    pub fn open(
        flash: &Rc<Flash>,
        banks: &Rc<RefCell<Banks>>,
        update_key: Option<[u8; 32]>,
        now_ms: u32,
    ) -> (Self, Booted) {
        let (Boot { bank, trial, storm }, build) = boot(flash, &mut banks.borrow_mut(), update_key);
        #[cfg(feature = "harness")]
        crate::harness::Alone::booting();
        mailbox::reset();
        wellfed_hal::interlock::INTERLOCK.reset();
        let store = Region::new(flash, storage::UPDATE);
        let stage = Box::new(Update::new(bank, build, update_key, store));
        let behaviour = banks.borrow().trial;
        let confirm_at =
            (trial && behaviour != Trial::NeverConfirms).then(|| now_ms.wrapping_add(CONFIRM_MS));
        let storms = behaviour == Trial::Storms && bank == 1;
        let fault_reset = (trial || storms).then(|| now_ms.wrapping_add(TRIAL_MS));
        let held = storm == Storm::Held;

        let updates = Self {
            flash: Rc::clone(flash),
            banks: Rc::clone(banks),
            bank,
            build,
            stage,
            trial,
            confirm_at,
            confirmed: false,
            storms,
            fault_reset,
            faulted: false,
            held,
            started_held: held,
            released_at: None,
            settled: false,
            gave_way: false,
        };
        let booted = Booted {
            bank,
            trial,
            storm,
            build,
        };
        (updates, booted)
    }

    /// A trial build that has not confirmed (yet): the firmware takes no
    /// feed meanwhile, so the confirmation never waits for one.
    pub const fn confirming(&self) -> bool {
        self.trial && !self.confirmed
    }

    /// A reset storm holds this boot until its hold has run: the feeding
    /// owner stays closed.
    pub const fn held(&self) -> bool {
        self.held
    }

    /// The device restarts: one it asked for is no fault, so the runtime
    /// ends a storm's count before it.
    pub fn restarting(&mut self) {
        if !self.faulted && !self.gave_way {
            self.banks.borrow_mut().deliberate();
        }
    }

    /// Give the trial build up for bank 0, for `reason`: KM0's count goes
    /// to its limit, and the caller restarts the device. Only while it is
    /// confirming, and only once; whether it gave way.
    pub fn give_way(&mut self, _reason: GiveWayReason) -> bool {
        if !self.confirming() || self.gave_way {
            return false;
        }
        self.banks.borrow_mut().spent(self.bank);
        self.gave_way = true;
        true
    }

    /// The update stage's turn at `now_ms`, as the service task gives it,
    /// after the runtime's own: the trial's confirmation (or its give-way)
    /// and the storm's count. `build_wifi` is the image's.
    pub fn turn(&mut self, now_ms: u32, build_wifi: BuildWifi) -> Turn {
        let mut restart = false;
        if self
            .fault_reset
            .is_some_and(|at| wellfed_core::deadline_passed(now_ms, at))
        {
            self.faulted = true;
            restart = true;
        }
        let unprovisioned = self.confirming() && self.unprovisioned(build_wifi);
        let (confirmed, failed) = match unprovisioned {
            true => (None, Some(GiveWayReason::NoNetwork)),
            false => self.confirm(now_ms),
        };
        let gave_way = failed.filter(|&reason| self.give_way(reason));
        restart |= gave_way.is_some();
        self.storm_step(now_ms);

        let mut banks = self.banks.borrow_mut();
        let mut bus = Bus {
            banks: &mut banks,
            bank: self.bank,
            handover: Region::new(&self.flash, storage::RESET),
        };
        let step = self.stage.step(&mut bus, now_ms);
        Turn {
            confirmed,
            gave_way,
            record: step.record,
            restart: restart || step.restart,
            busy: step.delay_ms <= update::BUSY_MS,
        }
    }

    /// The trial build's one confirmation, once it is due: what it did to
    /// the floor, and why it failed if it did. One that fails is not tried
    /// again this boot, as the firmware's confirmation ends on an error,
    /// and the trial gives way.
    fn confirm(&mut self, now_ms: u32) -> (Option<Floor>, Option<GiveWayReason>) {
        let Some(due) = self.confirm_at else {
            return (None, None);
        };
        if !wellfed_core::deadline_passed(now_ms, due) {
            return (None, None);
        }
        self.confirm_at = None;

        match confirm(&self.flash, &mut self.stage, &self.build) {
            Ok(floor) => {
                self.confirmed = true;
                if !self.storms {
                    self.fault_reset = None;
                }
                (Some(floor), None)
            }
            Err(reason) => (Some(Floor::Failed), Some(reason)),
        }
    }

    /// No network to join: the image has none built in and the credential
    /// store reads as holding none (not one it cannot read).
    fn unprovisioned(&self, build_wifi: BuildWifi) -> bool {
        if build_wifi == BuildWifi::Present {
            return false;
        }
        let mut net = NetStore::default();
        let status = net.open(Region::new(&self.flash, storage::NET));
        matches!(status, NetStatus::Empty | NetStatus::Loaded) && net.credentials.is_none()
    }

    /// The runtime's storm step: a held boot that has run its level's hold
    /// is released a level up, and a boot that has run long enough gives
    /// KM0 its count back, once.
    fn storm_step(&mut self, now_ms: u32) {
        if self.held {
            let level = self.banks.borrow().level(self.bank);
            if now_ms >= trial::hold_ms(level) {
                self.banks.borrow_mut().release(self.bank);
                self.held = false;
                self.released_at = Some(now_ms);
            }
            return;
        }
        if self.settled {
            return;
        }
        // A released boot counts its stable run from the release, and only
        // once it has outlasted the new level's hold.
        let stable = if self.started_held {
            let level = self.banks.borrow().level(self.bank);
            self.released_at
                .is_some_and(|at| now_ms.wrapping_sub(at) >= trial::hold_ms(level))
        } else {
            now_ms >= trial::STABLE_MS && !self.confirming()
        };
        if stable {
            self.settled = true;
            self.banks.borrow_mut().settle(self.bank);
        }
    }
}

/// The loader's choice and KM0's trial for this boot: the bank, whether its
/// build is in trial, and the build. A build bank 0 activated in bank 1 is
/// the one the update store names.
fn boot(flash: &Rc<Flash>, banks: &mut Banks, key: Option<[u8; 32]>) -> (Boot, [u8; 32]) {
    let mut store = UpdateStore::default();
    store.open(Region::new(flash, storage::UPDATE));
    if store.record().bank0.is_none()
        && let Some(bank0) = banks.bank0_record(key)
    {
        // As if bank 0 had booted once: bank 1 gives way only to it.
        let _ = store.set_bank0(bank0);
    }
    if banks.bank1_valid()
        && let State::Activated(candidate) = store.record().state
    {
        if banks.stock_reinstalls && candidate.kind == ImageKind::Stock {
            banks.stock_gave_way();
        } else {
            banks.bank1_build = banks.boots_instead.unwrap_or(candidate.build_id);
        }
    }

    // An unreadable record names no build, as erased flash does.
    let sector = Region::new(flash, storage::CONFIRM);
    let mut record = [0xffu8; CONFIRM_LEN];
    if !sector.read(0, 0, &mut record) {
        record = [0xff; CONFIRM_LEN];
    }
    // The device the simulation begins on confirmed its first bank-1 build
    // before it began. Only then: an erase later (bank 0's handover, say)
    // leaves that build to a trial of its own.
    let fresh = banks.take_fresh();
    if fresh
        && record == [0xff; CONFIRM_LEN]
        && banks.bank1_valid()
        && banks.bank1_build == BANK1_BUILD
    {
        record = ota::confirm_record(&BANK1_BUILD, 0);
        if !sector.program(0, 0, &record) {
            record = [0xff; CONFIRM_LEN];
        }
    }
    let booted = banks.boot(&record);
    (booted, banks.build(booted.bank))
}

/// The build confirms: the floor first, then the record, as the
/// firmware's confirmation writes them; which of them failed.
fn confirm(
    flash: &Rc<Flash>,
    stage: &mut Update<Region>,
    build: &[u8; 32],
) -> Result<Floor, GiveWayReason> {
    let floor = stage.raise_floor();
    let version = match floor {
        Floor::Raised { version } => version,
        Floor::Unchanged => 0,
        Floor::Failed => return Err(GiveWayReason::Floor),
    };
    let record = ota::confirm_record(build, version);
    let sector = Region::new(flash, storage::CONFIRM);
    if !(sector.erase(0) && sector.program(0, 0, &record)) {
        return Err(GiveWayReason::Write);
    }
    Ok(floor)
}
