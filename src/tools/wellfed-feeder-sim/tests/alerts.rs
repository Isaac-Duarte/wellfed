//! The alert rules on the simulator's loop, in virtual time: what they tell
//! from a feeder's own events and stores, across restarts, and that a meal
//! runs exactly as it would without them.

use std::cell::RefCell;
use std::rc::Rc;

use wellfed_core::buttons::{Button, Buttons, Gesture};
use wellfed_core::clock::wall::{Fix, Origin, Sync};
use wellfed_core::feed::controller::FeedReason as CoreReason;
use wellfed_core::schedule::{PLANS, Plan, Table, Weekdays};
use wellfed_device::store::events::Read;
use wellfed_device_api::mailbox::{MealSkip, PlansChange, PlansResult};
use wellfed_feeder::feeder::alert_stage::{BOOT_READ, BOOT_READ_MS, RING_WAIT_MS, WELCOME_MS};
use wellfed_feeder::feeder::platform::EventSlot;
use wellfed_feeder::feeder::{CLAIM_WAIT_MS, Platform};
use wellfed_feeder_sim::banks::Banks;
use wellfed_feeder_sim::flash::Flash;
use wellfed_feeder_sim::sim::Sim;
use wellfed_proto::api::{Alert, AlertKind, EventBody, FeedOutcome};

const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;
/// 2026-10-28T00:00:00Z.
const WEDNESDAY: u64 = 1_793_145_600_000;
/// `BOOT_Reason`'s watchdog bit.
const WATCHDOG: u32 = 1 << 17;

/// A simulated feeder on its own flash, and its uptime.
struct Feeder {
    flash: Rc<Flash>,
    banks: Rc<RefCell<Banks>>,
    sim: Sim,
    now: u32,
    boots: u32,
    rules: bool,
    /// The newest wall clock it has shown, which a restart forgets.
    wall: u64,
}

impl Feeder {
    fn new(rules: bool) -> Self {
        let flash = Flash::memory();
        let banks = Banks::new();
        let sim = open(&flash, &banks, 1, rules);
        Self {
            flash,
            banks,
            sim,
            now: 0,
            boots: 1,
            rules,
            wall: WEDNESDAY,
        }
    }

    /// A reset and a cold boot: RAM, the clock and the uptime start again.
    fn restart(&mut self, flags: u32) {
        self.boots += 1;
        self.sim = open(&self.flash, &self.banks, self.boots, self.rules);
        self.sim.platform().set_reset_flags(flags);
        self.now = 0;
    }

    fn sync(&mut self, unix_ms: u64) {
        let sync = Sync {
            unix_ms,
            offset_s: 0,
            next_change_s: 0,
            next_offset_s: 0,
        };
        let now = self.now;
        self.sim
            .platform()
            .time_sync(now, sync, Origin::Owner)
            .unwrap();
        self.wall = unix_ms;
    }

    /// Run the loop for `ms`, a pass at least every second, with `each`
    /// looking after every pass.
    fn run_watching(&mut self, ms: u64, mut each: impl FnMut(&mut Sim)) {
        let end = u64::from(self.now) + ms;
        while u64::from(self.now) < end {
            let delay = self.sim.pass(self.now, |_| {});
            each(&mut self.sim);
            let step = u64::from(delay.clamp(1, 1000)).min(end - u64::from(self.now));
            self.now += u32::try_from(step.max(1)).unwrap();
            self.wall += step.max(1);
        }
    }

    fn run(&mut self, ms: u64) {
        self.run_watching(ms, |_| {});
    }

    fn put(&mut self, table: Table) {
        let at_s = u32::try_from(self.wall / 1000).ok();
        let change = PlansChange::Replace { table, at_s };
        let queued = self.sim.platform().plans_change(change);
        assert!(matches!(queued, PlansResult::Current { pending: true, .. }));
        self.run(1);
    }

    /// Every record the ring holds, oldest first, with its time.
    fn events(&mut self) -> Vec<(Option<u32>, EventBody)> {
        let mut records = Vec::new();
        let mut after = None;
        while let Read::Record(record) = self.sim.platform().ring_after(after) {
            after = Some(record.seq);
            records.extend(
                wellfed_proto::decode_padded::<EventBody>(&record.body)
                    .map(|body| (record.at, body)),
            );
        }
        records
    }

