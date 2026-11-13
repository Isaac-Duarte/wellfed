//! What the runtime says of a boot reaches readiness on the simulator, in
//! virtual time, as on the feeder: a trial build reads `Starting` until it
//! confirms, and resets at its deadline if it never does, until KM0 gives
//! way to bank 0; one no boot of it could confirm (an update store it
//! cannot read, no network to join) or whose owner holds Reset gives way
//! at once, given a valid bank 0; a boot a reset storm holds reads
//! `Fault`, its feeding owner closed, until its hold has run.

use std::cell::RefCell;
use std::rc::Rc;

use wellfed_board::storage;
use wellfed_core::store::slot::SlotIo;
use wellfed_core_feeder::schedule::{PLANS, Plan, Table, Weekdays};
use wellfed_device_api::mailbox::{PlansChange, PlansResult};
use wellfed_feeder::feeder::Platform;
use wellfed_feeder_sim::banks::{Banks, TRIAL_LIMIT, Trial};
use wellfed_feeder_sim::flash::{Flash, Region};
use wellfed_feeder_sim::harness;
use wellfed_feeder_sim::sim::{CONFIRM_MS, Record, Sim, TRIAL_MS};
use wellfed_hal::ota::confirm::GiveWayReason;
use wellfed_hal::ota::update::Floor;
use wellfed_hal::trial::{self, Storm};
use wellfed_proto::api::{Blocked, Readiness};

/// A build bank 0 never activated, in bank 1 and unconfirmed: the next
/// boot is a trial.
const UNCONFIRMED: [u8; 32] = [0x77; 32];

/// One simulated feeder across its boots, and its uptime this boot.
struct Feeder {
    flash: Rc<Flash>,
    banks: Rc<RefCell<Banks>>,
    boots: u32,
    sim: Sim,
    now: u32,
    records: Vec<Record>,
}

impl Feeder {
    fn boot(flash: Rc<Flash>, banks: Rc<RefCell<Banks>>) -> Self {
        let sim = open(&flash, &banks, 1);
        Self {
            flash,
            banks,
            boots: 1,
            sim,
            now: 0,
            records: Vec::new(),
        }
    }

    /// A feeder whose bank 1 holds a build in trial.
    fn on_trial() -> Self {
        let banks = Banks::new();
        banks.borrow_mut().bank1_build = UNCONFIRMED;
        Self::boot(Flash::memory(), banks)
    }

    /// Run the loop for `ms`, a pass at least every second (the last may
    /// come up to a second before the end); whether the boot restarted.
    fn run(&mut self, ms: u32) -> bool {
        let end = self.now + ms;
        while self.now < end {
            let records = &mut self.records;
            let delay = self.sim.pass(self.now, |record| records.push(record));
            if self.sim.take_reboot() {
                return true;
            }
            self.now += delay.clamp(1, 1000).min(end - self.now);
        }
        false
    }

    /// A reset and the boot after it, with what KM0's register kept.
    fn restart(&mut self) {
        self.boots += 1;
        self.sim = open(&self.flash, &self.banks, self.boots);
        self.now = 0;
        self.records.clear();
    }

    /// The boot record of this boot, once its first pass ran.
    fn booted(&self) -> (u8, bool, Storm) {
        self.records
            .iter()
            .find_map(|record| match record {
                Record::Boot { bank, trial, storm } => Some((*bank, *trial, *storm)),
                _ => None,
            })
            .expect("the first pass logs the boot")
    }

    /// The give-ways this boot made.
    fn gave_way(&self) -> Vec<GiveWayReason> {
        self.records
            .iter()
            .filter_map(|record| match record {
                Record::GaveWay(reason) => Some(*reason),
                _ => None,
            })
            .collect()
    }

    /// The confirmations this boot made.
    fn confirmations(&self) -> Vec<Floor> {
        self.records
            .iter()
            .filter_map(|record| match record {
                Record::Confirmed(floor) => Some(*floor),
                _ => None,
            })
            .collect()
    }
}

