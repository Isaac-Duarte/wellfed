//! [`Updates`], a simulated device's boot and update side: the loader's
//! choice and KM0's trial ([`Banks::boot`]) as it boots, the confirmation a
//! new build in trial makes at once, and the firmware's own update stage
//! (`wellfed_hal::ota::update`) stepped as the service task gives it a
//! turn. A build that never confirms resets after [`TRIAL_MS`], as the
//! supervisor's deadline makes it; one that storms resets the same way.
//!
//! Nothing here reads a clock: every call takes the time, so a test can run
//! virtual time and a driver real time through the same code.

use std::cell::RefCell;
use std::rc::Rc;

use wellfed_board::storage;
use wellfed_core::ota::{self, CONFIRM_LEN};
use wellfed_core::store::slot::SlotIo;
use wellfed_core::store::update::{State, UpdateStore};
use wellfed_hal::ota::mailbox;
use wellfed_hal::ota::update::{self as update, Floor, Update};
use wellfed_hal::trial::{self, Storm};
use wellfed_signed::offer::ImageKind;

use crate::banks::{Banks, Boot, Bus, Trial};
use crate::flash::{Flash, Region};

/// A build that never confirms resets this long into each trial boot: the
/// firmware's supervisor waits ten minutes; the simulator does not.
pub const TRIAL_MS: u32 = 2_000;

/// What a boot did: the bank the loader chose, whether its build is in
/// trial and confirmed itself, what a reset storm left it, and the build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Booted {
    pub bank: u8,
    pub trial: bool,
    pub confirmed: Option<Floor>,
    pub storm: Storm,
    pub build: [u8; 32],
}

/// One turn of the update stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Turn {
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
    stage: Box<Update<Region>>,
    /// When this boot ends in a reset nobody asked for: a build that never
    /// confirms at its trial deadline, or one that storms.
    fault_reset: Option<u32>,
    /// The restart that comes is `fault_reset`'s, not one the device asked
    /// for.
    faulted: bool,
    /// This boot has run long enough to end a storm's count.
    settled: bool,
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
        mailbox::reset();
        wellfed_hal::interlock::INTERLOCK.reset();
        let store = Region::new(flash, storage::UPDATE);
        let mut stage = Box::new(Update::new(bank, build, update_key, store));
        let behaviour = banks.borrow().trial;
        let confirmed = (trial && behaviour != Trial::NeverConfirms)
            .then(|| confirm(flash, &mut stage, &build));
        let storms = behaviour == Trial::Storms && bank == 1;
        let fault_reset =
            ((trial && confirmed.is_none()) || storms).then(|| now_ms.wrapping_add(TRIAL_MS));

        let updates = Self {
            flash: Rc::clone(flash),
            banks: Rc::clone(banks),
            bank,
            stage,
            fault_reset,
            faulted: false,
            settled: false,
        };
        let booted = Booted {
            bank,
            trial,
            confirmed,
            storm,
            build,
        };
        (updates, booted)
    }

    /// The device restarts: one it asked for is no fault, so the runtime
    /// ends a storm's count before it.
    pub fn restarting(&mut self) {
        if !self.faulted {
            self.banks.borrow_mut().deliberate();
        }
    }

    /// The update stage's turn at `now_ms`, as the service task gives it.
    pub fn turn(&mut self, now_ms: u32) -> Turn {
        let mut restart = false;
        if self
            .fault_reset
            .is_some_and(|at| wellfed_core::deadline_passed(now_ms, at))
        {
            self.faulted = true;
            restart = true;
        }
        if !self.settled && now_ms >= trial::STABLE_MS {
            self.settled = true;
            self.banks.borrow_mut().settle(self.bank);
        }
        let mut banks = self.banks.borrow_mut();
        let mut bus = Bus {
            banks: &mut banks,
            bank: self.bank,
            handover: Region::new(&self.flash, storage::RESET),
        };
        let step = self.stage.step(&mut bus, now_ms);
        Turn {
            record: step.record,
            restart: restart || step.restart,
            busy: step.delay_ms <= update::BUSY_MS,
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
            banks.bank1_build = candidate.build_id;
        }
    }

    // An unreadable record names no build, as erased flash does.
    let mut record = [0xffu8; CONFIRM_LEN];
    if !Region::new(flash, storage::CONFIRM).read(0, 0, &mut record) {
        record = [0xff; CONFIRM_LEN];
    }
    let booted = banks.boot(&record);
    (booted, banks.build(booted.bank))
}

/// The build confirms: the floor first, then the record, as the
/// firmware's confirmation writes them.
fn confirm(flash: &Rc<Flash>, stage: &mut Update<Region>, build: &[u8; 32]) -> Floor {
    let floor = stage.raise_floor();
    let version = match floor {
        Floor::Raised { version } => version,
        Floor::Unchanged => 0,
        Floor::Failed => return floor,
    };
    let record = ota::confirm_record(build, version);
    let sector = Region::new(flash, storage::CONFIRM);
    if !(sector.erase(0) && sector.program(0, 0, &record)) {
        return Floor::Failed;
    }
    floor
}