    fn open_kinds(&mut self) -> Vec<AlertKind> {
        self.sim
            .platform()
            .alerts()
            .open()
            .map(|open| open.alert.kind())
            .collect()
    }

    /// A short press of `button`, for the next pass.
    fn press(&mut self, button: Button) {
        let chord = Buttons::of(button);
        self.sim.gesture(Gesture::Down { chord });
        self.sim.gesture(Gesture::Up {
            chord,
            level: 0,
            held_ms: 50,
        });
        self.run(100);
    }
}

fn open(flash: &Rc<Flash>, banks: &Rc<RefCell<Banks>>, boot: u32, rules: bool) -> Sim {
    let random = Box::new(|| Some([0x5a; 32]));
    let mut sim = Sim::open(flash, banks, None, random, 0, boot, None).unwrap();
    sim.platform().set_alerts_on(rules);
    sim
}

fn plan(minute: u16, portions: u8) -> Plan {
    Plan {
        minute,
        portions,
        days: Weekdays::EVERY_DAY,
        enabled: true,
        call: 0,
    }
}

fn table(plans: &[(usize, Plan)]) -> Table {
    let mut table: Table = [None; PLANS];
    for (slot, plan) in plans {
        table[*slot] = Some(*plan);
    }
    table
}

#[test]
fn a_failed_meal_opens_and_a_restart_reads_it_back() {
    let mut feeder = Feeder::new(true);
    feeder.sync(WEDNESDAY);
    feeder.put(table(&[(0, plan(10, 2))]));
    feeder.sim.fault_feed(CoreReason::Jammed);
    feeder.run(11 * MINUTE_MS);

    assert!(
        feeder.events().iter().any(|(_, body)| matches!(
            body,
            EventBody::Fed {
                outcome: FeedOutcome::Failed,
                ..
            }
        )),
        "the meal failed"
    );
    assert_eq!(feeder.open_kinds(), [AlertKind::MealFailed]);

    feeder.restart(0);
    assert!(!feeder.sim.platform().alerts().is_restored());
    feeder.run(1000);
    assert!(feeder.sim.platform().alerts().is_restored());
    assert_eq!(feeder.open_kinds(), [AlertKind::MealFailed]);
}

/// A meal the owner skipped never runs and is never missed, and the rules
/// open nothing for it, before a restart or after.
#[test]
fn a_skipped_meal_opens_no_meal_alert() {
    let mut feeder = Feeder::new(true);
    feeder.sync(WEDNESDAY);
    feeder.put(table(&[(0, plan(10, 2))]));
    let due_s = u32::try_from(WEDNESDAY / 1000).unwrap() + 10 * 60;
    let now = feeder.now;
    feeder.sim.skip_meal(now, 0, due_s, MealSkip::Skip).unwrap();
    feeder.run(2 * HOUR_MS);

    let PlansResult::Current { view, .. } = feeder.sim.platform().plans() else {
        panic!("the plans are loaded");
    };
    assert_eq!(view.skipped(0), Some(due_s), "the skip stands");
    let events = feeder.events();
    assert!(
        !events
            .iter()
            .any(|(_, body)| matches!(body, EventBody::Fed { .. } | EventBody::Missed { .. })),
        "{events:?}"
    );
    assert_eq!(feeder.open_kinds(), []);

    feeder.restart(0);
    let wall = feeder.wall;
    feeder.sync(wall);
    feeder.run(2 * MINUTE_MS);
    assert!(feeder.sim.platform().alerts().is_restored());
    assert_eq!(feeder.open_kinds(), []);
}