/// A table with one breakfast plan.
fn breakfast() -> Table {
    let mut table: Table = [None; PLANS];
    table[0] = Some(Plan {
        minute: 7 * 60,
        portions: 1,
        days: Weekdays::EVERY_DAY,
        enabled: true,
        call: 0,
    });
    table
}

/// Whether the stored plans hold `table`, and a change still waits for the
/// writer.
fn plans(sim: &mut Sim) -> (Table, bool) {
    match sim.platform().plans() {
        PlansResult::Current { view, pending } => (view.table, pending),
        other => panic!("{other:?}"),
    }
}

fn open(flash: &Rc<Flash>, banks: &Rc<RefCell<Banks>>, boot: u32) -> Sim {
    let random = harness::counter(0x41)();
    Sim::open(flash, banks, None, random, 0, boot, None).expect("the feeder opens")
}

/// A trial build serves no feed until it confirms, and then feeds as any
/// boot does; confirmed, it is no longer reset at the trial's deadline.
#[test]
fn a_trial_build_reads_starting_until_it_confirms() {
    let mut feeder = Feeder::on_trial();
    assert!(!feeder.run(1));
    assert_eq!(feeder.booted(), (1, true, Storm::Normal));
    assert!(feeder.sim.confirming());
    assert_eq!(
        feeder.sim.readiness(),
        Readiness::Blocked(Blocked::Starting)
    );

    assert!(!feeder.run(CONFIRM_MS - 10));
    assert_eq!(
        feeder.sim.readiness(),
        Readiness::Blocked(Blocked::Starting)
    );
    assert!(feeder.confirmations().is_empty());

    assert!(!feeder.run(TRIAL_MS - CONFIRM_MS));
    assert_eq!(feeder.confirmations(), [Floor::Unchanged]);
    assert!(!feeder.sim.confirming());
    assert_eq!(feeder.sim.readiness(), Readiness::Ready);

    assert!(
        !feeder.run(2 * TRIAL_MS),
        "a confirmed build runs past the deadline"
    );
}

/// A build that never confirms reads `Starting` for its whole trial, so it
/// takes no feed the deadline's reset would cut.
#[test]
fn a_build_that_never_confirms_reads_starting_until_its_deadline() {
    let mut feeder = Feeder::on_trial();
    feeder.banks.borrow_mut().trial = Trial::NeverConfirms;
    feeder.restart();

    assert!(!feeder.run(TRIAL_MS - 10));
    assert_eq!(
        feeder.sim.readiness(),
        Readiness::Blocked(Blocked::Starting)
    );
    assert!(feeder.confirmations().is_empty());
    assert!(feeder.run(1_000), "reset at the trial's deadline");
}

/// A confirmation whose record cannot be written may pass on the next
/// boot, so the build stays in trial: it reads `Starting` on, resets at the
/// deadline, and KM0 counts each reset until it gives way to bank 0.
#[test]
fn a_trial_whose_confirmation_record_fails_resets_until_bank_0_takes_over() {
    let flash = Flash::memory();
    flash.fail_programs(Some(storage::CONFIRM));
    let banks = Banks::new();
    banks.borrow_mut().bank1_build = UNCONFIRMED;
    let mut feeder = Feeder::boot(flash, banks);

    for boot in 1..=TRIAL_LIMIT {
        assert!(!feeder.run(TRIAL_MS - 10), "boot {boot}");
        assert_eq!(feeder.booted(), (1, true, Storm::Normal), "boot {boot}");
        assert_eq!(feeder.confirmations(), [Floor::Failed], "boot {boot}");
        assert!(feeder.gave_way().is_empty(), "boot {boot}");
        assert_eq!(
            feeder.sim.readiness(),
            Readiness::Blocked(Blocked::Starting),
            "boot {boot}"
        );
        assert!(feeder.run(1_000), "boot {boot} resets at its deadline");
        feeder.restart();
    }

    assert!(!feeder.run(TRIAL_MS + 10));
    assert_eq!(feeder.booted(), (0, false, Storm::Normal));
    assert!(!feeder.banks.borrow().bank1_valid());
    assert_eq!(feeder.sim.readiness(), Readiness::Ready);
}

