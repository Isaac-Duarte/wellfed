//! The lifetime statistics on the simulator, in virtual time: saved as the
//! firmware's writer saves them, on the loop's uptime rather than the
//! host's clock, so hours of a test's uptime make the timed saves that give
//! the early budget back, a finished feed is saved before a power cut can
//! lose it, and each boot counts the reset its flags name.

use std::cell::RefCell;
use std::rc::Rc;

use wellfed_core::buttons::{Button, Buttons, Gesture};
use wellfed_feeder::feeder::Platform;
use wellfed_feeder::stats::{EARLY_SAVES, Totals};
use wellfed_feeder_sim::banks::Banks;
use wellfed_feeder_sim::flash::Flash;
use wellfed_feeder_sim::harness::{self, run_loop};
use wellfed_feeder_sim::sim::Sim;

const HOUR_MS: u64 = 60 * 60_000;
/// The ROM's watchdog reset flag.
const WATCHDOG: u32 = 1 << 17;

fn open(flash: &Rc<Flash>, banks: &Rc<RefCell<Banks>>, boot: u32) -> Sim {
    let random = harness::counter(0x51)();
    Sim::open(flash, banks, None, random, 0, boot, None).expect("the feeder opens")
}

/// The totals the writer publishes once its first turn has run.
fn totals(sim: &mut Sim, now: &mut u32) -> Totals {
    run_loop(sim, now, 10, |_, _| {});
    sim.platform().totals().expect("a writable store")
}

/// Boots that each run two hours and restart on purpose keep every boot's
/// count, past the early budget: the timed saves those hours make give
/// the tokens back that each boot's own save spends.
#[test]
fn boots_far_past_the_early_budget_all_count() {
    let (flash, banks) = (Flash::memory(), Banks::new());
    let boots = u32::from(EARLY_SAVES) + 3;
    for boot in 1..=boots {
        let mut sim = open(&flash, &banks, boot);
        let mut now = 0;
        run_loop(&mut sim, &mut now, 2 * HOUR_MS, |_, _| {});
        sim.platform().reset();
        assert!(sim.take_reboot());
    }

    let mut sim = open(&flash, &banks, boots + 1);
    let totals = totals(&mut sim, &mut 0);
    assert_eq!(totals.resets.power_on, boots + 1, "{totals:?}");
    // Each restart's save holds its boot's uptime as the loop last handed
    // it over, within a minute of the two hours.
    let boot_s = u32::try_from((2 * HOUR_MS - 60_000) / 1000).unwrap();
    assert!(totals.counts.uptime_s >= boot_s * boots, "{totals:?}");
}

/// A boot counts the reset its flags name, set before its first pass.
#[test]
fn a_boot_counts_the_reset_its_flags_name() {
    let (flash, banks) = (Flash::memory(), Banks::new());
    let mut sim = open(&flash, &banks, 1);
    sim.platform().set_reset_flags(WATCHDOG);

    let totals = totals(&mut sim, &mut 0);
    assert_eq!(totals.resets.watchdog, 1);
    assert_eq!(totals.resets.power_on, 0);
}

/// A finished feed is saved soon after it ends, so a power cut minutes
/// later, long before the hourly save, keeps it.
#[test]
fn a_feed_is_kept_through_a_power_cut_before_the_hourly_save() {
    let (flash, banks) = (Flash::memory(), Banks::new());
    let mut sim = open(&flash, &banks, 1);
    let mut now = 0;
    run_loop(&mut sim, &mut now, 1_000, |_, _| {});

    // The owner unlocks the panel and presses Dispense.
    let lock = Buttons::of(Button::Lock);
    sim.gesture(Gesture::Down { chord: lock });
    sim.gesture(Gesture::Hold {
        chord: lock,
        level: 1,
    });
    sim.gesture(Gesture::Up {
        chord: lock,
        level: 1,
        held_ms: 5_000,
    });
    run_loop(&mut sim, &mut now, 100, |_, _| {});
    let dispense = Buttons::of(Button::Dispense);
    sim.gesture(Gesture::Down { chord: dispense });
    sim.gesture(Gesture::Up {
        chord: dispense,
        level: 0,
        held_ms: 80,
    });
    run_loop(&mut sim, &mut now, 60_000, |_, _| {});

    // Power is lost: no restart's save.
    banks.borrow_mut().power_cut();
    let mut sim = open(&flash, &banks, 2);
    assert_eq!(totals(&mut sim, &mut 0).counts.feeds, 1);
}