/// After a cold boot NTP sets the clock but carries no offset, and the
/// offset a client's sync gave lived in RAM. With no zone rule stored, every
/// plan holds: no meal runs and none is missed, so only `NoClock` can say so.
#[test]
fn a_clock_from_ntp_with_no_zone_opens_no_clock() {
    let mut feeder = Feeder::new(true);
    feeder.sync(WEDNESDAY);
    feeder.put(table(&[(0, plan(30, 1))]));
    feeder.restart(0);

    let now = feeder.now;
    let fix = Fix {
        unix_ms: WEDNESDAY + 5 * MINUTE_MS,
        error_ms: 25,
        origin: Origin::Ntp,
    };
    feeder.sim.platform().time_fix(now, fix).unwrap();
    feeder.run(HOUR_MS - MINUTE_MS);
    assert!(feeder.sim.platform().wall_ms(feeder.now).is_some());
    assert_eq!(feeder.open_kinds(), []);

    feeder.run(2 * MINUTE_MS);
    assert_eq!(feeder.open_kinds(), [AlertKind::NoClock]);
    let events = feeder.events();
    assert!(
        !events
            .iter()
            .any(|(_, body)| matches!(body, EventBody::Fed { .. } | EventBody::Missed { .. })),
        "the meal held: {events:?}"
    );

    // A client's sync gives local time: the condition ends, and the meal it
    // held, now past its grace, is reported missed.
    let wall = feeder.sim.platform().wall_ms(feeder.now).unwrap();
    feeder.sync(wall);
    feeder.run(2 * MINUTE_MS);
    assert_eq!(feeder.open_kinds(), [AlertKind::MealMissed]);
}

#[test]
fn a_good_meal_opens_nothing() {
    let mut feeder = Feeder::new(true);
    feeder.sync(WEDNESDAY);
    feeder.put(table(&[(0, plan(10, 2))]));
    feeder.run(11 * MINUTE_MS);

    assert_eq!(feeder.open_kinds(), []);
}

/// A clock that jumps past a meal leaves the schedule a moment to pass it.
/// The rules must wait for its word, not call the meal unrecorded meanwhile.
#[test]
fn a_clock_jumping_past_a_meal_reports_it_missed_and_never_as_no_record() {
    let mut feeder = Feeder::new(true);
    feeder.sync(WEDNESDAY);
    feeder.put(table(&[(0, plan(600, 1))]));
    feeder.run(2000);

    feeder.sync(WEDNESDAY + 11 * HOUR_MS);
    let mut no_record = false;
    feeder.run_watching(5 * MINUTE_MS, |sim| {
        no_record |= sim
            .platform()
            .alerts()
            .open()
            .any(|open| open.alert.kind() == AlertKind::MealNoRecord);
    });

    assert!(!no_record, "the schedule's record was on its way");
    assert_eq!(feeder.open_kinds(), [AlertKind::MealMissed]);
}

/// Each boot counts once, however its `Boot` reaches the rules: two more
/// boots in the ring and this one are three, and two are not.
#[test]
fn this_boots_boot_reaches_the_rules_exactly_once() {
    let mut feeder = Feeder::new(true);
    feeder.sim.platform().set_reset_flags(WATCHDOG);
    feeder.run(2000);
    assert_eq!(feeder.open_kinds(), []);

    feeder.restart(WATCHDOG);
    feeder.run(2000);
    assert_eq!(feeder.open_kinds(), [], "two boots are not three");
    assert_eq!(
        feeder
            .events()
            .iter()
            .filter(|(_, body)| matches!(body, EventBody::Boot { .. }))
            .count(),
        2
    );

    feeder.restart(WATCHDOG);
    feeder.run(2000);
    assert_eq!(feeder.open_kinds(), [AlertKind::RepeatedRestarts]);
}

/// A claim the writer never finishes holds every feed. Nothing else would say
/// so, as a meal is neither run nor missed while it is in hand.
#[test]
fn a_claim_the_writer_never_finishes_opens_cannot_feed_past_its_wait() {
    let mut feeder = Feeder::new(true);
    feeder.sync(WEDNESDAY);
    feeder.put(table(&[(0, plan(10, 2))]));
    feeder.run(9 * MINUTE_MS);
    feeder.sim.platform().hold_plans(true);

    feeder.run(61_000 + u64::from(CLAIM_WAIT_MS));
    let cannot_feed = feeder
        .sim
        .platform()
        .alerts()
        .open()
        .find(|open| open.alert.kind() == AlertKind::CannotFeed)
        .map(|open| open.alert);
    assert!(
        matches!(cannot_feed, Some(Alert::CannotFeed { .. })),
        "{:?}",
        feeder.open_kinds()
    );

    feeder.sim.platform().hold_plans(false);
    feeder.run(3 * MINUTE_MS);
    assert!(!feeder.open_kinds().contains(&AlertKind::CannotFeed));
}