/// Spoil both units of the update store, as an unreadable record leaves
/// it.
fn corrupt_update_store(flash: &Rc<Flash>) {
    let update = Region::new(flash, storage::UPDATE);
    for unit in 0..2 {
        assert!(update.erase(unit));
        assert!(update.program(unit, 0, &[0x42; 64]));
    }
}

/// A trial over an update store it cannot read cannot raise its floor, so
/// it cannot confirm: it gives way at once. Nothing is written for the
/// give-way, so the store stays as it was, and bank 0, whose writer runs
/// at once, serves.
#[test]
fn a_trial_over_a_corrupt_update_store_gives_way_at_once() {
    let mut feeder = Feeder::on_trial();
    corrupt_update_store(&feeder.flash);
    let before = Region::new(&feeder.flash, storage::UPDATE);
    let mut unit = [0u8; 64];
    assert!(before.read(0, 0, &mut unit));
    feeder.restart();

    assert!(feeder.run(TRIAL_MS - 10), "restarts before its deadline");
    assert_eq!(feeder.booted(), (1, true, Storm::Normal));
    assert_eq!(feeder.confirmations(), [Floor::Failed]);
    assert_eq!(feeder.gave_way(), [GiveWayReason::Floor]);
    let mut after = [0u8; 64];
    assert!(Region::new(&feeder.flash, storage::UPDATE).read(0, 0, &mut after));
    assert_eq!(after, unit, "the give-way wrote nothing there");

    feeder.restart();
    assert!(!feeder.run(TRIAL_MS + 10));
    assert_eq!(feeder.booted(), (0, false, Storm::Normal));
    assert_eq!(feeder.sim.readiness(), Readiness::Ready);
    assert!(
        feeder.sim.platform().totals().is_some(),
        "bank 0's writer runs"
    );
}

/// A consumer image with no network stored has nothing to join, and only
/// the settings writer, which waits for the confirmation, could save one:
/// the trial gives way on its first turn, before any confirmation.
#[test]
fn a_trial_with_no_network_to_join_gives_way_at_once() {
    use wellfed_device::store::net::BuildWifi;

    let mut feeder = Feeder::on_trial();
    feeder.sim.platform().set_build_wifi(BuildWifi::Absent);

    assert!(feeder.run(CONFIRM_MS), "restarts before it would confirm");
    assert_eq!(feeder.booted(), (1, true, Storm::Normal));
    assert!(feeder.confirmations().is_empty());
    assert_eq!(feeder.gave_way(), [GiveWayReason::NoNetwork]);

    feeder.restart();
    assert!(!feeder.run(1_000));
    assert_eq!(feeder.booted(), (0, false, Storm::Normal));
}

/// A network stored, or built in, is not this, even one that is not there:
/// the trial confirms as usual, or meets its deadline.
#[test]
fn a_trial_with_a_network_stored_does_not_give_way() {
    use wellfed_device::store::net::{BuildWifi, NetStore};
    use wellfed_device_api::wifi::Credentials;

    let mut feeder = Feeder::on_trial();
    let mut net = NetStore::default();
    net.open(Region::new(&feeder.flash, storage::NET));
    let away = Credentials::new(b"away", b"long gone").expect("valid credentials");
    assert!(net.save(Some(&away)));
    feeder.restart();
    feeder.sim.platform().set_build_wifi(BuildWifi::Absent);

    assert!(!feeder.run(TRIAL_MS - 10));
    assert!(feeder.gave_way().is_empty());
    assert_eq!(feeder.confirmations(), [Floor::Unchanged]);
}