/// A client's change that never finishes: `pending` for good, so no meal can
/// be judged and the schedule decides nothing.
#[test]
fn a_change_to_the_plans_that_never_finishes_opens_plans_stuck() {
    let mut feeder = Feeder::new(true);
    feeder.sync(WEDNESDAY);
    feeder.run(2000);
    feeder.sim.platform().hold_plans(true);
    let at_s = u32::try_from(feeder.wall / 1000).ok();
    let queued = feeder.sim.platform().plans_change(PlansChange::Replace {
        table: table(&[(0, plan(600, 1))]),
        at_s,
    });
    assert!(matches!(queued, PlansResult::Current { pending: true, .. }));

    feeder.run(24 * MINUTE_MS);
    assert!(!feeder.open_kinds().contains(&AlertKind::PlansStuck));
    feeder.run(2 * MINUTE_MS);
    assert_eq!(feeder.open_kinds(), [AlertKind::PlansStuck]);

    feeder.sim.platform().hold_plans(false);
    feeder.run(2000);
    assert_eq!(feeder.open_kinds(), []);
}

/// A store that never loads says `Busy` for good, and readiness takes nothing
/// from the plans, so only this says so.
#[test]
fn plans_that_never_load_open_plans_stuck_and_readiness_is_unchanged() {
    let mut feeder = Feeder::new(true);
    feeder.sync(WEDNESDAY);
    feeder.run(2000);
    feeder.sim.platform().hold_plans_loading(true);

    feeder.run(26 * MINUTE_MS);
    assert_eq!(feeder.open_kinds(), [AlertKind::PlansStuck]);

    feeder.sim.platform().hold_plans_loading(false);
    feeder.run(2000);
    assert_eq!(feeder.open_kinds(), []);
}

/// Bank 1 welcomes no session while the boot read has its first seconds.
/// Bank 0, the recovery bank, is there for installs: an installer
/// reconnecting after a give-way is welcomed at once, the read still running.
#[test]
fn only_bank_1_holds_sessions_for_the_boot_read() {
    for (bank, held) in [(1, true), (0, false)] {
        let mut feeder = Feeder::new(true);
        feeder.run(2000);
        if bank == 0 {
            feeder.banks.borrow_mut().stock_gave_way();
        }
        feeder.restart(0);
        feeder.sim.platform().hold_ring_reads(true);
        feeder.run(1000);

        assert_eq!(feeder.sim.platform().boot().bank, bank);
        assert!(!feeder.sim.platform().alerts().is_restored());
        let now = feeder.now;
        let welcomed = feeder.sim.welcome_epoch(now).is_some();
        assert_eq!(welcomed, !held, "bank {bank}");

        feeder.sim.platform().hold_ring_reads(false);
        feeder.run(2000);
        assert!(feeder.sim.platform().alerts().is_restored(), "bank {bank}");
        let now = feeder.now;
        assert!(feeder.sim.welcome_epoch(now).is_some(), "bank {bank}");
    }
}

/// The read's bound starts at the ring's opening, and the welcome's at the
/// boot: a late ring keeps its whole read, and holds no session longer.
#[test]
fn a_ring_that_opens_late_keeps_its_read_and_holds_no_session_longer() {
    let mut feeder = Feeder::new(true);
    feeder.run(2000);
    feeder.restart(0);
    feeder.sim.platform().hold_ring_closed(true);

    feeder.run(u64::from(WELCOME_MS) + 1000);
    assert!(feeder.sim.platform().alerts().welcoming());
    assert!(!feeder.sim.platform().alerts().is_restored());

    feeder.sim.platform().hold_ring_reads(true);
    feeder.sim.platform().hold_ring_closed(false);
    feeder.run(u64::from(BOOT_READ_MS) - 2000);
    assert!(!feeder.sim.platform().alerts().is_restored());
    feeder.sim.platform().hold_ring_reads(false);
    feeder.run(2000);
    assert!(feeder.sim.platform().alerts().is_restored());
    assert!(!feeder.sim.platform().alerts().more(), "it read all of it");
}

#[test]
fn a_read_the_writer_never_answers_gives_up_and_says_so() {
    let mut feeder = Feeder::new(true);
    feeder.run(2000);
    feeder.restart(0);
    feeder.sim.platform().hold_ring_reads(true);

    feeder.run(u64::from(BOOT_READ_MS) - 1000);
    assert!(!feeder.sim.platform().alerts().is_restored());
    assert!(!feeder.sim.platform().alerts().welcoming());
    feeder.run(2000);
    assert!(feeder.sim.platform().alerts().is_restored());
    assert!(feeder.sim.platform().alerts().welcoming());
    assert!(feeder.sim.platform().alerts().more());
}