/// A Reset hold on a trial still confirming is never queued for a writer
/// that has not started: the trial gives way, and bank 0, where the owner
/// holds Reset again, counts the build as one that failed its trial.
#[test]
fn a_reset_hold_on_a_trial_gives_way() {
    use wellfed_core::buttons::{Button, Buttons, Gesture};
    use wellfed_feeder::feeder::{self, owner};

    let held = |button| Gesture::Hold {
        chord: Buttons::of(button),
        level: 1,
    };
    let mut feeder = Feeder::on_trial();
    assert!(!feeder.run(100));
    feeder.sim.gesture(held(Button::Lock));
    assert!(!feeder.run(50));
    feeder.sim.gesture(held(Button::Reset));
    assert!(feeder.run(50), "the give-way's restart");

    let gave_way = Record::Feeder(feeder::Record::Owner(owner::Record::ResetGaveWay));
    assert!(feeder.records.contains(&gave_way), "{:?}", feeder.records);
    assert_eq!(feeder.gave_way(), [GiveWayReason::Reset]);
    assert!(feeder.confirmations().is_empty());

    feeder.restart();
    assert!(!feeder.run(1_000));
    assert_eq!(feeder.booted(), (0, false, Storm::Normal));
}

/// Resets nobody asked for, soon after each boot, make a storm: bank 1
/// gives way to bank 0, and bank 0, with nothing to give way to, boots
/// held. A held boot reads `Fault` with its feeding owner closed until its
/// level's hold has run, then feeds.
#[test]
fn a_boot_a_storm_holds_reads_fault_until_its_hold_ends() {
    let mut feeder = Feeder::boot(Flash::memory(), Banks::new());
    let mut boots = 0;
    loop {
        assert!(!feeder.run(1_000));
        if feeder.booted().2 == Storm::Held {
            break;
        }
        boots += 1;
        assert!(
            boots < 4 * trial::STORM_LIMIT,
            "no storm after {boots} resets"
        );
        feeder.restart();
    }
    assert_eq!(feeder.booted().0, 0, "bank 1 gave way first");

    assert!(feeder.sim.held());
    assert_eq!(feeder.sim.readiness(), Readiness::Blocked(Blocked::Fault));
    let level = feeder.banks.borrow().level(0);
    let hold = trial::hold_ms(level);
    assert!(!feeder.run(hold - 2_000));
    assert_eq!(feeder.sim.readiness(), Readiness::Blocked(Blocked::Fault));
    assert!(
        feeder.sim.food().is_none(),
        "the owner never opened to read"
    );

    assert!(!feeder.run(2_000));
    assert!(!feeder.sim.held());
    assert_eq!(
        feeder.banks.borrow().level(0),
        level + 1,
        "released a level up"
    );
    assert!(!feeder.run(1_000));
    assert_eq!(feeder.sim.readiness(), Readiness::Ready);
}

/// The settings writer starts only once a trial build confirms, as the
/// firmware's runtime starts it: a plans change queued in the window waits
/// in its slot and lands after the confirmation, and the boot's lifetime
/// totals are published only then.
#[test]
fn a_store_write_in_the_trial_window_lands_only_after_the_confirmation() {
    let mut feeder = Feeder::on_trial();
    assert!(!feeder.run(100));
    let queued = feeder.sim.platform().plans_change(
        PlansChange::Replace {
            table: breakfast(),
            at_s: None,
        },
        None,
    );
    assert!(
        matches!(queued, PlansResult::Current { pending: true, .. }),
        "{queued:?}"
    );

    assert!(!feeder.run(CONFIRM_MS - 200));
    assert!(feeder.confirmations().is_empty());
    assert_eq!(plans(&mut feeder.sim), ([None; PLANS], true), "it waits");
    assert_eq!(feeder.sim.platform().totals(), None);

    assert!(!feeder.run(TRIAL_MS - CONFIRM_MS));
    assert_eq!(feeder.confirmations(), [Floor::Unchanged]);
    assert_eq!(plans(&mut feeder.sim), (breakfast(), false));
    assert!(feeder.sim.platform().totals().is_some());
}