#[test]
fn a_ring_that_never_opens_does_not_stop_the_rules() {
    let mut feeder = Feeder::new(true);
    feeder.run(2000);
    feeder.restart(0);
    feeder.sim.platform().hold_ring_closed(true);

    feeder.run(u64::from(RING_WAIT_MS) + 1000);
    assert!(feeder.sim.platform().alerts().is_restored());
    assert!(feeder.sim.platform().alerts().more());
}

/// A ring past the read's reach: the read is cut, and the table says so.
#[test]
fn a_ring_past_the_reads_reach_keeps_more_set() {
    let mut feeder = Feeder::new(true);
    feeder.sync(WEDNESDAY);
    feeder.run(2000);
    for _ in 0..BOOT_READ {
        feeder
            .sim
            .platform()
            .queue_event(None, [0xff; 16], EventSlot::Shared);
    }
    feeder.run(1000);

    feeder.restart(0);
    feeder.run(2000);
    assert!(feeder.sim.platform().alerts().is_restored());
    assert!(feeder.sim.platform().outside_reads() > 0);
    assert!(feeder.sim.platform().alerts().more());
}

/// The same scripted days of meals, presses, faults, an emptied hopper and
/// restarts, from the same seed, with the rules and without.
fn script(seed: u64, rules: bool) -> (Vec<(Option<u32>, EventBody)>, Vec<AlertKind>) {
    let mut random = seed;
    let mut next = move |below: u64| {
        // xorshift64: nothing here needs more than a repeatable stream.
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        random % below
    };

    let mut feeder = Feeder::new(rules);
    let mut wall = WEDNESDAY;
    feeder.sync(wall);
    let minutes =
        [10 + next(50), 90 + next(60), 200 + next(120)].map(|m| u16::try_from(m).unwrap());
    feeder.put(table(&[
        (0, plan(minutes[0], 2)),
        (3, plan(minutes[1], 1)),
        (7, plan(minutes[2], 3)),
    ]));

    // The panel starts locked.
    let lock = Buttons::of(Button::Lock);
    feeder.sim.gesture(Gesture::Down { chord: lock });
    feeder.sim.gesture(Gesture::Hold {
        chord: lock,
        level: 1,
    });
    feeder.sim.gesture(Gesture::Up {
        chord: lock,
        level: 1,
        held_ms: 3000,
    });
    feeder.run(100);

    for _ in 0..24 {
        match next(8) {
            0 => feeder.press(Button::Dispense),
            1 => feeder.sim.fault_feed(CoreReason::Jammed),
            2 => feeder.sim.fill_hopper(false),
            3 => feeder.sim.fill_hopper(true),
            4 => {
                wall = feeder.wall;
                feeder.restart(WATCHDOG);
                feeder.sync(wall + next(10 * MINUTE_MS));
            }
            _ => {}
        }
        feeder.run(5 * MINUTE_MS + next(25 * MINUTE_MS));
    }

    let events = feeder.events();
    let kinds = feeder.open_kinds();
    (events, kinds)
}

/// The differential: nothing the feeder records, the `Fed` events first of
/// all, differs for the rules being there. No client takes part, as a script
/// of this kind drives the loop by its own passes, with no session.
#[test]
fn the_same_script_records_the_same_history_with_the_rules_and_without() {
    let mut opened = 0;
    let mut fed = 0;
    for seed in [0x1234_5678, 0xdead_beef, 0x0bad_cafe, 0x5eed_0004] {
        let (with_rules, alerts) = script(seed, true);
        let (without, none) = script(seed, false);

        let fed_in = |events: &[(Option<u32>, EventBody)]| {
            events
                .iter()
                .filter(|(_, body)| matches!(body, EventBody::Fed { .. }))
                .copied()
                .collect::<Vec<_>>()
        };
        assert_eq!(fed_in(&with_rules), fed_in(&without), "seed {seed:#x}");
        assert_eq!(with_rules, without, "seed {seed:#x}");
        assert_eq!(none, [], "the control runs no rules");

        fed += fed_in(&with_rules).len();
        opened += alerts.len();
    }

    assert!(fed >= 8, "the script fed: {fed}");
    assert!(opened >= 1, "the rules had something to say: {opened}");
}