/// A build that never confirms never starts its writer: a change queued
/// during its trial is never written, and the boot is not counted in the
/// lifetime totals the next boot reads.
#[test]
fn a_build_that_never_confirms_writes_no_store() {
    let mut feeder = Feeder::on_trial();
    feeder.banks.borrow_mut().trial = Trial::NeverConfirms;
    feeder.restart();
    assert!(!feeder.run(100));
    let queued = feeder.sim.platform().plans_change(
        PlansChange::Replace {
            table: breakfast(),
            at_s: None,
        },
        None,
    );
    assert!(
        matches!(queued, PlansResult::Current { pending: true, .. }),
        "{queued:?}"
    );
    assert!(feeder.run(TRIAL_MS), "reset at the trial's deadline");
    assert_eq!(feeder.sim.platform().totals(), None);

    // Every trial boot resets so (this was the second), and bank 0 then
    // boots with its writer.
    for _ in 2..TRIAL_LIMIT {
        feeder.restart();
        assert!(feeder.run(TRIAL_MS + 1_000));
    }
    feeder.restart();
    assert!(!feeder.run(1_000));
    assert_eq!(feeder.booted().0, 0);
    assert_eq!(
        plans(&mut feeder.sim),
        ([None; PLANS], false),
        "never written"
    );
    let totals = feeder
        .sim
        .platform()
        .totals()
        .expect("bank 0's writer runs");
    assert_eq!(
        totals.resets.power_on, 1,
        "bank 0's boot alone: no trial boot was counted"
    );
}

/// Only the device the simulation begins on had confirmed its first build:
/// a confirmation sector erased later (as bank 0's handover erases it)
/// leaves that build to a trial of its own when it boots again.
#[test]
fn a_confirmation_erased_after_the_first_boot_is_not_written_again() {
    let mut feeder = Feeder::boot(Flash::memory(), Banks::new());
    assert!(!feeder.run(1_000));
    assert_eq!(feeder.booted(), (1, false, Storm::Normal));

    assert!(Region::new(&feeder.flash, storage::CONFIRM).erase(0));
    feeder.restart();
    assert!(!feeder.run(1));
    assert_eq!(feeder.booted(), (1, true, Storm::Normal));
    assert_eq!(
        feeder.sim.readiness(),
        Readiness::Blocked(Blocked::Starting)
    );
}

/// With no valid bank 0 there is nothing to give way to: KM0 would run the
/// trial on held, feeding nothing. A Reset hold is queued as on any boot,
/// and the reset runs once the trial confirms and its writer starts.
#[test]
fn a_reset_hold_on_a_trial_with_no_bank_0_waits_for_the_confirmation() {
    use wellfed_core::buttons::{Button, Buttons, Gesture};
    use wellfed_device_api::mailbox::Requested;
    use wellfed_feeder::feeder::{self, owner};

    let held = |button| Gesture::Hold {
        chord: Buttons::of(button),
        level: 1,
    };
    let mut feeder = Feeder::on_trial();
    feeder.banks.borrow_mut().spoil_bank0();
    feeder.restart();
    assert!(!feeder.run(100));
    feeder.sim.gesture(held(Button::Lock));
    assert!(!feeder.run(50));
    feeder.sim.gesture(held(Button::Reset));
    assert!(!feeder.run(50), "no give-way");

    let queued = Record::Feeder(feeder::Record::Owner(owner::Record::Reset {
        requested: Requested::Queued,
    }));
    assert!(feeder.records.contains(&queued), "{:?}", feeder.records);
    assert!(feeder.gave_way().is_empty());

    assert!(feeder.run(TRIAL_MS), "the reset's restart, once confirmed");
    assert_eq!(feeder.confirmations(), [Floor::Unchanged]);
    assert!(feeder.gave_way().is_empty());
}

/// Nor does a trial with no network to join give way without a valid
/// bank 0: it waits, as the firmware's confirmation always has.
#[test]
fn a_trial_with_no_network_and_no_bank_0_does_not_give_way() {
    use wellfed_device::store::net::BuildWifi;

    let mut feeder = Feeder::on_trial();
    feeder.banks.borrow_mut().spoil_bank0();
    feeder.restart();
    feeder.sim.platform().set_build_wifi(BuildWifi::Absent);

    assert!(!feeder.run(CONFIRM_MS + 100));
    assert!(feeder.gave_way().is_empty());
}
